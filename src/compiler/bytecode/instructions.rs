//! Bytecode instructions, operands, and instruction stream relocation.
use super::*;

/// Which binary operator an [`Insn::BinOpConst`] applies to its register
/// operand and its embedded constant.
///
/// Deliberately tiny and closed: one fused variant covering the three
/// constant-fed ALU ops the census actually shows (`Add`, `Eq`, `CaseEq`)
/// keeps the `Insn` enum — and the ~25 analysis sites that match on it — with
/// a single new case to reason about instead of three.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AddC2 {
    pub d1: RegId,
    pub s1: RegId,
    pub k1: Value,
    pub d2: RegId,
    pub s2: RegId,
    pub k2: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CmpKind {
    Eq,
    Neq,
    CaseEq,
    Lt,
    Leq,
    Gt,
    Geq,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[repr(u8)]
pub enum BinOpConstKind {
    /// `dst = src + K` — same `Value` semantics as [`Insn::Add`].
    Add = 0,
    /// `dst = (src == K)` — same `Value` semantics as [`Insn::Eq`].
    Eq = 1,
    /// `dst = (src === K)` — same `Value` semantics as [`Insn::CaseEq`].
    CaseEq = 2,
    /// `dst = src ^ K` — same `Value` semantics as [`Insn::BitXor`].
    Xor = 3,
    /// `dst = src & K` — same `Value` semantics as [`Insn::BitAnd`].
    And = 4,
    /// `dst = src | K` — same `Value` semantics as [`Insn::BitOr`].
    Or = 5,
    /// `dst = src * K` — same `Value` semantics as [`Insn::Mul`].
    Mul = 6,
    /// `dst = src - K` — same `Value` semantics as [`Insn::Sub`].
    Sub = 7,
}

impl BinOpConstKind {
    /// Number of kinds; sizes the static fusion-count array.
    pub const COUNT: usize = 8;
}

/// Bytecode instruction set. Stack-free, register-based design.
/// Each instruction specifies source and destination registers explicitly,
/// enabling the VM to iterate a flat Vec<Insn> with predictable memory access.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CaseJumpData {
    /// Arm entry pc per selector value; holes hold `default`.
    pub table: Vec<u32>,
    /// Target when the selector has x/z bits or is outside the table.
    pub default: u32,
}

/// One parsed piece of a `$sformatf` template — literal text or a single
/// `%` conversion. Parsed ONCE at compile time; `Insn::Format` fills it from
/// register Values with the same value-level cores the AST formatter uses,
/// so the output is byte-identical for the supported specs.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum FmtSeg {
    Lit(String),
    /// `width`: None = spec's default width, Some(0) = minimal (`%0d`),
    /// Some(n) = explicit field width. `str_valued` applies to `%s` only:
    /// render the packed bytes as text (string variable/literal) rather
    /// than the §21.2.1.3 packed-operand byte dump.
    Spec {
        spec: char,
        width: Option<u32>,
        left: bool,
        str_valued: bool,
    },
}

/// Native string operation. Semantics mirror the AST interpreter's
/// implementations byte-for-byte (§6.16); each op reads its operands from
/// registers and writes a fresh Value to the destination — the two mutator
/// shapes (PutC, the *toa family) return the MODIFIED string and the
/// compiler emits the store-back separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum StrOpKind {
    Len,      // [s]        -> 32-bit byte count
    GetC,     // [s, i]     -> 8-bit byte, 0 out of range
    Substr,   // [s, a, b]  -> inclusive byte slice, "" out of range
    ToUpper,  // [s]
    ToLower,  // [s]
    Compare,  // [a, b]     -> signed 32 strcmp difference
    ICompare, // [a, b]     -> case-folded strcmp
    AToI,     // [s]        -> longest radix-10 prefix
    AToHex,   // [s]
    AToOct,   // [s]
    AToBin,   // [s]
    Concat,   // [parts...] -> joined bytes (§11.4.12 string concat)
    PutC,     // [s, i, c]  -> s with byte i replaced (unchanged if OOB/NUL)
    IToA,     // [v]        -> signed decimal text
    HexToA,   // [v]
    OctToA,   // [v]
    BinToA,   // [v]
    /// Unsigned byte ordering for relational operators (§6.16 Table 6-9).
    Order,
    /// [v] -> the operand converted to a `string` value: its bytes with
    /// every NUL removed (§6.16 — `string'(24'hab0063)` is 2 characters,
    /// not 3). Appended last so cached bytecode keeps its variant indices.
    ToStr,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FormatData {
    pub segs: Vec<FmtSeg>,
    pub args: Vec<RegId>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CaseMaskJumpData {
    /// Contiguous selector bit window `[lo, lo+width)` in which every
    /// pattern is fully defined; the dispatch index is those bits.
    pub lo: u32,
    pub width: u32,
    /// Bucket-chain entry pc per window value (`1 << width` entries).
    pub table: Vec<u32>,
    /// Full sequential compare chain, taken when the selector's window has
    /// any x/z bit (a wildcard selector can match several buckets).
    pub xz_path: u32,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CaseLutData {
    pub table: Vec<Value>,
    pub default: Value,
}

/// How a register-backed local is typed, for handing it to the AST
/// interpreter (`FbLocal`). Only shapes whose interpreter view is fully
/// described by the `Value` itself are listed: a plain integral vector with
/// a `[N-1:0]` range (or none), a real, a string. Anything else — structs,
/// enums, unresolved aliases, multi-dimensional or offset ranges — has metadata the
/// interpreter keeps by NAME, which a compiled local never registered, so a
/// fallback that names one is refused and the enclosing unit stays on the
/// interpreter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LocalKind {
    /// Integral, declared width and signedness (§6.11, §6.8), and whether
    /// the type is 2-state (§6.11.1: a write drops X/Z to 0).
    Int {
        width: u32,
        signed: bool,
        two_state: bool,
    },
    Real,
    Str,
    /// A class handle (method mode only): carried as its value; the
    /// interpreter types it by name from the activation's recorded metadata.
    Handle,
}

/// One register-backed local an AST fallback reads or writes. The executor
/// binds `name` to the register's value in a fresh interpreter frame around
/// the fallback and copies the (re-typed) value back afterwards.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FbLocal {
    pub name: Arc<str>,
    pub reg: RegId,
    pub kind: LocalKind,
    /// A compiled method's output / inout / ref formal: the call bound the
    /// name to the caller's actual as well, so the executor sets that
    /// redirection aside while the carried frame holds the value (the
    /// method epilogue writes the register back).
    #[serde(default)]
    pub via_binding: bool,
    /// A class handle declared in a nested block: its declared class, which
    /// the carried frame records (the activation records only formals and
    /// top-level declarations).
    #[serde(default)]
    pub class: Option<Arc<str>>,
}

/// The locals a fallback carries; empty for the ordinary fallback, which
/// sees no register at all.
pub type FbLocals = Box<[FbLocal]>;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Insn {
    /// Load a constant value into a register. `Box<Value>` keeps the
    /// variant small (8 B instead of 32 B for the inline Value) — LoadConst
    /// isn't on the hot dispatch path so the extra indirection is cheap
    /// and the 24 B saving compounds with the u32 signal_id fields below
    /// to shrink `Insn` from 40 B to 32 B.
    LoadConst(RegId, Box<Value>),
    /// Load a signal from signal_table[signal_id] into a register.
    LoadSignal(RegId, SigId), // (dest_reg, signal_id)
    /// Load a signal and mark it as signed.
    LoadSignalSigned(RegId, SigId),
    /// Load a value from the active process's innermost local frame.
    LoadProcessLocal(RegId, Box<str>),
    /// §9.7: Query the status of a process from its handle.
    LoadProcessStatus(RegId, RegId),
    /// Resize register to given width.
    Resize(RegId, u32),

    // Binary arithmetic/logic: dest = left op right
    Add(RegId, RegId, RegId),
    Sub(RegId, RegId, RegId),
    Mul(RegId, RegId, RegId),
    Div(RegId, RegId, RegId),
    Mod(RegId, RegId, RegId),
    BitAnd(RegId, RegId, RegId),
    BitOr(RegId, RegId, RegId),
    BitXor(RegId, RegId, RegId),
    BitXnor(RegId, RegId, RegId),
    LogAnd(RegId, RegId, RegId),
    LogOr(RegId, RegId, RegId),
    Eq(RegId, RegId, RegId),
    Neq(RegId, RegId, RegId),
    CaseEq(RegId, RegId, RegId),
    /// Constant-selector, constant-result `case` lowered to a table:
    /// `dst = table[src]`, `default` when `src` has x/z bits or is out of
    /// range. Built only for a plain `case` whose arms all assign constants
    /// to one variable — an S-box style decode ROM. Replaces a compare-and-
    /// branch chain of two insns per PATTERN per execution (~500 for AES's
    /// 256-entry S-box) with one.
    CaseLut(RegId, RegId, Box<CaseLutData>),
    /// Computed-goto `case`: constant, fully-defined patterns dispatch in one
    /// step instead of a compare-and-branch pair per arm. Bodies are ordinary
    /// statements (unlike `CaseLut`, which needs constant results), so this
    /// covers the CSR-read mux / decoder shape — dozens of arms evaluated
    /// every cycle, previously ~2 executed insns per SKIPPED arm.
    CaseJump(RegId, Box<CaseJumpData>),
    /// Two-level dispatch for dense `casez`/`casex` with constant wildcard
    /// patterns (the RISC-V tracer/decoder shape): jump-table on a window of
    /// always-defined bits, then a short residual compare chain per bucket.
    /// Replaces an average of half the arms' CasezEq+branch pairs per
    /// execution with one table hop plus ~1-3 compares.
    CaseMaskJump(RegId, Box<CaseMaskJumpData>),
    /// Fused compare + `BranchIfFalse`: branch to the target when the
    /// comparison is NOT true (X/Z compares are not-true, exactly like the
    /// unfused pair). `tmp` is the dead register the eliminated compare
    /// wrote — kept so the two-state lowering and the native backends can
    /// decompose to the already-validated unfused forms.
    CmpBranch(CmpKind, RegId, RegId, RegId, u32), // (kind, l, r, tmp, target)
    /// Fused `Move` + `Resize` (either order, dead intermediate):
    /// `dst = resize(src, w)`.
    MoveResize(RegId, RegId, u32), // (dst, src, width)
    /// `$sformatf` with a literal format string and supported specs,
    /// template-parsed at compile time and filled natively. Removes the
    /// whole-call AST fallback that made every tracer decode helper cost
    /// ~24µs per retired instruction.
    Format(RegId, Box<FormatData>),
    /// dst = op(args) over string Values — see `StrOpKind`.
    StrOp(RegId, StrOpKind, Box<Vec<RegId>>),
    /// Store to a `string` signal (§6.16): the value keeps its own width —
    /// the table width is a placeholder, and `BlockingAssign`'s resize
    /// would truncate the FRONT of the text.
    BlockingAssignString(SigId, RegId),
    CasezEq(RegId, RegId, RegId),
    CasexEq(RegId, RegId, RegId),
    Lt(RegId, RegId, RegId),
    Leq(RegId, RegId, RegId),
    Gt(RegId, RegId, RegId),
    Geq(RegId, RegId, RegId),
    Shl(RegId, RegId, RegId),
    Shr(RegId, RegId, RegId),
    AShr(RegId, RegId, RegId),

    // Unary: dest = op src
    BitNot(RegId, RegId),
    LogNot(RegId, RegId),
    Negate(RegId, RegId),
    ReduceAnd(RegId, RegId),
    ReduceOr(RegId, RegId),
    ReduceXor(RegId, RegId),

    /// Bit select: dest = src[index]
    BitSelect(RegId, RegId, RegId), // (dest, base, index)
    /// Bit select with compile-time constant index.
    BitSelectConst(RegId, RegId, u32), // (dest, base, index)
    /// Range select: dest = src[left:right]
    RangeSelect(RegId, RegId, RegId, RegId), // (dest, base, left, right)
    /// Indexed part-select `base[idx +: w]` (up) / `base[idx -: w]` (down)
    /// with a constant width: the width survives an x/z index, which the
    /// two-register form loses (§11.5.1: unknown index → whole-x result of
    /// the select's width). (dest, base, idx, width, up)
    RangeSelectW(RegId, RegId, RegId, u32, bool),
    /// Range select with compile-time constant bounds.
    RangeSelectConst(RegId, RegId, u32, u32), // (dest, base, left, right)
    /// Concatenation: dest = {parts...}, part register IDs stored in
    /// the boxed Vec. The `Box` keeps the variant at 16 B (Box ptr only)
    /// instead of inlining a 24 B Vec header — Concat is rare on the
    /// hot path so the extra indirection is cheap, and shrinking this
    /// variant lets the whole `Insn` enum drop from 32 B to 24 B.
    Concat(RegId, Box<Vec<RegId>>),
    /// Replicate: dest = {count{src}}
    Replicate(RegId, RegId, u32),

    /// Conditional branch: if reg is false, jump to target instruction index.
    BranchIfFalse(RegId, u32), // (cond_reg, jump_target)
    /// 4-state select: dest = cond ? then_reg : else_reg, with per-bit X merge
    /// (IEEE 1800 §11.4.11 Table 11-21) when cond has unknown bits. Both
    /// branches are always evaluated (no short-circuit) — used for `?:` so
    /// X conditions don't silently fall through to the false branch.
    Select(RegId, RegId, RegId, RegId), // (dest, cond, then, else)
    /// Unconditional jump.
    Jump(u32),

    /// Non-blocking assign: signal_table[id] <= reg (scheduled via NBA queue).
    NbaAssign(SigId, RegId, u32), // (signal_id, value_reg, width)
    /// Non-blocking partial assign: signal_table[id][hi:lo] <= reg.
    /// Read-modify-write at exec time using current signal value as base.
    NbaAssignRange(SigId, u32, u32, RegId), // (signal_id, hi, lo, value_reg)
    /// NBA partial assign with dynamic hi/lo (mirrors `BlockingAssignRangeDyn`):
    /// signal_table[id][hi_reg:lo_reg] <= reg. Lets us compile NBAs with
    /// run-time bit ranges (e.g. `q[idx +: W]`, `q[j:j-W+1]`) instead of
    /// falling back to the AST interpreter — critical on CPUs like c910
    /// where these patterns fire millions of times per simulation.
    NbaAssignRangeDyn(SigId, RegId, RegId, RegId), // (signal_id, hi_reg, lo_reg, value_reg)
    /// Non-blocking bit assign: signal_table[id][bit_idx_reg] <= reg.
    NbaAssignBitDyn(SigId, RegId, RegId), // (signal_id, idx_reg, value_reg)
    /// Blocking assign: signal_table[id] = reg.
    BlockingAssign(SigId, RegId, u32), // (signal_id, value_reg, width)
    /// Blocking range assign: signal_table[id][hi:lo] = reg (read-modify-write).
    BlockingAssignRange(SigId, u32, u32, RegId), // (signal_id, hi, lo, value_reg)
    /// Blocking range assign with dynamic hi/lo (for `[idx +: W]` / `[idx -: W]`).
    BlockingAssignRangeDyn(SigId, RegId, RegId, RegId), // (signal_id, hi_reg, lo_reg, value_reg)
    /// Blocking bit assign: signal_table[id][idx_reg] = reg[0] (read-modify-write).
    BlockingAssignBitDyn(SigId, RegId, RegId), // (signal_id, idx_reg, value_reg)

    /// Load array element: dest = signal_table[array_base + eval(index_reg)]
    /// Boxing the operand keeps the instruction compact.
    LoadArrayElem(RegId, Box<ArrayOperand>, RegId), // (dest, array, index_reg)
    /// NBA assign to array element.
    NbaAssignArray(Box<ArrayOperand>, RegId, RegId, u32), // (array, index_reg, value_reg, width)
    /// Blocking assign to array element.
    BlockingAssignArray(Box<ArrayOperand>, RegId, RegId, u32), // (array, index_reg, value_reg, width)
    /// NBA range assign to array element.
    NbaAssignArrayRange(Box<ArrayOperand>, RegId, RegId, RegId, RegId), // (array, index_reg, hi_reg, lo_reg, value_reg)
    /// Blocking range assign to array element.
    BlockingAssignArrayRange(Box<ArrayOperand>, RegId, RegId, RegId, RegId), // (array, index_reg, hi_reg, lo_reg, value_reg)

    /// Marks end of a compiled block (no-op, helps debugging).
    /// Copy src register to dest register.
    Move(RegId, RegId), // (dest, src)

    /// Fallback: invoke the AST interpreter on an untranslated statement.
    /// Used for rare constructs (e.g. $display, complex LHS) so an edge
    /// block containing one unsupported stmt can still run most of its
    /// body as fast bytecode instead of falling back wholesale to AST.
    /// Boxed payload keeps the variant at 8 B (Box ptr) instead of
    /// 24 B (Arc + fat-ptr str). StmtFallback is the AST-interpreter
    /// escape hatch — its dispatch cost dwarfs an extra deref.
    /// The third field lists the register-backed locals the statement
    /// names (see `FbLocal`); empty when none are live.
    StmtFallback(Box<(Arc<Statement>, Arc<str>, FbLocals)>),
    /// class-perf: a function-local QUEUE / unsized dynamic-array declaration
    /// with NO initializer (`chandle cached[$];`). The old emission routed
    /// this through `StmtFallback("VarDecl_local_coll")`, which tripped the
    /// fast-call admission gate (`has_forbidden`) and forced the WHOLE
    /// enclosing body onto the AST interpreter — e.g. `uvm_re_match` was
    /// never fast-called. This dedicated opcode declares the empty collection
    /// through `exec_stmt_var_decl` (the exact interpreter registration,
    /// guaranteed byte-identical) WITHOUT being classified as a fallback, so
    /// the enclosing block stays frame-free-eligible. The payload mirrors the
    /// StmtFallback shape: (synthetic VarDecl statement, reason, local regs).
    InitLocalColl(Box<(Arc<Statement>, Arc<str>)>),
    /// Expression-level AST escape hatch: interpret ONE sub-expression the
    /// compiler can't handle (unresolvable ident, member access, impure
    /// call, ...) into a register, keeping the REST of the statement
    /// compiled. (RegId dest, ctx width for §11.8.1 sizing.) The
    /// interpreter cannot see VM registers, so the register-backed locals
    /// the expression names travel in the payload (see `FbLocal`).
    EvalExprFallback(Box<(Arc<Expression>, Arc<str>, FbLocals)>, RegId, u32),

    SetSigned(RegId),
    /// §11.8.1: the enclosing expression is UNSIGNED (some operand is
    /// unsigned), so this operand must ZERO-extend at the coming Resize —
    /// clear the runtime signed flag the load stamped on it.
    ClearSigned(RegId),
    /// §6.11.1/§10.7: the register is a 2-state local just written — map
    /// its X/Z bits to 0. Emitted only after a store to a register-backed
    /// local of a 2-state type (see `emit_local_fit`).
    DropXz(RegId),
    /// §6.12.1/§6.24.1: a real in the register is rounded to a 64-bit
    /// integer (ties away from zero); any other value is left as it is.
    /// Emitted before the `Resize` that fits a value which may be real to
    /// a 64-bit integral type: real storage is 64 bits wide, so that
    /// `Resize` alone would keep the real and its IEEE 754 bits.
    RealToInt(RegId),
    /// §11.4.3 `**` with a non-constant base: left operand pre-resized to the
    /// operation width by the compiler; result width = left's width.
    Pow(RegId, RegId, RegId),
    /// Read a field of the object whose handle is in a register off the object
    /// heap: `dest = heap[handle].properties[field]`. Class methods (not the
    /// signal namespace) resolve `this.<field>` to this. `class-perf` tier-1
    /// opcode; always rendered as a bytecode op but only reachable from the
    /// (gated) method-body compile path.
    LoadClassMember(RegId, RegId, Box<str>), // (dest, this_handle_reg, field)
    /// Write a field of the object whose handle is in a register: `heap[handle].properties[field] = value`.
    StoreClassMember(RegId, RegId, Box<str>), // (this_handle_reg, value_reg, field)
    /// class-perf tier-2: call a method on the object whose handle is in
    /// `handle_reg`, passing the `n_args` actuals already computed into the
    /// contiguous registers `arg_start..arg_start+n_args`. Writes the scalar
    /// return into `dest`. At runtime the receiver + args are re-entered into
    /// the interpreter's `exec_method_call` dispatcher, so the callee keeps
    /// full AST semantics (virtual dispatch, defaults, output/ref/string);
    /// `class-perf` bridges a compiled caller's method call back to the
    /// battle-tested interpreter — correctness pilot, not the end-state.
    CallMethod(RegId, RegId, Box<Name>, RegId, u32), // (dest, handle_reg, method, arg_start, n_args)

    /// class-perf Step 9f: STATIC class-method dispatch with the
    /// resolution-chain START CLASS baked at compile time. Two surfaces:
    /// `super.m(args)` / `super.new(args)` (§8.15 — start = the parent of
    /// the DEFINING class, non-virtual) and a BARE call `m(args)` to a
    /// NON-VIRTUAL method (§8.20 — start = the first class from the
    /// lexical chain defining `m`, mirroring the AST fallback's
    /// `nonvirtual_target_class`). The receiver is always `this`
    /// (`this_reg`); the executor re-enters
    /// `exec_method_in_class_hierarchy(handle, start_class, m, args)`, so
    /// ctor chaining, defaults, and everything callee-side stay
    /// interpreter-owned. (dest, this_reg, start_class, method, arg_start,
    /// n_args)
    CallScopedMethod(RegId, RegId, Box<NamePair>, RegId, u32),
    /// class-perf P2: call a FREE FUNCTION (module/package scope) from a
    /// compiled class method — dest = f(regs[a..a+n]). The body executes
    /// via the interpreter's `exec_function_call` (identical semantics to
    /// the AST path); only positional `input`-only formals are admitted.
    CallFreeFunction(RegId, Box<Name>, RegId, u32),

    /// class-perf Step 9g: read a STATIC class property (§8.19) by its
    /// bare name — the resolution walk (and per-specialization keying)
    /// happens at runtime in `class_static_get`, exactly like the AST's
    /// unqualified-static fallback which starts at the LEXICAL class
    /// (`class_context_stack`). `class` is the defining class of the
    /// compiled method. (dest, class, prop)
    LoadClassStatic(RegId, Box<NamePair>),
    /// class-perf Step 9g: write a STATIC class property by bare name —
    /// `class_static_set` stores the Value UNRESIZED (the AST path never
    /// resizes a static store either). (class, prop, src_reg)
    StoreClassStatic(Box<NamePair>, RegId),
    /// class-perf Step 9g: `new(args)` / bare `new` as an EXPRESSION
    /// (§8.8) inside a method — constructs an instance of the DEFINING
    /// class of the compiled method and writes the fresh handle to `dest`.
    /// Allocation + the whole ctor chain stay interpreter-owned
    /// (`instantiate_class`). (dest, class, arg_start, n_args)
    ConstructObject(RegId, Box<Name>, RegId, u32),
    /// class-perf Step 9h: a CLASS-SCOPE static call `Cls.m(args)` /
    /// `alias.m(args)` — the receiver names a class, a typedef alias, or a
    /// type parameter (admitted at the compile site). The RAW receiver name
    /// is baked here; the executor re-enters the interpreter's shared
    /// `exec_class_scope_static_call`, so specialization derivation and
    /// per-spec static seeding are runtime-owned and byte-identical to the
    /// AST path. (dest, recv_name, method, arg_start, n_args)
    CallStaticScoped(RegId, Box<NamePair>, RegId, u32),

    /// Compiled class-method builtin-collection call. The receiver is a
    /// class handle in `handle_reg`; `member` names a member collection
    /// (associative array or queue) declared in the receiver's class;
    /// `method` is a builtin with value-only args (`size`/`num`,
    /// `exists`, `delete`, `push_back`/`push_front`, `pop_front`/`pop_back`,
    /// `insert`). Args are pre-evaluated values in contiguous registers
    /// `arg_start..arg_start+n_args`; the scalar result (0/1 for `exists`,
    /// count for `size`/`num`, popped element for pops) lands in `dest`.
    /// Runtime resolution goes through `handle_collection_name` (or the
    /// static-collection fallback) and then re-enters the interpreter's own
    /// `eval_builtin_method`, so storage semantics are byte-for-byte those
    /// of the AST path. Null receiver => `dest = zero(32)` (LRM no-op).
    /// `bare != 0` marks a bare-receiver call (`q.size()` inside a method —
    /// the runtime receiver is `this`): resolution passes the bare member
    /// name to `eval_builtin_method`, which re-runs the interpreter's own
    /// §8.10 member rewrite (`instance_assoc_member`, covering member
    /// collections, type-param-bound collections and static-collection
    /// stores) and answers identically to the AST funnel. `bare == 0`
    /// marks a dotted receiver (`obj.member.exists(k)`): resolution goes
    /// through `handle_collection_name(handle, member)` and re-enters
    /// `eval_builtin_method` on the scoped `<handle>#member` store.
    CallCollMethod(RegId, RegId, Box<NamePair>, RegId, u32, u8), // (dest, handle_reg, (member, method), arg_start, n_args, bare)

    /// class-perf Step 9b-ii: ELEMENT read of a class member collection,
    /// `q[i]` / `aa[k]` / `obj.aa[k]` inside a compiled method body.
    /// (dest, handle_reg, member, index_reg). The exec arm mirrors the
    /// interpreter's own member-element read body (the `expr_assoc_name`
    /// arm of eval_expr's Index) DIRECTLY — `assoc_key_str` narrowing,
    /// `store[key]` lookup with the zero(32) missing-key default, no width
    /// fit (the compiler emits the context Resize). `handle_reg == 0`
    /// selects the bare route (`instance_assoc_member`, statics included);
    /// otherwise the register carries the base handle and the store is
    /// `handle_collection_name(handle, member)` — a zero handle reads
    /// `Value::zero(32)`, matching the funnel's unresolved-store path.
    LoadCollElem(RegId, RegId, Box<str>, RegId),
    /// Element WRITE `q[i] = v` / `aa[k] = v` on the same receivers.
    /// (handle_reg, member, index_reg, value_reg). The exec arm mirrors the
    /// `expr_assoc_name` arm of `assign_value_index` directly — §7.10.1
    /// queue append, §10.7 width fit off the live store, exact changed-
    /// detect insert, touch_queue, waiter notify, settle — so storage and
    /// side effects match the AST path. No-op on a null receiver.
    StoreCollElem(RegId, Box<str>, RegId, RegId),

    /// class-perf Step 9c: `$cast(dest, src)` inside a compiled method body.
    /// (dest_route, src_reg, out_reg, dest_expr, stmt_form).
    ///
    /// The runtime compatibility check runs the interpreter's own
    /// `cast_type_ok` on the carried dest AST — the type overlays it reads
    /// (`local_type_stack`, `var_class_types`, `var_typedef_types`) are LIVE
    /// during compiled exec: formals and the return cell were recorded by
    /// the binding loop, and top-level body decls are pre-recorded at
    /// method entry. Assignment routes mirror the AST funnel: `Reg` = the
    /// frame arm's fit into the VM register (the interpreter frame is STALE
    /// during compiled exec, so a register-backed dest must NOT go through
    /// `assign_value`); `Member` = `StoreClassMember` semantics;
    /// `Scope` = `assign_value` on the carried AST (its name resolution
    /// reads live state). `stmt_form` selects the §8.16 task form, which —
    /// unlike the silent function form — prints a runtime error on failure.
    Cast(Box<CastInsn>, RegId, RegId),

    /// class-perf Step 9d: materialize the ITERATION KEYS of a class member
    /// collection for a compiled `foreach` — `foreach (this.m[k])` /
    /// `foreach (m[k])` / `foreach (obj.m[k])` — into a compile-time-
    /// indexed runtime slot. (slot, handle_reg, member). The exec arm
    /// mirrors the interpreter's single-loop-var foreach materialization on
    /// the resolved store — `array_iter_keys` (the `.size`-shadow dense
    /// range, the assoc first-key-seg scan with dedup, string vs numeric
    /// §7.9 ordering), `assoc_index_width_for` for the key width/signedness
    /// (§7.8.2), then the exact key→Value conversion of the sync foreach
    /// arm — so the iteration set and loop-var Values are byte-identical.
    /// `handle_reg == 0` selects the bare route (`instance_assoc_member`,
    /// statics included); otherwise the register carries the base handle
    /// and the store is `handle_collection_name(handle, member)` — an
    /// unresolved store materializes NO keys, which yields ZERO iterations
    /// for a null receiver, matching the interpreter's expr_assoc_name
    /// fall-through. The keys are SNAPSHOT at loop entry (the sync foreach
    /// materializes once too); the body cannot suspend in method mode
    /// (`allow_waits = false`), so no resume re-entry can reach them.
    ForeachKeys(u32, RegId, Box<str>),
    /// Advance a compiled foreach: pop the next key Value from slot `slot`
    /// into `var`; `ok` = 0 when the slot is exhausted (loop exit).
    /// (ok_dest, slot, var_dest). Pairs with ForeachKeys; the loop-var
    /// binding target is a VM register the compiler inserted into
    /// `local_var_regs` for the loop's duration.
    ForeachNext(RegId, u32, RegId),

    /// class-perf collection-return: materialize the method's `return '{…}`
    /// literal into the reserved interpreter snapshot store — element `i`
    /// from register `arg_start + i` — then hand it to the caller through
    /// `pending_ret_collection`. The result register is a dead passthrough
    /// (width 0), exactly like a class-handle return.
    RetCollection(u32, u32),

    /// class-perf collection-return store (§13.4): `q = f(..)` in a
    /// compiled body, `q` a bare LOCAL queue / dynamic array. `key` is the
    /// destination's `local_coll_key` (`\x01`-prefixed) and the register is
    /// the call's result. Emitted after the call whose result is that
    /// register; every call arm records the collection the callee handed
    /// back (`vm_ret_coll`), and this insn copies it whole into the local.
    /// With nothing handed back it assigns the scalar result, as the
    /// interpreter's blocking assign does.
    AssignPendingColl(Box<str>, RegId),

    Nop,

    /// Fused `LoadSignal` + `RangeSelectConst`: dest = signal_table[sig][left:right].
    /// Produced by the `finish()` peephole when the loaded register is dead
    /// after the select. Reads the slice straight out of the signal — decisive
    /// for wide (>64-bit) signals, where `LoadSignal` would copy the whole
    /// `Wide` storage (1 byte/bit) into a VM register only to slice a few
    /// bits out. Also removes one dispatch + one 32-byte register write.
    LoadSignalRange(RegId, SigId, u32, u32), // (dest, signal_id, left, right)
    /// `dest = signal[lo +: width]` with a RUNTIME low bound, read straight
    /// out of the signal. The packed-element read this replaces loaded the
    /// WHOLE signal into a register and sliced that, so one read of a packed
    /// memory copied the entire memory and its cost grew with the memory's
    /// size (3x per read on a 16x deeper array).
    LoadSignalRangeDyn(RegId, SigId, RegId, u32), // (dest, signal_id, lo_reg, width)
    /// Fused `LoadSignal` + `BitSelectConst`: dest = signal_table[sig][index].
    LoadSignalBit(RegId, SigId, u32), // (dest, signal_id, index)

    /// Fused `LoadConst` + `NbaAssign`: signal_table[id] <= K. The dominant
    /// reset-value NBA shape (33M dynamic pairs on the c910 memcpy census) —
    /// skips one dispatch and one 32-byte register write per execution.
    NbaAssignConst(SigId, Box<Value>, u32), // (signal_id, const, width)
    /// Fused `LogNot` + `BranchIfFalse`: jump unless the register is
    /// DEFINITE zero (`is_nonzero() == Some(false)`) — the exact composition
    /// of `logic_not` (Some(true)→0, Some(false)→1, None→X) with
    /// `!is_true(..)`, so X conditions branch exactly as before.
    BranchUnlessZero(RegId, u32), // (cond_reg, jump_target)
    /// Fused `LoadSignal` + `BranchIfFalse`: jump unless
    /// signal_table[id].is_true() — no register copy of the signal.
    ///
    /// The third field is a BIT INDEX, or `u32::MAX` for "test the whole
    /// signal". The bit form additionally folds in a constant bit-select, so
    /// `LoadSignalBit(d,sig,i) ; BranchIfFalse(d,T)` collapses to a single
    /// instruction. That pair is the most frequent adjacent pair in the C906
    /// memcpy opcode census — 25.4 M occurrences, 4.8% of all executed
    /// instructions — because it is what every `if (vec[i])` in the RTL
    /// lowers to. It only becomes adjacent AFTER the
    /// `LoadSignal`+`BitSelectConst` fusion below runs, which is why it needs
    /// its own pass.
    BranchIfSignalFalse(SigId, u32, u32), // (signal_id, jump_target, bit | u32::MAX)

    /// Fused `LoadSignal` + `LoadArrayElem` + `NbaAssign` — an RTL memory
    /// read feeding a flop:
    ///
    ///   LoadSignal(r1, idx_sig)          ; r1 = the array index, from a signal
    ///   LoadArrayElem(r2, array, r1)     ; r2 = array[r1]
    ///   NbaAssign(dst_sig, r2, width)    ; dst_sig <= r2
    ///       → NbaAssignArrayRead(dst_sig, array, idx_sig, width)
    ///
    /// The dominant shape in a CPU's register file and caches. On the C906
    /// memcpy census the two constituent adjacent pairs each fire 16.5 M times
    /// with IDENTICAL counts (3.7% of the stream apiece) — one idiom, not two.
    /// Collapsing it removes two dispatches and two 32-byte VM register
    /// writes per execution.
    ///
    /// NOTE the field order: the DESTINATION signal comes first (so the
    /// `NbaAssign*` write-extraction alternations bind it in their usual first
    /// position) and the INDEX signal — which this instruction READS — is
    /// third. The array element read is dynamically addressed, so like
    /// `LoadArrayElem` this variant makes an edge block non-gateable in
    /// `build_event_measure_state`.
    NbaAssignArrayRead(SigId, Box<ArrayOperand>, SigId, u32), // (dst_sig, array, idx_sig, width)

    /// Fused `LoadConst` + a binary ALU op that consumes it as its RIGHT
    /// operand:
    ///
    ///   LoadConst(c, K)                    ; c = K
    ///   Add|Eq|CaseEq(d, l, c)             ; d = l <op> c
    ///       → BinOpConst(d, l, K, kind)
    ///
    /// `LoadConst` is the #2 opcode on the C906 memcpy census (49.7 M, 12.0%)
    /// and 32.5 M of those feed exactly these three operators — 7.9% of the
    /// whole executed stream. Each fusion removes one dispatch and one 32-byte
    /// VM register write.
    ///
    /// ONE variant, not three: the enum's known silent-failure mode is the
    /// ~25 analysis sites that match `Insn` with a catch-all `_ =>` to pull
    /// out SIGNAL IDs. This variant carries no signal id — only two register
    /// ids and a constant — so `_ =>` is the correct answer at every one of
    /// them, and there is one thing to audit rather than three.
    ///
    /// Field order is (dest, src, K, kind). The exec arms substitute `&**K`
    /// for what would have been `&vm_regs[c]` and are otherwise character-for-
    /// character the unfused arms, so the 4-state, signedness and §5.7.1
    /// `is_fill` rules cannot drift.
    BinOpConst(RegId, RegId, Box<Value>, BinOpConstKind), // (dest, src, const, kind)
    /// Superinstruction: TWO independent (or chained — executed in order)
    /// constant adds in ONE dispatch. The c906 opcode census measured
    /// `AddC -> AddC` as the hottest adjacent pair (415 M, 6.5% of the
    /// stream) and they are NOT dataflow-foldable (different registers), so
    /// the win is deleting one trip through the discriminant-load /
    /// jump-table / indirect-jump dispatch chain per pair. Formed only under
    /// `XEZIM_FUSE_ADDC2=1` (see `fuse_addc2`).
    BinOpConstAdd2(Box<AddC2>),

    /// Roadmap steps 11-12 (compiled process FSMs): suspend the executing
    /// PROCESS for the delay whose VALUE is in the register (converted to
    /// ticks at runtime with the process's module-precision diff). Emitted
    /// only for process bodies (`allow_waits`), never edge/comb blocks.
    WaitDelayReg(RegId),
    /// Suspend the process until the wait spec (index into the owning
    /// FSM's `wait_specs` table) fires. Process bodies only.
    WaitEdge(u32),
    /// Code coverage: count counter `n` (see `simulator::code_cov`). Reads
    /// and writes no register; only instrumented designs contain it.
    CovHit(u32),
    /// Fused constant equality + `BranchIfFalse` (§11.4.5, §12.4):
    /// compare the source against the embedded constant and branch when the
    /// result is not true. Only Eq/CaseEq kinds are emitted. `tmp` is the
    /// eliminated result register, retained as scratch for two-state lowering;
    /// the four-state VM does not write it because it is dead on both paths.
    /// Appended so existing serialized instruction discriminants stay stable.
    CmpBranchConst(BinOpConstKind, RegId, Box<Value>, RegId, u32),
    /// §7.5 / §7.10 / §7.4.6: element read of a design-scope dynamic array or
    /// queue (dest, operand, index register). The exec arm reads the LIVE
    /// collection exactly as the interpreter's element read does: the element
    /// cell for an index inside both `size()` and the placeholder cell range,
    /// otherwise the interpreter's own element read (by-name storage past the
    /// cells, the element type's default past `size()` or for an x index). A
    /// dense `LoadArrayElem` over the placeholder cells read x from index 64
    /// on and a stale element past `size()`. Main executor only: every other
    /// executor and lowering treats it like `EvalExprFallback`.
    LoadDynElem(RegId, Box<DynElemOp>, RegId),
    /// Pop one value from a declared module queue (§7.10.2).
    QueuePop(RegId, Box<Name>, bool), // (dest, queue, from_front)
}

/// The collection a `LoadDynElem` reads.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DynElemOp {
    /// The collection, as its `arrays` key.
    pub name: Box<str>,
    /// `<name>.size`, the key its live size is kept under.
    pub size_key: Box<str>,
    /// The placeholder element cells `(first_id, lo, hi)`, if any.
    pub cells: Option<(usize, i64, i64)>,
    /// Context width of the read (the interpreter's x-index default).
    pub ctx_width: u32,
}

const _: () = assert!(std::mem::size_of::<Insn>() <= 24, "Insn grew past 24 bytes");

/// Pre-resolved unpacked-array addressing embedded in bytecode. The name is
/// retained for diagnostics and the rare unresolved fallback, while normal
/// execution uses only the dense base/range fields.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum ArrayOperand {
    Dense {
        name: String,
        first_id: usize,
        lo: i64,
        hi: i64,
    },
    Named(String),
}

impl ArrayOperand {
    pub fn name(&self) -> &str {
        match self {
            Self::Dense { name, .. } | Self::Named(name) => name,
        }
    }
}

/// class-perf Step 9c: the assignment route for a `$cast` destination.
/// Selects how the exec arm stores the (type-checked) source value so the
/// write lands where the AST path's `assign_value` would put it.
/// One boxed name — a thin (8 B) handle to a heap string, for variants
/// whose payload would otherwise blow the 24 B `Insn` footprint (a bare
/// `Box<str>` is a 16 B fat pointer).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Name(pub Box<str>);

/// One boxed `(name, name)` pair — the 2-string payloads of the class
/// call/static insns. Grouping them keeps `Insn` at 24 B: two separate
/// `Box<str>` fat pointers cost 32 B of inline payload.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NamePair(pub Box<str>, pub Box<str>);

/// Boxed payload of `Insn::Cast` — keeps the `Insn` enum inside its
/// 24-byte footprint (`CastDest::Member` carries a fat `Box<str>`, so an
/// unboxed `CastDest + Box<Expression> + bool` payload would grow every
/// instruction in every block to 48 B).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CastInsn {
    pub dest: CastDest,
    pub expr: Box<Expression>,
    pub stmt_form: bool,
    /// class-perf P2: the dest local's DECLARED class, threaded through
    /// for nested-block handle locals the runtime overlay never typed
    /// (`begin my_comp tmp; ... $cast(tmp, parent)`). The executor uses
    /// it as the cast's destination type instead of the overlay lookup.
    pub decl_class: Option<Box<str>>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum CastDest {
    /// Register-backed formal / top-level local: assign into this VM
    /// register with the frame arm's fit (resize against the slot's
    /// current width / realness — the interpreter frame itself is stale
    /// during compiled exec and must not be written).
    Reg(RegId),
    /// Class member: (handle_reg, field) — `fit_class_prop` + heap insert,
    /// byte-identical to `StoreClassMember`.
    Member(RegId, Box<str>),
    /// Any other bare dest (module-scope variable, `this`-member, static
    /// cell): the carried dest AST is handed to `assign_value`, whose name
    /// resolution reads LIVE state during compiled exec. Admitted only for
    /// bare names — a dotted dest whose base is a register-backed local
    /// would make `assign_value` evaluate the stale frame.
    Scope,
}

impl CompiledBlock {
    /// True when any instruction embeds a `SigId`. Signal ids are stable
    /// only within one elaboration of one design, so a method block that
    /// carries them is NEVER written to the persistent method cache (a hit
    /// from another design would read a different signal). The match is
    /// exhaustive over `Insn`: a new signal-carrying variant must be listed
    /// here or the write guard silently stops covering it.
    pub fn carries_sig_id(&self) -> bool {
        self.instructions.iter().any(|i| i.carries_sig_id())
    }
}

impl Insn {
    pub fn carries_sig_id(&self) -> bool {
        use Insn::*;
        matches!(
            self,
            LoadSignal(..)
                | LoadSignalSigned(..)
                | BlockingAssignString(..)
                | NbaAssign(..)
                | NbaAssignRange(..)
                | NbaAssignRangeDyn(..)
                | NbaAssignBitDyn(..)
                | BlockingAssign(..)
                | BlockingAssignRange(..)
                | BlockingAssignRangeDyn(..)
                | BlockingAssignBitDyn(..)
                | LoadSignalRange(..)
                | LoadSignalRangeDyn(..)
                | LoadSignalBit(..)
                | NbaAssignConst(..)
                | BranchIfSignalFalse(..)
                | NbaAssignArrayRead(..)
        )
    }

    /// Whether this instruction transfers control to a block-relative
    /// instruction index: the plain and fused branch forms and the jump
    /// tables. A pass that appends one block's instructions after another's
    /// must relocate every such target or keep the block out.
    pub fn is_branch(&self) -> bool {
        use Insn::*;
        matches!(
            self,
            Jump(..)
                | BranchIfFalse(..)
                | BranchUnlessZero(..)
                | BranchIfSignalFalse(..)
                | CmpBranch(..)
                | CmpBranchConst(..)
                | CaseJump(..)
                | CaseMaskJump(..)
        )
    }

    /// Shift every branch target of this instruction by `ib` (the index at
    /// which its block now starts in a concatenated stream), leaving the
    /// registers alone. Covers every form `is_branch` names.
    pub fn offset_branch_targets(&mut self, ib: u32) {
        use Insn::*;
        match self {
            Jump(t)
            | BranchIfFalse(_, t)
            | BranchUnlessZero(_, t)
            | BranchIfSignalFalse(_, t, _)
            | CmpBranch(_, _, _, _, t)
            | CmpBranchConst(_, _, _, _, t) => *t += ib,
            CaseJump(_, jump) => {
                for t in &mut jump.table {
                    *t += ib;
                }
                jump.default += ib;
            }
            CaseMaskJump(_, jump) => {
                for t in &mut jump.table {
                    *t += ib;
                }
                jump.xz_path += ib;
            }
            _ => {}
        }
    }
}

impl Insn {
    /// Region fusion (`build_comb_entries`): shift every register operand by
    /// `rb` and every branch target by `ib` so one member block can be
    /// appended after another WITHOUT register collisions — the two-state
    /// lowering tracks one static width per register, so reusing a slot at a
    /// different width would knock the whole fused block off the fast path.
    /// Returns false for variants fusion must not carry (interpreter escapes,
    /// masked jump tables, process-local reads). Callers also apply their own
    /// side-effect and scheduling admission guards.
    pub fn offset_regs_and_targets(&mut self, rb: RegId, ib: u32) -> bool {
        use Insn::*;
        match self {
            Nop | CovHit(_) => {}
            BinOpConstAdd2(a) => {
                a.d1 += rb;
                a.s1 += rb;
                a.d2 += rb;
                a.s2 += rb;
            }
            Jump(t) => *t += ib,
            BranchIfSignalFalse(_, t, _) => *t += ib,
            BranchIfFalse(a, t) | BranchUnlessZero(a, t) => {
                *a += rb;
                *t += ib;
            }
            CmpBranch(_, a, b, c, t) => {
                *a += rb;
                *b += rb;
                *c += rb;
                *t += ib;
            }
            CmpBranchConst(_, s, _, tmp, t) => {
                *s += rb;
                *tmp += rb;
                *t += ib;
            }
            // §12.5: arm entries, holes and the no-match/unknown path all
            // refer to this block's local instruction coordinates. Relocate
            // every target, including an exit that becomes the next member's
            // entry, so dense cases retain NBA-only edge-block merging.
            CaseJump(selector, jump) => {
                *selector += rb;
                for target in &mut jump.table {
                    *target += ib;
                }
                jump.default += ib;
            }
            LoadConst(a, _)
            | Resize(a, _)
            | SetSigned(a)
            | ClearSigned(a)
            | DropXz(a)
            | RealToInt(a)
            | LoadSignal(a, _)
            | LoadSignalSigned(a, _)
            | LoadSignalRange(a, _, _, _)
            | LoadSignalBit(a, _, _)
            | BlockingAssign(_, a, _)
            | BlockingAssignRange(_, _, _, a)
            | BlockingAssignString(_, a) => *a += rb,
            LoadSignalRangeDyn(a, _, b, _) => {
                *a += rb;
                *b += rb;
            }
            Move(a, b)
            | MoveResize(a, b, _)
            | BitNot(a, b)
            | LogNot(a, b)
            | Negate(a, b)
            | ReduceAnd(a, b)
            | ReduceOr(a, b)
            | ReduceXor(a, b)
            | Replicate(a, b, _)
            | BitSelectConst(a, b, _)
            | RangeSelectConst(a, b, _, _)
            | CaseLut(a, b, _)
            | BinOpConst(a, b, _, _)
            | BlockingAssignBitDyn(_, a, b)
            | LoadArrayElem(a, _, b)
            | LoadDynElem(a, _, b)
            | BlockingAssignArray(_, a, b, _)
            | LoadClassMember(a, b, _)
            | StoreClassMember(a, b, _) => {
                *a += rb;
                *b += rb;
            }
            Add(a, b, c)
            | Sub(a, b, c)
            | Mul(a, b, c)
            | Div(a, b, c)
            | Mod(a, b, c)
            | Pow(a, b, c)
            | BitAnd(a, b, c)
            | BitOr(a, b, c)
            | BitXor(a, b, c)
            | BitXnor(a, b, c)
            | LogAnd(a, b, c)
            | LogOr(a, b, c)
            | Eq(a, b, c)
            | Neq(a, b, c)
            | CaseEq(a, b, c)
            | CasezEq(a, b, c)
            | CasexEq(a, b, c)
            | Lt(a, b, c)
            | Leq(a, b, c)
            | Gt(a, b, c)
            | Geq(a, b, c)
            | Shl(a, b, c)
            | Shr(a, b, c)
            | AShr(a, b, c)
            | BitSelect(a, b, c)
            | BlockingAssignRangeDyn(_, a, b, c) => {
                *a += rb;
                *b += rb;
                *c += rb;
            }
            // `$cast`: rebase the result register, the source register and
            // whichever register the route targets (dest slot or handle).
            Cast(ci, s, o) => {
                match &mut ci.dest {
                    CastDest::Reg(d) => *d += rb,
                    CastDest::Member(h, _) => *h += rb,
                    CastDest::Scope => {}
                }
                *s += rb;
                *o += rb;
            }
            Select(a, b, c, d)
            | RangeSelect(a, b, c, d)
            | BlockingAssignArrayRange(_, a, b, c, d) => {
                *a += rb;
                *b += rb;
                *c += rb;
                *d += rb;
            }
            // Compiled foreach: the key slot is an ARENA index, not a
            // register — only the var register rebases. No branch targets.
            ForeachKeys(_, h, _) => *h += rb,
            ForeachNext(o, _, v) => {
                *o += rb;
                *v += rb;
            }
            // The collection-return hand-off reads a contiguous arg range
            // (no rebasing needed beyond the range itself — the base is an
            // absolute slot like CallFreeFunction's).
            RetCollection(_, _) => {}
            RangeSelectW(a, b, c, _, _) => {
                *a += rb;
                *b += rb;
                *c += rb;
            }
            Concat(a, v) => {
                *a += rb;
                for r in v.iter_mut() {
                    *r += rb;
                }
            }
            StrOp(a, _, v) => {
                *a += rb;
                for r in v.iter_mut() {
                    *r += rb;
                }
            }
            // NBA stores carry registers but never branch targets, so they
            // rebase like any other operand. (Comb-region fusion refuses NBA
            // members through its own `insn_ok` gate, so admitting them here
            // does not loosen that pass; edge-block merging needs them —
            // a flop body is NBAs and nothing else.)
            NbaAssign(_, a, _) => *a += rb,
            NbaAssignConst(..) => {}
            NbaAssignRange(_, _, _, a) => *a += rb,
            NbaAssignRangeDyn(_, a, b, c) => {
                *a += rb;
                *b += rb;
                *c += rb;
            }
            NbaAssignBitDyn(_, a, b) => {
                *a += rb;
                *b += rb;
            }
            NbaAssignArray(_, a, b, _) => {
                *a += rb;
                *b += rb;
            }
            NbaAssignArrayRange(_, a, b, c, d) => {
                *a += rb;
                *b += rb;
                *c += rb;
                *d += rb;
            }
            // Signal-to-signal fused array read: no registers at all.
            NbaAssignArrayRead(..) => {}
            // Step 9g: a static-property READ has exactly one register (its
            // destination); a static STORE / object construction is
            // non-offsettable (side effects / interpreter re-entry).
            LoadClassStatic(a, _) => *a += rb,
            LoadProcessLocal(..)
            | LoadProcessStatus(..)
            | Format(..)
            | CaseMaskJump(..)
            | StmtFallback(..)
            | InitLocalColl(..)
            | EvalExprFallback(..)
            | WaitDelayReg(..)
            | WaitEdge(..)
            | CallMethod(..)
            | CallCollMethod(..)
            | LoadCollElem(..)
            | StoreCollElem(..)
            | CallScopedMethod(..)
            | StoreClassStatic(..)
            | ConstructObject(..)
            | CallStaticScoped(..)
            | CallFreeFunction(..)
            | QueuePop(..)
            | AssignPendingColl(..) => return false,
        }
        true
    }
}

/// Opcode name only (no operands) — used by the settle profiler to aggregate
/// continuous-assignment RHS shapes across entries. Operands differ per
/// instance; the SHAPE is what a fused fast path would have to match.
pub fn insn_opcode_name(i: &Insn) -> &'static str {
    match i {
        Insn::LoadConst(..) => "Const",
        Insn::LoadSignal(..) => "Load",
        Insn::LoadSignalSigned(..) => "LoadS",
        Insn::LoadProcessLocal(..) => "LoadLocal",
        Insn::LoadProcessStatus(..) => "ProcStatus",
        Insn::Resize(..) => "Resize",
        Insn::Add(..) => "Add",
        Insn::Sub(..) => "Sub",
        Insn::Mul(..) => "Mul",
        Insn::Div(..) => "Div",
        Insn::Mod(..) => "Mod",
        Insn::BitAnd(..) => "And",
        Insn::BitOr(..) => "Or",
        Insn::BitXor(..) => "Xor",
        Insn::BitXnor(..) => "Xnor",
        Insn::LogAnd(..) => "LAnd",
        Insn::LogOr(..) => "LOr",
        Insn::Eq(..) => "Eq",
        Insn::Neq(..) => "Neq",
        Insn::CaseEq(..) => "CaseEq",
        Insn::CaseLut(..) => "CaseLut",
        Insn::CaseJump(..) => "CaseJump",
        Insn::CaseMaskJump(..) => "CaseMaskJump",
        Insn::Format(..) => "Format",
        Insn::StrOp(..) => "StrOp",
        Insn::BlockingAssignString(..) => "BlockingAssignString",
        Insn::CasezEq(..) => "CasezEq",
        Insn::CasexEq(..) => "CasexEq",
        Insn::Lt(..) => "Lt",
        Insn::Leq(..) => "Leq",
        Insn::Gt(..) => "Gt",
        Insn::Geq(..) => "Geq",
        Insn::Shl(..) => "Shl",
        Insn::Shr(..) => "Shr",
        Insn::AShr(..) => "AShr",
        Insn::BitNot(..) => "Not",
        Insn::LogNot(..) => "LNot",
        Insn::Negate(..) => "Neg",
        Insn::ReduceAnd(..) => "RedAnd",
        Insn::ReduceOr(..) => "RedOr",
        Insn::ReduceXor(..) => "RedXor",
        Insn::BitSelect(..) => "BitSel",
        Insn::BitSelectConst(..) => "BitSelC",
        Insn::RangeSelect(..) => "RngSel",
        Insn::RangeSelectW(..) => "RngSelW",
        Insn::RangeSelectConst(..) => "RngSelC",
        Insn::Concat(..) => "Concat",
        Insn::Replicate(..) => "Repl",
        Insn::Select(..) => "Select",
        Insn::Move(..) => "Move",
        Insn::SetSigned(..) => "SetSigned",
        Insn::DropXz(..) => "DropXz",
        Insn::RealToInt(..) => "RealToInt",
        Insn::ClearSigned(..) => "ClearSigned",
        Insn::Pow(..) => "Pow",
        Insn::LoadClassMember(..) => "LoadCls",
        Insn::StoreClassMember(..) => "StoreCls",
        Insn::CallMethod(..) => "CallM,",
        Insn::CallScopedMethod(..) => "CallScp,",
        Insn::CallFreeFunction(..) => "CallFre,",
        Insn::LoadClassStatic(..) => "LoadSta,",
        Insn::StoreClassStatic(..) => "StorSta,",
        Insn::ConstructObject(..) => "NewObj,",
        Insn::CallStaticScoped(..) => "CallStatScp,",
        Insn::CallCollMethod(..) => "CallColl,",
        Insn::LoadCollElem(..) => "CollElemR,",
        Insn::StoreCollElem(..) => "CollElemW,",
        Insn::Cast(..) => "Cast",
        Insn::ForeachKeys(..) => "FeKeys",
        Insn::ForeachNext(..) => "FeNext",
        Insn::RetCollection(..) => "RetColl",
        Insn::AssignPendingColl(..) => "AssignPC",
        Insn::QueuePop(..) => "QueuePop",
        Insn::Nop => "Nop",
        Insn::Jump(..) => "Jump",
        Insn::BranchIfFalse(..) => "Br",
        Insn::CmpBranch(..) => "CmpBr",
        Insn::CmpBranchConst(..) => "CmpBranchConst",
        Insn::MoveResize(..) => "MovRz",
        Insn::BranchIfSignalFalse(..) => "BrSig",
        Insn::BranchUnlessZero(..) => "BrNz",
        Insn::LoadSignalBit(..) => "LoadBit",
        Insn::LoadSignalRange(..) => "LoadRng",
        Insn::LoadSignalRangeDyn(..) => "LoadRngDyn",
        Insn::LoadArrayElem(..) => "LoadArr",
        Insn::LoadDynElem(..) => "LoadDynElem",
        Insn::BlockingAssign(..) => "Assign",
        Insn::BlockingAssignRange(..) => "AssignRng",
        Insn::BlockingAssignRangeDyn(..) => "AssignRngDyn",
        Insn::BlockingAssignBitDyn(..) => "AssignBitDyn",
        Insn::BlockingAssignArray(..) => "AssignArr",
        Insn::BlockingAssignArrayRange(..) => "AssignArrRng",
        Insn::NbaAssign(..) => "Nba",
        Insn::NbaAssignConst(..) => "NbaC",
        Insn::NbaAssignRange(..) => "NbaRng",
        Insn::NbaAssignRangeDyn(..) => "NbaRngDyn",
        Insn::NbaAssignBitDyn(..) => "NbaBitDyn",
        Insn::NbaAssignArray(..) => "NbaArr",
        Insn::NbaAssignArrayRange(..) => "NbaArrRng",
        Insn::NbaAssignArrayRead(..) => "NbaArrRd",
        Insn::BinOpConst(_, _, _, BinOpConstKind::Add) => "AddC",
        Insn::BinOpConstAdd2(..) => "AddC2",
        Insn::WaitDelayReg(..) => "WaitDly",
        Insn::WaitEdge(..) => "WaitEdge",
        Insn::CovHit(..) => "CovHit",
        Insn::BinOpConst(_, _, _, BinOpConstKind::Eq) => "EqC",
        Insn::BinOpConst(_, _, _, BinOpConstKind::CaseEq) => "CaseEqC",
        Insn::BinOpConst(_, _, _, BinOpConstKind::Xor) => "XorC",
        Insn::BinOpConst(_, _, _, BinOpConstKind::And) => "AndC",
        Insn::BinOpConst(_, _, _, BinOpConstKind::Or) => "OrC",
        Insn::BinOpConst(_, _, _, BinOpConstKind::Mul) => "MulC",
        Insn::BinOpConst(_, _, _, BinOpConstKind::Sub) => "SubC",
        Insn::StmtFallback(..) => "Fallback",
        Insn::InitLocalColl(..) => "InitColl",
        Insn::EvalExprFallback(..) => "EvalExpr",
    }
}
