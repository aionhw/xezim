//! Bytecode VM for high-performance simulation execution.
//! Compiles AST expressions and statements into a flat instruction array
//! that can be executed without pointer-chasing through Box<Expression> trees.

use super::value::Value;
use crate::ast::decl::{FunctionDeclaration, TaskDeclaration};
use crate::ast::types::PortDirection;
use crate::ast::expr::*;
use crate::ast::stmt::*;
use std::sync::Arc;
use xezim_core::hasher::{HashMap, HashSet};

const MAX_INLINE_DEPTH: usize = 8;

/// A register in the bytecode VM. Registers hold Values. The compact u16
/// encoding keeps each instruction at 24 bytes; the allocator uses a wider
/// counter and falls back before an ID would overflow this representation.
pub type RegId = u16;

/// A signal-table index inside an instruction. `u32`, not `usize`: the
/// largest design measured here has 35.1 M signals and `u32` covers 4.29 B,
/// so the extra four bytes per field were pure footprint. Fourteen `Insn`
/// variants carry one, and they are what pushed the enum to 24 bytes — an
/// awkward size that costs a three-instruction `lea/add/lea` to address and
/// packs only 2.67 instructions per 64-byte cache line.
type SigId = u32;

/// Narrow a `usize` signal-table index to the in-instruction [`SigId`].
///
/// Every id ultimately comes from a `Vec` index, so on any design that fits
/// in memory this cannot overflow — but a silent wrap would be catastrophic
/// and invisible: the instruction would read and write a completely
/// unrelated signal for the rest of the run, with no diagnostic. Checked
/// unconditionally (not `debug_assert!`) because this runs once per emitted
/// instruction at compile time, never in the VM dispatch loop.
#[inline]
pub(crate) fn as_sig_id(id: usize) -> SigId {
    assert!(
        id <= SigId::MAX as usize,
        "signal id {id} does not fit in a {}-bit Insn signal field \
         (limit {}); Insn::SigId must widen before such a design can run",
        SigId::BITS,
        SigId::MAX
    );
    id as SigId
}

/// Number of `LoadSignal ; LoadArrayElem ; NbaAssign` triples collapsed into
/// `Insn::NbaAssignArrayRead` across every block compiled in this process.
/// Reported once by the simulator's `[PROF]` summary so the static fusion
/// count can be compared against the dynamic opcode census.
static FUSED_ARRAY_READ_NBA: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

/// Static count of array-read→flop fusions performed. See
/// [`Insn::NbaAssignArrayRead`].
pub fn array_read_nba_fusions() -> u64 {
    FUSED_ARRAY_READ_NBA.load(std::sync::atomic::Ordering::Relaxed)
}

/// Number of element-wise packed identity NBA sites lowered to whole-vector
/// NBAs across every block compiled in this process.
static PACKED_LOOP_NBA_COPIES: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

pub fn packed_loop_nba_copies() -> u64 {
    PACKED_LOOP_NBA_COPIES.load(std::sync::atomic::Ordering::Relaxed)
}

/// Per-kind count of `LoadConst ; <binop>` pairs collapsed into
/// `Insn::BinOpConst`, indexed by `BinOpConstKind as usize`. See
/// [`BytecodeCompiler::fuse_binop_const`].
static FUSED_BINOP_CONST: [std::sync::atomic::AtomicU64; BinOpConstKind::COUNT] = [
    std::sync::atomic::AtomicU64::new(0),
    std::sync::atomic::AtomicU64::new(0),
    std::sync::atomic::AtomicU64::new(0),
    std::sync::atomic::AtomicU64::new(0),
];

/// Static count of `Move ; <assign>` pairs where the Move was forwarded into
/// the assign's value operand (see `forward_move_into_assign`).
static FUSED_MOVE_FWD: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Static count of `ClearSigned` insns deleted because the register provably
/// held an unsigned value already (see `elide_provably_unsigned_scrubs`).
static ELIDED_SCRUBS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Static count of `AddC ; AddC` pairs merged into one `BinOpConstAdd2`
/// dispatch (see `fuse_addc2`).
static FUSED_ADDC2: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn addc2_count() -> u64 {
    FUSED_ADDC2.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn elided_scrub_count() -> u64 {
    ELIDED_SCRUBS.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn census_pair_fusions() -> u64 {
    FUSED_MOVE_FWD.load(std::sync::atomic::Ordering::Relaxed)
}

/// Static count of constant-operand ALU fusions performed, per
/// [`BinOpConstKind`] (same index order as the enum).
pub fn binop_const_fusions() -> [u64; BinOpConstKind::COUNT] {
    std::array::from_fn(|i| FUSED_BINOP_CONST[i].load(std::sync::atomic::Ordering::Relaxed))
}

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
}

impl BinOpConstKind {
    /// Number of kinds; sizes the static fusion-count array.
    pub const COUNT: usize = 4;
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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Insn {
    /// Load a constant value into a register. `Box<Value>` keeps the
    /// variant small (8 B instead of 32 B for the inline Value) — LoadConst
    /// isn't on the hot dispatch path so the extra indirection is cheap
    /// and the 24 B saving compounds with the u32 signal_id fields below
    /// to shrink `Insn` from 40 B to 32 B.
    LoadConst(RegId, Box<Value>),
    /// Load a signal from signal_table[signal_id] into a register.
    LoadSignal(RegId, SigId),      // (dest_reg, signal_id)
    /// Load a signal and mark it as signed.
    LoadSignalSigned(RegId, SigId),
    /// Load a value from the active process's innermost local frame.
    LoadProcessLocal(RegId, Box<str>),
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
    StmtFallback(Box<(Arc<Statement>, Arc<str>)>),
    /// Expression-level AST escape hatch: interpret ONE sub-expression the
    /// compiler can't handle (unresolvable ident, member access, impure
    /// call, ...) into a register, keeping the REST of the statement
    /// compiled. (RegId dest, ctx width for §11.8.1 sizing.) Forbidden
    /// while any register-backed locals are live — the interpreter cannot
    /// see VM registers.
    EvalExprFallback(Box<(Arc<Expression>, Arc<str>)>, RegId, u32),

    SetSigned(RegId),
    /// §11.8.1: the enclosing expression is UNSIGNED (some operand is
    /// unsigned), so this operand must ZERO-extend at the coming Resize —
    /// clear the runtime signed flag the load stamped on it.
    ClearSigned(RegId),
    /// §11.4.3 `**` with a non-constant base: left operand pre-resized to the
    /// operation width by the compiler; result width = left's width.
    Pow(RegId, RegId, RegId),
    Nop,

    /// Fused `LoadSignal` + `RangeSelectConst`: dest = signal_table[sig][left:right].
    /// Produced by the `finish()` peephole when the loaded register is dead
    /// after the select. Reads the slice straight out of the signal — decisive
    /// for wide (>64-bit) signals, where `LoadSignal` would copy the whole
    /// `Wide` storage (1 byte/bit) into a VM register only to slice a few
    /// bits out. Also removes one dispatch + one 32-byte register write.
    LoadSignalRange(RegId, SigId, u32, u32), // (dest, signal_id, left, right)
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
}

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

/// A compiled bytecode program for one always block or continuous assign.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]

pub struct CompiledBlock {
    pub instructions: Vec<Insn>,
    pub num_regs: u32,
    /// True when any instruction is a `StmtFallback` (AST-interpreted). Those
    /// resolve bare names through `resolve_hier_name`, which needs the owning
    /// entry's scope hint installed — the pure-bytecode insns pre-resolve their
    /// signal ids and don't. Precomputed here so the settle hot loop pays one
    /// bool test instead of scanning the insn stream.
    pub has_fallback: bool,
    /// True when some signal is the target of MORE THAN ONE nonblocking write
    /// in this block — the only situation in which §10.4.2 last-write-wins can
    /// be observed within a single block, and so the only one where a queued
    /// NBA entry has to be located and overwritten instead of the value simply
    /// being compared against the signal table.
    ///
    /// Precomputed because the isolated executors have no O(1) index into
    /// their per-block queue and would otherwise pay a linear scan on EVERY
    /// nonblocking write; measured at ~6.5% on an NBA-heavy design. The
    /// overwhelming majority of blocks write each target once and take the
    /// plain push path.
    pub nba_dup_targets: bool,
}

impl Insn {
    /// Region fusion (`build_comb_entries`): shift every register operand by
    /// `rb` and every branch target by `ib` so one member block can be
    /// appended after another WITHOUT register collisions — the two-state
    /// lowering tracks one static width per register, so reusing a slot at a
    /// different width would knock the whole fused block off the fast path.
    /// Returns false for variants fusion must not carry (fallbacks, NBAs,
    /// jump tables with internal targets, process-local reads); the caller
    /// pre-filters with the same set and treats false as a bug.
    pub fn offset_regs_and_targets(&mut self, rb: RegId, ib: u32) -> bool {
        use Insn::*;
        match self {
            Nop => {}
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
            LoadConst(a, _)
            | Resize(a, _)
            | SetSigned(a)
            | ClearSigned(a)
            | LoadSignal(a, _)
            | LoadSignalSigned(a, _)
            | LoadSignalRange(a, _, _, _)
            | LoadSignalBit(a, _, _)
            | BlockingAssign(_, a, _)
            | BlockingAssignRange(_, _, _, a)
            | BlockingAssignString(_, a) => *a += rb,
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
            | BlockingAssignArray(_, a, b, _) => {
                *a += rb;
                *b += rb;
            }
            Add(a, b, c) | Sub(a, b, c) | Mul(a, b, c) | Div(a, b, c) | Mod(a, b, c)
            | Pow(a, b, c) | BitAnd(a, b, c) | BitOr(a, b, c) | BitXor(a, b, c)
            | BitXnor(a, b, c) | LogAnd(a, b, c) | LogOr(a, b, c) | Eq(a, b, c)
            | Neq(a, b, c) | CaseEq(a, b, c) | CasezEq(a, b, c) | CasexEq(a, b, c)
            | Lt(a, b, c) | Leq(a, b, c) | Gt(a, b, c) | Geq(a, b, c) | Shl(a, b, c)
            | Shr(a, b, c) | AShr(a, b, c) | BitSelect(a, b, c)
            | BlockingAssignRangeDyn(_, a, b, c) => {
                *a += rb;
                *b += rb;
                *c += rb;
            }
            Select(a, b, c, d) | RangeSelect(a, b, c, d)
            | BlockingAssignArrayRange(_, a, b, c, d) => {
                *a += rb;
                *b += rb;
                *c += rb;
                *d += rb;
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
            LoadProcessLocal(..) | Format(..) | CaseJump(..) | CaseMaskJump(..)
            | StmtFallback(..) | EvalExprFallback(..)
            | WaitDelayReg(..) | WaitEdge(..) => return false,
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
        Insn::RangeSelectConst(..) => "RngSelC",
        Insn::Concat(..) => "Concat",
        Insn::Replicate(..) => "Repl",
        Insn::Select(..) => "Select",
        Insn::Move(..) => "Move",
        Insn::SetSigned(..) => "SetSigned",
        Insn::ClearSigned(..) => "ClearSigned",
        Insn::Pow(..) => "Pow",
        Insn::Nop => "Nop",
        Insn::Jump(..) => "Jump",
        Insn::BranchIfFalse(..) => "Br",
        Insn::CmpBranch(..) => "CmpBr",
        Insn::MoveResize(..) => "MovRz",
        Insn::BranchIfSignalFalse(..) => "BrSig",
        Insn::BranchUnlessZero(..) => "BrNz",
        Insn::LoadSignalBit(..) => "LoadBit",
        Insn::LoadSignalRange(..) => "LoadRng",
        Insn::LoadArrayElem(..) => "LoadArr",
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
        Insn::BinOpConst(_, _, _, BinOpConstKind::Eq) => "EqC",
        Insn::BinOpConst(_, _, _, BinOpConstKind::CaseEq) => "CaseEqC",
        Insn::BinOpConst(_, _, _, BinOpConstKind::Xor) => "XorC",
        Insn::StmtFallback(..) => "Fallback",
        Insn::EvalExprFallback(..) => "EvalExpr",
    }
}

/// Compiler state for converting AST → bytecode.
#[derive(Clone)]
struct LocalArrayBind {
    regs: Vec<RegId>,
    lo: i64,
    elem_w: u32,
    is_real: bool,
}

pub struct BytecodeCompiler<'a> {
    insns: Vec<Insn>,
    next_reg: u32,
    register_overflow: bool,
    signal_name_to_id: &'a HashMap<Arc<str>, usize>,
    signal_signed: &'a [bool],
    signal_widths: &'a [u32],
    /// Per-signal `is_real`. Optional because only the simulator has it;
    /// absent means "assume possibly-real", which only costs missed
    /// `Resize` elisions, never correctness.
    signal_real: Option<&'a [bool]>,
    arrays: &'a HashMap<String, (i64, i64, u32)>,
    array_first_id: Option<&'a HashMap<Arc<str>, (usize, i64, i64)>>,
    widths: &'a HashMap<String, u32>,
    pub bail_reason: Option<&'static str>,
    /// When true, unsupported statements emit `StmtFallback` instead of
    /// failing compilation. Safe for edge blocks where the AST interpreter's
    /// statement path is the same one used by the non-compiled fallback.
    pub allow_ast_fallback: bool,
    /// Hierarchical scope for resolving unqualified identifiers. An Ident
    /// with a bare local name (`mem_valid`) is first tried verbatim, then
    /// with this prefix applied (`testbench.mem_valid`).
    pub scope_hint: Option<String>,
    /// Process-FSM mode (roadmap 11-12): statement-level `#delay` and
    /// `@(event)` compile to WaitDelayReg / WaitEdge instead of bailing.
    /// Never set for edge or comb blocks.
    pub allow_waits: bool,
    /// Event-control specs referenced by WaitEdge, in emission order.
    pub wait_specs: Vec<crate::ast::stmt::EventControl>,
    /// Per-for-loop leaf-name → signal_id override. Set by `compile_stmt`'s
    /// For arm before compiling condition/step expressions, cleared after.
    /// Re-routes bare-ident lookups for the loop variable so that the step
    /// `i = i+1` writes to the same signal as the init `i = 0`, even when
    /// the elaborator only scope-qualified init's lvalue (see compile_for
    /// for the full c910 hang context).
    pub for_loop_var_ids: std::collections::HashMap<String, usize>,
    /// Block-LOCAL variables held in bytecode registers rather than signals —
    /// currently a `for (int i = ...)` loop variable, which has no signal at
    /// all (§12.7.1 makes it automatic and local to the loop). Without this the
    /// whole loop fell back to the AST interpreter.
    pub local_var_regs: std::collections::HashMap<String, (RegId, u32)>,
    /// Names introduced by a `VarDecl` INSIDE the compiled block that live only
    /// in a VM register — the interpreter never ran the declaration, so it has
    /// no storage for them at all.
    ///
    /// A per-statement `StmtFallback` re-runs one statement in the interpreter.
    /// If that statement touches one of these names it reads a signal that does
    /// not exist (x), while the compiled statements around it read the register
    /// — one block, two different variables. `always_ff begin item_t item; …
    /// q.push_back(item); end` pushed an all-x item on every cycle because the
    /// push could not be compiled and fell back. Same failure mode as
    /// `reg_var_loop_depth`, which guards the loop-counter case; this guards
    /// the block-local-declaration case.
    ///
    /// Distinct from `local_var_regs`, which ALSO holds `set_process_locals`
    /// bindings — those do have an interpreter home and must not block a
    /// fallback.
    decl_local_regs: std::collections::HashSet<String>,
    /// Names imported from an already-active process frame. These registers
    /// are read-only for the compiled statement; writes leave compilation so
    /// the process interpreter can preserve full local-lifetime semantics.
    process_local_names: HashSet<String>,
    /// Depth of enclosing loops whose counter lives in a VM REGISTER
    /// (`for (int i = ...)`). While > 0, StmtFallback emission is FORBIDDEN:
    /// the AST interpreter cannot see VM registers, so a fallback statement
    /// inside such a loop silently reads the loop var as X. Any unsupported
    /// construct must instead fail the whole loop back to the AST path.
    reg_var_loop_depth: u32,
    /// Expression-level fallback is only sound where surrounding analysis
    /// doesn't need to SEE the expression's reads: edge blocks, whose
    /// sensitivity is the explicit clock list. Comb entries build their
    /// wake-up graph from LoadSignal scans, so a read hidden inside an
    /// interpreted fragment would stop the entry re-firing. Off by default;
    /// enabled only at the edge-block compile site.
    allow_expr_fallback: bool,
    /// User-task table for inlining zero-arg, non-blocking task bodies.
    /// Task-enable (`task_name;`) statements that resolve here get their
    /// bodies compiled in place instead of emitting a single StmtFallback
    /// for the whole call — lets the inner simple assigns compile cleanly
    /// and narrows the fallback to just the inner $write/$display.
    tasks: Option<&'a HashMap<String, TaskDeclaration>>,
    functions: Option<&'a HashMap<String, FunctionDeclaration>>,
    inlining_stack: Vec<String>,
    pub tasks_inlined: u32,
    /// Elaborated module parameters — used by `eval_const_expr` so that
    /// bytecode compilation can fold module params (e.g. `CARRY_CHAIN`) into
    /// the compile-time widths of `+:` / `-:` range selects.
    params: Option<&'a HashMap<String, Value>>,
    /// Leaf-segment index over `params` (last dotted segment -> full keys).
    /// The suffix-match fallback in `lookup_param_value` otherwise scans the
    /// WHOLE param map with memcmp per entry on every miss — 94% of the
    /// C910 SoC's 410s compile phase.
    param_leaf_idx: Option<&'a HashMap<String, Vec<String>>>,
    /// Typedef name -> total width (module + package scope), for local
    /// declarations of typedef'd packed types inside inlined functions.
    typedefs: Option<&'a HashMap<String, u32>>,
    /// Typedef name -> packed ELEMENT width (`typedef u8_t [15:0] v_t` -> 8).
    typedef_elems: Option<&'a HashMap<String, u32>>,
    /// Register-backed local -> packed element width, for `local[i]`
    /// splice/extract compilation.
    pub local_var_elem: std::collections::HashMap<String, u32>,
    /// Loop variables bound to COMPILE-TIME constants by the unroller. Read
    /// before every other name source, so `state[col*4]` folds to a constant
    /// index while `col` is unrolled.
    local_const_vars: HashMap<String, Value>,
    /// Register-bank locals: a small fixed-size local unpacked array whose
    /// elements live in consecutive VM registers. name -> (base reg, element
    /// width, length, declared lo index). Only CONSTANT indexes can address a
    /// bank — the registers have no runtime indexing — so any dynamic access
    /// fails the enclosing compile, which rolls back to the AST path.
    local_array_regs: HashMap<String, (RegId, u32, usize, i64)>,
    /// Packed-struct field layout of the CURRENT assignment's destination,
    /// installed by the assign arms around their rvalue compile so an
    /// `'{...}` assignment pattern can compile to a Concat. An assignment
    /// pattern is otherwise context-free at expression level — its meaning
    /// depends entirely on the target type.
    pattern_layout: Option<Vec<(String, u32, u32)>>,
    /// Named-cast targets known at compile time: name -> (width, signed).
    /// See the simulator's `cast_widths`.
    cast_widths: Option<&'a HashMap<String, (u32, bool)>>,
    /// Top-module name (e.g. "tb"). When a hierarchical identifier reads a
    /// signal whose absolute path is `<top>.<rest>` (e.g. xezim's
    /// port-rewriting baked the top name into a cross-hierarchical
    /// reference) the signal table actually stores the leaf as `<rest>`,
    /// because top-level instances have no prefix in the elaborated map.
    /// `lookup_signal_id` strips this prefix before re-trying the lookup
    /// to recover from those baked-in absolute paths.
    pub top_module_name: Option<String>,
    /// Per-signal packed-element width for multi-D packed vectors
    /// (e.g. `logic [3:0][7:0] x` → elem_w=8). Used by `compile_blocking_target`
    /// so that `x[i] = v` emits a 8-bit slice write at `i*8 +: 8` instead of
    /// the default bit-select-write (`BlockingAssignBitDyn`) which only sets
    /// bit `i` and silently drops the upper bits. Set via
    /// `set_packed_elem_widths`.
    packed_elem_widths: Option<&'a HashMap<String, u32>>,
    /// Declared element width of each associative array (§10.7). Without it an
    /// assoc lvalue fell through `infer_lhs_width` to the 1-bit "bit-select on
    /// a plain packed signal" default, so a compiled `aa[k] <= v` truncated the
    /// value to a single bit.
    assoc_elem_widths: Option<&'a HashMap<String, u32>>,
    /// Names of ASSOCIATIVE arrays. Their keys are not dense indices and their
    /// elements have no signal ids, so none of the bytecode store paths can
    /// address them: `lookup_array_name` misses (they are not in `arrays`), and
    /// the fall-through treated the base as a scalar and wrote a BIT of a
    /// phantom signal — an `aa[k] = v` inside an `always_ff` was silently lost
    /// (`exists()` stayed 0) while the same write from an `initial` block, which
    /// runs on the AST path, worked. Detected here so the statement bails to
    /// that AST path instead.
    assoc_arrays: Option<&'a HashMap<String, bool>>,
    /// Declared packed dimensions (outermost first) per signal, from the
    /// elaborated model. Needed because a packed element's LSB offset is
    /// `(idx - low_bound) * elem_w` for a DESCENDING range — the plain
    /// `idx * elem_w` is only correct for a normalized `[N-1:0]` range and
    /// mis-places (or drops) elements of e.g. `[2:1]` or an ascending `[0:1]`.
    packed_full_dims: Option<&'a HashMap<String, Vec<(i64, i64)>>>,
    /// Stack of pending `break` jump-target patches, one entry per enclosing
    /// loop. When the loop's end address is known we rewrite each `Jump(0)`
    /// at these insn-indices to the loop-exit address. LRM §12.7.
    loop_break_patches: Vec<Vec<usize>>,
    /// Same stack-of-Vecs shape, but for `continue` — patched to the loop's
    /// step (or condition-recheck) address.
    loop_continue_patches: Vec<Vec<usize>>,
    /// Set of signal names declared as `string` (LRM §6.16). When a
    /// concatenation involves any of these, the bytecode bails to the AST
    /// interpreter, which has byte-level (not bit-level) concat semantics.
    /// Set via `set_string_signals`. None = no string info available, in
    /// which case the compiler can only catch the literal-operand case.
    string_signals: Option<&'a HashSet<String>>,
    /// Recursion guard for `fn_is_pure_in`: its `Call` arm recurses into the
    /// CALLEE's purity, so a self- or mutually-recursive function walked the
    /// call graph forever and overflowed the stack (an `assign w = fact(6);`
    /// with a recursive `fact` aborted the process; the same call from an
    /// initial block was fine because only the compiled path asks about
    /// purity). Past the cap the answer is "not pure", which merely keeps the
    /// call on the AST interpreter — where recursion already works.
    purity_depth: std::cell::Cell<u32>,
    /// Local/formal names (in the current inline scope) bound to STRING
    /// values — drives `%s` semantics and resize suppression. Name-based on
    /// purpose: register ids are recycled across arms, names are not.
    local_var_is_string: HashSet<String>,
    /// Local/formal names (in the current inline scope) bound to REAL values
    /// (§13.3.1). Assignments into these coerce numerically via
    /// `emit_to_real` — a plain register bind would leave an integral actual
    /// as integer bits and `t / 2` style body arithmetic would truncate.
    local_var_is_real: HashSet<String>,
    /// SMALL fixed-shape local arrays of an inlined pure body (`real row
    /// [0:3]`): one register per element, values kept WHOLE (real elements
    /// stay real — no bit packing). A dynamic index compiles to a
    /// compare/branch chain over the elements.
    local_var_array: HashMap<String, LocalArrayBind>,
    /// Loop variables currently bound to a compile-time CONSTANT (an
    /// unrolled `foreach` iteration). Consulted by `eval_const_expr`, so an
    /// index like `drivers[i]` folds to a direct element register instead of
    /// a compare/branch chain.
    const_var_binds: HashMap<String, u64>,
    /// Active inline return context: (result register — None for a void
    /// function or task —, resize width; 0 = none, e.g. strings). `return`
    /// compiles to result-move + jump; the jump indexes collect in
    /// `inline_ret_jumps` and the inliner patches them to its body end.
    inline_ret: Option<(Option<RegId>, u32)>,
    inline_ret_jumps: Vec<usize>,
    /// Base names of 2D/ND UNPACKED arrays. When a continuous-assign LHS
    /// `m[0][j]` targets one of these, the flattening short-circuit
    /// (`flattened_outer_const_signal_id`) must NOT fire — the
    /// bogus scalar signal `m` would otherwise catch a bit-select write and
    /// silently drop the element. None = no info (older callers); the guard
    /// then only excludes 1D/packed bases as before. Set via
    /// `set_multi_dim_arrays`.
    multi_dim_arrays: Option<&'a HashSet<String>>,
    /// 2-D unpacked array shapes (`module.arrays_2d`): base ->
    /// ((lo1,hi1),(lo2,hi2),elem_w). Elements are materialized contiguously
    /// row-major from `base[lo1][lo2]`, so a DYNAMIC `a[i][j]` read compiles
    /// to a bounds-checked flat index over one Dense operand.
    arrays_2d: Option<&'a HashMap<String, ((i64, i64), (i64, i64), u32)>>,
    /// Names the elaborator recorded as DYNAMIC arrays / QUEUES. Used only to
    /// keep their element STORES off the dense array fast path (see
    /// `collection_store_denied`); their READS are unaffected.
    dynamic_arrays: Option<&'a HashSet<String>>,
    queue_vars: Option<&'a HashSet<String>>,
    /// Packed-struct field layout: container name → ordered
    /// `(member, lsb_offset, width)`. Lets a member-write LHS like
    /// `s.m0` (parsed as a 2-segment `Ident(["<scope>.s", "m0"])` after
    /// submodule inlining) compile to a constant bit-range write into the
    /// container signal, instead of bailing to the AST interpreter — where
    /// its read dependency would resolve bare-first to the wrong (top-scope)
    /// input and never re-trigger when the real scoped input changes. Set via
    /// `set_packed_struct_fields`.
    packed_struct_fields: Option<&'a HashMap<String, Vec<(String, u32, u32)>>>,
}

impl<'a> BytecodeCompiler<'a> {
    pub fn new(
        signal_name_to_id: &'a HashMap<Arc<str>, usize>,
        signal_signed: &'a [bool],
        signal_widths: &'a [u32],
        arrays: &'a HashMap<String, (i64, i64, u32)>,
        widths: &'a HashMap<String, u32>,
    ) -> Self {
        Self {
            insns: Vec::with_capacity(64),
            next_reg: 0,
            register_overflow: false,
            signal_name_to_id,
            signal_signed,
            signal_widths,
            signal_real: None,
            arrays,
            array_first_id: None,
            widths,
            bail_reason: None,
            allow_ast_fallback: false,
            scope_hint: None,
            allow_waits: false,
            wait_specs: Vec::new(),
            for_loop_var_ids: std::collections::HashMap::default(),
            local_var_regs: std::collections::HashMap::default(),
            decl_local_regs: std::collections::HashSet::default(),
            process_local_names: HashSet::default(),
            reg_var_loop_depth: 0,
            allow_expr_fallback: false,
            tasks: None,
            functions: None,
            inlining_stack: Vec::new(),
            tasks_inlined: 0,
            params: None,
            param_leaf_idx: None,
            typedefs: None,
            typedef_elems: None,
            local_var_elem: std::collections::HashMap::new(),
            cast_widths: None,
            pattern_layout: None,
            local_const_vars: HashMap::default(),
            local_array_regs: HashMap::default(),
            top_module_name: None,
            packed_elem_widths: None,
            assoc_elem_widths: None,
            assoc_arrays: None,
            packed_full_dims: None,
            loop_break_patches: Vec::new(),
            loop_continue_patches: Vec::new(),
            string_signals: None,
            local_var_is_string: HashSet::default(),
            local_var_is_real: HashSet::default(),
            local_var_array: HashMap::default(),
            const_var_binds: HashMap::default(),
            purity_depth: std::cell::Cell::new(0),
            inline_ret: None,
            inline_ret_jumps: Vec::new(),
            multi_dim_arrays: None,
            arrays_2d: None,
            dynamic_arrays: None,
            queue_vars: None,
            packed_struct_fields: None,
        }
    }

    /// Make the active process's innermost frame visible to a compiled,
    /// non-suspending statement. Each local is loaded once on block entry and
    /// then behaves like any other register-backed operand.
    pub fn set_process_locals(&mut self, locals: &HashMap<String, Value>) {
        for (name, value) in locals {
            let reg = self.alloc_reg();
            self.emit(Insn::LoadProcessLocal(reg, name.clone().into_boxed_str()));
            self.local_var_regs
                .insert(name.clone(), (reg, value.width));
            self.process_local_names.insert(name.clone());
        }
    }

    pub fn set_packed_struct_fields(
        &mut self,
        f: &'a HashMap<String, Vec<(String, u32, u32)>>,
    ) {
        self.packed_struct_fields = Some(f);
    }

    /// If `hier` names a packed-struct member (`base.member`, where the base
    /// resolves to a container signal with a registered field layout), return
    /// `(container_signal_id, lsb_offset, member_width)`. The base may be a
    /// single segment (`s`) or already scope-qualified with a dot inside the
    /// first path segment (`d1.s`) after submodule inlining; the member is the
    /// final path segment.
    fn packed_struct_member_target(
        &self,
        hier: &HierarchicalIdentifier,
    ) -> Option<(usize, u32, u32)> {
        let fields_map = self.packed_struct_fields?;
        if hier.path.len() < 2 || hier.path.iter().any(|s| !s.selects.is_empty()) {
            return None;
        }
        let seg = |i: usize| hier.path[i].name.name.as_str();
        // A NESTED member (`s.p.hi`, and every `union`-in-struct shape) is
        // flattened by elaboration into one dotted key — "p.hi" — stored under
        // the ROOT signal. Splitting only the last segment therefore missed
        // every member at depth >= 2 and sent it to the AST path. Walk the
        // split point from the longest base down, so a genuinely hierarchical
        // base that IS a signal (`top.dut.sig.field`) still wins over
        // reinterpreting part of it as a member path.
        for k in (1..hier.path.len()).rev() {
            let base: String = (0..k).map(seg).collect::<Vec<_>>().join(".");
            let member: String = (k..hier.path.len()).map(seg).collect::<Vec<_>>().join(".");
            // Resolve the container signal id, honoring scope_hint for a bare base.
            let Some(base_id) = self.lookup_signal_id_by_name(&base).or_else(|| {
                self.scope_hint
                    .as_ref()
                    .and_then(|sc| self.lookup_signal_id_by_name(&format!("{}.{}", sc, base)))
            }) else {
                continue;
            };
            // Field layout is keyed by both the bare and scope-qualified base name.
            let Some(fields) = fields_map.get(base.as_str()).or_else(|| {
                self.scope_hint
                    .as_ref()
                    .and_then(|sc| fields_map.get(&format!("{}.{}", sc, base)))
            }) else {
                continue;
            };
            if let Some((_, off, w)) = fields.iter().find(|(m, _, _)| *m == member) {
                return Some((base_id, *off, *w));
            }
        }
        None
    }

    /// Resolve a packed-struct container and clone its flattened field layout.
    /// The clone keeps later instruction emission free to mutably borrow the
    /// compiler without retaining a borrow into elaboration metadata.
    fn packed_struct_layout_for_hier(
        &self,
        hier: &HierarchicalIdentifier,
    ) -> Option<(String, Vec<(String, u32, u32)>)> {
        if hier.path.iter().any(|s| !s.selects.is_empty()) {
            return None;
        }
        let fields_map = self.packed_struct_fields?;
        let raw = Self::hier_raw_name(hier);
        let mut candidates = Vec::with_capacity(3);
        candidates.push(raw.clone());
        if !raw.contains('.') && let Some(scope) = &self.scope_hint {
            candidates.push(format!("{}.{}", scope, raw));
        }
        if let Some(leaf) = hier.path.last() {
            candidates.push(leaf.name.name.clone());
        }
        for candidate in candidates {
            if let Some(fields) = fields_map.get(candidate.as_str()) {
                return Some((candidate, fields.clone()));
            }
        }
        None
    }

    /// Compile either a packed-struct signal or a selected element of a packed
    /// array of structs, retaining the root layout used to interpret members.
    fn compile_packed_struct_value(
        &mut self,
        expr: &Expression,
    ) -> Option<(
        RegId,
        String,
        HierarchicalIdentifier,
        Vec<(String, u32, u32)>,
    )> {
        let mut unwrapped = expr;
        while let ExprKind::Paren(inner) = &unwrapped.kind {
            unwrapped = inner;
        }
        let hier = match &unwrapped.kind {
            ExprKind::Ident(hier) => hier.clone(),
            ExprKind::Index { expr: base, .. } => {
                let ExprKind::Ident(hier) = &base.kind else {
                    return None;
                };
                hier.clone()
            }
            _ => return None,
        };
        let (key, fields) = self.packed_struct_layout_for_hier(&hier)?;
        let value = self.compile_expr(unwrapped, 0)?;
        Some((value, key, hier, fields))
    }

    /// Metadata for a packed-array member of a packed struct. The elaborated
    /// tables key member dimensions as `<container>.<member>`.
    fn packed_member_array_shape(
        &self,
        root_key: &str,
        hier: &HierarchicalIdentifier,
        member: &str,
    ) -> Option<(u32, Option<(i64, i64)>)> {
        let widths = self.packed_elem_widths?;
        let raw = Self::hier_raw_name(hier);
        let mut candidates = Vec::with_capacity(4);
        candidates.push(format!("{}.{}", root_key, member));
        candidates.push(format!("{}.{}", raw, member));
        if !raw.contains('.') && let Some(scope) = &self.scope_hint {
            candidates.push(format!("{}.{}.{}", scope, raw, member));
        }
        if let Some(leaf) = hier.path.last() {
            candidates.push(format!("{}.{}", leaf.name.name, member));
        }
        for candidate in candidates {
            if let Some(&elem_w) = widths.get(candidate.as_str()).filter(|&&w| w > 0) {
                let dim = self
                    .packed_full_dims
                    .and_then(|dims| dims.get(candidate.as_str()))
                    .and_then(|dims| dims.first())
                    .copied();
                return Some((elem_w, dim));
            }
        }
        None
    }

    /// Emit a dynamic element/field slice from a packed struct container.
    fn emit_packed_member_slice(
        &mut self,
        root: RegId,
        index: &Expression,
        dim: Option<(i64, i64)>,
        stride: u32,
        base_offset: u32,
        width: u32,
    ) -> Option<RegId> {
        let index = self.compile_expr(index, 0)?;
        let slot = self.emit_packed_slot_index(dim, index);
        let stride_reg = self.alloc_reg();
        self.emit(Insn::LoadConst(
            stride_reg,
            Box::new(Value::from_u64(stride as u64, 32)),
        ));
        let dynamic_offset = self.alloc_reg();
        self.emit(Insn::Mul(dynamic_offset, slot, stride_reg));
        let lo = if base_offset == 0 {
            dynamic_offset
        } else {
            let base = self.alloc_reg();
            self.emit(Insn::LoadConst(
                base,
                Box::new(Value::from_u64(base_offset as u64, 32)),
            ));
            let lo = self.alloc_reg();
            self.emit(Insn::Add(lo, dynamic_offset, base));
            lo
        };
        let hi = if width == 1 {
            lo
        } else {
            let delta = self.alloc_reg();
            self.emit(Insn::LoadConst(
                delta,
                Box::new(Value::from_u64((width - 1) as u64, 32)),
            ));
            let hi = self.alloc_reg();
            self.emit(Insn::Add(hi, lo, delta));
            hi
        };
        let dest = self.alloc_reg();
        self.emit(Insn::RangeSelect(dest, root, hi, lo));
        Some(dest)
    }

    /// Compile `container.array_member[index]` when the member is a packed
    /// array embedded in a packed struct.
    fn compile_packed_member_index(
        &mut self,
        base: &Expression,
        index: &Expression,
    ) -> Option<RegId> {
        let ExprKind::MemberAccess { expr: root, member } = &base.kind else {
            return None;
        };
        // `a.m1[i].m2[j]`: the container is itself an element of an
        // array-of-struct member. Slice that element out first, then the
        // member lane within it (offsets inside the element are relative to
        // the element-0 layout). Without this the inner member read compiled
        // to a value and the outer index took ONE BIT of it.
        if let ExprKind::Index { expr: mid, index: idx1 } = &root.kind
            && let ExprKind::MemberAccess { expr: r2, member: m1 } = &mid.kind
        {
            let (v, key, hier, fields) = self.compile_packed_struct_value(r2)?;
            let &(_, m1_off, _) = fields.iter().find(|(n, _, _)| n == &m1.name)?;
            let (stride1, dim1) = self.packed_member_array_shape(&key, &hier, &m1.name)?;
            let elem = self.emit_packed_member_slice(v, idx1, dim1, stride1, m1_off, stride1)?;
            let (_, mem_off, _) = Self::elem_zero_field(&fields, &m1.name, &member.name)?;
            let rel = mem_off.checked_sub(m1_off)?;
            let nested = format!("{}.{}", m1.name, member.name);
            let (elem_w, dim2) = self.packed_member_array_shape(&key, &hier, &nested)?;
            return self.emit_packed_member_slice(elem, index, dim2, elem_w, rel, elem_w);
        }
        let (root_value, root_key, hier, fields) =
            self.compile_packed_struct_value(root)?;
        let (_, field_offset, _) = fields.iter().find(|(name, _, _)| name == &member.name)?;
        let (elem_w, dim) = self.packed_member_array_shape(&root_key, &hier, &member.name)?;
        self.emit_packed_member_slice(
            root_value,
            index,
            dim,
            elem_w,
            *field_offset,
            elem_w,
        )
    }

    /// Compile `container.array_member[index].field` using the flattened
    /// packed-struct layout plus the member array's element stride.
    fn compile_indexed_packed_member(
        &mut self,
        indexed: &Expression,
        leaf: &str,
    ) -> Option<RegId> {
        let ExprKind::Index { expr: base, index } = &indexed.kind else {
            return None;
        };
        // Inside an inlined instance the member access is folded into the
        // identifier (`cw.loc.descriptors[i].region` arrives as the single
        // path `cw.loc.descriptors`), so the shape below never matched and
        // every such read ran interpreted.
        if let ExprKind::Ident(hier) = &base.kind {
            if hier.root.is_some()
                || hier.path.len() < 2
                || hier.path.iter().any(|s| !s.selects.is_empty())
            {
                return None;
            }
            let mut root_hier = hier.clone();
            let member = root_hier.path.pop()?.name.name;
            return self.compile_indexed_folded_member(&root_hier, &member, index, leaf);
        }
        let ExprKind::MemberAccess { expr: root, member } = &base.kind else {
            return None;
        };
        let (root_value, root_key, hier, fields) =
            self.compile_packed_struct_value(root)?;
        let (_, field_offset, field_width) =
            Self::elem_zero_field(&fields, &member.name, leaf)?;
        let (elem_w, dim) = self.packed_member_array_shape(&root_key, &hier, &member.name)?;
        self.emit_packed_member_slice(
            root_value,
            index,
            dim,
            elem_w,
            field_offset,
            field_width,
        )
    }

    /// Element-0 layout of `<member>[i].<leaf>`. Elaboration registers an
    /// array-of-struct member under two conventions: element-relative
    /// (`descriptors.region`, plain struct nets) or expanded per element
    /// (`descriptors[0].region`, packed arrays of structs). Both put element
    /// 0 at the member's base, so either key gives the stride origin.
    fn elem_zero_field(
        fields: &[(String, u32, u32)],
        member: &str,
        leaf: &str,
    ) -> Option<(String, u32, u32)> {
        let rel = format!("{}.{}", member, leaf);
        let per_elem = format!("{}[0].{}", member, leaf);
        fields
            .iter()
            .find(|(name, _, _)| name == &rel || name == &per_elem)
            .cloned()
    }

    /// `<path>.<member>[i].<leaf>` where the container and member arrive as
    /// one folded identifier: split the member off the path, resolve the
    /// container's layout under the remaining name, and slice like the
    /// member-access shape.
    fn compile_indexed_folded_member(
        &mut self,
        root_hier: &HierarchicalIdentifier,
        member: &str,
        index: &Expression,
        leaf: &str,
    ) -> Option<RegId> {
        if root_hier.root.is_some() || root_hier.path.is_empty() {
            return None;
        }
        // The container may itself be an element select (`grid[1]`): the
        // layout is keyed by the bare name, the value is the element.
        let mut bare = root_hier.clone();
        for seg in &mut bare.path {
            seg.selects.clear();
        }
        let (root_key, fields) = self.packed_struct_layout_for_hier(&bare)?;
        let (_, field_offset, field_width) = Self::elem_zero_field(&fields, member, leaf)?;
        let (elem_w, dim) = self.packed_member_array_shape(&root_key, &bare, member)?;
        let root = if root_hier.path.iter().any(|s| !s.selects.is_empty()) {
            let e = Expression::new(ExprKind::Ident(root_hier.clone()), root_hier.span);
            self.compile_expr(&e, 0)?
        } else {
            let base_id = self.lookup_signal_id_by_name(&root_key)?;
            let root = self.alloc_reg();
            self.emit(Insn::LoadSignal(root, as_sig_id(base_id)));
            root
        };
        self.emit_packed_member_slice(root, index, dim, elem_w, field_offset, field_width)
    }

    pub fn set_string_signals(&mut self, s: &'a HashSet<String>) {
        self.string_signals = Some(s);
    }

    /// Supply per-signal `is_real` so `elide_redundant_resizes` can prove a
    /// loaded signal is not real. `Value::add` and friends special-case a real
    /// operand (returning a 64-bit `from_f64`), so without this every
    /// arithmetic result on a signal is "possibly real" and its `Resize`
    /// survives — measured at ~337K of the ~553K resizes still executing.
    pub fn set_signal_real(&mut self, r: &'a [bool]) {
        self.signal_real = Some(r);
    }

    pub fn set_multi_dim_arrays(&mut self, s: &'a HashSet<String>) {
        self.multi_dim_arrays = Some(s);
    }

    pub fn set_arrays_2d(&mut self, m: &'a HashMap<String, ((i64, i64), (i64, i64), u32)>) {
        self.arrays_2d = Some(m);
    }

    pub fn set_collection_denies(&mut self, dyn_arrays: &'a HashSet<String>, queues: &'a HashSet<String>) {
        self.dynamic_arrays = Some(dyn_arrays);
        self.queue_vars = Some(queues);
    }

    /// Must an element STORE to this name avoid the dense array fast path?
    ///
    /// A dynamic array / queue element carries a signal TWIN, so
    /// `lookup_array_name` resolved it and the store wrote that id --- but a
    /// reader of the COLLECTION does not depend on it, so nothing was ever
    /// re-evaluated and readers silently served stale (or x) data. Bailing
    /// sends the enclosing block to the interpreter, whose `assign_value` /
    /// queued-lvalue NBA commit notify correctly --- the same route
    /// ASSOCIATIVE arrays already take just above. Static arrays keep the fast
    /// path: their element ids ARE what readers depend on.
    fn collection_store_denied(&self, hier: &HierarchicalIdentifier) -> bool {
        let (Some(d), Some(q)) = (self.dynamic_arrays, self.queue_vars) else {
            return false;
        };
        let raw = Self::hier_raw_name(hier);
        if d.contains(&raw) || q.contains(&raw) {
            return true;
        }
        if let Some(scope) = &self.scope_hint {
            let qual = format!("{}.{}", scope, raw);
            if d.contains(&qual) || q.contains(&qual) {
                return true;
            }
        }
        hier.path
            .last()
            .is_some_and(|s| d.contains(&s.name.name) || q.contains(&s.name.name))
    }

    pub fn set_array_first_id(&mut self, arrays: &'a HashMap<Arc<str>, (usize, i64, i64)>) {
        self.array_first_id = Some(arrays);
    }

    fn array_operand(&self, name: String) -> Box<ArrayOperand> {
        if let Some(&(first_id, lo, hi)) = self
            .array_first_id
            .and_then(|arrays| arrays.get(name.as_str()))
        {
            Box::new(ArrayOperand::Dense {
                name,
                first_id,
                lo,
                hi,
            })
        } else {
            Box::new(ArrayOperand::Named(name))
        }
    }

    pub fn set_params(&mut self, params: &'a HashMap<String, Value>) {
        self.params = Some(params);
    }

    pub fn set_param_leaf_idx(&mut self, idx: &'a HashMap<String, Vec<String>>) {
        self.param_leaf_idx = Some(idx);
    }

    pub fn set_packed_elem_widths(&mut self, w: &'a HashMap<String, u32>) {
        self.packed_elem_widths = Some(w);
    }

    pub fn set_typedefs(
        &mut self,
        widths: &'a HashMap<String, u32>,
        elems: &'a HashMap<String, u32>,
    ) {
        self.typedefs = Some(widths);
        self.typedef_elems = Some(elems);
    }

    pub fn set_assoc_elem_widths(&mut self, w: &'a HashMap<String, u32>) {
        self.assoc_elem_widths = Some(w);
    }

    pub fn set_assoc_arrays(&mut self, a: &'a HashMap<String, bool>) {
        self.assoc_arrays = Some(a);
    }

    /// Does this identifier name an associative array (in any of the spellings
    /// the lvalue paths try)?
    fn is_assoc_target(&self, hier: &HierarchicalIdentifier) -> bool {
        let Some(m) = self.assoc_arrays else {
            return false;
        };
        let raw = Self::hier_raw_name(hier);
        if m.contains_key(&raw) {
            return true;
        }
        if let Some(scope) = &self.scope_hint {
            if m.contains_key(&format!("{}.{}", scope, raw)) {
                return true;
            }
        }
        hier.path
            .last()
            .is_some_and(|s| m.contains_key(&s.name.name))
    }

    pub fn set_packed_full_dims(&mut self, d: &'a HashMap<String, Vec<(i64, i64)>>) {
        self.packed_full_dims = Some(d);
    }

    /// The declared OUTERMOST packed dimension `(left, right)` of the signal
    /// named by `hier`, if recorded. Same raw / last-segment lookup the
    /// `packed_elem_widths` sites use.
    fn packed_outer_dim(&self, hier: &HierarchicalIdentifier) -> Option<(i64, i64)> {
        self.packed_full_dims_of(hier).and_then(|d| d.first().copied())
    }

    /// Declared packed dimensions of `hier`: exact name, then the name under
    /// the current instance scope, then — for a SINGLE-segment name only —
    /// the bare leaf. A multi-segment name (a struct member, a hierarchical
    /// reference) must never match an unrelated same-named declaration
    /// elsewhere in the design (see `packed_elem_width_of`).
    fn packed_full_dims_of(&self, hier: &HierarchicalIdentifier) -> Option<&'a Vec<(i64, i64)>> {
        let raw = Self::hier_raw_name(hier);
        let m = self.packed_full_dims?;
        m.get(raw.as_str())
            .or_else(|| {
                self.scope_hint
                    .as_ref()
                    .and_then(|sc| m.get(format!("{}.{}", sc, raw).as_str()))
            })
            .or_else(|| {
                if hier.path.len() != 1 {
                    return None;
                }
                hier.path.last().and_then(|s| m.get(s.name.name.as_str()))
            })
    }

    /// LSB bit offset of packed element `idx` given the declared outer
    /// dimension. `[N-1:0]` reduces to the historical `idx * elem_w`.
    fn packed_elem_lsb(dim: Option<(i64, i64)>, idx: i64, elem_w: u32) -> i64 {
        let Some((l, r)) = dim else {
            return idx * elem_w as i64;
        };
        let (lo_b, hi_b) = (l.min(r), l.max(r));
        let count = hi_b - lo_b + 1;
        let off = idx - lo_b;
        // A descending range labels the LEFT bound as the most-significant
        // element; an ascending one reverses the slot order (§7.4.1).
        let slot = if l >= r { off } else { count - 1 - off };
        slot * elem_w as i64
    }

    /// Emit the register holding the element's *slot* index (already
    /// normalized to 0-based, LSB-first) for a DYNAMIC index. Returns the
    /// original register unchanged for a normalized `[N-1:0]` range.
    fn emit_packed_slot_index(&mut self, dim: Option<(i64, i64)>, idx_reg: RegId) -> RegId {
        let Some((l, r)) = dim else { return idx_reg };
        let (lo_b, hi_b) = (l.min(r), l.max(r));
        if l >= r {
            if lo_b == 0 {
                return idx_reg; // normalized [N-1:0]
            }
            let lo_reg = self.alloc_reg();
            self.emit(Insn::LoadConst(lo_reg, Box::new(Value::from_u64(lo_b as u64, 32))));
            let out = self.alloc_reg();
            self.emit(Insn::Sub(out, idx_reg, lo_reg));
            out
        } else {
            // ascending: slot = (count-1) - (idx - lo_b) = (count-1+lo_b) - idx
            let count = hi_b - lo_b + 1;
            let k = self.alloc_reg();
            self.emit(Insn::LoadConst(
                k,
                Box::new(Value::from_u64((count - 1 + lo_b) as u64, 32)),
            ));
            let out = self.alloc_reg();
            self.emit(Insn::Sub(out, k, idx_reg));
            out
        }
    }

    pub fn set_ast_fallback(&mut self, allow: bool) {
        self.allow_ast_fallback = allow;
    }

    pub fn set_expr_fallback(&mut self, allow: bool) {
        self.allow_expr_fallback = allow;
    }

    pub fn set_scope_hint(&mut self, scope: Option<String>) {
        self.scope_hint = scope;
    }

    pub fn set_cast_widths(&mut self, m: &'a HashMap<String, (u32, bool)>) {
        self.cast_widths = Some(m);
    }

    pub fn set_functions(&mut self, functions: &'a HashMap<String, FunctionDeclaration>) {
        self.functions = Some(functions);
    }

    pub fn set_tasks(&mut self, tasks: &'a HashMap<String, TaskDeclaration>) {
        self.tasks = Some(tasks);
    }

    /// Static-only heuristic: does this expression CLEARLY produce a string?
    /// Used to bail string-concat to the interpreter (which has byte-level
    /// concat semantics). We can only see syntactic clues at compile time —
    /// the full `string_signals` set lives on the simulator, not the
    /// bytecode compiler. A string-literal operand is always a string; a
    /// `$sformatf` / `$psprintf` call returns a string. Bare idents whose
    /// declared type we don't have here remain false — those cases get
    /// folded into the bit-vector concat path, which is the existing
    /// behavior. The interpreter side's special-case is what carries the
    /// LRM-correct path when the compiler can't see the type.
    fn expr_is_string_concat_operand(&self, e: &Expression) -> bool {
        match &e.kind {
            ExprKind::StringLiteral(_) => true,
            ExprKind::Paren(inner) => self.expr_is_string_concat_operand(inner),
            ExprKind::Concatenation(parts) => {
                parts.iter().any(|p| self.expr_is_string_concat_operand(p))
            }
            ExprKind::SystemCall { name, .. } => matches!(name.as_str(), "$sformatf" | "$psprintf"),
            ExprKind::Ident(h) => {
                if let Some(set) = self.string_signals {
                    let last = h.path.last().map(|s| s.name.name.as_str()).unwrap_or("");
                    if set.contains(last) {
                        return true;
                    }
                    // Try scope-qualified form too.
                    if let Some(scope) = &self.scope_hint {
                        let q = format!("{}.{}", scope, last);
                        if set.contains(&q) {
                            return true;
                        }
                    }
                }
                false
            }
            _ => false,
        }
    }

    fn stmt_has_break_or_continue(stmt: &Statement) -> bool {
        match &stmt.kind {
            StatementKind::Break | StatementKind::Continue => true,
            StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
                stmts.iter().any(Self::stmt_has_break_or_continue)
            }
            StatementKind::If {
                then_stmt,
                else_stmt,
                ..
            } => {
                Self::stmt_has_break_or_continue(then_stmt)
                    || else_stmt
                        .as_ref()
                        .is_some_and(|e| Self::stmt_has_break_or_continue(e))
            }
            StatementKind::Case { items, .. } => items
                .iter()
                .any(|it| Self::stmt_has_break_or_continue(&it.stmt)),
            // Don't descend into nested loops — break/continue there target the
            // inner loop, not the enclosing one.
            _ => false,
        }
    }

    fn stmt_is_blocking(stmt: &Statement) -> bool {
        match &stmt.kind {
            StatementKind::TimingControl { .. } => true,
            StatementKind::Wait { .. } => true,
            StatementKind::Forever { .. } => true,
            StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
                stmts.iter().any(Self::stmt_is_blocking)
            }
            StatementKind::If {
                then_stmt,
                else_stmt,
                ..
            } => {
                Self::stmt_is_blocking(then_stmt)
                    || else_stmt
                        .as_ref()
                        .is_some_and(|e| Self::stmt_is_blocking(e))
            }
            StatementKind::For { body, .. } | StatementKind::While { body, .. } => {
                Self::stmt_is_blocking(body)
            }
            _ => false,
        }
    }

    /// Try to inline a zero-arg, non-blocking user task's body at this
    /// call site. Returns true if successfully inlined.
    /// Inline a call to a pure combinational function, yielding the register
    /// holding its result. Accepts only argument-contained functions: input
    /// formals and directly writable `ref` actuals, with no external reads.
    fn compile_pure_call(
        &mut self,
        func: &Expression,
        args: &[Expression],
        ctx_width: u32,
    ) -> Option<RegId> {
        let name = match &func.kind {
            ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.is_empty() => {
                h.path[0].name.name.clone()
            }
            _ => {
                self.bail("Expr_Call");
                return None;
            }
        };
        if self.inlining_stack.len() >= MAX_INLINE_DEPTH
            || self.inlining_stack.iter().any(|n| *n == name)
        {
            self.bail("Expr_Call_depth");
            return None;
        }
        let Some(fd) = self.functions.and_then(|f| f.get(&name)).cloned() else {
            self.bail("Expr_Call");
            return None;
        };
        // §6.6.7 resolver dispatch: a DYNAMIC-ARRAY formal (`input real
        // drivers[]`) whose actual is a FIXED assignment pattern is
        // monomorphic at this call site — the nettype resolution machinery
        // synthesizes one Ordered element per driver — so it binds as a
        // fixed local array of per-element registers (the #129 machinery).
        // This was the single largest RNM cost: every node re-ran the
        // resolver on the AST path each settle (issue #137, ~25µs/eval for
        // a 4-term multiply-accumulate).
        let dyn_array_formal = |p: &_, a: &Expression| -> bool {
            matches!(p, &crate::ast::decl::FunctionPort { .. })
                && matches!(
                    &a.kind,
                    ExprKind::AssignmentPattern(items)
                        if (1..=32).contains(&items.len())
                            && items.iter().all(|it| matches!(
                                it,
                                crate::ast::expr::AssignmentPatternItem::Ordered(_)
                            ))
                )
        };
        let is_unsized_dim = |p: &crate::ast::decl::FunctionPort| {
            matches!(
                p.dimensions.as_slice(),
                [crate::ast::types::UnpackedDimension::Unsized(_)]
            ) && matches!(p.direction, PortDirection::Input)
        };
        if fd.ports.len() != args.len()
            || fd
                .ports
                .iter()
                .zip(args)
                .any(|(p, a)| {
                    // §13.5.2 output formals ride the existing ref-writeback
                    // machinery: the body sees a register, the caller's actual
                    // is written on return. AES-style helpers
                    // (`mix_column(c0.., r0..)`) are exactly this shape.
                    (!matches!(
                        p.direction,
                        PortDirection::Input | PortDirection::Ref | PortDirection::Output
                    ) || !p.dimensions.is_empty())
                        && !(is_unsized_dim(p) && dyn_array_formal(p, a))
                })
        {
            self.bail("Expr_Call_ports");
            return None;
        }
        // REAL-typed formals or return: register binding resizes through
        // integral widths, which destroys real semantics (an integral actual
        // must CONVERT to the real formal per §13.3.1, not bit-copy). The
        // AST call path does this correctly; stay on it.
        let dt_is_real = |dt: &crate::ast::types::DataType| {
            crate::compiler::elaborate::is_type_real(dt)
        };
        // REAL formals/return are admitted with §13.3.1 conversion at the
        // register bind (`emit_to_real`); `Value` arithmetic and the store
        // paths are real-aware, and `resize(64)` is identity on a real. Only
        // ref/output REAL formals stay on the AST path — their write-back
        // would need the reverse conversion against the actual's own type.
        let ret_is_real = dt_is_real(&fd.return_type);
        if fd.ports.iter().any(|p| {
            dt_is_real(&p.data_type)
                && !matches!(p.direction, PortDirection::Input)
        }) {
            self.bail("Expr_Call_real_ref");
            return None;
        }
        // Only inline a function that is PURE IN ITS ARGUMENTS: every name its
        // body reads must be a formal, one of its own locals, or a constant.
        // A function that reads module signals must NOT be inlined here — the
        // elaborator registers an instance's functions under BOTH the bare and
        // the instance-qualified name, so a bare-name lookup can pick the
        // un-rewritten copy whose free names belong to another scope. It also
        // keeps the AST path's sensitivity handling (which follows a callee's
        // reads) authoritative for such functions.
        // Module-signal READS are admissible when the free names resolve
        // unambiguously: a DOTTED registration's body was rewritten to
        // instance-qualified (absolute) names, and a bare registration with
        // no dotted twin can only be the top module's own copy (§13.4.2 —
        // the value may then depend on module state, and the CA dependency
        // machinery follows callee reads, so re-evaluation still triggers).
        // Writes to module state stay disqualifying either way.
        let allow_ext_reads = name.contains('.')
            || self.functions.is_some_and(|f| {
                let suffix = format!(".{}", name);
                !f.keys().any(|k| k.ends_with(suffix.as_str()))
            });
        if !self.fn_is_pure_in_ext(
            &fd,
            name.rsplit_once('.').map(|(p, _)| p),
            allow_ext_reads,
        ) {
            if std::env::var_os("XEZIM_PROBE_INLINE").is_some() {
                eprintln!("[INLINE-FAIL] fn {} reason=impure", name);
            }
            self.bail("Expr_Call_impure");
            return None;
        }
        // Unwrap the body, through one level of begin/end.
        let items: Vec<Statement> = match fd.items.as_slice() {
            [one] => match &one.kind {
                StatementKind::SeqBlock { stmts, .. } => stmts.clone(),
                _ => vec![one.clone()],
            },
            other => other.to_vec(),
        };
        // Nothing that suspends, and no nested calls we cannot see through.
        if items.iter().any(Self::stmt_is_blocking) {
            self.bail("Expr_Call_blocking");
            return None;
        }
        let ret_is_string = matches!(
            fd.return_type,
            crate::ast::types::DataType::Simple {
                kind: crate::ast::types::SimpleType::String,
                ..
            }
        );
        let ret_w = if ret_is_string {
            0
        } else {
            // Resolve through the typedef table (and the instance scope): the
            // old bare `resolve_type_width(.., None)` call defaulted every
            // typedef'd return type to 32 bits, silently truncating a wide
            // result on `return`.
            self.decl_width_in(&fd.return_type, Some(&name))
        };
        // Evaluate the arguments in the CALLER's scope first, then bind them as
        // register-backed locals while compiling the body.
        let mut binds: Vec<(String, (RegId, u32))> = Vec::with_capacity(args.len());
        let mut elem_binds: Vec<(String, u32)> = Vec::new();
        let mut ref_writes: Vec<(Expression, RegId, u32)> = Vec::new();
        let mut string_formal_names: Vec<String> = Vec::new();
        let mut real_formal_names: Vec<String> = Vec::new();
        let mut array_binds: Vec<(String, LocalArrayBind)> = Vec::new();
        for (i, (p, a)) in fd.ports.iter().zip(args).enumerate() {
            if is_unsized_dim(p) {
                // Dynamic-array formal from a fixed assignment pattern: one
                // register per element, evaluated in the CALLER's scope. The
                // gate above already proved the shape.
                let ExprKind::AssignmentPattern(items) = &a.kind else {
                    self.bail("Expr_Call_ports");
                    return None;
                };
                let is_real = dt_is_real(&p.data_type);
                let elem_w = if is_real {
                    64
                } else {
                    self.decl_width_in(&p.data_type, Some(&name))
                };
                if elem_w == 0 || elem_w > 64 {
                    self.bail("Expr_Call_ports");
                    return None;
                }
                let mut regs = Vec::with_capacity(items.len());
                for it in items {
                    let crate::ast::expr::AssignmentPatternItem::Ordered(e) = it else {
                        self.bail("Expr_Call_ports");
                        return None;
                    };
                    let slot = self.alloc_reg();
                    let v = self.compile_expr(e, if is_real { 0 } else { elem_w })?;
                    self.emit(Insn::Move(slot, v));
                    if is_real {
                        // §13.3.1: each element CONVERTS to the real
                        // element type, exactly like a scalar real formal.
                        self.emit_to_real(slot);
                    } else {
                        self.emit(Insn::Resize(slot, elem_w));
                    }
                    regs.push(slot);
                }
                array_binds.push((
                    p.name.name.clone(),
                    LocalArrayBind { regs, lo: 0, elem_w, is_real },
                ));
                continue;
            }
            if matches!(p.direction, PortDirection::Ref)
                && (matches!(&a.kind, ExprKind::Ident(h) if self.local_var_reg_of(h).is_some())
                    || self.expr_to_signal_id(a).is_none())
            {
                self.bail("Expr_Call_ref_target");
                return None;
            }
            let formal_is_string = matches!(
                &p.data_type,
                crate::ast::types::DataType::Simple {
                    kind: crate::ast::types::SimpleType::String,
                    ..
                }
            );
            let w = if formal_is_string {
                0
            } else {
                self.port_effective_width(&fd.ports, i, Some(&name))
            };
            let formal_is_real = dt_is_real(&p.data_type);
            let slot = self.alloc_reg();
            if matches!(p.direction, PortDirection::Output) {
                // §13.5.3: an output formal does NOT read the actual; it
                // starts at its type's default and is copied out on return.
                let init = self.type_default_value(&p.data_type, w);
                self.emit(Insn::LoadConst(slot, Box::new(init)));
            } else {
                let v = self.compile_expr(a, w)?;
                self.emit(Insn::Move(slot, v));
            }
            if w > 0 && !formal_is_real {
                self.emit(Insn::Resize(slot, w));
            }
            if formal_is_real {
                // §13.3.1: an integral actual CONVERTS to the real formal.
                self.emit_to_real(slot);
            }
            if formal_is_string {
                string_formal_names.push(p.name.name.clone());
            }
            if formal_is_real {
                real_formal_names.push(p.name.name.clone());
            }
            binds.push((p.name.name.clone(), (slot, w)));
            if let Some(ew) = self.decl_elem_width_in(&p.data_type, Some(&name)) {
                if ew > 0 && w > ew && w % ew == 0 {
                    elem_binds.push((p.name.name.clone(), ew));
                }
            }
            if matches!(p.direction, PortDirection::Ref | PortDirection::Output) {
                ref_writes.push((a.clone(), slot, w));
            }
        }
        // Elaboration rewrites an instantiated module's function body to
        // instance-qualified names, so the body assigns `u0.onehot` and reads
        // `u0.c` while these bindings are keyed on the bare spelling. Bind
        // BOTH, or the inlined body's own result variable resolves to no
        // signal and the enclosing statement bails.
        let qpfx = name.rsplit_once('.').map(|(p, _)| p.to_string());
        let saved_locals = std::mem::take(&mut self.local_var_regs);
        let saved_local_strings = std::mem::take(&mut self.local_var_is_string);
        let saved_local_reals = std::mem::take(&mut self.local_var_is_real);
        let saved_local_arrays = std::mem::take(&mut self.local_var_array);
        let saved_local_elems = std::mem::take(&mut self.local_var_elem);
        for (n, ab) in array_binds {
            if let Some(pfx) = &qpfx {
                self.local_var_array.insert(format!("{pfx}.{n}"), ab.clone());
            }
            self.local_var_array.insert(n, ab);
        }
        for (n, b) in binds {
            if let Some(pfx) = &qpfx {
                self.local_var_regs.insert(format!("{pfx}.{n}"), b);
            }
            self.local_var_regs.insert(n, b);
        }
        for (n, ew) in elem_binds {
            if let Some(pfx) = &qpfx {
                self.local_var_elem.insert(format!("{pfx}.{n}"), ew);
            }
            self.local_var_elem.insert(n, ew);
        }
        for n in &string_formal_names {
            if let Some(pfx) = &qpfx {
                self.local_var_is_string.insert(format!("{pfx}.{n}"));
            }
            self.local_var_is_string.insert(n.clone());
        }
        for n in &real_formal_names {
            if let Some(pfx) = &qpfx {
                self.local_var_is_real.insert(format!("{pfx}.{n}"));
            }
            self.local_var_is_real.insert(n.clone());
        }
        if ret_is_real {
            if let Some(pfx) = &qpfx {
                self.local_var_is_real
                    .insert(format!("{}.{}", pfx, fd.name.name.name));
            }
            self.local_var_is_real.insert(fd.name.name.name.clone());
        }
        if ret_is_string {
            if let Some(pfx) = &qpfx {
                self.local_var_is_string
                    .insert(format!("{}.{}", pfx, fd.name.name.name));
            }
            self.local_var_is_string.insert(fd.name.name.name.clone());
        }
        // The function's own name is its return variable (§13.4.1): give it a
        // register too, so a body that assigns it (possibly across several
        // statements) works exactly like the single-assignment form.
        let ret_slot = self.alloc_reg();
        let ret_init = self.type_default_value(&fd.return_type, ret_w);
        self.emit(Insn::LoadConst(ret_slot, Box::new(ret_init)));
        if let Some(pfx) = &qpfx {
            self.local_var_regs
                .insert(format!("{}.{}", pfx, fd.name.name.name), (ret_slot, ret_w));
        }
        self.local_var_regs
            .insert(fd.name.name.name.clone(), (ret_slot, ret_w));
        self.inlining_stack.push(name);
        // AST fallback MUST be off inside an inlined body. `emit_fallback`
        // defers a statement to the AST interpreter, which resolves names
        // through the signal tables — but this body's locals (and the return
        // variable) live in REGISTERS that the interpreter cannot see. A
        // deferred statement therefore reads and writes the wrong storage and
        // its effect is silently lost: a pure helper whose accumulator is
        // updated in a `for` loop returned its initial value instead of the
        // sum, with no fallback counted and no diagnostic. If any statement
        // will not compile, the whole inline has to fail so the caller uses
        // the ordinary (correct) call path.
        let saved_fallback = self.allow_ast_fallback;
        self.allow_ast_fallback = false;
        let saved_ret = self.inline_ret;
        let saved_ret_jumps = std::mem::take(&mut self.inline_ret_jumps);
        self.inline_ret = Some((Some(ret_slot), ret_w));
        let mut ok = self.compile_pure_body(&items, ret_slot, ret_w, ctx_width);
        // Early returns land HERE — after the body, before the output-formal
        // copy-out, which §13.5.3 performs on every return path.
        let body_end = self.insns.len() as u32;
        for j in std::mem::take(&mut self.inline_ret_jumps) {
            self.insns[j] = Insn::Jump(body_end);
        }
        self.inline_ret = saved_ret;
        self.inline_ret_jumps = saved_ret_jumps;
        if ok {
            for (target, value, width) in &ref_writes {
                if !self.compile_blocking_target(target, *value, *width) {
                    ok = false;
                    break;
                }
            }
        }
        self.allow_ast_fallback = saved_fallback;
        self.inlining_stack.pop();
        self.local_var_regs = saved_locals;
        self.local_var_is_string = saved_local_strings;
        self.local_var_is_real = saved_local_reals;
        self.local_var_array = saved_local_arrays;
        self.local_var_elem = saved_local_elems;
        if !ok {
            if std::env::var_os("XEZIM_PROBE_INLINE").is_some() {
                eprintln!(
                    "[INLINE-FAIL] fn {} reason={}",
                    fd.name.name.name,
                    self.bail_reason.unwrap_or("unknown")
                );
            }
            return None;
        }
        if ret_w > 0 && !ret_is_real {
            self.emit(Insn::Resize(ret_slot, ret_w));
        }
        Some(ret_slot)
    }

    /// Compile the statements of an inlined pure function. Local `VarDecl`s
    /// become register-backed locals; a `return <expr>` assigns the return
    /// register (only valid as the final statement, which is the shape any
    /// combinational helper uses).
    fn compile_pure_body(
        &mut self,
        items: &[Statement],
        ret_slot: RegId,
        ret_w: u32,
        ctx_width: u32,
    ) -> bool {
        for (idx, st) in items.iter().enumerate() {
            match &st.kind {
                StatementKind::VarDecl {
                    data_type,
                    declarators,
                    ..
                } => {
                    for d in declarators {
                        let is_real =
                            crate::compiler::elaborate::is_type_real(data_type);
                        if !d.dimensions.is_empty() {
                            // A SMALL fixed-shape local array (`real row
                            // [0:3]`) — the working-buffer shape every
                            // interpolation helper uses — becomes one
                            // register per element; a dynamic index selects
                            // by compare/branch chain (see
                            // `compile_local_array_read`). Anything else
                            // (queues, big or non-constant shapes) keeps the
                            // AST path.
                            if d.init.is_some()
                                || !self.bind_local_array(&d.name.name, &d.dimensions, data_type, is_real)
                            {
                                self.bail("Expr_Call_local_array");
                                return false;
                            }
                            continue;
                        }
                        let is_string = matches!(
                            data_type,
                            crate::ast::types::DataType::Simple {
                                kind: crate::ast::types::SimpleType::String,
                                ..
                            }
                        );
                        let w = if is_string { 0 } else { self.decl_width(data_type) };
                        let slot = self.alloc_reg();
                        match &d.init {
                            Some(e) => {
                                let Some(v) = self.compile_expr(e, w) else {
                                    return false;
                                };
                                self.emit(Insn::Move(slot, v));
                            }
                            None => {
                                let init = self.type_default_value(data_type, w);
                                self.emit(Insn::LoadConst(slot, Box::new(init)));
                            }
                        }
                        if is_real {
                            self.local_var_is_real.insert(d.name.name.clone());
                            self.emit_to_real(slot);
                        } else if w > 0 {
                            self.emit(Insn::Resize(slot, w));
                        }
                        if is_string {
                            self.local_var_is_string.insert(d.name.name.clone());
                        }
                        if let Some(ew) = self.decl_elem_width(data_type) {
                            if ew > 0 && w > ew && w % ew == 0 {
                                self.local_var_elem.insert(d.name.name.clone(), ew);
                            }
                        }
                        self.local_var_regs.insert(d.name.name.clone(), (slot, w));
                    }
                }
                // `return` — early or final — compiles via compile_stmt's
                // Return arm (result move + jump patched by the caller).
                StatementKind::Return(_) => {
                    if !self.compile_stmt(st) {
                        return false;
                    }
                }
                _ => {
                    if !self.compile_stmt(st) {
                        return false;
                    }
                }
            }
        }
        true
    }

    fn try_inline_task(&mut self, task_name: &str) -> bool {
        self.try_inline_task_args(task_name, &[])
    }

    /// Inline a non-blocking task call, arguments included. Input formals
    /// bind the actual's value to a register-backed local; output formals get
    /// a default-initialized register and are copied back to the caller's
    /// lvalue on return (§13.5.3 — an output does not read its actual).
    /// Everything is rolled back on failure, so a task the compiler cannot
    /// handle costs nothing and keeps the AST path.
    ///
    /// This is what lets a task-structured FSM compile at all: an AES round
    /// (`sub_bytes(); shift_rows(); mix_columns(); add_round_key(round+1);`)
    /// was ONE interpreted 280µs block per clock because a single call with
    /// an argument bailed the whole always_ff.
    fn try_inline_task_args(&mut self, task_name: &str, args: &[Expression]) -> bool {
        if self.inlining_stack.len() >= MAX_INLINE_DEPTH {
            return false;
        }
        if self.inlining_stack.iter().any(|n| n == task_name) {
            return false;
        }
        // §13.4.1: a VOID function called as a statement is a task enable in
        // all but name — inline it through the same machinery. (Non-void
        // returns would need a result variable; those stay on the AST.)
        let (ports, body): (Vec<crate::ast::decl::FunctionPort>, Vec<Statement>) =
            if let Some(td) = self.tasks.and_then(|t| t.get(task_name)) {
                (td.ports.clone(), td.items.clone())
            } else if let Some(fd) = self.functions.and_then(|f| f.get(task_name)) {
                if !matches!(fd.return_type, crate::ast::types::DataType::Void(_)) {
                    return false;
                }
                (fd.ports.clone(), fd.items.clone())
            } else {
                return false;
            };
        if ports.len() != args.len()
            || ports.iter().any(|p| {
                !matches!(p.direction, PortDirection::Input | PortDirection::Output)
                    || !p.dimensions.is_empty()
            })
        {
            return false;
        }
        // Process-FSM mode inlines BLOCKING task bodies too — their waits
        // compile to Wait insns like any other statement, and the register-
        // backed formals live in the process frame across suspensions.
        if !self.allow_waits && body.iter().any(Self::stmt_is_blocking) {
            return false;
        }
        let start = self.insns.len();
        let start_reg = self.next_reg;

        // Evaluate input actuals in the CALLER's scope, before any binding.
        let mut binds: Vec<(String, (RegId, u32))> = Vec::with_capacity(ports.len());
        let mut out_writes: Vec<(Expression, RegId, u32)> = Vec::new();
        let mut ok = true;
        let mut string_formals: Vec<String> = Vec::new();
        for (i, (p, a)) in ports.iter().zip(args).enumerate() {
            // §6.16 string formal: no declared width, so no Resize — a
            // resize would truncate the front of the text.
            let is_string = matches!(
                &p.data_type,
                crate::ast::types::DataType::Simple {
                    kind: crate::ast::types::SimpleType::String,
                    ..
                }
            );
            let w = if is_string {
                0
            } else {
                self.port_effective_width(&ports, i, Some(task_name))
            };
            let slot = self.alloc_reg();
            if matches!(p.direction, PortDirection::Output) {
                if is_string {
                    ok = false;
                    break;
                }
                let init = self.type_default_value(&p.data_type, w);
                self.emit(Insn::LoadConst(slot, Box::new(init)));
                out_writes.push((a.clone(), slot, w));
            } else {
                match self.compile_expr(a, w) {
                    Some(v) => self.emit(Insn::Move(slot, v)),
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            if w > 0 {
                self.emit(Insn::Resize(slot, w));
            }
            if is_string {
                string_formals.push(p.name.name.clone());
            }
            binds.push((p.name.name.clone(), (slot, w)));
        }

        if ok {
            // Formals shadow for the body only. The qualified spellings cover
            // elaboration's instance-rewritten bodies, same as pure calls.
            let qpfx = task_name.rsplit_once('.').map(|(p, _)| p.to_string());
            let saved_locals = self.local_var_regs.clone();
            let saved_local_strings = self.local_var_is_string.clone();
            for (n, b) in &binds {
                if let Some(pfx) = &qpfx {
                    self.local_var_regs.insert(format!("{pfx}.{n}"), *b);
                }
                self.local_var_regs.insert(n.clone(), *b);
            }
            for n in &string_formals {
                if let Some(pfx) = &qpfx {
                    self.local_var_is_string.insert(format!("{pfx}.{n}"));
                }
                self.local_var_is_string.insert(n.clone());
            }
            // No AST fallback inside: formals live in registers the
            // interpreter cannot see, so a deferred statement would read and
            // write the wrong storage (silently). All-or-nothing.
            let saved_fallback = self.allow_ast_fallback;
            self.allow_ast_fallback = false;
            self.inlining_stack.push(task_name.to_string());
            let saved_ret = self.inline_ret;
            let saved_ret_jumps = std::mem::take(&mut self.inline_ret_jumps);
            self.inline_ret = Some((None, 0));
            for st in &body {
                if !self.compile_stmt(st) {
                    ok = false;
                    break;
                }
            }
            // `return;` sites jump here, BEFORE the §13.5.3 output copy-out.
            let body_end = self.insns.len() as u32;
            for j in std::mem::take(&mut self.inline_ret_jumps) {
                self.insns[j] = Insn::Jump(body_end);
            }
            self.inline_ret = saved_ret;
            self.inline_ret_jumps = saved_ret_jumps;
            if ok {
                for (target, value, width) in &out_writes {
                    if !self.compile_blocking_target(target, *value, *width) {
                        ok = false;
                        break;
                    }
                }
            }
            self.inlining_stack.pop();
            self.allow_ast_fallback = saved_fallback;
            self.local_var_regs = saved_locals;
            self.local_var_is_string = saved_local_strings;
        }
        if !ok {
            if std::env::var_os("XEZIM_PROBE_INLINE").is_some() {
                eprintln!(
                    "[INLINE-FAIL] task {} reason={}",
                    task_name,
                    self.bail_reason.unwrap_or("unknown")
                );
            }
            // A half-emitted body writes REAL signals — roll everything back.
            self.insns.truncate(start);
            self.next_reg = start_reg;
            return false;
        }
        self.tasks_inlined += 1;
        true
    }

    /// Conservative structural test for loop bodies that the NEW for-loop
    /// compilation capabilities (register-backed `for (int i...)` vars and
    /// signal-backed `i++` steps) are allowed to handle. Nested indexing,
    /// member access, and non-assign statements go through addressing paths
    /// whose register/loop-var handling is not yet audited — those loops
    /// keep the old whole-loop AST fallback. Never regresses: these bodies
    /// always fell back before the new capabilities existed.
    /// Is this expression compilable inside a register-backed loop body?
    /// No member access anywhere; everything else resolves through
    /// `compile_expr`'s normal paths.
    fn expr_loop_simple(&self, e: &Expression) -> bool {
        match &e.kind {
            // Packed-struct member reads are lowered to bit slices. Unsupported
            // member shapes still fail strict compilation later, which rolls
            // the whole register-backed loop back to the interpreter.
            ExprKind::MemberAccess { expr, .. } => self.expr_loop_simple(expr),
            ExprKind::Unary { operand, .. } | ExprKind::Paren(operand) => {
                self.expr_loop_simple(operand)
            }
            ExprKind::Binary { left, right, .. } => {
                self.expr_loop_simple(left) && self.expr_loop_simple(right)
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.expr_loop_simple(condition)
                    && self.expr_loop_simple(then_expr)
                    && self.expr_loop_simple(else_expr)
            }
            ExprKind::Index { expr, index } => {
                self.expr_loop_simple(expr) && self.expr_loop_simple(index)
            }
            ExprKind::RangeSelect { expr, left, right, .. } => {
                self.expr_loop_simple(expr)
                    && self.expr_loop_simple(left)
                    && self.expr_loop_simple(right)
            }
            ExprKind::SystemCall { args, .. } | ExprKind::Concatenation(args) => {
                args.iter().all(|a| self.expr_loop_simple(a))
            }
            // A call to a function `compile_pure_call` can INLINE is fine:
            // it becomes ordinary expression code with no call at all. This
            // gate used to reject every call, so one helper in a loop body
            // (`vec[i] <= onehot(code[i])` — a ubiquitous RTL shape) dragged
            // the whole loop, and every instance of the enclosing module,
            // onto the AST interpreter. Being optimistic is safe: a body that
            // fails to compile later bails the loop exactly as before,
            // because `StmtFallback` cannot be emitted while
            // `reg_var_loop_depth > 0`.
            ExprKind::Call { func, args } => {
                args.iter().all(|a| self.expr_loop_simple(a))
                    && self.call_is_inlinable(func, args)
            }
            _ => true,
        }
    }

    /// Would `compile_pure_call` accept this call? Mirrors its admission
    /// tests (single-segment callee, arity, supported scalar formals,
    /// argument-pure body, nothing that suspends) so the loop gate and the
    /// compiler agree on what "call-free after inlining" means.
    fn call_is_inlinable(&self, func: &Expression, args: &[Expression]) -> bool {
        let ExprKind::Ident(h) = &func.kind else {
            return false;
        };
        if h.path.len() != 1 || !h.path[0].selects.is_empty() {
            return false;
        }
        let Some(fd) = self.functions.and_then(|f| f.get(&h.path[0].name.name)) else {
            return false;
        };
        if fd.ports.len() != args.len()
            || fd.ports.iter().any(|p| {
                !matches!(p.direction, PortDirection::Input | PortDirection::Ref)
                    || !p.dimensions.is_empty()
            })
        {
            return false;
        }
        let pfx = h.path[0].name.name.rsplit_once('.').map(|(p, _)| p);
        if !self.fn_is_pure_in(fd, pfx) {
            return false;
        }
        let items: Vec<Statement> = match fd.items.as_slice() {
            [one] => match &one.kind {
                StatementKind::SeqBlock { stmts, .. } => stmts.clone(),
                _ => vec![one.clone()],
            },
            other => other.to_vec(),
        };
        !items.iter().any(Self::stmt_is_blocking)
    }

    fn for_body_is_simple(&self, stmt: &Statement) -> bool {
        let expr_simple = |e: &Expression| -> bool { self.expr_loop_simple(e) };
        let lv_simple = |e: &Expression| -> bool {
            match &e.kind {
                ExprKind::Ident(h) => h.path.iter().all(|s| s.selects.is_empty()),
                ExprKind::Index { expr, index } => {
                    (matches!(&expr.kind, ExprKind::Ident(h)
                        if h.path.iter().all(|s| s.selects.is_empty()))
                        // `a[i][j]` on a 2-D unpacked array — the store arm
                        // lowers it to the same row-major flat index the read
                        // uses.
                        || matches!(&expr.kind, ExprKind::Index { expr: b, index: bi }
                            if matches!(&b.kind, ExprKind::Ident(h)
                                if h.path.iter().all(|s| s.selects.is_empty()))
                                && expr_simple(bi)))
                        && expr_simple(index)
                }
                // `arr[i].m` / `s.m` on a packed struct: the assign arms
                // splice the member's static bit range, so the same two base
                // shapes above are what they can resolve.
                ExprKind::MemberAccess { expr, .. } => match &expr.kind {
                    ExprKind::Ident(h) => h.path.iter().all(|s| s.selects.is_empty()),
                    ExprKind::Index { expr: base, index } => {
                        matches!(&base.kind, ExprKind::Ident(h)
                            if h.path.iter().all(|s| s.selects.is_empty()))
                            && expr_simple(index)
                    }
                    _ => false,
                },
                _ => false,
            }
        };
        fn lv_base_name(e: &Expression) -> Option<&str> {
            match &e.kind {
                ExprKind::Index { expr, .. } => match &expr.kind {
                    ExprKind::Ident(h) => h.path.last().map(|s| s.name.name.as_str()),
                    // 2-D: peel the outer index and ask again, or widening
                    // lv_simple above smuggles `m[i][j] <= m[i][j]+1` past the
                    // alias guard.
                    ExprKind::Index { .. } => lv_base_name(expr),
                    _ => None,
                },
                // See through `.m` so `arr[i].m <= arr[i].m + 1` still reaches
                // the self-read audit below — widening lv_simple without this
                // would let exactly the aliasing case it guards slip through.
                ExprKind::MemberAccess { expr, .. } => lv_base_name(expr),
                _ => None,
            }
        }
        // Full dotted form of an Index lvalue's base ident.
        fn lv_base_full(e: &Expression) -> Option<String> {
            match &e.kind {
                ExprKind::Index { expr, .. } => match &expr.kind {
                    ExprKind::Ident(h) => Some(
                        h.path
                            .iter()
                            .map(|s| s.name.name.as_str())
                            .collect::<Vec<_>>()
                            .join("."),
                    ),
                    ExprKind::Index { .. } => lv_base_full(expr),
                    _ => None,
                },
                ExprKind::MemberAccess { expr, .. } => lv_base_full(expr),
                _ => None,
            }
        }
        // Does the rvalue read leaf `name` through a DIFFERENT dotted form
        // than `full`? Same-form self-reads (`cnt[b] <= cnt[b]+1`) resolve
        // through the identical lookup and cannot skew; the audited hazard
        // was an elaboration-rewritten dotted lvalue paired with a bare read.
        fn expr_reads_name_other_form(e: &Expression, name: &str, full: &str) -> bool {
            match &e.kind {
                ExprKind::Ident(h) => {
                    if h.path.last().is_some_and(|s| s.name.name == name) {
                        let f = h
                            .path
                            .iter()
                            .map(|s| s.name.name.as_str())
                            .collect::<Vec<_>>()
                            .join(".");
                        f != full
                    } else {
                        false
                    }
                }
                ExprKind::Unary { operand, .. } | ExprKind::Paren(operand) => {
                    expr_reads_name_other_form(operand, name, full)
                }
                ExprKind::Binary { left, right, .. } => {
                    expr_reads_name_other_form(left, name, full)
                        || expr_reads_name_other_form(right, name, full)
                }
                ExprKind::Conditional { condition, then_expr, else_expr } => {
                    expr_reads_name_other_form(condition, name, full)
                        || expr_reads_name_other_form(then_expr, name, full)
                        || expr_reads_name_other_form(else_expr, name, full)
                }
                ExprKind::Index { expr, index } => {
                    expr_reads_name_other_form(expr, name, full)
                        || expr_reads_name_other_form(index, name, full)
                }
                ExprKind::RangeSelect { expr, left, right, .. } => {
                    expr_reads_name_other_form(expr, name, full)
                        || expr_reads_name_other_form(left, name, full)
                        || expr_reads_name_other_form(right, name, full)
                }
                ExprKind::SystemCall { args, .. } | ExprKind::Concatenation(args) => {
                    args.iter().any(|a| expr_reads_name_other_form(a, name, full))
                }
                _ => false,
            }
        }
        fn expr_reads_name(e: &Expression, name: &str) -> bool {
            match &e.kind {
                ExprKind::Ident(h) => {
                    h.path.last().is_some_and(|s| s.name.name == name)
                }
                ExprKind::Unary { operand, .. } | ExprKind::Paren(operand) => {
                    expr_reads_name(operand, name)
                }
                ExprKind::Binary { left, right, .. } => {
                    expr_reads_name(left, name) || expr_reads_name(right, name)
                }
                ExprKind::Conditional { condition, then_expr, else_expr } => {
                    expr_reads_name(condition, name)
                        || expr_reads_name(then_expr, name)
                        || expr_reads_name(else_expr, name)
                }
                ExprKind::Index { expr, index } => {
                    expr_reads_name(expr, name) || expr_reads_name(index, name)
                }
                ExprKind::RangeSelect { expr, left, right, .. } => {
                    expr_reads_name(expr, name)
                        || expr_reads_name(left, name)
                        || expr_reads_name(right, name)
                }
                ExprKind::SystemCall { args, .. } | ExprKind::Concatenation(args) => {
                    args.iter().any(|a| expr_reads_name(a, name))
                }
                _ => false,
            }
        }
        match &stmt.kind {
            StatementKind::Null => true,
            StatementKind::VarDecl { declarators, .. } => declarators
                .iter()
                .all(|decl| decl.init.as_ref().map(&expr_simple).unwrap_or(true)),
            StatementKind::NonblockingAssign { lvalue, rvalue, .. }
            | StatementKind::BlockingAssign { lvalue, rvalue } => {
                // A SELF-READING array update (`ptr[i] <= ptr[i] + 1`) is
                // excluded: in an inlined instance the compiled read and
                // write paths can resolve the array through different
                // aliases (port copy vs local), skewing the pre-edge read.
                // Keep such loops on the audited AST path.
                let self_read_skewed = match (lv_base_name(lvalue), lv_base_full(lvalue)) {
                    (Some(n), Some(f)) => expr_reads_name_other_form(rvalue, n, &f),
                    _ => false,
                };
                let _ = expr_reads_name; // superseded by the form-aware audit
                lv_simple(lvalue) && expr_simple(rvalue) && !self_read_skewed
            }
            StatementKind::SeqBlock { stmts, .. } => {
                stmts.iter().all(|st| self.for_body_is_simple(st))
            }
            StatementKind::If {
                condition,
                then_stmt,
                else_stmt,
                ..
            } => {
                expr_simple(condition)
                    && self.for_body_is_simple(then_stmt)
                    && else_stmt
                        .as_ref()
                        .map(|e| self.for_body_is_simple(e))
                        .unwrap_or(true)
            }
            // Case over a simple selector with simple arm bodies. The arb
            // datapath loops a customer profile flagged (86% of wall time in
            // the For_init_vardecl AST fallback, 364µs/execution) are exactly
            // banks×entries nests of case-inside-for; a mid-body construct
            // the compiler still can't handle bails the WHOLE loop as before
            // (StmtFallback emission is forbidden inside reg-var loops), so
            // widening this filter can't smuggle a half-compiled body.
            StatementKind::Case { expr, items, .. } => {
                expr_simple(expr)
                    && items.iter().all(|it| {
                        it.pattern.is_none()
                            && it.patterns.iter().all(expr_simple)
                            && self.for_body_is_simple(&it.stmt)
                    })
            }
            // Nested `for` — both the assign-init and VarDecl-init forms.
            // The per-assign audits (simple lvalue, no member access, no
            // self-reading array update) apply recursively through the body.
            StatementKind::For { init, condition, step, body } => {
                let init_ok = init.iter().all(|fi| match fi {
                    crate::ast::stmt::ForInit::Assign { lvalue, rvalue } => {
                        lv_simple(lvalue) && expr_simple(rvalue)
                    }
                    crate::ast::stmt::ForInit::VarDecl { init, .. } => expr_simple(init),
                });
                let step_ok = step.iter().all(expr_simple);
                init_ok
                    && condition.as_ref().map(|c| expr_simple(c)).unwrap_or(true)
                    && step_ok
                    && self.for_body_is_simple(body)
            }
            _ => false,
        }
    }

    fn expr_has_sampled_value_call(e: &Expression) -> bool {
        let sub = |x: &Expression| Self::expr_has_sampled_value_call(x);
        match &e.kind {
            ExprKind::SystemCall { name, args } => {
                matches!(
                    name.as_str(),
                    "$past" | "$rose" | "$fell" | "$stable" | "$changed" | "$sampled"
                ) || args.iter().any(sub)
            }
            ExprKind::Unary { operand, .. } => sub(operand),
            ExprKind::Binary { left, right, .. } => sub(left) || sub(right),
            ExprKind::Conditional { condition, then_expr, else_expr } => {
                sub(condition) || sub(then_expr) || sub(else_expr)
            }
            ExprKind::Paren(i) => sub(i),
            ExprKind::Concatenation(items) => items.iter().any(sub),
            ExprKind::Replication { count, exprs } => {
                sub(count) || exprs.iter().any(sub)
            }
            ExprKind::Call { args, .. } => args.iter().any(sub),
            _ => false,
        }
    }

    /// Expression-level escape hatch (see Insn::EvalExprFallback). Returns
    /// None when forbidden (no ast-fallback, or register-backed locals are
    /// live and the interpreter couldn't see them).
    fn emit_expr_fallback(
        &mut self,
        e: &Expression,
        ctx_width: u32,
        reason: &'static str,
    ) -> Option<RegId> {
        if !self.allow_ast_fallback
            || !self.allow_expr_fallback
            || self.reg_var_loop_depth > 0
            || !self.local_var_regs.is_empty()
        {
            return None;
        }
        // Sampled-value functions ($past/$rose/...) take their clock from the
        // ENCLOSING block's inferred clocking — an isolated expression eval
        // has no block context, so the whole statement must fall back.
        if Self::expr_has_sampled_value_call(e) {
            return None;
        }
        let r = self.alloc_reg();
        self.emit(Insn::EvalExprFallback(
            Box::new((Arc::new(e.clone()), Arc::from(reason))),
            r,
            ctx_width,
        ));
        Some(r)
    }

    fn emit_fallback(&mut self, stmt: &Statement) -> bool {
        if !self.decl_local_regs.is_empty() {
            // See decl_local_regs — the interpreter has no storage for a
            // register-backed block local, so bail the whole block instead.
            return false;
        }
        if self.reg_var_loop_depth > 0 {
            // See reg_var_loop_depth — a fallback here would mis-read the
            // register-backed loop var; force the whole loop to bail.
            return false;
        }
        if self.allow_ast_fallback {
            let reason = self
                .bail_reason
                .unwrap_or_else(|| Self::stmt_kind_label(stmt));
            self.emit(Insn::StmtFallback(Box::new((
                Arc::new(stmt.clone()),
                Arc::from(reason),
            ))));
            true
        } else {
            false
        }
    }

    fn stmt_kind_label(stmt: &Statement) -> &'static str {
        match &stmt.kind {
            StatementKind::Null => "Stmt_Null",
            StatementKind::NonblockingAssign { .. } => "Stmt_Nba",
            StatementKind::BlockingAssign { .. } => "Stmt_Blk",
            StatementKind::If { .. } => "Stmt_If",
            StatementKind::Case { .. } => "Stmt_Case",
            StatementKind::SeqBlock { .. } => "Stmt_SeqBlock",
            StatementKind::ParBlock { .. } => "Stmt_ParBlock",
            StatementKind::Expr(_) => "Stmt_Expr",
            StatementKind::For { .. } => "Stmt_For",
            StatementKind::Foreach { .. } => "Stmt_Foreach",
            StatementKind::While { .. } => "Stmt_While",
            StatementKind::DoWhile { .. } => "Stmt_DoWhile",
            StatementKind::Repeat { .. } => "Stmt_Repeat",
            StatementKind::Forever { .. } => "Stmt_Forever",
            StatementKind::TimingControl { .. } => "Stmt_Timing",
            StatementKind::Wait { .. } => "Stmt_Wait",
            StatementKind::Assertion(_) => "Stmt_Assertion",
            StatementKind::VarDecl { .. } => "Stmt_VarDecl",
            _ => "Stmt_other",
        }
    }

    /// Register holding `hier` when it names a block-local variable.
    fn local_var_reg_of(&self, hier: &crate::ast::expr::HierarchicalIdentifier) -> Option<(RegId, u32)> {
        if self.local_var_regs.is_empty() || hier.path.len() != 1 {
            return None;
        }
        let seg = &hier.path[0];
        if !seg.selects.is_empty() {
            return None;
        }
        // A dotted name is normally a hierarchical reference and never a block
        // local — except for the instance-qualified spellings an inlined
        // function body uses for its OWN formals and result, which
        // `compile_pure_call` binds explicitly. `local_var_regs` holds only
        // names this compiler bound, so a hit here is by construction one of
        // those, not a design signal that happens to be dotted.
        self.local_var_regs.get(&seg.name.name).copied()
    }

    /// Bind a small fixed-shape local array to per-element registers. One
    /// dimension, constant bounds, at most 32 elements; every element starts
    /// at the element type's default. Returns false when the shape does not
    /// qualify (the caller bails to the AST path).
    fn bind_local_array(
        &mut self,
        name: &str,
        dims: &[crate::ast::types::UnpackedDimension],
        data_type: &crate::ast::types::DataType,
        is_real: bool,
    ) -> bool {
        use crate::ast::types::UnpackedDimension as UD;
        if dims.len() != 1 {
            return false;
        }
        let (lo, hi): (i64, i64) = match &dims[0] {
            UD::Range { left, right, .. } => {
                let (Some(l), Some(r)) = (
                    self.eval_const_expr(left).map(|v| v as i64),
                    self.eval_const_expr(right).map(|v| v as i64),
                ) else {
                    return false;
                };
                (l.min(r), l.max(r))
            }
            UD::Expression { expr: e, .. } => {
                let Some(n) = self.eval_const_expr(e).map(|v| v as i64) else {
                    return false;
                };
                if n <= 0 {
                    return false;
                }
                (0, n - 1)
            }
            _ => return false,
        };
        let count = hi - lo + 1;
        if !(1..=32).contains(&count) {
            return false;
        }
        let elem_w = self.decl_width(data_type);
        if elem_w == 0 || (elem_w > 64 && !is_real) {
            return false;
        }
        let default = self.type_default_value(data_type, elem_w);
        let mut regs = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let r = self.alloc_reg();
            self.emit(Insn::LoadConst(r, Box::new(default.clone())));
            regs.push(r);
        }
        self.local_var_array.insert(
            name.to_string(),
            LocalArrayBind { regs, lo, elem_w, is_real },
        );
        true
    }

    /// Element READ of a register-bound local array. A constant index picks
    /// the register directly; a dynamic one selects by compare/branch chain
    /// (out-of-range reads keep the element default — the type's x/0 — which
    /// matches §7.4.6 for the x case and is unreachable for well-formed
    /// loops).
    fn compile_local_array_read(&mut self, name: &str, idx: &Expression) -> Option<RegId> {
        let ab = self.local_var_array.get(name)?.clone();
        if let Some(k) = self.eval_const_expr(idx) {
            let i = (k as i64).checked_sub(ab.lo)?;
            let src = *ab.regs.get(usize::try_from(i).ok()?)?;
            let r = self.alloc_reg();
            self.emit(Insn::Move(r, src));
            return Some(r);
        }
        let idx_r = self.compile_expr(idx, 0)?;
        let out = self.alloc_reg();
        let default = if ab.is_real {
            Value::from_f64(0.0)
        } else {
            Value::new(ab.elem_w)
        };
        self.emit(Insn::LoadConst(out, Box::new(default)));
        let kreg = self.alloc_reg();
        let creg = self.alloc_reg();
        for (i, &er) in ab.regs.iter().enumerate() {
            self.emit(Insn::LoadConst(
                kreg,
                Box::new(Value::from_u64((ab.lo + i as i64) as u64, 32)),
            ));
            self.emit(Insn::Eq(creg, idx_r, kreg));
            let br = self.insns.len();
            self.emit(Insn::BranchIfFalse(creg, 0));
            self.emit(Insn::Move(out, er));
            let after = self.insns.len() as u32;
            self.insns[br] = Insn::BranchIfFalse(creg, after);
        }
        Some(out)
    }

    /// Element WRITE of a register-bound local array (same chain shape as the
    /// read). The stored value is coerced per the element type: real elements
    /// convert numerically, integral ones resize.
    fn compile_local_array_write(
        &mut self,
        name: &str,
        idx: &Expression,
        val_reg: RegId,
    ) -> Option<bool> {
        let ab = self.local_var_array.get(name)?.clone();
        // Coerce ONCE into a scratch, then move into the selected element.
        let v = self.alloc_reg();
        self.emit(Insn::Move(v, val_reg));
        if ab.is_real {
            self.emit_to_real(v);
        } else {
            self.emit(Insn::Resize(v, ab.elem_w));
        }
        if let Some(k) = self.eval_const_expr(idx) {
            let i = (k as i64).checked_sub(ab.lo)?;
            let dst = *ab.regs.get(usize::try_from(i).ok()?)?;
            self.emit(Insn::Move(dst, v));
            return Some(true);
        }
        let idx_r = self.compile_expr(idx, 0)?;
        let kreg = self.alloc_reg();
        let creg = self.alloc_reg();
        for (i, &er) in ab.regs.iter().enumerate() {
            self.emit(Insn::LoadConst(
                kreg,
                Box::new(Value::from_u64((ab.lo + i as i64) as u64, 32)),
            ));
            self.emit(Insn::Eq(creg, idx_r, kreg));
            let br = self.insns.len();
            self.emit(Insn::BranchIfFalse(creg, 0));
            self.emit(Insn::Move(er, v));
            let after = self.insns.len() as u32;
            self.insns[br] = Insn::BranchIfFalse(creg, after);
        }
        Some(true)
    }

    /// DYNAMIC element read of a 2-D unpacked array (`a[i][j]` with
    /// non-constant indices). Elements are materialized contiguously
    /// row-major (see the simulator's arrays_2d build loop), so the read is a
    /// bounds-checked flat index over one Dense operand:
    /// `flat = (i-lo1)*ncols + (j-lo2)`; an out-of-range index in EITHER
    /// dimension yields x (§7.4.6), which the per-dim guard folds into the
    /// flat index by driving it out of the Dense operand's own range.
    fn compile_2d_array_read(
        &mut self,
        hier: &crate::ast::expr::HierarchicalIdentifier,
        i_expr: &Expression,
        j_expr: &Expression,
    ) -> Option<RegId> {
        let (array, flat) = self.compile_2d_flat_index(hier, i_expr, j_expr)?;
        let dest = self.alloc_reg();
        self.emit(Insn::LoadArrayElem(dest, array, flat));
        Some(dest)
    }

    /// Shared row-major addressing for a DYNAMIC 2-D unpacked element
    /// (`a[i][j]`): returns the Dense operand and the register holding
    /// `flat = (i-lo1)*ncols + (j-lo2)`, with an out-of-range index in EITHER
    /// dimension forced one past the operand's range so the element access
    /// itself reports it (§7.4.6: read yields x, write is discarded).
    fn compile_2d_flat_index(
        &mut self,
        hier: &crate::ast::expr::HierarchicalIdentifier,
        i_expr: &Expression,
        j_expr: &Expression,
    ) -> Option<(Box<ArrayOperand>, RegId)> {
        let raw = Self::hier_raw_name(hier);
        let arrays_2d = self.arrays_2d?;
        let (key, ((lo1, hi1), (lo2, hi2), _w)) = arrays_2d
            .get(raw.as_str())
            .map(|s| (raw.clone(), *s))
            .or_else(|| {
                self.scope_hint.as_ref().and_then(|sc| {
                    let q = format!("{}.{}", sc, raw);
                    arrays_2d.get(q.as_str()).map(|s| (q, *s))
                })
            })?;
        if lo1 < 0 || lo2 < 0 {
            return None;
        }
        let first_id = *self
            .signal_name_to_id
            .get(format!("{}[{}][{}]", key, lo1, lo2).as_str())?;
        let ncols = hi2 - lo2 + 1;
        let count = (hi1 - lo1 + 1) * ncols;
        let iv = self.compile_expr(i_expr, 0)?;
        let jv = self.compile_expr(j_expr, 0)?;
        // In-range test per §7.4.6. The flat index is forced to `count`
        // (one past the Dense range) when either dimension is out of
        // range, and LoadArrayElem's own bounds check turns that into x.
        let lo1_r = self.alloc_reg();
        self.emit(Insn::LoadConst(lo1_r, Box::new(Value::from_u64(lo1 as u64, 32))));
        let hi1_r = self.alloc_reg();
        self.emit(Insn::LoadConst(hi1_r, Box::new(Value::from_u64(hi1 as u64, 32))));
        let lo2_r = self.alloc_reg();
        self.emit(Insn::LoadConst(lo2_r, Box::new(Value::from_u64(lo2 as u64, 32))));
        let hi2_r = self.alloc_reg();
        self.emit(Insn::LoadConst(hi2_r, Box::new(Value::from_u64(hi2 as u64, 32))));
        let ok = self.alloc_reg();
        let t = self.alloc_reg();
        self.emit(Insn::Geq(ok, iv, lo1_r));
        self.emit(Insn::Leq(t, iv, hi1_r));
        self.emit(Insn::BitAnd(ok, ok, t));
        self.emit(Insn::Geq(t, jv, lo2_r));
        self.emit(Insn::BitAnd(ok, ok, t));
        self.emit(Insn::Leq(t, jv, hi2_r));
        self.emit(Insn::BitAnd(ok, ok, t));
        // flat = (i-lo1)*ncols + (j-lo2)
        let flat = self.alloc_reg();
        self.emit(Insn::Sub(flat, iv, lo1_r));
        let nc = self.alloc_reg();
        self.emit(Insn::LoadConst(nc, Box::new(Value::from_u64(ncols as u64, 32))));
        self.emit(Insn::Mul(flat, flat, nc));
        let jrel = self.alloc_reg();
        self.emit(Insn::Sub(jrel, jv, lo2_r));
        self.emit(Insn::Add(flat, flat, jrel));
        // guard: !ok -> flat = count (out of the Dense range -> x)
        let br = self.insns.len();
        self.emit(Insn::BranchIfFalse(ok, 0));
        let after_ok = self.insns.len() as u32 + 2;
        self.emit(Insn::Jump(after_ok));
        let oob = self.insns.len() as u32;
        self.emit(Insn::LoadConst(flat, Box::new(Value::from_u64(count as u64, 32))));
        self.insns[br] = Insn::BranchIfFalse(ok, oob);
        Some((
            Box::new(ArrayOperand::Dense {
                name: key,
                first_id,
                lo: 0,
                hi: count - 1,
            }),
            flat,
        ))
    }

    /// §13.3.1 numeric conversion to real, in place. Emitted as
    /// `reg = reg * 1.0(real)`: `Value::mul` with a real operand converts the
    /// other side via `to_f64`, which is EXACTLY `Value::from_f64(v.to_f64())`
    /// — so no new opcode (and the JIT/AOT handle it for free). A no-op for a
    /// value that is already real.
    fn emit_to_real(&mut self, reg: RegId) {
        let one = self.alloc_reg();
        self.emit(Insn::LoadConst(one, Box::new(Value::from_f64(1.0))));
        self.emit(Insn::Mul(reg, reg, one));
    }

    /// True when `fd`'s body reads only its formals, its own declared locals
    /// and compile-time constants — i.e. its result depends on nothing but the
    /// arguments. Conservative: any construct not understood here says "no".
    fn fn_is_pure(&self, fd: &FunctionDeclaration) -> bool {
        self.fn_is_pure_in(fd, None)
    }

    /// `prefix` is the instance scope the function was registered under
    /// (`u0` for a key `u0.onehot`). Elaboration rewrites an instantiated
    /// module's function body to instance-qualified names, so its OWN
    /// formals and result come back as `u0.c` / `u0.onehot`. Judging those
    /// as free names made every function inside an instantiated module look
    /// impure — i.e. inlinable only at the top level, which is nowhere in
    /// real RTL. Stripping the function's own prefix restores the intended
    /// test: is every name a formal, a local, or a constant?
    fn fn_is_pure_in(&self, fd: &FunctionDeclaration, prefix: Option<&str>) -> bool {
        self.fn_is_pure_in_ext(fd, prefix, false)
    }

    /// `allow_ext_reads`: additionally accept free names as module-state
    /// READS (the caller has established they resolve unambiguously — see
    /// `compile_pure_call`). Assignment TARGETS are always held to the strict
    /// rule: an inlined body must not write module state.
    fn fn_is_pure_in_ext(
        &self,
        fd: &FunctionDeclaration,
        prefix: Option<&str>,
        allow_ext_reads: bool,
    ) -> bool {
        const MAX_PURITY_DEPTH: u32 = 8;
        if self.purity_depth.get() >= MAX_PURITY_DEPTH {
            return false;
        }
        self.purity_depth.set(self.purity_depth.get() + 1);
        let ok = self.fn_is_pure_in_inner(fd, prefix, allow_ext_reads);
        self.purity_depth.set(self.purity_depth.get() - 1);
        ok
    }

    fn fn_is_pure_in_inner(
        &self,
        fd: &FunctionDeclaration,
        prefix: Option<&str>,
        allow_ext_reads: bool,
    ) -> bool {
        let mut bound: HashSet<String> = HashSet::default();
        bound.insert(fd.name.name.name.clone());
        for p in &fd.ports {
            bound.insert(p.name.name.clone());
        }
        // Accept the qualified spellings of exactly those same names.
        if let Some(pfx) = prefix {
            for n in bound.clone() {
                bound.insert(format!("{pfx}.{n}"));
            }
        }
        fn expr_ok(
            e: &Expression,
            bound: &HashSet<String>,
            me: &BytecodeCompiler,
            ext: bool,
        ) -> bool {
            match &e.kind {
                ExprKind::Ident(h) => {
                    if h.path.len() != 1 {
                        // `fname.member` may arrive as TWO segments; it is
                        // function-local when the head is bound (the return
                        // variable or a formal).
                        return h.path.len() == 2
                            && bound.contains(&h.path[0].name.name)
                            && h.path.iter().all(|seg| {
                                seg.selects.iter().all(|x| expr_ok(x, bound, me, ext))
                            });
                    }
                    let n = &h.path[0].name.name;
                    // A dotted name is only acceptable when it is one of the
                    // qualified spellings inserted above; anything else
                    // reaching outside the function stays impure — unless the
                    // caller allows module-state READS (`ext`), in which case
                    // a free name (dotted or bare) is a signal read the body
                    // compiler resolves (or bails on) itself.
                    let head_bound = n
                        .split_once('.')
                        .is_some_and(|(head, _)| bound.contains(head));
                    if n.contains('.') && !bound.contains(n) && !head_bound && !ext {
                        return false;
                    }
                    let known = ext
                        || head_bound
                        || bound.contains(n)
                        || me.params.is_some_and(|p| p.contains_key(n));
                    known && h.path[0].selects.iter().all(|sel| expr_ok(sel, bound, me, ext))
                }
                ExprKind::Number(_) | ExprKind::StringLiteral(_) => true,
                ExprKind::Paren(i) => expr_ok(i, bound, me, ext),
                ExprKind::Unary { operand, .. } => expr_ok(operand, bound, me, ext),
                ExprKind::Binary { left, right, .. } => {
                    expr_ok(left, bound, me, ext) && expr_ok(right, bound, me, ext)
                }
                ExprKind::Conditional {
                    condition,
                    then_expr,
                    else_expr,
                } => {
                    expr_ok(condition, bound, me, ext)
                        && expr_ok(then_expr, bound, me, ext)
                        && expr_ok(else_expr, bound, me, ext)
                }
                ExprKind::Concatenation(parts) => parts.iter().all(|p| expr_ok(p, bound, me, ext)),
                ExprKind::Replication { count, exprs } => {
                    expr_ok(count, bound, me, ext) && exprs.iter().all(|p| expr_ok(p, bound, me, ext))
                }
                ExprKind::Index { expr, index } => {
                    expr_ok(expr, bound, me, ext) && expr_ok(index, bound, me, ext)
                }
                ExprKind::RangeSelect {
                    expr, left, right, ..
                } => {
                    expr_ok(expr, bound, me, ext)
                        && expr_ok(left, bound, me, ext)
                        && expr_ok(right, bound, me, ext)
                }
                // A nested call keeps the function pure iff the CALLEE is
                // itself pure-in-arguments and every argument is. AES's
                // `mix_column` calls `xtime(a ^ b)`; without this arm the
                // whole helper — and the task enclosing it — was branded
                // impure and stayed on the interpreter.
                // Pure formatting/interpretation builtins: value depends
                // only on the (checked) arguments. `$sformatf` here is what
                // lets the string decode helpers (get_csr_name-style: a big
                // case of returns with a formatted default) inline at all.
                ExprKind::SystemCall { name, args } => {
                    // Casts desugar to these; the FIRST argument is the
                    // target type/size NAME (not a data read), so only the
                    // operand arguments gate purity — `u8v_t'(128'(x))` kept
                    // branding its whole function impure.
                    if matches!(
                        name.as_str(),
                        "$__xz_named_cast" | "$__xz_size_cast" | "$__xz_type_cast"
                    ) {
                        return args.iter().skip(1).all(|a| expr_ok(a, bound, me, ext));
                    }
                    matches!(
                        name.as_str(),
                        "$sformatf" | "$psprintf" | "$signed" | "$unsigned"
                    ) && args.iter().all(|a| expr_ok(a, bound, me, ext))
                }
                ExprKind::Call { func, args } => {
                    let ExprKind::Ident(h) = &func.kind else { return false };
                    if h.root.is_some() || h.path.iter().any(|s| !s.selects.is_empty()) {
                        return false;
                    }
                    let name = BytecodeCompiler::hier_raw_name(h);
                    let Some(fd2) = me.functions.and_then(|f| {
                        f.get(&name).or_else(|| {
                            name.rsplit('.').next().and_then(|leaf| f.get(leaf))
                        })
                    }) else {
                        return false;
                    };
                    // Input-only callees here (an output arg would write a
                    // caller name this walker cannot track).
                    fd2.ports
                        .iter()
                        .all(|p| matches!(p.direction, PortDirection::Input))
                        && args.iter().all(|a| expr_ok(a, bound, me, ext))
                        && me.fn_is_pure(fd2)
                }
                // §10.9.2: a pattern builds its value from nothing but the
                // expressions inside it, so it is exactly as pure as they are.
                // Falling through to `false` here made every function whose
                // body was `return '{...}` — the ordinary way to build a
                // struct result — impure, and so never inlined.
                // `fname.member` / `formal.member`: as pure as its base, which
                // is the function's own return variable or a formal -- both
                // bound, so this stays function-local. Inside a function body
                // the member access is NOT collapsed to a dotted Ident.
                ExprKind::MemberAccess { expr, .. } => expr_ok(expr, bound, me, ext),
                ExprKind::AssignmentPattern(items) => {
                    use crate::ast::expr::AssignmentPatternItem as It;
                    items.iter().all(|it| match it {
                        It::Named(_, e) | It::Ordered(e) | It::Default(e) => {
                            expr_ok(e, bound, me, ext)
                        }
                        _ => false,
                    })
                }
                _ => false,
            }
        }
        fn stmt_ok(st: &Statement, bound: &mut HashSet<String>, me: &BytecodeCompiler, ext: bool) -> bool {
            match &st.kind {
                StatementKind::Null => true,
                StatementKind::VarDecl {
                    declarators,
                    ..
                } => {
                    for d in declarators {
                        if let Some(e) = &d.init {
                            if !expr_ok(e, bound, me, ext) {
                                return false;
                            }
                        }
                        bound.insert(d.name.name.clone());
                    }
                    true
                }
                StatementKind::BlockingAssign { lvalue, rvalue } => {
                    // The TARGET is held strict regardless of `ext`: an
                    // inlined body must not write module state. A local-array
                    // element target (`row[n] = …`) is a bound name with a
                    // select, which the strict Ident arm accepts; only the
                    // INDEX may use ext reads.
                    expr_ok(lvalue, bound, me, false) && expr_ok(rvalue, bound, me, ext)
                }
                StatementKind::Return(e) => e.as_ref().is_none_or(|e| expr_ok(e, bound, me, ext)),
                StatementKind::While { condition, body } => {
                    // Same gap as the Foreach arm (issue #146): no arm meant
                    // `_ => false`, branding a pure while-loop helper
                    // (popcnt-style) impure.
                    expr_ok(condition, bound, me, ext)
                        && stmt_ok(body, &mut bound.clone(), me, ext)
                }
                StatementKind::DoWhile { body, condition } => {
                    expr_ok(condition, bound, me, ext)
                        && stmt_ok(body, &mut bound.clone(), me, ext)
                }
                StatementKind::Repeat { count, body } => {
                    expr_ok(count, bound, me, ext)
                        && stmt_ok(body, &mut bound.clone(), me, ext)
                }
                // §12.7: control flow only; reads nothing, writes nothing.
                StatementKind::Break | StatementKind::Continue => true,
                StatementKind::Foreach { array, vars, body } => {
                    // §12.7.3: the loop variables are implicitly DECLARED by
                    // the foreach for its body — without this arm they read
                    // as free module names and every resolver body using
                    // `foreach (drivers[i])` was branded impure (the
                    // Expr_Call_impure half of issue #137).
                    if !expr_ok(array, bound, me, ext) {
                        return false;
                    }
                    let mut inner = bound.clone();
                    for v in vars.iter().flatten() {
                        inner.insert(v.name.clone());
                    }
                    stmt_ok(body, &mut inner, me, ext)
                }
                StatementKind::SeqBlock { stmts, .. } => {
                    // A block's declarations are visible to the statements that
                    // FOLLOW them, so thread one scope through the sequence.
                    let mut inner = bound.clone();
                    stmts.iter().all(|s| stmt_ok(s, &mut inner, me, ext))
                }
                StatementKind::If {
                    condition,
                    then_stmt,
                    else_stmt,
                    ..
                } => {
                    expr_ok(condition, bound, me, ext)
                        && stmt_ok(then_stmt, &mut bound.clone(), me, ext)
                        && else_stmt
                            .as_ref()
                            .is_none_or(|e| stmt_ok(e, &mut bound.clone(), me, ext))
                }
                StatementKind::For {
                    init,
                    condition,
                    step,
                    body,
                } => {
                    let mut inner = bound.clone();
                    for fi in init {
                        match fi {
                            ForInit::VarDecl { name, init, .. } => {
                                if !expr_ok(init, &inner, me, ext) {
                                    return false;
                                }
                                inner.insert(name.name.clone());
                            }
                            ForInit::Assign { lvalue, rvalue } => {
                                if !expr_ok(lvalue, &inner, me, false) || !expr_ok(rvalue, &inner, me, ext) {
                                    return false;
                                }
                            }
                        }
                    }
                    condition.as_ref().is_none_or(|c| expr_ok(c, &inner, me, ext))
                        && step.iter().all(|e| expr_ok(e, &inner, me, ext))
                        && stmt_ok(body, &mut inner, me, ext)
                }
                // case/casez/casex — the shape of virtually every decode
                // helper (`assign x = f(op)` over a big casez). Leaving this
                // arm out branded ALL of them impure, which kept the whole
                // enclosing assign on the AST interpreter. The `matches`
                // pattern/guard form stays conservative.
                StatementKind::Case { expr, items, .. } => {
                    expr_ok(expr, bound, me, ext)
                        && items.iter().all(|it| {
                            it.pattern.is_none()
                                && it.guard.is_none()
                                && it.patterns.iter().all(|p| expr_ok(p, bound, me, ext))
                                && stmt_ok(&it.stmt, &mut bound.clone(), me, ext)
                        })
                }
                _ => false,
            }
        }
        let mut scope = bound;
        fd.items
            .iter()
            .all(|st| stmt_ok(st, &mut scope, self, allow_ext_reads))
    }

    /// §13.4.1 / §6.8: the initial value of a variable of `dt` — x for a
    /// 4-state type, 0 for a 2-state one. Getting this wrong for an inlined
    /// function's return variable makes a partially-assigned function return 0
    /// where it must return x.
    fn type_default_value(&self, dt: &crate::ast::types::DataType, w: u32) -> Value {
        if matches!(
            dt,
            crate::ast::types::DataType::Simple {
                kind: crate::ast::types::SimpleType::String,
                ..
            }
        ) {
            // §6.16: a string variable initializes to "" — and its Value
            // must never be resized to the 1024-bit placeholder width.
            return Value::from_string("");
        }
        let w = if w > 0 { w } else { 32 };
        if crate::compiler::elaborate::is_type_two_state(dt) {
            Value::zero(w)
        } else {
            Value::new(w)
        }
    }

    /// Declared width of a block-local variable's data type, resolved against
    /// the module's parameters/typedefs (0 when unknown, meaning "leave as-is").
    /// §13.5.2: a formal that omits its type inherits the PREVIOUS formal's.
    /// The parser leaves such a port's data_type Implicit, and sizing it
    /// directly gives 1 bit — `input logic [7:0] a0, a1` bound a1 one bit
    /// wide and truncated every argument passed through it.
    fn port_effective_width(
        &self,
        ports: &[crate::ast::decl::FunctionPort],
        i: usize,
        scope: Option<&str>,
    ) -> u32 {
        for k in (0..=i).rev() {
            let dt = &ports[k].data_type;
            if !matches!(dt, crate::ast::types::DataType::Implicit { dimensions, .. } if dimensions.is_empty())
            {
                return self.decl_width_in(dt, scope);
            }
        }
        self.decl_width_in(&ports[i].data_type, scope)
    }

    fn decl_width(&self, dt: &crate::ast::types::DataType) -> u32 {
        self.decl_width_in(dt, None)
    }

    /// Declared width with INSTANCE-SCOPED typedef fallback. Elaboration keys
    /// a submodule's typedef as `<inst>.<name>` while a declaration inside an
    /// inlined body still spells the bare name; the bare miss used to fall to
    /// the 32-bit default, truncating every wide typedef'd local. `scope` is
    /// the inlined subroutine's qualified name (`d.conv`) when the caller has
    /// it; enclosing inline frames are tried otherwise.
    fn decl_width_in(&self, dt: &crate::ast::types::DataType, scope: Option<&str>) -> u32 {
        if let crate::ast::types::DataType::TypeReference { name: tn, dimensions, .. } = dt {
            if dimensions.is_empty() && tn.scope.is_none() {
                let bare = tn.name.name.as_str();
                if let Some(t) = self.typedefs {
                    if !t.contains_key(bare) {
                        if let Some(w) = self.scoped_typedef_hit(t, bare, scope) {
                            if w > 0 {
                                return w;
                            }
                        }
                    }
                }
            }
        }
        crate::compiler::elaborate::resolve_type_width(dt, self.params, self.typedefs)
    }

    /// Look `<pfx>.<bare>` up in `t`, where `pfx` is the instance prefix of
    /// `scope` or of an enclosing inline frame.
    fn scoped_typedef_hit(
        &self,
        t: &HashMap<String, u32>,
        bare: &str,
        scope: Option<&str>,
    ) -> Option<u32> {
        let try_pfx = |p: &str| t.get(&format!("{p}.{bare}")).copied();
        scope
            .and_then(|s| s.rsplit_once('.'))
            .and_then(|(p, _)| try_pfx(p))
            .or_else(|| {
                self.inlining_stack
                    .iter()
                    .rev()
                    .find_map(|f| f.rsplit_once('.').and_then(|(p, _)| try_pfx(p)))
            })
    }

    /// Packed ELEMENT width of a declared type, for register-backed locals:
    /// direct packed-of-packed declarations resolve structurally; a bare
    /// typedef name resolves through the elaborator's typedef elem table.
    fn decl_elem_width(&self, dt: &crate::ast::types::DataType) -> Option<u32> {
        self.decl_elem_width_in(dt, None)
    }

    fn decl_elem_width_in(
        &self,
        dt: &crate::ast::types::DataType,
        scope: Option<&str>,
    ) -> Option<u32> {
        if let (Some(p), Some(t)) = (self.params, self.typedefs) {
            if let Some(ew) = crate::compiler::elaborate::packed_inner_elem_width(dt, p, t) {
                return Some(ew);
            }
        }
        if let crate::ast::types::DataType::TypeReference { name, dimensions, .. } = dt {
            if dimensions.is_empty() {
                let bare = name.name.name.as_str();
                let m = self.typedef_elems?;
                // Scoped key FIRST: unlike the width table, the elem table has
                // no save/restore rail, so a bare entry may belong to another
                // instance's same-named typedef.
                return self
                    .scoped_typedef_hit(m, bare, scope)
                    .or_else(|| m.get(bare).copied());
            }
        }
        None
    }

    /// Is this expression statically known to produce a STRING value?
    /// (Literal, `string` signal, `string`-bound formal, or a nested
    /// `$sformatf`.) Conservative: false means "unknown", not "packed".
    fn expr_is_string_static(&self, e: &Expression) -> bool {
        match &e.kind {
            ExprKind::StringLiteral(_) => true,
            ExprKind::Paren(inner) => self.expr_is_string_static(inner),
            ExprKind::SystemCall { name, .. } => {
                matches!(name.as_str(), "$sformatf" | "$psprintf")
            }
            ExprKind::Ident(h) => {
                let raw = Self::hier_raw_name(h);
                if self.local_var_is_string.contains(&raw) {
                    return true;
                }
                let leaf = h.path.last().map(|p| p.name.name.as_str()).unwrap_or("");
                self.string_signals.is_some_and(|ss| {
                    ss.contains(&raw) || ss.contains(leaf)
                })
            }
            ExprKind::Concatenation(parts) => {
                !parts.is_empty() && parts.iter().all(|p| self.expr_is_string_static(p))
            }
            ExprKind::Call { func, .. } => {
                let ExprKind::Ident(h) = &func.kind else { return false };
                // String method with a string result on a string receiver.
                if h.path.len() >= 2 {
                    let m = h.path.last().unwrap().name.name.as_str();
                    if matches!(m, "substr" | "toupper" | "tolower") {
                        let mut recv = h.clone();
                        recv.path.pop();
                        let recv_expr =
                            Expression::new(ExprKind::Ident(recv), func.span);
                        if self.expr_is_string_static(&recv_expr) {
                            return true;
                        }
                    }
                }
                let raw = Self::hier_raw_name(h);
                self.functions
                    .and_then(|f| {
                        f.get(&raw).or_else(|| {
                            raw.rsplit('.').next().and_then(|l| f.get(l))
                        })
                    })
                    .is_some_and(|fd| {
                        matches!(
                            fd.return_type,
                            crate::ast::types::DataType::Simple {
                                kind: crate::ast::types::SimpleType::String,
                                ..
                            }
                        )
                    })
            }
            _ => false,
        }
    }

    /// `recv.method(args)` where the receiver is a statically-known STRING
    /// (dotted-Ident call shape). Returns the receiver as its own Ident
    /// expression plus the method leaf, or None when the shape/typing does
    /// not match.
    fn string_method_shape<'e>(
        &self,
        func: &'e Expression,
        span: crate::ast::Span,
    ) -> Option<(Expression, &'e str)> {
        let ExprKind::Ident(h) = &func.kind else { return None };
        if h.root.is_some() || h.path.len() < 2 {
            return None;
        }
        if h.path.iter().any(|seg| !seg.selects.is_empty()) {
            return None;
        }
        let method = h.path.last().unwrap().name.name.as_str();
        let mut recv = h.clone();
        recv.path.pop();
        let recv_expr = Expression::new(ExprKind::Ident(recv), span);
        if !self.expr_is_string_static(&recv_expr) {
            return None;
        }
        Some((recv_expr, method))
    }

    /// Compile a PURE string method call to a `StrOp`. Mutators (putc, the
    /// *toa family) are handled at statement level, not here.
    fn compile_string_method(
        &mut self,
        func: &Expression,
        args: &[Expression],
        span: crate::ast::Span,
    ) -> Option<RegId> {
        let (recv, method) = self.string_method_shape(func, span)?;
        let (kind, nargs) = match method {
            "len" => (StrOpKind::Len, 0),
            "getc" => (StrOpKind::GetC, 1),
            "substr" => (StrOpKind::Substr, 2),
            "toupper" => (StrOpKind::ToUpper, 0),
            "tolower" => (StrOpKind::ToLower, 0),
            "compare" => (StrOpKind::Compare, 1),
            "icompare" => (StrOpKind::ICompare, 1),
            "atoi" => (StrOpKind::AToI, 0),
            "atohex" => (StrOpKind::AToHex, 0),
            "atooct" => (StrOpKind::AToOct, 0),
            "atobin" => (StrOpKind::AToBin, 0),
            _ => return None,
        };
        if args.len() != nargs {
            return None;
        }
        let start = self.insns.len();
        let start_reg = self.next_reg;
        let mut regs: Vec<RegId> = Vec::with_capacity(1 + nargs);
        let Some(r) = self.compile_expr(&recv, 0) else {
            return None;
        };
        regs.push(r);
        for a in args {
            match self.compile_expr(a, 0) {
                Some(r) => regs.push(r),
                None => {
                    self.insns.truncate(start);
                    self.next_reg = start_reg;
                    return None;
                }
            }
        }
        let dst = self.alloc_reg();
        self.emit(Insn::StrOp(dst, kind, Box::new(regs)));
        Some(dst)
    }

    fn signal_is_string_name(&self, hier: &HierarchicalIdentifier) -> bool {
        let raw = Self::hier_raw_name(hier);
        let leaf = hier.path.last().map(|p| p.name.name.as_str()).unwrap_or("");
        self.string_signals
            .is_some_and(|ss| ss.contains(&raw) || ss.contains(leaf))
    }

    /// Parse a `$sformatf` template into segments, or None when any piece
    /// falls outside the natively-supported subset (specs d/b/h/x/o/s/c and
    /// `%%`; optional `-` flag and numeric width; no `+`, no precision, no
    /// t/p/m/u/z/e/f/g/v). The consumed-argument COUNT must be settled here
    /// so the compile arm can match it against the actual list.
    fn parse_format_template(fmt: &str) -> Option<(Vec<FmtSeg>, usize)> {
        let mut segs: Vec<FmtSeg> = Vec::new();
        let mut lit = String::new();
        let mut nargs = 0usize;
        let mut chars = fmt.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '%' {
                lit.push(c);
                continue;
            }
            let mut left = false;
            if chars.peek() == Some(&'-') {
                left = true;
                chars.next();
            }
            if chars.peek() == Some(&'+') {
                return None;
            }
            let mut wstr = String::new();
            while chars.peek().is_some_and(|c| c.is_ascii_digit()) {
                wstr.push(chars.next().unwrap());
            }
            if chars.peek() == Some(&'.') {
                return None;
            }
            let spec = chars.next()?;
            if spec == '%' {
                if left || !wstr.is_empty() {
                    return None;
                }
                lit.push('%');
                continue;
            }
            let spec_lc = spec.to_ascii_lowercase();
            if !matches!(spec_lc, 'd' | 'b' | 'h' | 'x' | 'o' | 's' | 'c') {
                return None;
            }
            let width = if wstr.is_empty() {
                None
            } else {
                Some(wstr.parse::<u32>().ok()?)
            };
            if !lit.is_empty() {
                segs.push(FmtSeg::Lit(std::mem::take(&mut lit)));
            }
            segs.push(FmtSeg::Spec {
                spec: spec_lc,
                width,
                left,
                str_valued: false,
            });
            nargs += 1;
        }
        if !lit.is_empty() {
            segs.push(FmtSeg::Lit(lit));
        }
        Some((segs, nargs))
    }

    fn bail(&mut self, reason: &'static str) {
        if self.bail_reason.is_none() {
            self.bail_reason = Some(reason);
        }
    }

    fn alloc_reg(&mut self) -> RegId {
        let Ok(r) = RegId::try_from(self.next_reg) else {
            self.register_overflow = true;
            return 0;
        };
        self.next_reg += 1;
        r
    }

    fn emit(&mut self, insn: Insn) {
        self.insns.push(insn);
    }

    fn hier_raw_name(hier: &HierarchicalIdentifier) -> String {
        hier.path
            .iter()
            .map(|s| s.name.name.as_str())
            .collect::<Vec<_>>()
            .join(".")
    }

    /// §7.2.1: `base.member` where `base` resolves to a packed-struct SIGNAL
    /// and `member` is a field of its recorded layout — a constant bit slice
    /// of the container. The interpreter served every such read through the
    /// metadata tables at ~7us a piece; ibex's CSR and interrupt logic does
    /// ~8 of them per cycle (`mstatus_q.mpp`, `irqs_i.irq_fast`, ...), which
    /// made this the largest single fallback after the named-cast fix.
    /// Field layout of `lv` when it names a packed-struct SIGNAL directly.
    fn lvalue_struct_layout(&self, lv: &Expression) -> Option<Vec<(String, u32, u32)>> {
        let fields_tbl = self.packed_struct_fields?;
        // `arr[i] <= '{...}`: an ELEMENT of an array of packed structs carries
        // the element layout, which is keyed by the array name — same keying
        // the `arr[i].m` member store uses. Without this the pattern had no
        // layout and the whole statement fell to the AST path.
        let h = match &lv.kind {
            ExprKind::Ident(h) => h,
            ExprKind::Index { expr, .. } => match &expr.kind {
                ExprKind::Ident(h) => h,
                _ => return None,
            },
            _ => return None,
        };
        if h.root.is_some() || h.path.iter().any(|s| !s.selects.is_empty()) {
            return None;
        }
        let raw = Self::hier_raw_name(h);
        if let Some(scope) = &self.scope_hint {
            let q = format!("{}.{}", scope, raw);
            if let Some(l) = fields_tbl.get(&q) {
                return Some(l.clone());
            }
        }
        fields_tbl.get(&raw).cloned()
    }

    /// §10.9.2: an assignment pattern applied to a PACKED-struct target is a
    /// concatenation of its members in declared order (first field = MSBs).
    /// Named items bind by field, `default:` fills whatever is left, ordered
    /// items map by position. Every field must be covered exactly once, and
    /// each member expression compiles at ITS OWN field's width — that is
    /// what makes this exactly the interpreter's member-wise semantics.
    /// ibex's CSR write logic rebuilds `mstatus_d`/`mcause_d` this way every
    /// cycle; the pattern bail dragged the whole block (11% of the bench)
    /// onto the AST path.
    fn compile_packed_struct_pattern(
        &mut self,
        items: &[crate::ast::expr::AssignmentPatternItem],
        layout: &[(String, u32, u32)],
    ) -> Option<RegId> {
        use crate::ast::expr::AssignmentPatternItem as Item;
        if layout.is_empty() {
            return None;
        }
        // MSB-first: highest offset first.
        let mut fields: Vec<(String, u32, u32)> = layout.to_vec();
        fields.sort_by(|a, b| b.1.cmp(&a.1));

        let mut named: Vec<(usize, &Expression)> = Vec::new();
        let mut ordered: Vec<&Expression> = Vec::new();
        let mut default: Option<&Expression> = None;
        for it in items {
            match it {
                Item::Named(id, e) => {
                    let idx = fields.iter().position(|(n, _, _)| *n == id.name)?;
                    if named.iter().any(|(i, _)| *i == idx) {
                        return None;
                    }
                    named.push((idx, e));
                }
                Item::Ordered(e) => ordered.push(e),
                Item::Default(e) => {
                    if default.is_some() {
                        return None;
                    }
                    default = Some(e);
                }
                _ => return None,
            }
        }
        // Mixed ordered+named is not a legal pattern; ordered must cover all.
        if !ordered.is_empty() && (!named.is_empty() || default.is_some()) {
            return None;
        }
        if !ordered.is_empty() && ordered.len() != fields.len() {
            return None;
        }
        let mut regs: Vec<RegId> = Vec::with_capacity(fields.len());
        for (idx, (_, _, w)) in fields.iter().enumerate() {
            let expr = if !ordered.is_empty() {
                ordered[idx]
            } else if let Some((_, e)) = named.iter().find(|(i, _)| *i == idx) {
                e
            } else {
                default?
            };
            let src = self.compile_expr(expr, *w)?;
            let r = self.alloc_reg();
            self.emit(Insn::Move(r, src));
            self.emit(Insn::Resize(r, *w));
            regs.push(r);
        }
        let dest = self.alloc_reg();
        self.emit(Insn::Concat(dest, Box::new(regs)));
        Some(dest)
    }

    fn compile_packed_member_read(&mut self, hier: &HierarchicalIdentifier) -> Option<RegId> {
        let fields_tbl = self.packed_struct_fields?;
        if hier.root.is_some() || hier.path.iter().any(|s| !s.selects.is_empty()) {
            return None;
        }
        let raw = Self::hier_raw_name(hier);
        let (base, member) = raw.rsplit_once('.')?;
        // Resolve the BASE like a plain identifier: scope-qualified first,
        // then flat.
        let mut resolved: Option<(usize, String)> = None;
        if let Some(scope) = &self.scope_hint {
            let q = format!("{}.{}", scope, base);
            if let Some(&id) = self.signal_name_to_id.get(q.as_str()) {
                resolved = Some((id, q));
            }
        }
        if resolved.is_none() {
            if let Some(&id) = self.signal_name_to_id.get(base) {
                resolved = Some((id, base.to_string()));
            }
        }
        let (base_id, key) = resolved?;
        let layout = fields_tbl
            .get(&key)
            .or_else(|| fields_tbl.get(base))?;
        let &(_, off, w) = layout.iter().find(|(m, _, _)| m == member)?;
        if w == 0 {
            return None;
        }
        let root = self.alloc_reg();
        self.emit(Insn::LoadSignal(root, as_sig_id(base_id)));
        let dest = self.alloc_reg();
        self.emit(Insn::RangeSelectConst(dest, root, off + w - 1, off));
        Some(dest)
    }

    /// Does this statement read or write a register-bank local array? Such a
    /// body needs CONSTANT indexes, i.e. the unroller — a register-backed
    /// runtime loop variable can never address the bank.
    fn stmt_touches_reg_bank(&self, st: &Statement) -> bool {
        if self.local_array_regs.is_empty() {
            return false;
        }
        fn expr_hits(e: &Expression, banks: &HashMap<String, (RegId, u32, usize, i64)>) -> bool {
            match &e.kind {
                ExprKind::Ident(h) => {
                    h.path.len() == 1 && banks.contains_key(&h.path[0].name.name)
                }
                ExprKind::Index { expr, index } => {
                    expr_hits(expr, banks) || expr_hits(index, banks)
                }
                ExprKind::Paren(i) | ExprKind::Unary { operand: i, .. } => expr_hits(i, banks),
                ExprKind::Binary { left, right, .. } => {
                    expr_hits(left, banks) || expr_hits(right, banks)
                }
                ExprKind::Conditional { condition, then_expr, else_expr } => {
                    expr_hits(condition, banks)
                        || expr_hits(then_expr, banks)
                        || expr_hits(else_expr, banks)
                }
                ExprKind::Concatenation(xs) => xs.iter().any(|x| expr_hits(x, banks)),
                ExprKind::Replication { count, exprs } => {
                    expr_hits(count, banks) || exprs.iter().any(|x| expr_hits(x, banks))
                }
                ExprKind::RangeSelect { expr, left, right, .. } => {
                    expr_hits(expr, banks) || expr_hits(left, banks) || expr_hits(right, banks)
                }
                ExprKind::Call { args, .. } | ExprKind::SystemCall { args, .. } => {
                    args.iter().any(|a| expr_hits(a, banks))
                }
                _ => false,
            }
        }
        let banks = &self.local_array_regs;
        fn walk(st: &Statement, banks: &HashMap<String, (RegId, u32, usize, i64)>) -> bool {
            match &st.kind {
                StatementKind::BlockingAssign { lvalue, rvalue }
                | StatementKind::NonblockingAssign { lvalue, rvalue, .. } => {
                    expr_hits(lvalue, banks) || expr_hits(rvalue, banks)
                }
                StatementKind::Expr(e) => expr_hits(e, banks),
                StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
                    stmts.iter().any(|s| walk(s, banks))
                }
                StatementKind::If { condition, then_stmt, else_stmt, .. } => {
                    expr_hits(condition, banks)
                        || walk(then_stmt, banks)
                        || else_stmt.as_ref().is_some_and(|e| walk(e, banks))
                }
                StatementKind::Case { expr, items, .. } => {
                    expr_hits(expr, banks)
                        || items.iter().any(|it| {
                            it.patterns.iter().any(|p| expr_hits(p, banks))
                                || walk(&it.stmt, banks)
                        })
                }
                StatementKind::For { init, condition, step, body } => {
                    init.iter().any(|fi| match fi {
                        ForInit::VarDecl { init, .. } => expr_hits(init, banks),
                        ForInit::Assign { lvalue, rvalue } => {
                            expr_hits(lvalue, banks) || expr_hits(rvalue, banks)
                        }
                    }) || condition.as_ref().is_some_and(|c| expr_hits(c, banks))
                        || step.iter().any(|e| expr_hits(e, banks))
                        || walk(body, banks)
                }
                _ => false,
            }
        }
        walk(st, banks)
    }

    /// Compile-time unroll of `for (int i = C; cond(i); i += C2)` with a
    /// small constant trip count. The loop variable becomes a per-iteration
    /// COMPILE-TIME constant, so array indexes fold (`state[col*4]`), local
    /// register-bank arrays become addressable, and call statements inline
    /// per iteration. All-or-nothing: any statement that fails rolls the
    /// whole unroll back and the loop takes the pre-existing paths.
    fn try_unroll_for(
        &mut self,
        name: &crate::ast::Identifier,
        init: &Expression,
        condition: &Option<Expression>,
        step: &[Expression],
        body: &Statement,
    ) -> bool {
        const MAX_TRIPS: usize = 64;
        let Some(cond) = condition else { return false };
        if step.len() != 1 {
            return false;
        }
        // A body that can SUSPEND must go one iteration at a time through the
        // scheduler — an unrolled copy would run synchronously through the
        // wait. And break/continue need per-iteration jump targets the unroll
        // does not lay down; leave those loops to the AST path.
        if Self::stmt_is_blocking(body) || Self::stmt_has_break_or_continue(body) {
            return false;
        }
        // Step: i++/++i/i--/--i, or (i = i + C) / (i += C) via AssignExpr.
        let vname = name.name.clone();
        let step_delta: i64 = match &step[0].kind {
            ExprKind::Unary { op, operand } => {
                let ExprKind::Ident(h) = &operand.kind else { return false };
                if Self::hier_raw_name(h) != vname {
                    return false;
                }
                match op {
                    UnaryOp::PostIncr | UnaryOp::PreIncr => 1,
                    UnaryOp::PostDecr | UnaryOp::PreDecr => -1,
                    _ => return false,
                }
            }
            ExprKind::AssignExpr { lvalue, rvalue } => {
                let ExprKind::Ident(h) = &lvalue.kind else { return false };
                if Self::hier_raw_name(h) != vname {
                    return false;
                }
                let ExprKind::Binary { op, left, right } = &rvalue.kind else {
                    return false;
                };
                let ExprKind::Ident(lh) = &left.kind else { return false };
                if Self::hier_raw_name(lh) != vname {
                    return false;
                }
                let Some(c) = self.fold_const(right).and_then(|v| v.to_u64()) else {
                    return false;
                };
                match op {
                    BinaryOp::Add => c as i64,
                    BinaryOp::Sub => -(c as i64),
                    _ => return false,
                }
            }
            _ => return false,
        };
        if step_delta == 0 {
            return false;
        }
        let Some(mut cur) = self
            .fold_const(init)
            .and_then(|v| v.to_u64())
            .map(|v| v as i64)
        else {
            return false;
        };

        let start = self.insns.len();
        let start_reg = self.next_reg;
        let outer_const = self.local_const_vars.remove(&vname);
        let mut ok = true;
        let mut trips = 0usize;
        loop {
            self.local_const_vars
                .insert(vname.clone(), Value::from_u64(cur as u64, 32));
            let c = match self.fold_const(cond) {
                Some(v) => v.is_true(),
                None => {
                    ok = false;
                    break;
                }
            };
            if !c {
                break;
            }
            trips += 1;
            if trips > MAX_TRIPS {
                ok = false;
                break;
            }
            // The interpreter cannot see the const-bound loop variable (or
            // any register bank), so a statement deferring to AST fallback
            // inside the body would silently read the WRONG storage. Compile
            // fully or roll the whole unroll back.
            let saved_fb = self.allow_ast_fallback;
            self.allow_ast_fallback = false;
            let body_ok = self.compile_stmt(body);
            self.allow_ast_fallback = saved_fb;
            if !body_ok {
                ok = false;
                break;
            }
            cur += step_delta;
        }
        self.local_const_vars.remove(&vname);
        if let Some(v) = outer_const {
            self.local_const_vars.insert(vname.clone(), v);
        }
        if !ok {
            self.insns.truncate(start);
            self.next_reg = start_reg;
            return false;
        }
        true
    }

    /// Fold `e` to a compile-time constant, consulting unrolled loop vars.
    /// Conservative: 4-state-clean integers only.
    fn fold_const(&mut self, e: &Expression) -> Option<Value> {
        let v = match &e.kind {
            ExprKind::Number(n) => self.eval_number_static(n)?,
            ExprKind::Paren(i) => return self.fold_const(i),
            ExprKind::Ident(h) => {
                if h.root.is_none() && h.path.len() == 1 && h.path[0].selects.is_empty() {
                    if let Some(v) = self.local_const_vars.get(&h.path[0].name.name) {
                        v.clone()
                    } else {
                        self.lookup_param_value(h)?
                    }
                } else {
                    self.lookup_param_value(h)?
                }
            }
            ExprKind::Unary { op, operand } => {
                let v = self.fold_const(operand)?;
                match op {
                    UnaryOp::Plus => v,
                    UnaryOp::Minus => Value::zero(v.width.max(32)).sub(&v),
                    UnaryOp::BitNot => v.bitwise_not(),
                    UnaryOp::LogNot => v.logic_not(),
                    _ => return None,
                }
            }
            ExprKind::Binary { op, left, right } => {
                let l = self.fold_const(left)?;
                let r = self.fold_const(right)?;
                match op {
                    BinaryOp::Add => l.add(&r),
                    BinaryOp::Sub => l.sub(&r),
                    BinaryOp::Mul => l.mul(&r),
                    BinaryOp::Div => l.div(&r),
                    BinaryOp::Mod => l.modulo(&r),
                    BinaryOp::ShiftLeft | BinaryOp::ArithShiftLeft => l.shift_left(&r),
                    BinaryOp::ShiftRight => l.shift_right(&r),
                    BinaryOp::BitAnd => l.bitwise_and(&r),
                    BinaryOp::BitOr => l.bitwise_or(&r),
                    BinaryOp::BitXor => l.bitwise_xor(&r),
                    BinaryOp::Lt => l.less_than(&r),
                    BinaryOp::Leq => l.less_equal(&r),
                    BinaryOp::Gt => l.greater_than(&r),
                    BinaryOp::Geq => l.greater_equal(&r),
                    BinaryOp::Eq => l.is_equal(&r),
                    BinaryOp::Neq => l.is_not_equal(&r),
                    _ => return None,
                }
            }
            _ => return None,
        };
        (!v.has_xz() && !v.is_real).then_some(v)
    }

    /// Detect the ROM `case` shape: every arm `L = <const>` with constant
    /// patterns, optional `default: L = <const>`. Returns the shared lvalue,
    /// the dense table (default-filled holes), the default value, and the
    /// result width. Bounded at 4096 entries so a sparse 32-bit decode cannot
    /// balloon the block.
    fn case_lut_shape(
        &mut self,
        items: &[crate::ast::stmt::CaseItem],
    ) -> Option<(Expression, Vec<Value>, Value, u32)> {
        let mut lhs: Option<Expression> = None;
        let mut entries: Vec<(u64, Value)> = Vec::new();
        let mut default: Option<Value> = None;
        let mut same_lhs = |l: &Expression, lhs: &mut Option<Expression>| -> bool {
            let ExprKind::Ident(h) = &l.kind else { return false };
            if h.path.iter().any(|s| !s.selects.is_empty()) {
                return false;
            }
            match lhs {
                None => {
                    *lhs = Some(l.clone());
                    true
                }
                Some(prev) => {
                    let ExprKind::Ident(ph) = &prev.kind else { return false };
                    Self::hier_raw_name(ph) == Self::hier_raw_name(h)
                }
            }
        };
        let const_of = |me: &mut Self, e: &Expression| -> Option<Value> {
            let v = match &e.kind {
                ExprKind::Number(n) => me.eval_number_static(n)?,
                ExprKind::Ident(h) => me.lookup_param_value(h)?,
                _ => return None,
            };
            (!v.has_xz() && !v.is_real).then_some(v)
        };
        for it in items {
            if it.pattern.is_some() || it.guard.is_some() {
                return None;
            }
            let StatementKind::BlockingAssign { lvalue, rvalue } = &it.stmt.kind else {
                return None;
            };
            if !same_lhs(lvalue, &mut lhs) {
                return None;
            }
            let rv = const_of(self, rvalue)?;
            if it.is_default {
                if default.is_some() {
                    return None;
                }
                default = Some(rv);
                continue;
            }
            for pat in &it.patterns {
                let pv = const_of(self, pat)?;
                let idx = pv.to_u64()?;
                if idx >= 4096 {
                    return None;
                }
                entries.push((idx, rv.clone()));
            }
        }
        let lhs = lhs?;
        let default = default?; // no default: keep the generic chain (lhs holds)
        let res_w = entries
            .iter()
            .map(|(_, v)| v.width)
            .chain(std::iter::once(default.width))
            .max()
            .unwrap_or(1);
        let size = entries.iter().map(|(i, _)| *i + 1).max().unwrap_or(0) as usize;
        let mut table = vec![default.clone(); size];
        for (i, v) in entries {
            table[i as usize] = v;
        }
        Some((lhs, table, default, res_w))
    }

    /// Computed-goto lowering for a dense plain `case`: constant, fully
    /// defined patterns dispatch straight to their arm's entry pc, replacing
    /// the compare-and-branch pair executed per SKIPPED arm. Unlike
    /// `case_lut_shape` the arm bodies are arbitrary compilable statements,
    /// so this covers the readback-mux / decoder shape that dominates the
    /// RISC-V core bench. Falls back to the generic chain (returns false,
    /// nothing emitted) on any non-conforming item.
    ///
    /// Correctness gates:
    /// - selector must be UNSIGNED and <=64 bits wide: dispatch compares raw
    ///   numeric values, which equals `===` only under zero-extension
    ///   (§11.8.1) and only when `to_u64` cannot truncate high bits;
    /// - patterns constant, x/z-free, < 4096 (table stays small);
    /// - >=8 pattern values (below that the chain is competitive);
    /// - §12.5 first-match-wins kept for duplicate pattern values.
    fn compile_case_jump(
        &mut self,
        expr: &Expression,
        items: &[crate::ast::stmt::CaseItem],
    ) -> bool {
        if self.expr_signedness(expr) != Some(false) || self.expr_max_width(expr) > 64 {
            return false;
        }
        // Bisect knobs (mirrors XEZIM_CAST_COMPILE_LIMIT): OFF disables the
        // lowering entirely, LIMIT=n keeps only the first n emissions.
        if std::env::var("XEZIM_CASEJUMP_OFF").is_ok() {
            return false;
        }
        // Shape scan first — nothing is emitted until every pattern proves
        // constant, so the common non-conforming case costs no rollback.
        let mut default_seen = false;
        let mut arm_vals: Vec<Vec<u64>> = Vec::with_capacity(items.len());
        let mut n_entries = 0usize;
        let mut max_idx = 0u64;
        for it in items {
            if it.pattern.is_some() || it.guard.is_some() {
                return false;
            }
            if it.is_default {
                if default_seen {
                    return false;
                }
                default_seen = true;
                arm_vals.push(Vec::new());
                continue;
            }
            let mut vals = Vec::with_capacity(it.patterns.len());
            for pat in &it.patterns {
                let pv = match &pat.kind {
                    ExprKind::Number(n) => self.eval_number_static(n),
                    ExprKind::Ident(h) => self.lookup_param_value(h),
                    _ => None,
                };
                let Some(pv) = pv else { return false };
                if pv.has_xz() || pv.is_real || pv.width > 64 {
                    return false;
                }
                let idx = match pv.to_u64() {
                    Some(i) if i < 4096 => i,
                    _ => return false,
                };
                max_idx = max_idx.max(idx);
                vals.push(idx);
            }
            n_entries += vals.len();
            arm_vals.push(vals);
        }
        if n_entries < 8 {
            return false;
        }
        // Emit with all-or-nothing rollback so a body that fails to compile
        // leaves the generic chain (and its own fallback path) untouched.
        let start = self.insns.len();
        let start_reg = self.next_reg;
        let Some(sel) = self.compile_expr(expr, 0) else {
            self.insns.truncate(start);
            self.next_reg = start_reg;
            return false;
        };
        let cj_idx = self.insns.len();
        self.emit(Insn::CaseJump(
            sel,
            Box::new(CaseJumpData {
                table: Vec::new(),
                default: 0,
            }),
        ));
        let mut table: Vec<Option<u32>> = vec![None; max_idx as usize + 1];
        let mut end_jumps: Vec<usize> = Vec::new();
        let mut default_entry: Option<u32> = None;
        // Arms are mutually exclusive; recycle their temporaries exactly as
        // the generic chain does so many-armed cases don't exhaust reg ids.
        let arm_reg_start = self.next_reg;
        let mut peak_reg = arm_reg_start;
        let mut ok = true;
        for (it, vals) in items.iter().zip(&arm_vals) {
            let entry = self.insns.len() as u32;
            if it.is_default {
                default_entry = Some(entry);
            } else {
                for &v in vals {
                    let slot = &mut table[v as usize];
                    if slot.is_none() {
                        *slot = Some(entry);
                    }
                }
            }
            if !self.compile_stmt(&it.stmt) {
                ok = false;
                break;
            }
            end_jumps.push(self.insns.len());
            self.emit(Insn::Jump(0));
            peak_reg = peak_reg.max(self.next_reg);
            self.next_reg = arm_reg_start;
        }
        if !ok {
            self.insns.truncate(start);
            self.next_reg = start_reg;
            return false;
        }
        self.next_reg = peak_reg;
        let end = self.insns.len() as u32;
        for idx in end_jumps {
            self.insns[idx] = Insn::Jump(end);
        }
        let default = default_entry.unwrap_or(end);
        let table: Vec<u32> = table.into_iter().map(|t| t.unwrap_or(default)).collect();
        static CJ_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = CJ_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if let Ok(lim) = std::env::var("XEZIM_CASEJUMP_LIMIT") {
            if let Ok(lim) = lim.parse::<usize>() {
                if n >= lim {
                    self.insns.truncate(start);
                    self.next_reg = start_reg;
                    return false;
                }
            }
        }
        if std::env::var("XEZIM_CASEJUMP_TRACE").is_ok() {
            eprintln!("[CASEJUMP] #{n} arms={} table={} scope={:?}", n_entries, table.len(), self.scope_hint);
        }
        self.insns[cj_idx] = Insn::CaseJump(sel, Box::new(CaseJumpData { table, default }));
        true
    }

    /// Two-level lowering for dense `casez`/`casex` with constant patterns
    /// (wildcard bits allowed). Picks the contiguous selector-bit window in
    /// which EVERY pattern is fully defined that best discriminates the
    /// patterns, then emits:
    ///
    ///   CaseMaskJump sel -> table[window bits] -> per-bucket residual chain
    ///                    -> xz_path (full chain) when the window has x/z
    ///
    /// Buckets are mutually exclusive for a defined-window selector (two
    /// patterns in different buckets differ in a bit both define), so
    /// per-bucket order == global order and §12.5 first-match-wins holds.
    /// The selector gate mirrors `compile_case_jump`: unsigned and <=64 bits,
    /// so extension is zero-fill and the window read equals the chain's
    /// CasezEq/CasexEq view of those bits.
    fn compile_case_mask_jump(
        &mut self,
        kind: &crate::ast::stmt::CaseKind,
        expr: &Expression,
        items: &[crate::ast::stmt::CaseItem],
    ) -> bool {
        use crate::ast::stmt::CaseKind;
        let casex = matches!(kind, CaseKind::Casex);
        if self.expr_signedness(expr) != Some(false) {
            return false;
        }
        let sel_w = self.expr_max_width(expr);
        if sel_w == 0 || sel_w > 64 {
            return false;
        }
        if std::env::var("XEZIM_CASEJUMP_OFF").is_ok() {
            return false;
        }
        // ---- shape scan: constant patterns, wildcard masks ----
        let mut default_seen = false;
        // (item index, value bits, defined mask) per pattern
        let mut pats: Vec<(usize, u64, u64)> = Vec::new();
        let mut w_cmp = sel_w;
        for (ii, it) in items.iter().enumerate() {
            if it.pattern.is_some() || it.guard.is_some() {
                return false;
            }
            if it.is_default {
                if default_seen {
                    return false;
                }
                default_seen = true;
                continue;
            }
            for pat in &it.patterns {
                let pv = match &pat.kind {
                    ExprKind::Number(n) => self.eval_number_static(n),
                    ExprKind::Ident(h) => self.lookup_param_value(h),
                    _ => None,
                };
                let Some(pv) = pv else { return false };
                if pv.is_real || pv.is_signed || pv.width == 0 || pv.width > 64 || pv.is_fill {
                    return false;
                }
                let Some((v, xz)) = pv.inline_bits() else { return false };
                let m = if pv.width >= 64 { u64::MAX } else { (1u64 << pv.width) - 1 };
                let (v, xz) = (v & m, xz & m);
                // casez: Z (val&xz both set) is wild; a plain X in the
                // pattern can match nothing defined -> keep it on the chain.
                let wild = if casex { xz } else { v & xz };
                if !casex && (xz & !wild) != 0 {
                    return false;
                }
                w_cmp = w_cmp.max(pv.width);
                // Bits past the pattern's width zero-extend (defined 0s).
                pats.push((ii, v & !wild, !wild | !m));
            }
        }
        if pats.len() < 8 {
            return false;
        }
        // ---- window choice: contiguous, all-defined, best discrimination ----
        let all_defined = pats.iter().fold(u64::MAX, |acc, (_, _, d)| acc & d)
            & if w_cmp >= 64 { u64::MAX } else { (1u64 << w_cmp) - 1 };
        let mut best: Option<(u32, u32, usize)> = None; // (lo, width, distinct)
        for lo in 0..w_cmp {
            if all_defined & (1u64 << lo) == 0 {
                continue;
            }
            let max_k = 12.min(w_cmp - lo);
            for k in 1..=max_k {
                if all_defined & (1u64 << (lo + k - 1)) == 0 {
                    break;
                }
                let wmask = (1u64 << k) - 1;
                let mut vals: Vec<u64> = pats.iter().map(|(_, v, _)| (v >> lo) & wmask).collect();
                vals.sort_unstable();
                vals.dedup();
                let distinct = vals.len();
                let better = match best {
                    None => true,
                    Some((_, bk, bd)) => {
                        distinct > bd || (distinct == bd && k < bk)
                    }
                };
                if better {
                    best = Some((lo, k, distinct));
                }
            }
        }
        let Some((lo, wk, distinct)) = best else { return false };
        // A window that cannot split the patterns at all buys nothing.
        if distinct < 2 {
            return false;
        }
        let wmask = (1u64 << wk) - 1;
        // ---- emission with all-or-nothing rollback ----
        let start = self.insns.len();
        let start_reg = self.next_reg;
        let Some(sel) = self.compile_expr(expr, 0) else {
            self.insns.truncate(start);
            self.next_reg = start_reg;
            return false;
        };
        let mj_idx = self.insns.len();
        self.emit(Insn::CaseMaskJump(
            sel,
            Box::new(CaseMaskJumpData {
                lo,
                width: wk,
                table: Vec::new(),
                xz_path: 0,
            }),
        ));
        // Arm bodies, compiled once each, ending in Jump(end).
        let mut body_entry: Vec<u32> = vec![0; items.len()];
        let mut default_entry: Option<u32> = None;
        let mut end_jumps: Vec<usize> = Vec::new();
        let arm_reg_start = self.next_reg;
        let mut peak_reg = arm_reg_start;
        let mut ok = true;
        for (ii, it) in items.iter().enumerate() {
            let entry = self.insns.len() as u32;
            body_entry[ii] = entry;
            if it.is_default {
                default_entry = Some(entry);
            }
            if !self.compile_stmt(&it.stmt) {
                ok = false;
                break;
            }
            end_jumps.push(self.insns.len());
            self.emit(Insn::Jump(0));
            peak_reg = peak_reg.max(self.next_reg);
            self.next_reg = arm_reg_start;
        }
        let mut bucket_entries: Vec<u32> = Vec::new();
        let mut xz_entry: u32 = 0;
        if ok {
            // Per-bucket residual chains.
            let n_buckets = 1usize << wk;
            bucket_entries = vec![0; n_buckets];
            'buckets: for b in 0..n_buckets {
                let subset: Vec<usize> = pats
                    .iter()
                    .enumerate()
                    .filter(|(_, (_, v, _))| ((v >> lo) & wmask) as usize == b)
                    .map(|(pi, _)| pi)
                    .collect();
                if subset.is_empty() {
                    bucket_entries[b] = u32::MAX; // patched to default later
                    continue;
                }
                bucket_entries[b] = self.insns.len() as u32;
                for pi in subset {
                    let (ii, _, _) = pats[pi];
                    let pat_expr = Self::nth_pattern(items, pi);
                    let Some(pat_reg) = self.compile_expr(pat_expr, 0) else {
                        ok = false;
                        break 'buckets;
                    };
                    let cmp = self.alloc_reg();
                    self.emit(if casex {
                        Insn::CasexEq(cmp, sel, pat_reg)
                    } else {
                        Insn::CasezEq(cmp, sel, pat_reg)
                    });
                    let bidx = self.insns.len();
                    self.emit(Insn::BranchIfFalse(cmp, 0));
                    self.emit(Insn::Jump(body_entry[ii]));
                    let next = self.insns.len() as u32;
                    self.insns[bidx] = Insn::BranchIfFalse(cmp, next);
                    peak_reg = peak_reg.max(self.next_reg);
                    self.next_reg = arm_reg_start;
                }
                let dj = self.insns.len();
                self.emit(Insn::Jump(0)); // -> default, patched below
                end_jumps.push(usize::MAX - dj); // marker: default jump
            }
        }
        if ok {
            // Full chain for wildcard selectors.
            xz_entry = self.insns.len() as u32;
            for pi in 0..pats.len() {
                let (ii, _, _) = pats[pi];
                let pat_expr = Self::nth_pattern(items, pi);
                let Some(pat_reg) = self.compile_expr(pat_expr, 0) else {
                    ok = false;
                    break;
                };
                let cmp = self.alloc_reg();
                self.emit(if casex {
                    Insn::CasexEq(cmp, sel, pat_reg)
                } else {
                    Insn::CasezEq(cmp, sel, pat_reg)
                });
                let bidx = self.insns.len();
                self.emit(Insn::BranchIfFalse(cmp, 0));
                self.emit(Insn::Jump(body_entry[ii]));
                let next = self.insns.len() as u32;
                self.insns[bidx] = Insn::BranchIfFalse(cmp, next);
                peak_reg = peak_reg.max(self.next_reg);
                self.next_reg = arm_reg_start;
            }
            let dj = self.insns.len();
            self.emit(Insn::Jump(0));
            end_jumps.push(usize::MAX - dj);
        }
        if !ok {
            self.insns.truncate(start);
            self.next_reg = start_reg;
            return false;
        }
        self.next_reg = peak_reg;
        let end = self.insns.len() as u32;
        let default = default_entry.unwrap_or(end);
        for ej in end_jumps {
            if ej > usize::MAX / 2 {
                self.insns[usize::MAX - ej] = Insn::Jump(default);
            } else {
                self.insns[ej] = Insn::Jump(end);
            }
        }
        let table: Vec<u32> = bucket_entries
            .into_iter()
            .map(|t| if t == u32::MAX { default } else { t })
            .collect();
        self.insns[mj_idx] = Insn::CaseMaskJump(
            sel,
            Box::new(CaseMaskJumpData {
                lo,
                width: wk,
                table,
                xz_path: xz_entry,
            }),
        );
        static CMJ_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = CMJ_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if std::env::var("XEZIM_CASEJUMP_TRACE").is_ok() {
            eprintln!(
                "[CASEMASKJUMP] #{n} pats={} lo={lo} w={wk} distinct={distinct} scope={:?}",
                pats.len(),
                self.scope_hint
            );
        }
        true
    }

    /// The `pi`-th pattern in flat (item, pattern) order — the order
    /// `compile_case_mask_jump`'s scan built its `pats` list in.
    fn nth_pattern<'e>(
        items: &'e [crate::ast::stmt::CaseItem],
        pi: usize,
    ) -> &'e Expression {
        let mut k = 0usize;
        for it in items {
            if it.is_default {
                continue;
            }
            for pat in &it.patterns {
                if k == pi {
                    return pat;
                }
                k += 1;
            }
        }
        unreachable!("pattern index out of range")
    }

    /// A set-membership member as a compile-time CONSTANT with no x/z bits —
    /// the only shape `compile_inside` accepts (see there).
    fn inside_member_const(&mut self, m: &Expression) -> Option<Value> {
        let v = match &m.kind {
            ExprKind::Number(num) => self.eval_number_static(num)?,
            ExprKind::Ident(h) => self.lookup_param_value(h)?,
            ExprKind::Paren(inner) => return self.inside_member_const(inner),
            _ => return None,
        };
        if v.has_xz() || v.is_real {
            return None;
        }
        Some(v)
    }

    fn lookup_signal_id(&self, hier: &HierarchicalIdentifier) -> Option<usize> {
        let raw = Self::hier_raw_name(hier);
        // Targeted override for for-loop variables — see for_loop_var_ids
        // doc + compile_for's comment for the c910 motivation.
        if !self.for_loop_var_ids.is_empty() && hier.path.len() == 1 && !raw.contains('.') {
            if let Some(&id) = self.for_loop_var_ids.get(&raw) {
                return Some(id);
            }
        }
        // Scope-first for SINGLE-SEGMENT bare names — LRM §22.4 / §23.6: a
        // local declaration shadows a same-named wildcard-imported member.
        // Without this, a module's local anon-enum FINISH=2 resolves to
        // pkg mult_state_e::FINISH=4 because the flat signal_name_to_id
        // registers BOTH `FINISH` (pkg) and `<scope>.FINISH` (local).
        // A parent-rooted ident (substituted expression port actual) is an
        // absolute name — the block's child scope hint must not qualify it.
        let rooted = hier.root.is_some();
        if !raw.contains('.') && !rooted {
            if let Some(scope) = &self.scope_hint {
                let qualified = format!("{}.{}", scope, raw);
                if let Some(&id) = self.signal_name_to_id.get(qualified.as_str()) {
                    return Some(id);
                }
            }
        }
        if let Some(&id) = self.signal_name_to_id.get(raw.as_str()) {
            return Some(id);
        }
        if !rooted {
            if let Some(scope) = &self.scope_hint {
                let qualified = format!("{}.{}", scope, raw);
                if let Some(&id) = self.signal_name_to_id.get(qualified.as_str()) {
                    return Some(id);
                }
            }
        }
        if hier.path.len() == 1 {
            let leaf = &hier.path[0].name.name;
            if let Some(&id) = self.signal_name_to_id.get(leaf.as_str()) {
                return Some(id);
            }
        }
        // Top-prefix strip: `<top>.<rest>` → `<rest>` for cross-hierarchical
        // refs whose absolute path was baked in by xezim's port-rewriting
        // (top-level instances have no prefix in signal_name_to_id).
        if let Some(top) = &self.top_module_name {
            let with_dot = format!("{}.", top);
            if let Some(stripped) = raw.strip_prefix(&with_dot) {
                if let Some(&id) = self.signal_name_to_id.get(stripped) {
                    return Some(id);
                }
            }
        }
        None
    }

    fn lookup_signal_id_by_name(&self, name: &str) -> Option<usize> {
        self.signal_name_to_id.get(name).copied()
    }

    fn lookup_param_value(&self, hier: &HierarchicalIdentifier) -> Option<Value> {
        let params = self.params?;
        let raw = Self::hier_raw_name(hier);
        if let Some(v) = params.get(&raw) {
            return Some(v.clone());
        }
        if let Some(scope) = &self.scope_hint {
            let q = format!("{}.{}", scope, raw);
            if let Some(v) = params.get(&q) {
                return Some(v.clone());
            }
        }
        if hier.path.len() == 1 {
            if let Some(v) = params.get(&hier.path[0].name.name) {
                return Some(v.clone());
            }
        }
        // Suffix-match: bare `CARRY_CHAIN` may be stored as
        // `top.uut.picorv32_core.pcpi_mul.CARRY_CHAIN`. Only accept if a
        // single param key matches — multiple matches are ambiguous.
        //
        // Any match in either direction shares the LAST dotted segment with
        // `raw`, so the leaf index narrows the scan to same-leaf keys.
        let is_match = |name: &str| -> bool {
            let raw_has_key_suffix = raw.len() >= name.len()
                && raw.ends_with(name)
                && (raw.len() == name.len() || raw.as_bytes()[raw.len() - name.len() - 1] == b'.');
            let key_has_raw_suffix = name.len() >= raw.len()
                && name.ends_with(raw.as_str())
                && (name.len() == raw.len() || name.as_bytes()[name.len() - raw.len() - 1] == b'.');
            raw_has_key_suffix || key_has_raw_suffix
        };
        let mut found: Option<&Value> = None;
        if let Some(idx) = self.param_leaf_idx {
            let leaf = raw.rsplit('.').next().unwrap_or(raw.as_str());
            for name in idx.get(leaf).map(|v| v.as_slice()).unwrap_or(&[]) {
                if is_match(name) {
                    if found.is_some() {
                        return None;
                    }
                    found = params.get(name);
                }
            }
        } else {
            for (name, value) in params {
                if is_match(name) {
                    if found.is_some() {
                        return None;
                    }
                    found = Some(value);
                }
            }
        }
        found.cloned()
    }

    fn expr_to_signal_id(&self, expr: &Expression) -> Option<usize> {
        match &expr.kind {
            ExprKind::Ident(hier) => self.lookup_signal_id(hier),
            ExprKind::Paren(inner) => self.expr_to_signal_id(inner),
            _ => None,
        }
    }

    /// Resolve a fully-indexed 2D/ND unpacked-array element whose indices are
    /// compile-time constants. Elaboration materializes these cells as scalar
    /// signals named `base[i][j]...`, so generated flop arrays can use the
    /// ordinary scalar bytecode paths instead of falling back to the AST.
    fn const_multi_dim_array_elem_signal_id(&self, expr: &Expression) -> Option<usize> {
        if !matches!(expr.kind, ExprKind::Index { .. }) {
            return None;
        }

        fn collect<'e>(
            compiler: &BytecodeCompiler<'_>,
            expr: &'e Expression,
            indices: &mut Vec<u32>,
        ) -> Option<&'e HierarchicalIdentifier> {
            match &expr.kind {
                ExprKind::Index { expr, index } => {
                    let hier = collect(compiler, expr, indices)?;
                    indices.push(compiler.eval_const_expr(index)?);
                    Some(hier)
                }
                ExprKind::Paren(inner) => collect(compiler, inner, indices),
                ExprKind::Ident(hier) => Some(hier),
                _ => None,
            }
        }

        let mut indices = Vec::new();
        let hier = collect(self, expr, &mut indices)?;
        if indices.len() < 2 || !self.is_multi_dim_array(hier) {
            return None;
        }

        let raw = Self::hier_raw_name(hier);
        let mut indexed = raw.clone();
        for index in indices {
            indexed.push('[');
            indexed.push_str(&index.to_string());
            indexed.push(']');
        }

        if !raw.contains('.') {
            if let Some(scope) = &self.scope_hint {
                if let Some(&id) = self
                    .signal_name_to_id
                    .get(format!("{}.{}", scope, indexed).as_str())
                {
                    return Some(id);
                }
            }
        }
        if let Some(&id) = self.signal_name_to_id.get(indexed.as_str()) {
            return Some(id);
        }
        if let Some(scope) = &self.scope_hint {
            if let Some(&id) = self
                .signal_name_to_id
                .get(format!("{}.{}", scope, indexed).as_str())
            {
                return Some(id);
            }
        }
        None
    }

    fn flattened_outer_const_signal_id(&self, expr: &Expression) -> Option<usize> {
        let ExprKind::Index { expr: base, index } = &expr.kind else {
            return None;
        };
        // Generated-loop expansion has already selected the unpacked element
        // and baked its index into the hierarchical instance path. The AST
        // retains the now-redundant constant select (`flat[i][bit]`), while
        // the signal table contains `flat` as the selected packed element.
        // Accept every constant here, not only zero; the shape guards below
        // keep real unpacked and multi-dimensional packed arrays out.
        self.eval_const_expr(index)?;
        let ExprKind::Ident(hier) = &base.kind else {
            return None;
        };
        if self.lookup_array_name(hier).is_some() {
            return None;
        }
        // A multi-D PACKED base (`logic [1:0][3:0][7:0] foo`) is NOT a
        // flattening no-op: `foo[0]` selects a slice, so `foo[0][j]` must
        // not degrade to a bit-select of the whole vector (§7.4.1).
        if self.packed_elem_width_of(hier).is_some() {
            return None;
        }
        // A genuine 2D/ND UNPACKED array (`logic [7:0] m [2][2]`) also carries
        // a bogus scalar signal for its base name; `m[0][j]` must select the
        // element (interpreter path), NOT bit-select that scalar.
        if self.is_multi_dim_array(hier) {
            return None;
        }
        self.lookup_signal_id(hier)
    }

    /// True when `hier`'s base name is a registered 2D/ND unpacked array.
    fn is_multi_dim_array(&self, hier: &HierarchicalIdentifier) -> bool {
        let Some(set) = self.multi_dim_arrays else {
            return false;
        };
        let raw = Self::hier_raw_name(hier);
        if set.contains(raw.as_str()) {
            return true;
        }
        if let Some(scope) = &self.scope_hint {
            if set.contains(format!("{}.{}", scope, raw).as_str()) {
                return true;
            }
        }
        if hier.path.len() == 1
            && set.contains(hier.path[0].name.name.as_str()) {
                return true;
            }
        false
    }

    /// Walk a chain of `Index` nodes down to its root identifier. Returns the
    /// root and the index EXPRESSIONS outermost-first; indices need not be
    /// constant.
    fn flatten_index_chain_exprs<'e>(
        &self,
        base: &'e Expression,
        index: &'e Expression,
    ) -> Option<(&'e HierarchicalIdentifier, Vec<&'e Expression>)> {
        let mut idxs: Vec<&Expression> = vec![index];
        let mut cur = base;
        loop {
            match &cur.kind {
                ExprKind::Index { expr, index } => {
                    idxs.push(index);
                    cur = expr.as_ref();
                }
                ExprKind::Ident(h) => {
                    idxs.reverse();
                    return Some((h, idxs));
                }
                _ => return None,
            }
        }
    }

    /// Walk a chain of constant `Index` nodes down to its root identifier.
    /// Returns the root and the indices outermost-first.
    fn flatten_const_index_chain<'e>(
        &self,
        base: &'e Expression,
        index: &Expression,
    ) -> Option<(&'e HierarchicalIdentifier, Vec<i64>)> {
        let mut idxs = vec![self.eval_const_expr(index)? as i64];
        let mut cur = base;
        loop {
            match &cur.kind {
                ExprKind::Index { expr, index } => {
                    idxs.push(self.eval_const_expr(index)? as i64);
                    cur = expr.as_ref();
                }
                ExprKind::Ident(h) => {
                    idxs.reverse();
                    return Some((h, idxs));
                }
                _ => return None,
            }
        }
    }

    /// Declared dimensions of a chain's root, if registered.
    fn chain_root_dims(&self, hier: &HierarchicalIdentifier) -> Option<Vec<(i64, i64)>> {
        self.packed_full_dims_of(hier).cloned()
    }

    /// Emit a chained packed element select whose indices are NOT all
    /// constant — `a[i][j][3]` inside a loop. The selected WIDTH is still
    /// static (it depends only on how many dimensions were consumed), so only
    /// the offset needs computing at run time:
    /// `off = Σ slot_k * (product of the counts below level k)`.
    ///
    /// Without this, a dynamic chain fell through to the plain bit-select path
    /// and read x — the shape a `for (i) for (j) vld[i][j][0]` checker loop
    /// produces, which is why such loops had to be hand-unrolled to work.
    fn emit_chained_packed_slice_dyn(
        &mut self,
        base: &Expression,
        index: &Expression,
    ) -> Option<RegId> {
        let (hier, idx_exprs) = self.flatten_index_chain_exprs(base, index)?;
        if idx_exprs.len() < 2 {
            return None;
        }
        let dims = self.chain_root_dims(hier)?;
        if dims.len() < idx_exprs.len() {
            return None;
        }
        let counts: Vec<i64> = dims.iter().map(|(l, r)| (l - r).abs() + 1).collect();
        let width: i64 = counts[idx_exprs.len()..].iter().product();
        if width <= 0 {
            return None;
        }
        // Compile every index first; bail before emitting any accumulation if
        // one of them cannot be compiled.
        let mut idx_regs = Vec::with_capacity(idx_exprs.len());
        for e in &idx_exprs {
            idx_regs.push(self.compile_expr(e, 0)?);
        }
        let root = self.compile_expr_root_of(base)?;
        let mut off_reg: Option<RegId> = None;
        for (k, idx_reg) in idx_regs.into_iter().enumerate() {
            let (l, r) = dims[k];
            let (lo_b, hi_b) = (l.min(r), l.max(r));
            let elem_w: i64 = counts[k + 1..].iter().product();
            // §7.4.1: descending labels the LEFT bound most-significant, so the
            // slot counts up from the low bound; ascending reverses it.
            let slot = if l >= r {
                if lo_b == 0 {
                    idx_reg
                } else {
                    let c = self.alloc_reg();
                    self.emit(Insn::LoadConst(c, Box::new(Value::from_u64(lo_b as u64, 32))));
                    let d = self.alloc_reg();
                    self.emit(Insn::Sub(d, idx_reg, c));
                    d
                }
            } else {
                let c = self.alloc_reg();
                self.emit(Insn::LoadConst(c, Box::new(Value::from_u64(hi_b as u64, 32))));
                let d = self.alloc_reg();
                self.emit(Insn::Sub(d, c, idx_reg));
                d
            };
            let term = if elem_w == 1 {
                slot
            } else {
                let w = self.alloc_reg();
                self.emit(Insn::LoadConst(w, Box::new(Value::from_u64(elem_w as u64, 32))));
                let t = self.alloc_reg();
                self.emit(Insn::Mul(t, slot, w));
                t
            };
            off_reg = Some(match off_reg {
                None => term,
                Some(acc) => {
                    let a = self.alloc_reg();
                    self.emit(Insn::Add(a, acc, term));
                    a
                }
            });
        }
        let lo_reg = off_reg?;
        let hi_reg = if width == 1 {
            lo_reg
        } else {
            let wm1 = self.alloc_reg();
            self.emit(Insn::LoadConst(wm1, Box::new(Value::from_u64((width - 1) as u64, 32))));
            let h = self.alloc_reg();
            self.emit(Insn::Add(h, lo_reg, wm1));
            h
        };
        let dest = self.alloc_reg();
        self.emit(Insn::RangeSelect(dest, root, hi_reg, lo_reg));
        Some(dest)
    }

    /// `(lsb, width)` of a chained packed element select, from the root's
    /// declared dimensions. None unless there are at least TWO indices and the
    /// root has enough registered dimensions — a single-level select keeps its
    /// existing, separately-tested path.
    fn chained_packed_slice(&self, base: &Expression, index: &Expression) -> Option<(u32, u32)> {
        let (hier, idxs) = self.flatten_const_index_chain(base, index)?;
        if idxs.len() < 2 {
            return None;
        }
        let dims: Vec<(i64, i64)> = self.chain_root_dims(hier)?;
        if dims.len() < idxs.len() {
            return None;
        }
        let counts: Vec<i64> = dims.iter().map(|(l, r)| (l - r).abs() + 1).collect();
        let mut off: i64 = 0;
        for (k, &d) in idxs.iter().enumerate() {
            let (l, r) = dims[k];
            let (lo_b, hi_b) = (l.min(r), l.max(r));
            if d < lo_b || d > hi_b {
                return None;
            }
            // §7.4.1: a descending range labels the LEFT bound as the most
            // significant element; an ascending one reverses the slot order.
            let slot = if l >= r { d - lo_b } else { hi_b - d };
            let elem_w: i64 = counts[k + 1..].iter().product();
            off = off.checked_add(slot.checked_mul(elem_w)?)?;
        }
        let width: i64 = counts[idxs.len()..].iter().product();
        Some((u32::try_from(off).ok()?, u32::try_from(width).ok()?))
    }

    /// Compile the ROOT identifier of an index chain (the whole backing
    /// vector), so a chained slice can be taken out of it.
    fn compile_expr_root_of(&mut self, e: &Expression) -> Option<RegId> {
        let mut cur = e;
        while let ExprKind::Index { expr, .. } = &cur.kind {
            cur = expr.as_ref();
        }
        let root = cur.clone();
        self.compile_expr(&root, 0)
    }

    /// The base's registered packed ELEMENT width (>1), if it is a
    /// multi-dimensional packed vector (`logic [3:0][7:0] x`).
    fn packed_elem_width_of(&self, hier: &HierarchicalIdentifier) -> Option<u32> {
        let raw = Self::hier_raw_name(hier);
        self.packed_elem_widths
            .and_then(|m| {
                m.get(raw.as_str()).copied()
                    // Inside an inlined instance the name is spelled bare
                    // while the table holds it under the instance path.
                    .or_else(|| {
                        self.scope_hint
                            .as_ref()
                            .and_then(|sc| m.get(format!("{}.{}", sc, raw).as_str()).copied())
                    })
                    // The bare-leaf fallback is for a SINGLE-segment name only.
                    // Applying it to `inp.sram_renA` (a packed-struct member
                    // of a port) matched an unrelated `logic [3:0][1:0]
                    // sram_renA` declared in another module, and the
                    // member's bit-select compiled as that array's two-bit
                    // element select: bits 2 and 3 read x.
                    .or_else(|| {
                        if hier.path.len() != 1 {
                            return None;
                        }
                        hier.path
                            .last()
                            .and_then(|s| m.get(s.name.name.as_str()).copied())
                    })
            })
            .filter(|&w| w > 1)
    }

    /// §7.4.1: physical LSB offset of declared bit index 0 does not exist for
    /// a non-zero-based vector — `logic [3:1] w` stores declared bit 1 at
    /// physical offset 0. Returns the declared range's LOW bound when it is
    /// non-zero (descending ranges only; ascending is handled elsewhere), so
    /// write emission can rebase declared indices the way the read path
    /// already does. Every `*AssignRange`/`*AssignBitDyn` the compiler emits
    /// carries PHYSICAL offsets by contract — the interpreter and the JIT
    /// both index raw bits.
    /// Emit `idx_reg - declared_low_bound` when the vector is non-zero-based;
    /// pass-through otherwise. Used by every dynamic-index WRITE emission.
    fn emit_rebased_index(
        &mut self,
        hier: &HierarchicalIdentifier,
        idx_reg: RegId,
    ) -> RegId {
        let base_lo = self.declared_low_bound(hier);
        if base_lo == 0 {
            return idx_reg;
        }
        let base_reg = self.alloc_reg();
        self.emit(Insn::LoadConst(
            base_reg,
            Box::new(Value::from_u64(base_lo as u64, 32)),
        ));
        let adj = self.alloc_reg();
        self.emit(Insn::Sub(adj, idx_reg, base_reg));
        adj
    }

    fn declared_low_bound(&self, hier: &HierarchicalIdentifier) -> i64 {
        self.packed_outer_dim(hier)
            .map(|(dl, dr)| dl.min(dr))
            .unwrap_or(0)
    }

    /// ASCENDING declared range (`logic [0:23]`): bit/part labels mirror
    /// against the right bound — the low-bound rebase the emitted insns use
    /// cannot express that, so such selects must run on the AST interpreter.
    fn dim_is_ascending(d: Option<(i64, i64)>) -> bool {
        matches!(d, Some((l, r)) if l < r)
    }

    /// Trailing bit/part select whose base is an Index CHAIN over a signal
    /// whose packed dims need label mapping — an ELEMENT of an unpacked
    /// collection declared `logic [31:8]`/`logic [0:23]` style, or a packed
    /// element under such an inner dim. The emitted select/store insns index
    /// raw physical bits, so these shapes must bail to the AST interpreter
    /// (which maps labels via `select_base_elem_dim`).
    fn elem_chain_needs_ast(&self, base: &Expression) -> bool {
        let mut cur = base;
        let mut layers = 0usize;
        while let ExprKind::Index { expr: inner, .. } = &cur.kind {
            cur = inner;
            layers += 1;
        }
        if layers == 0 {
            return false;
        }
        let ExprKind::Ident(h) = &cur.kind else {
            return false;
        };
        let raw = Self::hier_raw_name(h);
        let leaf = h.path.last().map(|s| s.name.name.as_str()).unwrap_or("");
        let Some(dims) = self.packed_full_dims.and_then(|m| {
            m.get(raw.as_str()).or_else(|| m.get(leaf))
        }) else {
            return false;
        };
        let is_coll = |n: &str| -> bool {
            self.array_first_id.is_some_and(|m| m.contains_key(n))
                || self.multi_dim_arrays.is_some_and(|s| s.contains(n))
                || self.dynamic_arrays.is_some_and(|s| s.contains(n))
                || self.queue_vars.is_some_and(|s| s.contains(n))
                || self.assoc_arrays.is_some_and(|m| m.contains_key(n))
        };
        let needs = |d: &(i64, i64)| d.0 < d.1 || d.0.min(d.1) != 0;
        if is_coll(raw.as_str()) || is_coll(leaf) {
            // The exact unpacked depth is not knowable here — be
            // conservative: any mapping dim forces the AST path.
            dims.iter().any(needs)
        } else {
            dims.get(layers).is_some_and(needs)
        }
    }

    /// Combined guard for the plain bit/range select and store arms: bail
    /// when the base needs a label mapping those arms do not emit. A packed
    /// multi-D Ident base is excluded — its element machinery normalizes
    /// slots itself (including ascending outer dims).
    fn sel_base_needs_ast(&self, base: &Expression) -> bool {
        match &base.kind {
            ExprKind::Ident(h) => {
                self.packed_elem_width_of(h).filter(|&w| w > 1).is_none()
                    && Self::dim_is_ascending(self.packed_outer_dim(h))
            }
            ExprKind::Index { .. } => self.elem_chain_needs_ast(base),
            _ => false,
        }
    }

    fn flattened_const_range_target(
        &self,
        expr: &Expression,
        left: &Expression,
        right: &Expression,
    ) -> Option<(usize, u32, u32)> {
        let ExprKind::Index { expr: base, index } = &expr.kind else {
            return None;
        };
        let outer = self.eval_const_expr(index)?;
        let ExprKind::Ident(hier) = &base.kind else {
            return None;
        };
        if self.lookup_array_name(hier).is_some() {
            return None;
        }
        if self.is_multi_dim_array(hier) {
            return None;
        }
        let id = self.lookup_signal_id(hier)?;
        let l = self.eval_const_expr(left)?;
        let r = self.eval_const_expr(right)?;
        let (lo, hi) = if l >= r { (r, l) } else { (l, r) };
        // §7.4.1: the element stride is the DECLARED element width, not the
        // slice width — `d[1][31:0]` on `logic [4:0][63:0] d` targets bits
        // [95:64], not [63:32]. The slice-width fallback stays for signals
        // with no packed metadata (the generated-loop flattening case this
        // helper was built for, where slice == element by construction).
        let (stride, base_bit) = match self.packed_elem_width_of(hier) {
            Some(decl_ew) => {
                if hi >= decl_ew {
                    return None; // slice exceeds the element: not this shape
                }
                let dim = self.packed_outer_dim(hier);
                let lsb = Self::packed_elem_lsb(dim, outer as i64, decl_ew);
                if lsb < 0 {
                    return None;
                }
                (0, lsb as u32)
            }
            None => (hi - lo + 1, 0),
        };
        let flat_lo = if stride == 0 {
            base_bit.checked_add(lo)?
        } else {
            outer.checked_mul(stride)?.checked_add(lo)?
        };
        let flat_hi = if stride == 0 {
            base_bit.checked_add(hi)?
        } else {
            outer.checked_mul(stride)?.checked_add(hi)?
        };
        if flat_hi < self.signal_widths[id] {
            Some((id, flat_hi, flat_lo))
        } else {
            None
        }
    }

    /// Signal id of a multi-dimensional unpacked array element addressed as
    /// `base[i][j]…` with CONSTANT indices. `outer_base` is the expression to
    /// the left of the final index (itself one or more Index nodes over an
    /// Ident); `last_index` is the final subscript. Returns None for a dynamic
    /// index or when no such element is registered.
    fn multi_dim_elem_signal_id(
        &self,
        outer_base: &Expression,
        last_index: &Expression,
    ) -> Option<usize> {
        // Only nested indexing can name a multi-dim element.
        if !matches!(outer_base.kind, ExprKind::Index { .. }) {
            return None;
        }
        let mut subs: Vec<u32> = vec![self.eval_const_expr(last_index)?];
        let mut cur = outer_base;
        let hier = loop {
            match &cur.kind {
                ExprKind::Index { expr: b, index: i } => {
                    subs.push(self.eval_const_expr(i)?);
                    cur = b;
                }
                ExprKind::Ident(h) => break h,
                _ => return None,
            }
        };
        subs.reverse();
        let raw = Self::hier_raw_name(hier);
        let suffix: String = subs.iter().map(|i| format!("[{}]", i)).collect();
        let mut candidates = vec![format!("{}{}", raw, suffix)];
        if let Some(scope) = &self.scope_hint {
            candidates.push(format!("{}.{}{}", scope, raw, suffix));
        }
        if let Some(leaf) = hier.path.last() {
            candidates.push(format!("{}{}", leaf.name.name, suffix));
        }
        candidates
            .iter()
            .find_map(|n| self.lookup_signal_id_by_name(n.as_str()))
    }

    fn lookup_array_name(&self, hier: &HierarchicalIdentifier) -> Option<String> {
        // An ASSOCIATIVE array is never a dense array, whatever tables its
        // name also appears in: a dense LoadArrayElem keyed by (say) a string
        // literal reads garbage. Assoc lvalues have their own path
        // (`is_assoc_target`); reads that land here must bail instead.
        if self.is_assoc_target(hier) {
            return None;
        }
        let raw = Self::hier_raw_name(hier);
        let dense = |name: &str| -> bool {
            // An ARRAY OF COLLECTIONS registers its outer shape in `arrays`
            // but each element is its own queue/dynamic/associative container
            // (`int a[2][u8_t]`), living OUTSIDE the dense cells this name
            // resolves to — a compiled LoadArrayElem read the fake backing
            // and `mem[h][addr]` compared against garbage the moment the
            // enclosing loop compiled (found when the new do-while arm
            // compiled a block the old bail had kept on the AST path). The
            // element registrations are keyed `name[lo]`, so probe that.
            let Some((lo, _, _)) = self.arrays.get(name) else {
                return false;
            };
            let elem = format!("{}[{}]", name, lo);
            !(self.assoc_arrays.is_some_and(|m| m.contains_key(&elem))
                || self.queue_vars.is_some_and(|m| m.contains(&elem))
                || self.dynamic_arrays.is_some_and(|m| m.contains(&elem)))
        };
        if self.arrays.contains_key(&raw) {
            return dense(&raw).then_some(raw);
        }
        if let Some(scope) = &self.scope_hint {
            let qualified = format!("{}.{}", scope, raw);
            if self.arrays.contains_key(&qualified) {
                return dense(&qualified).then_some(qualified);
            }
        }
        if hier.path.len() == 1 {
            let leaf = &hier.path[0].name.name;
            if self.arrays.contains_key(leaf) {
                return dense(leaf).then_some(leaf.clone());
            }
        }
        None
    }

    /// Hoist disjoint full-range `dst[i] <= src[i]` copies out of a canonical
    /// packed loop. All slices are written in the same NBA region and the
    /// sources are side-effect-free signal reads, so a single whole-vector NBA
    /// is equivalent while avoiding loop-control and per-slice queue work.
    /// Every uncertain shape returns None and uses normal lowering.
    fn full_range_nba_copy_plan(
        &self,
        init: &[ForInit],
        condition: Option<&Expression>,
        step: &[Expression],
        body: &Statement,
    ) -> Option<(Vec<(usize, usize, u32)>, Statement)> {
        let [ForInit::VarDecl {
            name: loop_id,
            init: start_expr,
            ..
        }] = init else {
            return None;
        };
        let loop_name = loop_id.name.as_str();
        let start = self.eval_const_expr(start_expr)? as i64;
        let ExprKind::Binary {
            op: cmp,
            left,
            right: bound_expr,
        } = &condition?.kind else {
            return None;
        };
        if !matches!(cmp, BinaryOp::Lt | BinaryOp::Leq)
            || Self::plain_loop_ident(left) != Some(loop_name)
        {
            return None;
        }
        let bound = self.eval_const_expr(bound_expr)? as i64;
        let end = if matches!(cmp, BinaryOp::Lt) {
            bound.checked_sub(1)?
        } else {
            bound
        };
        if end < start {
            return None;
        }
        let [step_expr] = step else { return None };
        let ExprKind::Unary { op, operand } = &step_expr.kind else {
            return None;
        };
        if !matches!(op, UnaryOp::PreIncr | UnaryOp::PostIncr)
            || Self::plain_loop_ident(operand) != Some(loop_name)
        {
            return None;
        }
        let count = u32::try_from(end - start + 1).ok()?;
        if count <= 1 {
            return None;
        }

        let (block_name, stmts): (Option<_>, &[Statement]) = match &body.kind {
            StatementKind::SeqBlock { name, stmts } => (name.clone(), stmts),
            _ => (None, std::slice::from_ref(body)),
        };
        // A blocking assignment, timing control, or call-as-statement could
        // change a copied source between iterations. The target loop is all
        // delay-free NBAs, whose queued updates cannot affect later samples.
        if !stmts.iter().all(|st| {
            matches!(
                &st.kind,
                StatementKind::NonblockingAssign { delay: None, .. }
            )
        }) {
            return None;
        }

        let mut candidates: Vec<(usize, usize, usize, u32)> = Vec::new();
        for (si, st) in stmts.iter().enumerate() {
            let StatementKind::NonblockingAssign {
                lvalue,
                delay: None,
                rvalue,
            } = &st.kind else {
                continue;
            };
            let Some((dst_h, dst_idx)) = Self::plain_indexed_signal(lvalue) else {
                continue;
            };
            let Some((src_h, src_idx)) = Self::plain_indexed_signal(rvalue) else {
                continue;
            };
            if Self::plain_loop_ident(dst_idx) != Some(loop_name)
                || Self::plain_loop_ident(src_idx) != Some(loop_name)
            {
                continue;
            }
            if self.lookup_array_name(dst_h).is_some() || self.lookup_array_name(src_h).is_some() {
                continue;
            }
            let Some(dst_dim) = self.packed_outer_dim(dst_h) else {
                continue;
            };
            let Some(src_dim) = self.packed_outer_dim(src_h) else {
                continue;
            };
            let covers = |(l, r): (i64, i64)| {
                start == l.min(r) && end == l.max(r) && count as i64 == (l - r).abs() + 1
            };
            // A whole assignment preserves element identity only when source
            // and destination use the same declared orientation.
            if dst_dim != src_dim || !covers(dst_dim) || !covers(src_dim) {
                continue;
            }
            let Some(dst) = self.lookup_signal_id(dst_h) else {
                continue;
            };
            let Some(src) = self.lookup_signal_id(src_h) else {
                continue;
            };
            let Some(&dw) = self.signal_widths.get(dst) else {
                continue;
            };
            let Some(&sw) = self.signal_widths.get(src) else {
                continue;
            };
            let de = self.infer_lhs_width(lvalue).max(1);
            let se = self.infer_lhs_width(rvalue).max(1);
            if dw != sw
                || de != se
                || de.checked_mul(count) != Some(dw)
                || se.checked_mul(count) != Some(sw)
                || dst == src
            {
                continue;
            }
            candidates.push((si, dst, src, dw));
        }
        if candidates.is_empty() {
            return None;
        }
        let mut unique_dests: HashSet<usize> = HashSet::default();
        if candidates
            .iter()
            .any(|(_, dst, _, _)| !unique_dests.insert(*dst))
        {
            return None;
        }
        // Sampling may move ahead of the residual loop only when no copied
        // source is also written by another hoisted copy.
        let dests: HashSet<usize> = candidates.iter().map(|(_, d, _, _)| *d).collect();
        if candidates.iter().any(|(_, _, s, _)| dests.contains(s)) {
            return None;
        }
        let skipped: HashSet<usize> = candidates.iter().map(|(i, ..)| *i).collect();
        // Preserve last-NBA-wins ordering. A non-identity NBA to a candidate
        // destination may occur before or after the identity copy in the
        // original body; moving every identity copy ahead of the loop would
        // otherwise reverse one of those cases.
        for (i, st) in stmts.iter().enumerate() {
            if skipped.contains(&i) {
                continue;
            }
            let StatementKind::NonblockingAssign { lvalue, .. } = &st.kind else {
                return None;
            };
            let Some(root) = Self::plain_selected_signal_root(lvalue) else {
                return None;
            };
            let Some(id) = self.lookup_signal_id(root) else {
                return None;
            };
            if dests.contains(&id) {
                return None;
            }
        }
        let kept: Vec<Statement> = stmts
            .iter()
            .enumerate()
            .filter(|(i, _)| !skipped.contains(i))
            .map(|(_, st)| st.clone())
            .collect();
        let pruned = if matches!(&body.kind, StatementKind::SeqBlock { .. }) {
            Statement::new(
                StatementKind::SeqBlock {
                    name: block_name,
                    stmts: kept,
                },
                body.span,
            )
        } else {
            Statement::new(StatementKind::Null, body.span)
        };
        let plans = candidates
            .into_iter()
            .map(|(_, d, s, w)| (d, s, w))
            .collect();
        Some((plans, pruned))
    }

    fn plain_loop_ident(expr: &Expression) -> Option<&str> {
        match &expr.kind {
            ExprKind::Ident(h) if h.root.is_none() && h.path.len() == 1 => {
                Some(h.path[0].name.name.as_str())
            }
            ExprKind::Paren(inner) => Self::plain_loop_ident(inner),
            _ => None,
        }
    }

    fn plain_indexed_signal(
        expr: &Expression,
    ) -> Option<(&HierarchicalIdentifier, &Expression)> {
        let ExprKind::Index { expr: base, index } = &expr.kind else {
            return None;
        };
        let ExprKind::Ident(h) = &base.kind else {
            return None;
        };
        if h.path.iter().any(|seg| !seg.selects.is_empty()) {
            return None;
        }
        Some((h, index))
    }

    fn plain_selected_signal_root(expr: &Expression) -> Option<&HierarchicalIdentifier> {
        match &expr.kind {
            ExprKind::Ident(h) if h.path.iter().all(|seg| seg.selects.is_empty()) => Some(h),
            ExprKind::Paren(inner)
            | ExprKind::Index { expr: inner, .. }
            | ExprKind::RangeSelect { expr: inner, .. } => {
                Self::plain_selected_signal_root(inner)
            }
            _ => None,
        }
    }

    /// Compile a statement. Returns true on success.
    /// When `allow_ast_fallback` is set, any nested failure rolls back and
    /// emits a single `StmtFallback` for the whole statement.
    pub fn compile_stmt(&mut self, stmt: &Statement) -> bool {
        // §6.21: a block-local declaration that SHADOWS a module signal needs
        // the whole enclosing block interpreted as one unit — the AST path
        // pushes a shadow frame for the block's duration, which per-statement
        // StmtFallback insns cannot reproduce (the local would clobber the
        // module variable). Failing WITHOUT fallback here makes the enclosing
        // SeqBlock's own wrapper roll back and emit a single whole-block
        // StmtFallback instead.
        if let StatementKind::VarDecl { declarators, .. } = &stmt.kind {
            if declarators
                .iter()
                .any(|d| {
                    self.signal_name_to_id.contains_key(d.name.name.as_str())
                        || self.process_local_names.contains(d.name.name.as_str())
                })
            {
                self.bail("VarDecl_shadows_signal");
                return false;
            }
        }
        let start = self.insns.len();
        let start_reg = self.next_reg;
        let saved_reason = self.bail_reason;
        let saved_overflow = self.register_overflow;
        self.bail_reason = None;
        self.register_overflow = false;
        let strict_ok = self.compile_stmt_strict(stmt);
        if strict_ok && !self.register_overflow {
            self.bail_reason = saved_reason;
            self.register_overflow = saved_overflow;
            return true;
        }
        if self.register_overflow {
            self.bail("bytecode_register_limit");
        }
        // Same guard as emit_fallback: inside a loop whose counter lives in
        // a VM register, a per-statement fallback reads the loop var as a
        // (non-existent) SIGNAL — `wr[i] = req.vld` inside `for (int i;…)`
        // silently indexed with x and wrote NOTHING, 16 times per fire,
        // while the entry reported success. Fail the statement instead so
        // the whole loop (or block) rolls back to one AST-interpreted unit
        // where the loop var is a real interpreter local.
        if self.allow_ast_fallback && self.reg_var_loop_depth == 0 && self.decl_local_regs.is_empty() {
            let reason = self
                .bail_reason
                .unwrap_or_else(|| Self::stmt_kind_label(stmt));
            self.insns.truncate(start);
            self.next_reg = start_reg;
            self.emit(Insn::StmtFallback(Box::new((
                Arc::new(stmt.clone()),
                Arc::from(reason),
            ))));
            self.bail_reason = saved_reason;
            self.register_overflow = saved_overflow;
            return true;
        }
        self.register_overflow = saved_overflow;
        false
    }

    fn compile_stmt_strict(&mut self, stmt: &Statement) -> bool {
        match &stmt.kind {
            // Process-FSM mode: a statement-level timing control becomes a
            // wait insn followed by its guarded statement. Star (`@*`) and
            // intra-assignment forms never reach here (gated by the caller /
            // canonicalized into marker calls that fail compile_expr).
            StatementKind::TimingControl { control, stmt: inner }
                if self.allow_waits =>
            {
                match control {
                    crate::ast::stmt::TimingControl::Delay(d) => {
                        let Some(r) = self.compile_expr(d, 0) else {
                            return false;
                        };
                        self.emit(Insn::WaitDelayReg(r));
                    }
                    crate::ast::stmt::TimingControl::Event(ev) => {
                        if matches!(
                            ev,
                            crate::ast::stmt::EventControl::Star
                                | crate::ast::stmt::EventControl::ParenStar
                        ) {
                            return false;
                        }
                        self.wait_specs.push(ev.clone());
                        self.emit(Insn::WaitEdge((self.wait_specs.len() - 1) as u32));
                    }
                }
                return self.compile_stmt(inner);
            }
            // Process-FSM mode: `forever <body-with-waits>` is the FSM's
            // native shape — body then an unconditional back-jump. The ≥1
            // wait gate at registration guarantees each iteration suspends.
            StatementKind::Forever { body }
                if self.allow_waits && Self::stmt_is_blocking(body) =>
            {
                let top = self.insns.len() as u32;
                if !self.compile_stmt(body) {
                    return false;
                }
                self.emit(Insn::Jump(top));
                return true;
            }
            // Process-FSM mode: `repeat (N) <body-with-waits>` compiles to a
            // counted loop (count evaluated ONCE, §12.7.2) so the classic
            // `repeat (n) @(posedge clk);` cycle-wait works. Wait-free
            // repeats keep the existing unroll paths below.
            StatementKind::Repeat { count, body }
                if self.allow_waits && Self::stmt_is_blocking(body) =>
            {
                let Some(cnt) = self.compile_expr(count, 0) else {
                    return false;
                };
                let ctr = self.alloc_reg();
                self.emit(Insn::Move(ctr, cnt));
                let top = self.insns.len() as u32;
                let branch_idx = self.insns.len();
                // Exits when the counter is no longer definitely non-zero
                // (X/Z counts as zero, matching the interpreter's repeat).
                self.emit(Insn::BranchIfFalse(ctr, 0));
                if !self.compile_stmt(body) {
                    return false;
                }
                let one = self.alloc_reg();
                self.emit(Insn::LoadConst(
                    one,
                    Box::new(Value::from_u64(1, 32)),
                ));
                self.emit(Insn::Sub(ctr, ctr, one));
                self.emit(Insn::Jump(top));
                let end = self.insns.len() as u32;
                self.insns[branch_idx] = Insn::BranchIfFalse(ctr, end);
                return true;
            }

            StatementKind::Null => true,
            // §13.4.1 early return inside an INLINED body: move the value
            // into the result register and jump to the body end (patched by
            // the inliner). Outside an inline there is nothing to return to.
            StatementKind::Return(e) => {
                let Some((slot, w)) = self.inline_ret else {
                    self.bail("Return_outside_inline");
                    return false;
                };
                if let Some(e) = e {
                    let Some(slot) = slot else {
                        self.bail("Return_value_in_void");
                        return false;
                    };
                    let Some(v) = self.compile_expr(e, w) else {
                        return false;
                    };
                    self.emit(Insn::Move(slot, v));
                    if w > 0 {
                        self.emit(Insn::Resize(slot, w));
                    }
                }
                let j = self.insns.len();
                self.emit(Insn::Jump(0));
                self.inline_ret_jumps.push(j);
                true
            }
            StatementKind::VarDecl {
                data_type,
                declarators,
                ..
            } => {
                for decl in declarators {
                    if !decl.dimensions.is_empty() {
                        // One constant-bounded unpacked dimension, small: a
                        // register bank. (`logic [7:0] tmp [0:15]` in an
                        // inlined AES shift_rows / rcon table.) Every access
                        // must fold to a constant index or the enclosing
                        // compile fails and rolls back.
                        use crate::ast::types::UnpackedDimension as UD;
                        let bounds = if decl.dimensions.len() == 1 {
                            match &decl.dimensions[0] {
                                UD::Range { left, right, .. } => {
                                    let l = self.fold_const(left).and_then(|v| v.to_u64());
                                    let r = self.fold_const(right).and_then(|v| v.to_u64());
                                    match (l, r) {
                                        (Some(l), Some(r)) => {
                                            let (lo, hi) =
                                                (l.min(r) as i64, l.max(r) as i64);
                                            Some((lo, (hi - lo + 1) as usize))
                                        }
                                        _ => None,
                                    }
                                }
                                UD::Expression { expr, .. } => self
                                    .fold_const(expr)
                                    .and_then(|v| v.to_u64())
                                    .filter(|&n| n > 0)
                                    .map(|n| (0i64, n as usize)),
                                _ => None,
                            }
                        } else {
                            None
                        };
                        let Some((lo, len)) = bounds.filter(|&(_, n)| n <= 64) else {
                            self.bail("VarDecl_array");
                            return false;
                        };
                        if decl.init.is_some() {
                            self.bail("VarDecl_array_init");
                            return false;
                        }
                        let ew = self.decl_width(data_type);
                        let base = self.next_reg;
                        for _ in 0..len {
                            let slot = self.alloc_reg();
                            let init = self.type_default_value(data_type, ew);
                            self.emit(Insn::LoadConst(slot, Box::new(init)));
                        }
                        let Ok(base) = RegId::try_from(base) else {
                            self.bail("VarDecl_array");
                            return false;
                        };
                        self.local_array_regs
                            .insert(decl.name.name.clone(), (base, ew, len, lo));
                        continue;
                    }
                    // §6.16 string local: no declared width — bind at width
                    // 0 so no Resize ever lands on it, and mark it for the
                    // %s / string-assign paths.
                    let is_string = matches!(
                        data_type,
                        crate::ast::types::DataType::Simple {
                            kind: crate::ast::types::SimpleType::String,
                            ..
                        }
                    );
                    let width = if is_string { 0 } else { self.decl_width(data_type) };
                    let slot = self.alloc_reg();
                    match &decl.init {
                        Some(expr) => {
                            let Some(value) = self.compile_expr(expr, width) else {
                                self.bail("VarDecl_init");
                                return false;
                            };
                            self.emit(Insn::Move(slot, value));
                        }
                        None => {
                            let value = self.type_default_value(data_type, width);
                            self.emit(Insn::LoadConst(slot, Box::new(value)));
                        }
                    }
                    if width > 0 {
                        self.emit(Insn::Resize(slot, width));
                    }
                    if is_string {
                        self.local_var_is_string.insert(decl.name.name.clone());
                    }
                    self.local_var_regs
                        .insert(decl.name.name.clone(), (slot, width));
                    self.decl_local_regs.insert(decl.name.name.clone());
                }
                true
            }
            StatementKind::NonblockingAssign { lvalue, rvalue, .. } => {
                let width = self.infer_lhs_width(lvalue);
                let start = self.insns.len();
                let start_reg = self.next_reg;
                self.pattern_layout = self.lvalue_struct_layout(lvalue);
                let compiled = self.compile_expr(rvalue, width);
                self.pattern_layout = None;
                if let Some(val_reg) = compiled {
                    // Note: NbaAssign itself performs §10.7 assignment-padding resize,
                    // so we don't emit a generic (zero-extending) Resize here — that
                    // would strip X/Z from the MSB before the assignment could X/Z-extend.
                    if self.compile_nba_target(lvalue, val_reg, width) {
                        return true;
                    }
                    self.bail("nba_target");
                } else {
                    self.bail("nba_rvalue");
                }
                // Roll back partial work and emit fallback if allowed.
                self.insns.truncate(start);
                self.next_reg = start_reg;
                self.emit_fallback(stmt)
            }
            StatementKind::BlockingAssign { lvalue, rvalue } => {
                let width = self.infer_lhs_width(lvalue);
                let start = self.insns.len();
                let start_reg = self.next_reg;
                self.pattern_layout = self.lvalue_struct_layout(lvalue);
                let compiled = self.compile_expr(rvalue, width);
                self.pattern_layout = None;
                if let Some(val_reg) = compiled {
                    if width > 0 {
                        self.emit(Insn::Resize(val_reg, width));
                    }
                    if self.compile_blocking_target(lvalue, val_reg, width) {
                        return true;
                    }
                    self.bail("blocking_target");
                } else {
                    self.bail("blocking_rvalue");
                }
                self.insns.truncate(start);
                self.next_reg = start_reg;
                self.emit_fallback(stmt)
            }
            StatementKind::If {
                condition,
                then_stmt,
                else_stmt,
                ..
            } => {
                // §12.6 `if (e matches p)` binds the pattern's `.name`s for the
                // then-branch. That needs the AST interpreter — compiling it to
                // a conditional jump would evaluate the match but drop the
                // bindings, so the branch ran with `n` unset.
                if matches!(condition.kind, ExprKind::Matches { .. }) {
                    return false;
                }
                if let Some(cond_reg) = self.compile_expr(condition, 0) {
                    let branch_idx = self.insns.len();
                    self.emit(Insn::BranchIfFalse(cond_reg, 0)); // placeholder target
                    if !self.compile_stmt(then_stmt) {
                        return false;
                    }
                    if let Some(el) = else_stmt {
                        let jump_idx = self.insns.len();
                        self.emit(Insn::Jump(0)); // placeholder
                        let else_start = self.insns.len() as u32;
                        self.insns[branch_idx] = Insn::BranchIfFalse(cond_reg, else_start);
                        if !self.compile_stmt(el) {
                            return false;
                        }
                        let end = self.insns.len() as u32;
                        self.insns[jump_idx] = Insn::Jump(end);
                    } else {
                        let end = self.insns.len() as u32;
                        self.insns[branch_idx] = Insn::BranchIfFalse(cond_reg, end);
                    }
                    true
                } else {
                    false
                }
            }
            StatementKind::Case {
                kind, expr, items, ..
            } => {
                // ROM shape: plain `case` whose every arm assigns a CONSTANT
                // to the SAME variable, constant patterns throughout. One
                // table lookup replaces the whole compare chain. `===`
                // semantics hold exactly: the patterns are fully defined, so
                // a selector with any x/z bit matches none of them → default.
                if matches!(kind, crate::ast::stmt::CaseKind::Case) {
                    if let Some((lhs, table, default, res_w)) = self.case_lut_shape(items) {
                        if let Some(sel) = self.compile_expr(expr, 0) {
                            let dst = self.alloc_reg();
                            self.emit(Insn::CaseLut(
                                dst,
                                sel,
                                Box::new(CaseLutData { table, default }),
                            ));
                            if self.compile_blocking_target(&lhs, dst, res_w) {
                                return true;
                            }
                        }
                        // fall through to the generic chain on any failure
                    }
                    if self.compile_case_jump(expr, items) {
                        return true;
                    }
                }
                if matches!(
                    kind,
                    crate::ast::stmt::CaseKind::Casez | crate::ast::stmt::CaseKind::Casex
                ) && self.compile_case_mask_jump(kind, expr, items)
                {
                    return true;
                }
                if let Some(val_reg) = self.compile_expr(expr, 0) {
                    let mut end_jumps: Vec<usize> = Vec::new();
                    let mut default_item: Option<&Statement> = None;
                    // Every pattern arm is mutually exclusive and jumps to the
                    // case end after its body. Reuse its temporary registers so
                    // a large constant map does not exhaust the register id
                    // space merely by having many alternatives.
                    let arm_reg_start = self.next_reg;
                    let mut peak_reg = arm_reg_start;
                    for item in items {
                        if item.is_default {
                            default_item = Some(&item.stmt);
                            continue;
                        }
                        // Compile pattern match: val === pattern (or casez/casex
                        // wildcard match per CaseKind).
                        for pat in &item.patterns {
                            if let Some(pat_reg) = self.compile_expr(pat, 0) {
                                let cmp_reg = self.alloc_reg();
                                self.emit(match kind {
                                    crate::ast::stmt::CaseKind::Casez => {
                                        Insn::CasezEq(cmp_reg, val_reg, pat_reg)
                                    }
                                    crate::ast::stmt::CaseKind::Casex => {
                                        Insn::CasexEq(cmp_reg, val_reg, pat_reg)
                                    }
                                    _ => Insn::CaseEq(cmp_reg, val_reg, pat_reg),
                                });
                                let branch_idx = self.insns.len();
                                self.emit(Insn::BranchIfFalse(cmp_reg, 0));
                                if !self.compile_stmt(&item.stmt) {
                                    return false;
                                }
                                end_jumps.push(self.insns.len());
                                self.emit(Insn::Jump(0));
                                let next = self.insns.len() as u32;
                                self.insns[branch_idx] = Insn::BranchIfFalse(cmp_reg, next);
                                peak_reg = peak_reg.max(self.next_reg);
                                self.next_reg = arm_reg_start;
                            } else {
                                return false;
                            }
                        }
                    }
                    // Default case
                    if let Some(def_stmt) = default_item {
                        if !self.compile_stmt(def_stmt) {
                            return false;
                        }
                        peak_reg = peak_reg.max(self.next_reg);
                    }
                    self.next_reg = peak_reg;
                    let end = self.insns.len() as u32;
                    for idx in end_jumps {
                        self.insns[idx] = Insn::Jump(end);
                    }
                    true
                } else {
                    false
                }
            }
            // §9.3.2: a fork's children are CONCURRENT PROCESSES. Compiling
            // a ParBlock like a SeqBlock ran them sequentially INLINE — for
            // `fork ... join_none` with a delaying child that meant the edge
            // block itself executed the child's infinite `#1` loop on its own
            // stack: the design froze (no other always block could ever fire
            // again, so even $finish became unreachable) and the run had to
            // be SIGKILLed. Always leave fork/join to the AST interpreter's
            // ParBlock arm, which spawns real child processes.
            StatementKind::ParBlock { .. } => {
                self.bail("Stmt_ParBlock");
                self.emit_fallback(stmt)
            }
            StatementKind::SeqBlock { stmts, .. } => {
                let saved_locals = self.local_var_regs.clone();
                let saved_decl_locals = self.decl_local_regs.clone();
                for s in stmts {
                    if !self.compile_stmt(s) {
                        self.local_var_regs = saved_locals;
                        self.decl_local_regs = saved_decl_locals;
                        return false;
                    }
                }
                self.local_var_regs = saved_locals;
                self.decl_local_regs = saved_decl_locals;
                true
            }
            // Bail out on anything else (timing controls, loops, system tasks, etc.)
            StatementKind::Expr(e) => {
                // §6.24.1: `void'(expr)` lowers to `Paren(expr)` (the cast is
                // a pure discard). The old `Paren(_) => no-op` arm below then
                // swallowed the whole statement, so `void'(q.pop_front())`
                // inside a compiled always block never popped — while the
                // bare `q.pop_front();` form fell through to the AST fallback
                // and worked, which is exactly how the difference hid. Peel
                // the wrappers so the inner expression's own arm decides.
                let mut e = e;
                while let ExprKind::Paren(inner) = &e.kind {
                    e = inner;
                }
                match &e.kind {
                    // Bare identifier as statement: side-effect-free read, compile as no-op
                    // — BUT only if it actually resolves to a signal. A bare ident that
                    // doesn't resolve is typically a task-enable (`task_name;`) whose
                    // dispatch must happen in the AST interpreter's `exec_expr_stmt`.
                    ExprKind::Ident(hier) if hier.path.len() == 1 => {
                        if self.lookup_signal_id(hier).is_some() {
                            return true;
                        }
                        let name = hier.path[0].name.name.clone();
                        if self.try_inline_task(&name) {
                            return true;
                        }
                        self.bail("Expr_TaskEnable");
                        return self.emit_fallback(stmt);
                    }
                    ExprKind::Ident(hier) if hier.path.len() > 1 => {
                        let mname = hier.path.last().unwrap().name.name.as_str();
                        if matches!(
                            mname,
                            "delete"
                                | "sort"
                                | "rsort"
                                | "reverse"
                                | "unique"
                                | "unique_index"
                                | "pop_front"
                                | "pop_back"
                        ) {
                            return self
                                .emit_fallback(&Statement::new(stmt.kind.clone(), stmt.span));
                        }
                        if self.lookup_signal_id(hier).is_some() {
                            return true;
                        }
                        let leaf = hier.path.last().unwrap().name.name.clone();
                        if self.try_inline_task(&leaf) {
                            return true;
                        }
                        self.bail("Expr_TaskEnable");
                        return self.emit_fallback(stmt);
                    }
                    // A literal as a statement is genuinely side-effect-free.
                    // (`Paren` can no longer appear here — peeled above.)
                    ExprKind::Number(_) => {
                        return true;
                    }
                    // Pre/post increment/decrement have side effects — compile them
                    ExprKind::Unary {
                        op: UnaryOp::PreIncr,
                        operand,
                    }
                    | ExprKind::Unary {
                        op: UnaryOp::PostIncr,
                        operand,
                    } => {
                        if let Some(sig_id) = self.expr_to_signal_id(operand) {
                            let r = self.alloc_reg();
                            self.emit(Insn::LoadSignal(r, as_sig_id(sig_id)));
                            let one = self.alloc_reg();
                            let w = self.signal_widths[sig_id];
                            self.emit(Insn::LoadConst(one, Box::new(Value::from_u64(1, w))));
                            let result = self.alloc_reg();
                            self.emit(Insn::Add(result, r, one));
                            self.emit(Insn::Resize(result, w));
                            self.emit(Insn::BlockingAssign(as_sig_id(sig_id), result, w));
                            return true;
                        }
                        self.bail("Expr_PreIncr");
                        return self.emit_fallback(stmt);
                    }
                    ExprKind::Unary {
                        op: UnaryOp::PreDecr,
                        operand,
                    }
                    | ExprKind::Unary {
                        op: UnaryOp::PostDecr,
                        operand,
                    } => {
                        if let Some(sig_id) = self.expr_to_signal_id(operand) {
                            let r = self.alloc_reg();
                            self.emit(Insn::LoadSignal(r, as_sig_id(sig_id)));
                            let one = self.alloc_reg();
                            let w = self.signal_widths[sig_id];
                            self.emit(Insn::LoadConst(one, Box::new(Value::from_u64(1, w))));
                            let result = self.alloc_reg();
                            self.emit(Insn::Sub(result, r, one));
                            self.emit(Insn::Resize(result, w));
                            self.emit(Insn::BlockingAssign(as_sig_id(sig_id), result, w));
                            return true;
                        }
                        self.bail("Expr_PreDecr");
                        return self.emit_fallback(stmt);
                    }
                    // Call-as-statement: a task enable with arguments, or a
                    // void function. Tasks first (they are not in the function
                    // table); a void function call compiles through the
                    // ordinary expression path below, discarding its (empty)
                    // result register.
                    ExprKind::Call { func, args } => {
                        // §6.16.4/§6.16.10 in-place string mutators as
                        // statements: compute the modified text natively,
                        // store it back to the receiver.
                        if let Some((recv, method)) =
                            self.string_method_shape(func, e.span)
                        {
                            let kind = match (method, args.len()) {
                                ("putc", 2) => Some(StrOpKind::PutC),
                                ("itoa", 1) => Some(StrOpKind::IToA),
                                ("hextoa", 1) => Some(StrOpKind::HexToA),
                                ("octtoa", 1) => Some(StrOpKind::OctToA),
                                ("bintoa", 1) => Some(StrOpKind::BinToA),
                                _ => None,
                            };
                            if let Some(kind) = kind {
                                let start = self.insns.len();
                                let start_reg = self.next_reg;
                                let mut regs: Vec<RegId> = Vec::new();
                                let mut ok = true;
                                // PutC reads the current text; the *toa
                                // family overwrites it wholesale.
                                if kind == StrOpKind::PutC {
                                    match self.compile_expr(&recv, 0) {
                                        Some(r) => regs.push(r),
                                        None => ok = false,
                                    }
                                }
                                if ok {
                                    for a in args {
                                        match self.compile_expr(a, 0) {
                                            Some(r) => regs.push(r),
                                            None => {
                                                ok = false;
                                                break;
                                            }
                                        }
                                    }
                                }
                                if ok {
                                    let dst = self.alloc_reg();
                                    self.emit(Insn::StrOp(dst, kind, Box::new(regs)));
                                    if self.compile_blocking_target(&recv, dst, 0) {
                                        return true;
                                    }
                                }
                                self.insns.truncate(start);
                                self.next_reg = start_reg;
                                // fall through to the task/AST paths
                            }
                        }
                        if let ExprKind::Ident(h) = &func.kind {
                            let raw = Self::hier_raw_name(h);
                            if self.try_inline_task_args(&raw, args) {
                                return true;
                            }
                            if let Some(leaf) = raw.rsplit('.').next() {
                                if leaf != raw && self.try_inline_task_args(leaf, args) {
                                    return true;
                                }
                            }
                        }
                        if self.compile_expr(e, 0).is_some() {
                            return true;
                        }
                        self.bail("Expr_Call");
                        return self.emit_fallback(stmt);
                    }
                    _ => {}
                }
                let n: &'static str = match &e.kind {
                    ExprKind::SystemCall { name, .. } => match name.as_str() {
                        "$display" => "Expr_display",
                        "$write" => "Expr_write",
                        "$strobe" => "Expr_strobe",
                        "$monitor" => "Expr_monitor",
                        "$finish" => "Expr_finish",
                        "$stop" => "Expr_stop",
                        _ => "Expr_syscall_other",
                    },
                    ExprKind::Call { .. } => "Expr_Call",
                    ExprKind::Binary { .. } => "Expr_Binary",
                    ExprKind::Concatenation(_) => "Expr_Concat",
                    ExprKind::Replication { .. } => "Expr_Replication",
                    ExprKind::MemberAccess { .. } => "Expr_MemberAccess",
                    ExprKind::AssignmentPattern(_) => "Expr_AsgnPat",
                    ExprKind::Index { .. } => "Expr_Index",
                    ExprKind::RangeSelect { .. } => "Expr_RangeSelect",
                    ExprKind::Conditional { .. } => "Expr_Conditional",
                    _ => "Expr_other",
                };
                self.bail(n);
                self.emit_fallback(stmt)
            }
            StatementKind::For {
                init,
                condition,
                step,
                body,
            } => {
                let (vector_plans, vectorized_body) = match self.full_range_nba_copy_plan(
                    init,
                    condition.as_ref(),
                    step,
                    body,
                ) {
                    Some((plans, pruned)) => (plans, Some(pruned)),
                    None => (Vec::new(), None),
                };
                let body_to_compile = vectorized_body.as_ref().unwrap_or(body);
                // LRM §12.7 — `break`/`continue` are now compiled to direct
                // jumps; we push fresh patch lists on entry and apply them
                // once we know the step-start and loop-end addresses.
                self.loop_break_patches.push(Vec::new());
                self.loop_continue_patches.push(Vec::new());
                // Save outer for-loop overrides so nested loops don't leak.
                let saved_for_vars = std::mem::take(&mut self.for_loop_var_ids);
                let saved_locals = self.local_var_regs.clone();
                let mut reg_vars_registered: u32 = 0;
                // Inherit the outer overrides too — a nested loop's body
                // can still reference the outer counter.
                self.for_loop_var_ids = saved_for_vars.clone();
                for fi in init {
                    match fi {
                        ForInit::Assign { lvalue, rvalue } => {
                            let width = self.infer_lhs_width(lvalue);
                            let val_reg = match self.compile_expr(rvalue, width) {
                                Some(r) => r,
                                None => {
                                    self.bail("For_init_rvalue");
                                    return false;
                                }
                            };
                            if width > 0 {
                                self.emit(Insn::Resize(val_reg, width));
                            }
                            if !self.compile_blocking_target(lvalue, val_reg, width) {
                                self.bail("For_init_target");
                                return false;
                            }
                            // Capture init's lvalue signal_id keyed by leaf
                            // name. The for-loop's step / body expressions
                            // often re-parse bare-ident references that the
                            // elaborator did not scope-qualify (only init's
                            // lvalue gets qualified through an elaboration
                            // path). Without this, a bare `i` in step
                            // `i = i+1` collides with an unrelated top-level
                            // signal of the same name and resolves to the
                            // wrong signal_id. On c910 the always-block
                            // counter was clobbering the testbench's
                            // top-level `integer i` (signal_id 9), and the
                            // actual counter never advanced — the loop ran
                            // forever (10M+ insns per call, hung the sim
                            // in iter 1 of the event loop).
                            // Capture init's resolved signal_id keyed by the
                            // *leaf* of the lvalue's hier path. The
                            // elaborator may have rewritten init's lvalue
                            // from bare `i` to a multi-segment `module.i`
                            // form (which is why init resolves correctly
                            // to the module-local id), while leaving the
                            // for-step's bare `i` untouched. Capturing by
                            // leaf bridges that asymmetry: bare `i` in step
                            // gets re-routed to init's resolved id.
                            if let ExprKind::Ident(hier) = &lvalue.kind {
                                let leaf = if hier.path.len() == 1
                                    && hier.path[0].name.name.contains('.')
                                {
                                    // Parser flattened a hier path into one segment with dots.
                                    hier.path[0]
                                        .name
                                        .name
                                        .rsplit('.')
                                        .next()
                                        .unwrap_or("")
                                        .to_string()
                                } else {
                                    hier.path
                                        .last()
                                        .map(|s| s.name.name.clone())
                                        .unwrap_or_default()
                                };
                                if !leaf.is_empty() && !leaf.contains('.') {
                                    if let Some(id) = self.lookup_signal_id(hier) {
                                        self.for_loop_var_ids.insert(leaf, id);
                                    }
                                }
                            }
                        }
                        // NOTE the ORDER: the register-backed path (below)
                        // keeps every loop the specialized fast paths
                        // (vectorized packed-copy, fused array NBA) know how
                        // to handle; the unroller is the CATCH-ALL for bodies
                        // they cannot compile at all — task/function calls,
                        // register-bank locals, const-index folding.
                        ForInit::VarDecl { data_type, name, init }
                            if self.for_body_is_simple(body)
                                && !self.stmt_touches_reg_bank(body) =>
                        {
                            // §12.7.1: `for (int i = ...)` — the loop var
                            // lives in a VM REGISTER (it has no signal).
                            // Body/step reads resolve through local_var_regs,
                            // which compile_expr consults BEFORE the signal
                            // tables, so a same-named outer signal can never
                            // capture. Array indexing through the register is
                            // safe: the register path (Nba/BlockingAssignArray)
                            // takes a RegId, and the SigId-fusion peepholes
                            // pattern-match LoadSignal, which a register-backed
                            // index never emits. This bail was 83% of a
                            // customer run's wall time (For_init_vardecl,
                            // 204µs per AST execution of a lane-copy loop).
                            let w = self.decl_width(data_type);
                            let slot = self.alloc_reg();
                            let Some(v) = self.compile_expr(init, w) else {
                                self.for_loop_var_ids = saved_for_vars;
                                self.local_var_regs = saved_locals;
                                self.bail("For_init_vardecl_rvalue");
                                return false;
                            };
                            self.emit(Insn::Move(slot, v));
                            if w > 0 {
                                self.emit(Insn::Resize(slot, w));
                            }
                            // §6.11: int/byte/shortint/longint/integer are
                            // SIGNED by default — the init literal may not be
                            // (`for (int i = 4'hF; ...)`), and an unsigned
                            // slot makes `i >= 0` never terminate / negative
                            // comparisons go unsigned.
                            use crate::ast::types::{
                                DataType as FDt, IntegerAtomType as FIat, Signing as FSg,
                            };
                            let decl_signed = match data_type {
                                FDt::IntegerAtom { kind, signing, .. } => {
                                    !matches!(signing, Some(FSg::Unsigned))
                                        && !matches!(kind, FIat::Time)
                                }
                                FDt::IntegerVector { signing, .. } => {
                                    matches!(signing, Some(FSg::Signed))
                                }
                                _ => false,
                            };
                            if decl_signed {
                                self.emit(Insn::SetSigned(slot));
                            } else {
                                self.emit(Insn::ClearSigned(slot));
                            }
                            self.local_var_regs.insert(name.name.clone(), (slot, w));
                            self.reg_var_loop_depth += 1;
                            reg_vars_registered += 1;
                        }
                        // Catch-all: bodies the specialized paths above
                        // decline (task/function calls, register-bank locals,
                        // const-index folding) unroll with a const-bound loop
                        // variable.
                        #[allow(unreachable_patterns)]
                        ForInit::VarDecl { name, init, .. }
                            if self.try_unroll_for(name, init, condition, step, body) =>
                        {
                            self.for_loop_var_ids = saved_for_vars;
                            self.local_var_regs = saved_locals;
                            return true;
                        }
                        #[allow(unreachable_patterns)]
                        ForInit::VarDecl { .. } => {
                            self.for_loop_var_ids = saved_for_vars;
                            self.local_var_regs = saved_locals;
                            self.bail("For_init_vardecl");
                            return false;
                        }
                    }
                }
                // The plan proves this canonical loop executes its complete
                // packed range. Queue each identity copy once at the point
                // where its first iteration would sample the source.
                for &(dst, src, width) in &vector_plans {
                    let r = self.alloc_reg();
                    self.emit(Insn::LoadSignal(r, as_sig_id(src)));
                    self.emit(Insn::NbaAssign(as_sig_id(dst), r, width));
                }
                let loop_start = self.insns.len() as u32;
                let cond_branch_idx = if let Some(c) = condition {
                    let cond_reg = match self.compile_expr(c, 0) {
                        Some(r) => r,
                        None => {
                            self.bail("For_condition");
                            self.for_loop_var_ids = saved_for_vars;
                            self.local_var_regs = saved_locals;
                            self.reg_var_loop_depth -=
                                reg_vars_registered.min(self.reg_var_loop_depth);
                            return false;
                        }
                    };
                    let idx = self.insns.len();
                    self.emit(Insn::BranchIfFalse(cond_reg, 0));
                    Some(idx)
                } else {
                    None
                };
                if !self.compile_stmt(body_to_compile) {
                    // Bail path — pop patches so they don't leak.
                    self.loop_break_patches.pop();
                    self.loop_continue_patches.pop();
                    self.for_loop_var_ids = saved_for_vars;
                    self.local_var_regs = saved_locals;
                    self.reg_var_loop_depth -=
                        reg_vars_registered.min(self.reg_var_loop_depth);
                    return false;
                }
                let step_start = self.insns.len() as u32;
                // `continue` jumps to the step (or to loop_start if there is
                // no step) — patch now.
                let cont_targ = if step.is_empty() {
                    loop_start
                } else {
                    step_start
                };
                if let Some(patches) = self.loop_continue_patches.pop() {
                    for idx in patches {
                        self.insns[idx] = Insn::Jump(cont_targ);
                    }
                }
                for s in step {
                    // For-loop step can be either the legacy `Binary{Assign,…}`
                    // shape or the newer `AssignExpr { lvalue, rvalue }` emitted
                    // by the parser for `i = i+1` / `i += 2` / etc. after
                    // xezim-core 8b9c88c (ibex parsing). Both collapse to a
                    // blocking assign.
                    // `i++` / `++i` / `i--` / `--i` on a REGISTER-backed block
                    // local: increment in place. (The signal-backed case is
                    // handled by the generic assign shapes below.)
                    if let ExprKind::Unary { op, operand } = &s.kind {
                        let delta: i64 = match op {
                            UnaryOp::PostIncr | UnaryOp::PreIncr => 1,
                            UnaryOp::PostDecr | UnaryOp::PreDecr => -1,
                            _ => 0,
                        };
                        if delta != 0 {
                            if let ExprKind::Ident(h) = &operand.kind {
                                if let Some((slot, dw)) = self.local_var_reg_of(h) {
                                    let one = self.alloc_reg();
                                    let w = if dw > 0 { dw } else { 32 };
                                    // SIGNED one: signed+signed stays signed
                                    // (an unsigned 1 silently stripped the
                                    // loop var's sign on the first step, so
                                    // `i >= -2` compared unsigned);
                                    // signed+unsigned still yields unsigned,
                                    // so unsigned loop vars are unaffected.
                                    let mut one_v = Value::from_u64(1, w);
                                    one_v.is_signed = true;
                                    self.emit(Insn::LoadConst(one, Box::new(one_v)));
                                    let dst = self.alloc_reg();
                                    self.emit(Insn::Move(dst, slot));
                                    if delta > 0 {
                                        self.emit(Insn::Add(dst, dst, one));
                                    } else {
                                        self.emit(Insn::Sub(dst, dst, one));
                                    }
                                    if w > 0 {
                                        self.emit(Insn::Resize(dst, w));
                                    }
                                    self.emit(Insn::Move(slot, dst));
                                    continue;
                                }
                                // SIGNAL-backed loop counter (`int i;` at
                                // module/block scope): load, ±1, store.
                                // Previously bailed the whole loop to the AST
                                // path ("For_step_other") — ~30µs per edge.
                                if let Some(id) = self
                                    .lookup_signal_id(h)
                                    .filter(|_| self.for_body_is_simple(body))
                                {
                                    let w = self
                                        .signal_widths
                                        .get(id)
                                        .copied()
                                        .unwrap_or(32)
                                        .max(1);
                                    let cur = self.alloc_reg();
                                    self.emit(Insn::LoadSignal(cur, id as u32));
                                    let one = self.alloc_reg();
                                    let mut one_v = Value::from_u64(1, w);
                                    one_v.is_signed = true; // see register arm
                                    self.emit(Insn::LoadConst(one, Box::new(one_v)));
                                    if delta > 0 {
                                        self.emit(Insn::Add(cur, cur, one));
                                    } else {
                                        self.emit(Insn::Sub(cur, cur, one));
                                    }
                                    self.emit(Insn::Resize(cur, w));
                                    self.emit(Insn::BlockingAssign(id as u32, cur, w));
                                    continue;
                                }
                            }
                        }
                    }
                    let (lhs, rhs) = match &s.kind {
                        ExprKind::Binary {
                            op: BinaryOp::Assign,
                            left,
                            right,
                        } => (&**left, &**right),
                        ExprKind::AssignExpr { lvalue, rvalue } => (&**lvalue, &**rvalue),
                        _ => {
                            self.bail("For_step_other");
                            return false;
                        }
                    };
                    let width = self.infer_lhs_width(lhs);
                    let val_reg = match self.compile_expr(rhs, width) {
                        Some(r) => r,
                        None => {
                            self.bail("For_step_rvalue");
                            return false;
                        }
                    };
                    if width > 0 {
                        self.emit(Insn::Resize(val_reg, width));
                    }
                    if !self.compile_blocking_target(lhs, val_reg, width) {
                        self.bail("For_step_target");
                        return false;
                    }
                }
                self.emit(Insn::Jump(loop_start));
                let end = self.insns.len() as u32;
                if let Some(idx) = cond_branch_idx {
                    if let Insn::BranchIfFalse(reg, _) = self.insns[idx] {
                        self.insns[idx] = Insn::BranchIfFalse(reg, end);
                    }
                }
                // `break` jumps to the loop-exit address.
                if let Some(patches) = self.loop_break_patches.pop() {
                    for idx in patches {
                        self.insns[idx] = Insn::Jump(end);
                    }
                }
                // Restore outer for-loop's override map and block locals.
                self.for_loop_var_ids = saved_for_vars;
                self.local_var_regs = saved_locals;
                self.reg_var_loop_depth -= reg_vars_registered.min(self.reg_var_loop_depth);
                if !vector_plans.is_empty() {
                    PACKED_LOOP_NBA_COPIES.fetch_add(
                        vector_plans.len() as u64,
                        std::sync::atomic::Ordering::Relaxed,
                    );
                }
                true
            }
            StatementKind::Break => {
                // LRM §12.7 — exits innermost enclosing loop. Compiled as a
                // forward Jump(0) patched after the loop body+step finish.
                // Outside a loop in the compiled scope: bail so the AST path
                // can produce the right diagnostic.
                if self.loop_break_patches.last().is_some() {
                    let idx = self.insns.len();
                    self.emit(Insn::Jump(0));
                    self.loop_break_patches.last_mut().unwrap().push(idx);
                    true
                } else {
                    self.bail("Break_outside_loop");
                    self.emit_fallback(stmt)
                }
            }
            StatementKind::Continue => {
                // LRM §12.7 — restart innermost enclosing loop at its step.
                if self.loop_continue_patches.last().is_some() {
                    let idx = self.insns.len();
                    self.emit(Insn::Jump(0));
                    self.loop_continue_patches.last_mut().unwrap().push(idx);
                    true
                } else {
                    self.bail("Continue_outside_loop");
                    self.emit_fallback(stmt)
                }
            }
            StatementKind::While { condition, body } => {
                // §12.7.2: a while is a For with no init and no step. The
                // condition is re-evaluated at the loop head each iteration;
                // `continue` jumps back to the head, `break` to the end.
                // Compiling it (rather than bailing "Stmt_While") is what
                // lets a pure while-loop helper inline (issue #146) — the
                // purity arm alone would only have moved the bail here.
                self.loop_break_patches.push(Vec::new());
                self.loop_continue_patches.push(Vec::new());
                let top = self.insns.len() as u32;
                let Some(c) = self.compile_expr(condition, 0) else {
                    self.loop_break_patches.pop();
                    self.loop_continue_patches.pop();
                    self.bail("While_cond");
                    return false;
                };
                let br = self.insns.len();
                self.emit(Insn::BranchIfFalse(c, 0));
                if !self.compile_stmt(body) {
                    self.loop_break_patches.pop();
                    self.loop_continue_patches.pop();
                    return false;
                }
                self.emit(Insn::Jump(top));
                let end = self.insns.len() as u32;
                if let Insn::BranchIfFalse(reg, _) = self.insns[br] {
                    self.insns[br] = Insn::BranchIfFalse(reg, end);
                }
                if let Some(patches) = self.loop_continue_patches.pop() {
                    for idx in patches {
                        self.insns[idx] = Insn::Jump(top);
                    }
                }
                if let Some(patches) = self.loop_break_patches.pop() {
                    for idx in patches {
                        self.insns[idx] = Insn::Jump(end);
                    }
                }
                true
            }
            StatementKind::DoWhile { body, condition } => {
                // Body first, then the condition; `continue` re-tests the
                // condition (§12.7.4), `break` exits.
                self.loop_break_patches.push(Vec::new());
                self.loop_continue_patches.push(Vec::new());
                let top = self.insns.len() as u32;
                if !self.compile_stmt(body) {
                    self.loop_break_patches.pop();
                    self.loop_continue_patches.pop();
                    return false;
                }
                let cond_at = self.insns.len() as u32;
                let Some(c) = self.compile_expr(condition, 0) else {
                    self.loop_break_patches.pop();
                    self.loop_continue_patches.pop();
                    self.bail("DoWhile_cond");
                    return false;
                };
                self.emit(Insn::BranchUnlessZero(c, top));
                let end = self.insns.len() as u32;
                if let Some(patches) = self.loop_continue_patches.pop() {
                    for idx in patches {
                        self.insns[idx] = Insn::Jump(cond_at);
                    }
                }
                if let Some(patches) = self.loop_break_patches.pop() {
                    for idx in patches {
                        self.insns[idx] = Insn::Jump(end);
                    }
                }
                true
            }
            StatementKind::Foreach { array, vars, body } => {
                // §12.7.3 foreach over a register-bound local array — an
                // inlined resolver's dynamic-array formal or a #129 local
                // buffer. The element count is a compile-time constant, so
                // the loop UNROLLS with the loop variable bound both as a
                // register (value uses) and as a constant (index uses fold
                // to a direct element register via const_var_binds).
                let arr_name = match &array.kind {
                    ExprKind::Ident(h)
                        if h.path.len() == 1 && h.path[0].selects.is_empty() =>
                    {
                        h.path[0].name.name.clone()
                    }
                    _ => {
                        self.bail("Stmt_Foreach");
                        return false;
                    }
                };
                let Some(ab) = self.local_var_array.get(&arr_name).cloned() else {
                    self.bail("Stmt_Foreach");
                    return false;
                };
                let var = match vars.as_slice() {
                    [Some(v)] => v.name.clone(),
                    _ => {
                        self.bail("Stmt_Foreach_vars");
                        return false;
                    }
                };
                if ab.lo < 0 || Self::stmt_has_break_or_continue(body) {
                    // §12.7 break/continue jump-patch lists belong to REAL
                    // compiled loops; an unrolled iteration has no loop to
                    // exit. Rolls back to one AST-interpreted unit instead.
                    self.bail("Stmt_Foreach_break");
                    return false;
                }
                let var_reg = self.alloc_reg();
                let saved_local = self.local_var_regs.insert(var.clone(), (var_reg, 32));
                let saved_const = self.const_var_binds.get(&var).copied();
                let saved_fallback = self.allow_ast_fallback;
                // A per-statement AST fallback inside the unrolled body
                // would read the loop var as a (non-existent) SIGNAL — same
                // hazard as the register-var for-loop guard. Fail the body
                // instead, so the WHOLE foreach rolls back as one unit.
                self.allow_ast_fallback = false;
                let mut ok = true;
                for k in 0..ab.regs.len() {
                    let idx = (ab.lo + k as i64) as u64;
                    self.emit(Insn::LoadConst(
                        var_reg,
                        Box::new(Value::from_u64(idx, 32)),
                    ));
                    self.const_var_binds.insert(var.clone(), idx);
                    if !self.compile_stmt(body) {
                        ok = false;
                        break;
                    }
                }
                self.allow_ast_fallback = saved_fallback;
                match saved_const {
                    Some(v) => {
                        self.const_var_binds.insert(var.clone(), v);
                    }
                    None => {
                        self.const_var_binds.remove(&var);
                    }
                }
                match saved_local {
                    Some(b) => {
                        self.local_var_regs.insert(var, b);
                    }
                    None => {
                        self.local_var_regs.remove(&var);
                    }
                }
                if !ok {
                    self.bail("Stmt_Foreach_body");
                    return false;
                }
                true
            }
            other => {
                let name: &'static str = match other {
                    StatementKind::Expr(_) => "Expr",
                    StatementKind::For { .. } => "For",
                    StatementKind::Foreach { .. } => "Foreach",
                    StatementKind::While { .. } => "While",
                    StatementKind::DoWhile { .. } => "DoWhile",
                    StatementKind::Repeat { .. } => "Repeat",
                    StatementKind::Forever { .. } => "Forever",
                    StatementKind::TimingControl { .. } => "TimingControl",
                    StatementKind::EventTrigger { .. } => "EventTrigger",
                    StatementKind::Wait { .. } => "Wait",
                    StatementKind::WaitFork => "WaitFork",
                    StatementKind::Disable(_) => "Disable",
                    StatementKind::Return(_) => "Return",
                    StatementKind::Break => "Break",
                    StatementKind::Continue => "Continue",
                    StatementKind::Assertion(_) => "Assertion",
                    StatementKind::ProceduralContinuous(_) => "ProceduralContinuous",
                    StatementKind::VarDecl { .. } => "VarDecl",
                    StatementKind::Coverpoint { .. } => "Coverpoint",
                    StatementKind::Cross { .. } => "Cross",
                    _ => "Other",
                };
                self.bail_reason = Some(name);
                self.emit_fallback(stmt)
            }
        }
    }

    /// Compile an expression, returning the register holding the result.
    /// Returns None if the expression can't be compiled to bytecode.
    fn compile_expr(&mut self, expr: &Expression, ctx_width: u32) -> Option<RegId> {
        if let Some(id) = self.const_multi_dim_array_elem_signal_id(expr) {
            let dest = self.alloc_reg();
            if self.signal_signed[id] {
                self.emit(Insn::LoadSignalSigned(dest, as_sig_id(id)));
            } else {
                self.emit(Insn::LoadSignal(dest, as_sig_id(id)));
            }
            return Some(dest);
        }
        match &expr.kind {
            ExprKind::Number(num) => {
                let val = self.eval_number_static(num)?;
                let r = self.alloc_reg();
                self.emit(Insn::LoadConst(r, Box::new(val)));
                Some(r)
            }
            ExprKind::Ident(hier) => {
                // An UNROLLED loop variable is a compile-time constant and
                // shadows everything else.
                if hier.root.is_none()
                    && hier.path.len() == 1
                    && hier.path[0].selects.is_empty()
                {
                    if let Some(v) = self
                        .local_const_vars
                        .get(&hier.path[0].name.name)
                        .cloned()
                    {
                        let r = self.alloc_reg();
                        self.emit(Insn::LoadConst(r, Box::new(v)));
                        return Some(r);
                    }
                }
                // A register-backed block local (a for-loop variable) shadows
                // any same-named signal for the duration of its loop.
                if let Some((src, _)) = self.local_var_reg_of(hier) {
                    let r = self.alloc_reg();
                    self.emit(Insn::Move(r, src));
                    return Some(r);
                }
                // Element of a register-bound LOCAL ARRAY (`row[n]` parsed as
                // an Ident with one select).
                if !self.local_var_array.is_empty()
                    && hier.path.len() == 1
                    && hier.path[0].selects.len() == 1
                    && self.local_var_array.contains_key(hier.path[0].name.name.as_str())
                {
                    let name = hier.path[0].name.name.clone();
                    let idx = hier.path[0].selects[0].clone();
                    if let Some(r) = self.compile_local_array_read(&name, &idx) {
                        return Some(r);
                    }
                    self.bail("local_array_read");
                    return None;
                }
                // Dynamic element of a 2-D unpacked array, Ident-with-selects
                // shape (`TBL[i][j]` inside an inlined body).
                if hier.path.len() == 1 && hier.path[0].selects.len() == 2 {
                    let i_e = hier.path[0].selects[0].clone();
                    let j_e = hier.path[0].selects[1].clone();
                    let mut bare = hier.clone();
                    bare.path[0].selects.clear();
                    if let Some(r) = self.compile_2d_array_read(&bare, &i_e, &j_e) {
                        return Some(r);
                    }
                }
                // A REAL-valued parameter wins over its signal-table twin: a
                // header parameter gets a placeholder signal entry whose
                // stored Value is integral/x, so loading the signal compared
                // raw bits against a real literal and `r1 != 5.0` on
                // `parameter real r1 = 5.0` came out true. The parameter
                // table is authoritative and carries the f64. Integral
                // parameters keep the historical signal-first order.
                if let Some(v) = self.lookup_param_value(hier) {
                    if v.is_real {
                        let r = self.alloc_reg();
                        self.emit(Insn::LoadConst(r, Box::new(v)));
                        return Some(r);
                    }
                }
                if let Some(id) = self.lookup_signal_id(hier) {
                    let r = self.alloc_reg();
                    if self.signal_signed[id] {
                        self.emit(Insn::LoadSignalSigned(r, as_sig_id(id)));
                    } else {
                        self.emit(Insn::LoadSignal(r, as_sig_id(id)));
                    }
                    return Some(r);
                }
                if let Some(v) = self.lookup_param_value(hier) {
                    let r = self.alloc_reg();
                    self.emit(Insn::LoadConst(r, Box::new(v)));
                    return Some(r);
                }
                if let Some(r) = self.compile_packed_member_read(hier) {
                    return Some(r);
                }
                if std::env::var_os("XEZIM_PROBE_IDENT").is_some() {
                    if let ExprKind::Ident(h) = &expr.kind {
                        eprintln!("[IDENT_FALLBACK] {}", Self::hier_raw_name(h));
                    }
                }
                if let Some(r) = self.emit_expr_fallback(expr, ctx_width, "ident_lookup") {
                    return Some(r);
                }
                self.bail("ident_lookup");
                None
            }
            ExprKind::StringLiteral(s) => {
                let mut v = Value::from_string(s);
                if ctx_width > 0 {
                    v = v.resize(ctx_width);
                }
                let r = self.alloc_reg();
                self.emit(Insn::LoadConst(r, Box::new(v)));
                Some(r)
            }
            ExprKind::Unary { op, operand } => {
                // Reduction (&a, |a, ^a, ~&a, ~|a, ~^a) and logical-NOT (!a)
                // are SELF-DETERMINED: operand keeps its natural width, the
                // unary produces 1 bit. Passing parent ctx_width here would
                // resize the operand and corrupt the reduction
                // (e.g. zero-extending a 32-bit value to 64 makes &a = 0
                // even when the 32-bit value was all 1s).
                let operand_ctx = if matches!(
                    op,
                    UnaryOp::BitAnd
                        | UnaryOp::BitNand
                        | UnaryOp::BitOr
                        | UnaryOp::BitNor
                        | UnaryOp::BitXor
                        | UnaryOp::BitXnor
                        | UnaryOp::LogNot
                ) {
                    0
                } else {
                    ctx_width
                };
                let src = self.compile_expr(operand, operand_ctx)?;
                // §11.6.1: `~` and unary `-` are CONTEXT-determined — the
                // operand is extended to the context width BEFORE the
                // operation, not after. Passing `operand_ctx` down is not
                // enough on its own: a plain signal load returns its declared
                // width, so `logic [31:0] r = ~a;` with an 8-bit `a` computed
                // ~a in 8 bits and zero-extended, giving 0000004b where
                // ffffff4b is required (and 0000004c for `-a`). Resize
                // explicitly; the value carries its own signedness, so a
                // signed operand still sign-extends.
                let src = if operand_ctx > 0
                    && matches!(op, UnaryOp::Minus | UnaryOp::BitNot)
                {
                    self.emit(Insn::Resize(src, operand_ctx));
                    src
                } else {
                    src
                };
                let dest = self.alloc_reg();
                match op {
                    UnaryOp::Plus => return Some(src),
                    UnaryOp::Minus => self.emit(Insn::Negate(dest, src)),
                    UnaryOp::LogNot => self.emit(Insn::LogNot(dest, src)),
                    UnaryOp::BitNot => self.emit(Insn::BitNot(dest, src)),
                    UnaryOp::BitAnd => self.emit(Insn::ReduceAnd(dest, src)),
                    UnaryOp::BitNand => {
                        self.emit(Insn::ReduceAnd(dest, src));
                        self.emit(Insn::BitNot(dest, dest));
                    }
                    UnaryOp::BitOr => self.emit(Insn::ReduceOr(dest, src)),
                    UnaryOp::BitNor => {
                        self.emit(Insn::ReduceOr(dest, src));
                        self.emit(Insn::BitNot(dest, dest));
                    }
                    UnaryOp::BitXor => self.emit(Insn::ReduceXor(dest, src)),
                    UnaryOp::BitXnor => {
                        self.emit(Insn::ReduceXor(dest, src));
                        self.emit(Insn::BitNot(dest, dest));
                    }
                    _ => {
                        self.bail("UnaryOp_other");
                        return None;
                    }
                }
                Some(dest)
            }
            ExprKind::Binary { op, left, right } => {
                // Verilog operand-width rules: comparison and logical ops
                // (==, !=, <, <=, >, >=, &&, ||, ===, !==, case-eq) are
                // self-determined — their operands' widths are max(L,R) of
                // the operands themselves, NOT the surrounding context.
                // Propagating the (often narrow, e.g. 1-bit LHS) ctx_width
                // into them silently truncates wide sub-expressions like
                // `(addr[31:20] & mask[11:0]) == base[11:0]` where the
                // 12-bit BitAnd would get resized to 1 bit, producing
                // wrong results on any high-order bits. (Bug seen on E902
                // cr_bmu_dbus_if iahbl_hit cont-assign at cyc 14: addr
                // 0x20000000 → 0x200, AND'd with 0xe00 should be 0x200,
                // but resized to 1 bit gives 0, so == 0 returns 1 instead
                // of 0.)
                let is_self_determined = matches!(
                    op,
                    BinaryOp::Eq
                        | BinaryOp::Neq
                        | BinaryOp::CaseEq
                        | BinaryOp::CaseNeq
                        | BinaryOp::WildcardEq
                        | BinaryOp::WildcardNeq
                        | BinaryOp::Lt
                        | BinaryOp::Leq
                        | BinaryOp::Gt
                        | BinaryOp::Geq
                        | BinaryOp::LogAnd
                        | BinaryOp::LogOr
                        | BinaryOp::LogImplies
                        | BinaryOp::LogEquiv
                );
                // §11.6.1: for the operators whose operands are
                // CONTEXT-determined, the context width is the MAXIMUM of the
                // surrounding context and the operands' own widths — it must
                // never NARROW an operand. Propagating a narrow LHS width down
                // truncated the left operand before the operation, which is
                // observably wrong wherever the low bits are not preserved:
                // `logic [4:0] r; r <= (1 << s) >> 3;` computed `1 << 5` at 5
                // bits (0) instead of 32 bits (32), so r read 0 instead of 4.
                // (For +/-/*/&/|/^ the low bits are the same either way, which
                // is why only the shift/divide family showed it.)
                let widens_operands = matches!(
                    op,
                    BinaryOp::ShiftLeft
                        | BinaryOp::ShiftRight
                        | BinaryOp::ArithShiftLeft
                        | BinaryOp::ArithShiftRight
                        | BinaryOp::Div
                        | BinaryOp::Mod
                        | BinaryOp::Power
                );
                let sub_ctx = if is_self_determined {
                    let lw = self.expr_max_width(left);
                    let rw = self.expr_max_width(right);
                    lw.max(rw)
                } else if widens_operands {
                    ctx_width.max(self.expr_max_width(left))
                } else {
                    ctx_width
                };
                let l = self.compile_expr(left, sub_ctx)?;
                // §11.4.10: a shift's RIGHT operand is SELF-DETERMINED — its
                // width never affects the result, so it keeps its own.
                let is_shift = matches!(
                    op,
                    BinaryOp::ShiftLeft
                        | BinaryOp::ShiftRight
                        | BinaryOp::ArithShiftLeft
                        | BinaryOp::ArithShiftRight
                );
                let r = if is_shift {
                    self.compile_expr(right, self.expr_max_width(right))?
                } else {
                    self.compile_expr(right, sub_ctx)?
                };
                // Context width resizing for arithmetic / bitwise ops only.
                // For self-determined comparisons we must NOT resize to
                // ctx_width — that would clobber the operands.
                if !is_self_determined
                    && ctx_width > 0
                    && matches!(
                        op,
                        BinaryOp::Add
                            | BinaryOp::Sub
                            | BinaryOp::Mul
                            | BinaryOp::BitAnd
                            | BinaryOp::BitOr
                            | BinaryOp::BitXor
                            | BinaryOp::BitXnor
                    )
                {
                    // §11.8.1: the expression is UNSIGNED if ANY operand is
                    // unsigned — widening must then ZERO-extend both. The
                    // runtime Resize extends by each VALUE's own signed flag,
                    // so a signed operand in a mixed expression sign-extended:
                    // `sa + b` in a 32-bit context read fffffff9 instead of
                    // 000000f9 (the $display path was already correct).
                    // Unknown signedness keeps the historical behavior.
                    let ls = self.expr_signedness(left);
                    let rs = self.expr_signedness(right);
                    if ls == Some(false) || rs == Some(false) {
                        if !self.operand_scrub_is_noop(left) {
                            self.emit(Insn::ClearSigned(l));
                        }
                        if !self.operand_scrub_is_noop(right) {
                            self.emit(Insn::ClearSigned(r));
                        }
                    }
                    self.emit(Insn::Resize(l, ctx_width));
                    self.emit(Insn::Resize(r, ctx_width));
                }
                let dest = self.alloc_reg();
                match op {
                    BinaryOp::Add => self.emit(Insn::Add(dest, l, r)),
                    BinaryOp::Sub => self.emit(Insn::Sub(dest, l, r)),
                    BinaryOp::Mul => self.emit(Insn::Mul(dest, l, r)),
                    BinaryOp::Div | BinaryOp::Mod => {
                        // §11.6.1 Table 11-21: BOTH operands are context-
                        // determined. The registers kept their declared
                        // widths, so `smin / sm1` divided at 8 bits (wrapping
                        // -128/-1) and a divide-by-zero produced x at 8 bits.
                        // §11.8.1 signedness applies exactly as for +/-.
                        let opw = ctx_width
                            .max(self.lrm_self_width(left))
                            .max(self.lrm_self_width(right))
                            .max(1);
                        let ls = self.expr_signedness(left);
                        let rs = self.expr_signedness(right);
                        if ls == Some(false) || rs == Some(false) {
                            if !self.operand_scrub_is_noop(left) {
                                self.emit(Insn::ClearSigned(l));
                            }
                            if !self.operand_scrub_is_noop(right) {
                                self.emit(Insn::ClearSigned(r));
                            }
                        }
                        self.emit(Insn::Resize(l, opw));
                        self.emit(Insn::Resize(r, opw));
                        if matches!(op, BinaryOp::Div) {
                            self.emit(Insn::Div(dest, l, r));
                        } else {
                            self.emit(Insn::Mod(dest, l, r));
                        }
                    }
                    BinaryOp::BitAnd => self.emit(Insn::BitAnd(dest, l, r)),
                    BinaryOp::BitOr => self.emit(Insn::BitOr(dest, l, r)),
                    BinaryOp::BitXor => self.emit(Insn::BitXor(dest, l, r)),
                    BinaryOp::BitXnor => self.emit(Insn::BitXnor(dest, l, r)),
                    BinaryOp::LogAnd => self.emit(Insn::LogAnd(dest, l, r)),
                    BinaryOp::LogOr => self.emit(Insn::LogOr(dest, l, r)),
                    // a -> b  ==  !a || b   (IEEE 1800-2023 §11.4.7)
                    BinaryOp::LogImplies => {
                        self.emit(Insn::LogNot(dest, l));
                        self.emit(Insn::LogOr(dest, dest, r));
                    }
                    // a <-> b  ==  (!a || b) && (!b || a)
                    BinaryOp::LogEquiv => {
                        let nl = self.alloc_reg();
                        let nr = self.alloc_reg();
                        let t1 = self.alloc_reg();
                        self.emit(Insn::LogNot(nl, l));
                        self.emit(Insn::LogNot(nr, r));
                        self.emit(Insn::LogOr(t1, nl, r));
                        self.emit(Insn::LogOr(dest, nr, l));
                        self.emit(Insn::LogAnd(dest, t1, dest));
                    }
                    // §6.16 / Table 6-9: `==`/`!=` on STRING operands are
                    // 2-STATE textual compares. The integral Eq on the
                    // 1024-bit packed storage returned X whenever either
                    // side's unused capacity was X padding — an empty-string
                    // compare always was. Same StrOp routing as the
                    // relational arm below.
                    BinaryOp::Eq | BinaryOp::Neq
                        if self.expr_is_string_static(left)
                            && self.expr_is_string_static(right) =>
                    {
                        let cmp = self.alloc_reg();
                        self.emit(Insn::StrOp(
                            cmp,
                            StrOpKind::Compare,
                            Box::new(vec![l, r]),
                        ));
                        let z = self.alloc_reg();
                        let mut zero = Value::from_u64(0, 32);
                        zero.is_signed = true;
                        self.emit(Insn::LoadConst(z, Box::new(zero)));
                        self.emit(if matches!(op, BinaryOp::Eq) {
                            Insn::Eq(dest, cmp, z)
                        } else {
                            Insn::Neq(dest, cmp, z)
                        });
                    }
                    BinaryOp::Eq => self.emit(Insn::Eq(dest, l, r)),
                    BinaryOp::Neq => self.emit(Insn::Neq(dest, l, r)),
                    BinaryOp::CaseEq => self.emit(Insn::CaseEq(dest, l, r)),
                    // LRM §11.4.5: `!==` is the bit-exact negation of `===`.
                    // No dedicated Insn; compose CaseEq → LogNot. (Previously
                    // this hit the catch-all and bailed to the AST interp.)
                    BinaryOp::CaseNeq => {
                        self.emit(Insn::CaseEq(dest, l, r));
                        self.emit(Insn::LogNot(dest, dest));
                    }
                    // §6.16.6: relational operators on STRING operands are
                    // lexicographic. The numeric Lt/Gt insns compare packed
                    // magnitudes, which diverges once lengths differ
                    // ("abc" < "b" is TRUE lexicographically, false
                    // numerically) — route through the native strcmp and
                    // compare its signed difference against zero.
                    BinaryOp::Lt | BinaryOp::Leq | BinaryOp::Gt | BinaryOp::Geq
                        if self.expr_is_string_static(left)
                            && self.expr_is_string_static(right) =>
                    {
                        let cmp = self.alloc_reg();
                        self.emit(Insn::StrOp(
                            cmp,
                            StrOpKind::Compare,
                            Box::new(vec![l, r]),
                        ));
                        let z = self.alloc_reg();
                        let mut zero = Value::from_u64(0, 32);
                        zero.is_signed = true;
                        self.emit(Insn::LoadConst(z, Box::new(zero)));
                        self.emit(match op {
                            BinaryOp::Lt => Insn::Lt(dest, cmp, z),
                            BinaryOp::Leq => Insn::Leq(dest, cmp, z),
                            BinaryOp::Gt => Insn::Gt(dest, cmp, z),
                            _ => Insn::Geq(dest, cmp, z),
                        });
                    }
                    BinaryOp::Lt => self.emit(Insn::Lt(dest, l, r)),
                    BinaryOp::Leq => self.emit(Insn::Leq(dest, l, r)),
                    BinaryOp::Gt => self.emit(Insn::Gt(dest, l, r)),
                    BinaryOp::Geq => self.emit(Insn::Geq(dest, l, r)),
                    BinaryOp::ShiftLeft | BinaryOp::ArithShiftLeft => {
                        // §11.4.10/§11.6.1: the LEFT operand takes the LRM
                        // operation width — ctx joined with the operand's own
                        // LRM width (never the carry-aware estimate, which
                        // shifted dropped carries back into range).
                        let opw = ctx_width.max(self.lrm_self_width(left)).max(1);
                        self.emit(Insn::Resize(l, opw));
                        self.emit(Insn::Shl(dest, l, r));
                    }
                    BinaryOp::ShiftRight | BinaryOp::ArithShiftRight => {
                        // Same rule for right shifts — previously the operand
                        // register kept whatever width its sub-expression
                        // produced: a signed 8-bit value in a 32-bit context
                        // shifted at 8 bits then zero-extended (00000013 for
                        // 1ffffff3), and `(a+a) >> 1` shifted the carry back
                        // in (0xa3 for 0x23).
                        let opw = ctx_width.max(self.lrm_self_width(left)).max(1);
                        self.emit(Insn::Resize(l, opw));
                        if matches!(op, BinaryOp::ShiftRight) {
                            self.emit(Insn::Shr(dest, l, r));
                        } else {
                            self.emit(Insn::AShr(dest, l, r));
                        }
                    }
                    // LRM §11.4.3 power. There is no runtime Pow instruction;
                    // every `**` seen in RTL has constant operands (`2**level`
                    // after genvar substitution, `2**N` parameters), so fold
                    // it to a constant here. Without this arm `**` hit the
                    // catch-all `bail` below — which, for a `**` inside an
                    // array-element LHS index like `mem[2**lvl-1+k]`, dropped
                    // the whole continuous assign to the AST interpreter and
                    // mis-evaluated the RHS. A genuinely non-constant `a**b`
                    // still bails (rare; preserves prior behavior).
                    BinaryOp::Power => {
                        // Fold `**` to a constant (no runtime Pow insn). Compute
                        // the result in u64 and load it at the expression's
                        // natural width: `eval_const_expr` truncates to u32 and
                        // the old `from_u64(v, 32)` truncated again, so 2**N for
                        // N>=32 collapsed to 0 (e.g. 2**51 -> 0). (pr2865563)
                        if let (Some(base), Some(exp)) =
                            (self.eval_const_expr(left), self.eval_const_expr(right))
                        {
                            let mut result: u64 = 1;
                            for _ in 0..(exp as u64).min(64) {
                                result = result.wrapping_mul(base as u64);
                            }
                            let w = self.expr_max_width(expr).max(ctx_width).max(1);
                            self.emit(Insn::LoadConst(dest, Box::new(Value::from_u64(result, w))));
                        } else {
                            // Non-constant base: a REAL Pow insn. The left
                            // operand is context-determined (§11.6.1) — a
                            // load returns its declared width, so resize it
                            // to the operation width first; `a ** 2` in a
                            // 32-bit context computed at 8 bits (0x90) and
                            // then bailed the whole block to the interpreter.
                            let opw = sub_ctx.max(self.expr_max_width(left)).max(1);
                            self.emit(Insn::Resize(l, opw));
                            self.emit(Insn::Pow(dest, l, r));
                        }
                    }
                    _ => {
                        self.bail("BinaryOp_other");
                        return None;
                    }
                }
                Some(dest)
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                // Evaluate both branches unconditionally so Select can do a
                // per-bit merge when the condition has X/Z (IEEE 1800 §11.4.11).
                let cond = self.compile_expr(condition, 0)?;
                let then_reg = self.compile_expr(then_expr, ctx_width)?;
                let else_reg = self.compile_expr(else_expr, ctx_width)?;
                // §11.8.1: a ternary with ANY unsigned arm is unsigned — both
                // arms then ZERO-extend (a signed arm sign-extended, so
                // `c ? sa : b` in a 32-bit context read ffffff9c for
                // 0000009c). And §11.4.11's x-condition per-bit merge must
                // happen at the CONTEXT width — merging at arm width and
                // zero-extending after produced 000000XX for xxxxxxXX.
                if ctx_width > 0 {
                    let ts = self.expr_signedness(then_expr);
                    let es = self.expr_signedness(else_expr);
                    if ts == Some(false) || es == Some(false) {
                        if !self.operand_scrub_is_noop(then_expr) {
                            self.emit(Insn::ClearSigned(then_reg));
                        }
                        if !self.operand_scrub_is_noop(else_expr) {
                            self.emit(Insn::ClearSigned(else_reg));
                        }
                    }
                    self.emit(Insn::Resize(then_reg, ctx_width));
                    self.emit(Insn::Resize(else_reg, ctx_width));
                }
                let dest = self.alloc_reg();
                self.emit(Insn::Select(dest, cond, then_reg, else_reg));
                Some(dest)
            }
            ExprKind::Paren(inner) => self.compile_expr(inner, ctx_width),
            ExprKind::Index { expr, index } => {
                // Register-bank local array with a CONSTANT (possibly folded
                // through an unrolled loop var) index: a plain register move.
                if let ExprKind::Ident(h) = &expr.kind {
                    if h.root.is_none()
                        && h.path.len() == 1
                        && h.path[0].selects.is_empty()
                    {
                        if let Some(&(base, ew, len, lo)) =
                            self.local_array_regs.get(&h.path[0].name.name)
                        {
                            let Some(iv) =
                                self.fold_const(index).and_then(|v| v.to_u64())
                            else {
                                // Dynamic index into registers is impossible.
                                self.bail("local_array_dyn_index");
                                return None;
                            };
                            let slot = (iv as i64) - lo;
                            if slot < 0 || slot as usize >= len {
                                self.bail("local_array_oob");
                                return None;
                            }
                            let r = self.alloc_reg();
                            self.emit(Insn::Move(r, base + slot as RegId));
                            let _ = ew;
                            return Some(r);
                        }
                    }
                }
                // A packed-array field embedded in a packed struct has no
                // standalone signal id. Slice it directly from its container;
                // the generic non-Ident index path below would otherwise take
                // one bit instead of one packed element.
                let member_start = self.insns.len();
                let member_reg = self.next_reg;
                if let Some(dest) = self.compile_packed_member_index(expr, index) {
                    return Some(dest);
                }
                self.insns.truncate(member_start);
                self.next_reg = member_reg;
                // §7.4.1 CHAINED packed element select — `v[i][j][k]` on
                // `logic [0:0][1:0][1:0]`. Only the innermost `Index` has an
                // Ident base, so the outer ones fell through to the bit select
                // below: `v[0]` gave a 4-bit slice, `[0]` then took ONE BIT of
                // it, and `[1]` ran off the end and produced x. The AST
                // interpreter walks the whole chain, so `$display` printed the
                // right bit while the same expression in an `assign` or an `if`
                // condition read x — a guard that never fired while the value
                // still looked correct in a print.
                //
                // Must precede the Ident-base branch, which by construction
                // only ever sees one level. Constant indices only: that is the
                // shape RTL uses (and what a genvar unrolls to), and it leaves
                // the single-level dynamic path untouched.
                if let Some((lo, w)) = self.chained_packed_slice(expr, index) {
                    let base = self.compile_expr_root_of(expr)?;
                    let dest = self.alloc_reg();
                    self.emit(Insn::RangeSelectConst(dest, base, lo + w - 1, lo));
                    return Some(dest);
                }
                // Same chain with a DYNAMIC index somewhere — `a[i][j][3]`.
                if let Some(dest) = self.emit_chained_packed_slice_dyn(expr, index) {
                    return Some(dest);
                }
                // §11.5.1: `(X[a:b])[i]` on a packed ARRAY selects ELEMENT i
                // (labels pass through a constant part-select). This shape
                // comes from port inlining of `.p(arr[15:0])`-style
                // connections; the bit-select compilation below would read
                // bit i of a bit-slice. Bail to the AST interpreter, which
                // normalizes it to a plain element select.
                if let ExprKind::RangeSelect { expr: rs_base, .. } = &expr.kind {
                    if let ExprKind::Ident(h) = &rs_base.kind {
                        let nm = Self::hier_raw_name(h);
                        let elemish = self
                            .packed_elem_widths
                            .is_some_and(|m| m.get(&nm).is_some_and(|&ew| ew > 1))
                            || self
                                .packed_full_dims
                                .is_some_and(|m| m.get(&nm).is_some_and(|d| d.len() > 1));
                        if elemish {
                            self.bail("Index_of_ranged_packed_array");
                            return None;
                        }
                    }
                }
                // Packed element of a REGISTER-BACKED local (`x[i]` on a
                // `u8_vec16_t x` formal/local inside an inlined function):
                // slice [i*ew +: ew] straight out of the register. Assumes a
                // [N-1:0] outer range (what a packed-of-packed typedef
                // declares); locals with exotic outer bounds simply have no
                // elem entry and keep the old path.
                if let ExprKind::Ident(h) = &expr.kind {
                    let raw = Self::hier_raw_name(h);
                    if let Some(&ew) = self.local_var_elem.get(&raw) {
                        if let Some((src, _w)) = self.local_var_reg_of(h) {
                            let idx_reg = self.compile_expr(index, 0)?;
                            let ew_reg = self.alloc_reg();
                            self.emit(Insn::LoadConst(
                                ew_reg,
                                Box::new(Value::from_u64(ew as u64, 32)),
                            ));
                            let lo_reg = self.alloc_reg();
                            self.emit(Insn::Mul(lo_reg, idx_reg, ew_reg));
                            self.emit(Insn::Resize(lo_reg, 32));
                            let ewm1 = self.alloc_reg();
                            self.emit(Insn::LoadConst(
                                ewm1,
                                Box::new(Value::from_u64((ew - 1) as u64, 32)),
                            ));
                            let hi_reg = self.alloc_reg();
                            self.emit(Insn::Add(hi_reg, lo_reg, ewm1));
                            self.emit(Insn::Resize(hi_reg, 32));
                            let dest = self.alloc_reg();
                            self.emit(Insn::RangeSelect(dest, src, hi_reg, lo_reg));
                            return Some(dest);
                        }
                    }
                }
                // Element of a MULTI-dimensional unpacked array (`grid[i][j]`).
                // The base of the outer Index is itself an Index, so none of
                // the arms below match and the whole thing fell through to the
                // plain BIT-SELECT path: `grid[1][2]` compiled to bit 2 of bit
                // 1 of the array's base signal, and the read came back x.
                // Elements are stored under their flat name, so with constant
                // indices the element resolves directly. A dynamic index has no
                // flat name — leave those to the AST fallback, which handles
                // them.
                if let Some(id) = self.multi_dim_elem_signal_id(expr, index) {
                    let dest = self.alloc_reg();
                    self.emit(Insn::LoadSignal(dest, as_sig_id(id)));
                    return Some(dest);
                }
                // Array element access
                if let ExprKind::Ident(hier) = &expr.kind {
                    // An ASSOCIATIVE array element has no dense storage and no
                    // bit offset: falling through compiled `aa["bob"]` as a
                    // BIT-SELECT of the array's placeholder signal, with the
                    // packed string as the bit index. Fail the compile so the
                    // read stays on the AST path.
                    if self.is_assoc_target(hier) {
                        self.bail("assoc_elem_read");
                        return None;
                    }
                    if let Some(name) = self.lookup_array_name(hier) {
                        let idx_reg = self.compile_expr(index, 0)?;
                        let dest = self.alloc_reg();
                        let array = self.array_operand(name);
                        self.emit(Insn::LoadArrayElem(dest, array, idx_reg));
                        return Some(dest);
                    }
                    // Packed multi-D READ: `mem_q[i]` for `logic [N-1:0][W-1:0]`
                    // must extract a W-bit slice at `i*W +: W`, not a single
                    // bit. Mirror the LHS variable-index slice path so reads
                    // and writes stay symmetric.
                    let elem_w = self.packed_elem_width_of(hier);
                    if let Some(elem_w) = elem_w {
                        let base = self.compile_expr(expr, 0)?;
                        // Constant index (the common case — genvar-unrolled
                        // `idx_nodes[n] = idx_lut[k]` in rr_arb_tree/lzc, and
                        // any literal `b[4]`): emit a CONSTANT-range slice.
                        // The dynamic RangeSelect below produces a result whose
                        // width is only known at runtime; feeding that into a
                        // packed-2D element LHS write (BlockingAssignRangeDyn)
                        // mis-places the bits and the target reads back X. A
                        // RangeSelectConst carries a static width, so the LHS
                        // write lands correctly. (This was the FlooNOC "router
                        // never forwards" root cause: the arbiter's selected
                        // index came out X.)
                        if let Some(idx) = self.eval_const_expr(index) {
                            let lo = Self::packed_elem_lsb(
                                self.packed_outer_dim(hier),
                                idx as i64,
                                elem_w,
                            )
                            .max(0) as u32;
                            let hi = lo + elem_w - 1;
                            let dest = self.alloc_reg();
                            self.emit(Insn::RangeSelectConst(dest, base, hi, lo));
                            return Some(dest);
                        }
                        let idx_reg = self.compile_expr(index, 0)?;
                        // §7.4.1: normalize a DYNAMIC index against the
                        // declared outer range, exactly like the constant
                        // branch above and the write path. Without this,
                        // `src[i]` on a `[N:1]` outer dimension read slice
                        // i+1 — and the top element read out of range,
                        // injecting X into whatever it fed (a lane-expander's
                        // per-lane status flops all went X on the first
                        // multi-lane advance).
                        let idx_reg =
                            self.emit_packed_slot_index(self.packed_outer_dim(hier), idx_reg);
                        let elem_w_reg = self.alloc_reg();
                        self.emit(Insn::LoadConst(
                            elem_w_reg,
                            Box::new(Value::from_u64(elem_w as u64, 32)),
                        ));
                        let lo_reg = self.alloc_reg();
                        self.emit(Insn::Mul(lo_reg, idx_reg, elem_w_reg));
                        let em1_reg = self.alloc_reg();
                        self.emit(Insn::LoadConst(
                            em1_reg,
                            Box::new(Value::from_u64((elem_w - 1) as u64, 32)),
                        ));
                        let hi_reg = self.alloc_reg();
                        self.emit(Insn::Add(hi_reg, lo_reg, em1_reg));
                        let dest = self.alloc_reg();
                        self.emit(Insn::RangeSelect(dest, base, hi_reg, lo_reg));
                        return Some(dest);
                    }
                }
                // Bit select
                //
                // §7.4.1: a non-zero-based vector stores its declared low bit
                // at PHYSICAL offset 0 — `logic [3:1] w` keeps declared bit 1
                // at offset 0, and `logic [1:1] h` is one bit at offset 0.
                // Both `Insn::BitSelect*` index raw physical bits, so the
                // declared index has to be rebased first. The WRITE path
                // already does this via `emit_rebased_index`; the read path
                // did not, so `h[1]` selected physical bit 1 of a one-bit
                // signal and evaluated to x. `$display("%b", h[1])` was
                // correct throughout because it goes through the AST
                // interpreter, which rebases — so the bug only showed in
                // assign / always_comb / always_ff, which compile to bytecode.
                //
                // Dynamic element of a 2-D unpacked array, nested-Index
                // shape (`TBL[i][j]`).
                if let ExprKind::Index { expr: inner_e, index: i_idx } = &expr.kind {
                    if let ExprKind::Ident(h) = &inner_e.kind {
                        if h.path.len() == 1 && h.path[0].selects.is_empty() {
                            let (i_e, j_e, h2) = (i_idx.as_ref().clone(), index.as_ref().clone(), h.clone());
                            if let Some(r) = self.compile_2d_array_read(&h2, &i_e, &j_e) {
                                return Some(r);
                            }
                        }
                    }
                }
                // Element of a register-bound LOCAL ARRAY (`row[n]` parsed
                // as Index over a bare Ident).
                if !self.local_var_array.is_empty() {
                    if let ExprKind::Ident(h) = &expr.kind {
                        if h.path.len() == 1
                            && h.path[0].selects.is_empty()
                            && self.local_var_array.contains_key(h.path[0].name.name.as_str())
                        {
                            let name = h.path[0].name.name.clone();
                            if let Some(r) = self.compile_local_array_read(&name, index) {
                                return Some(r);
                            }
                            self.bail("local_array_read");
                            return None;
                        }
                    }
                }
                // §7.4.1/§11.5.1: ascending or element-of-collection bases
                // need label mapping the rebase cannot express — AST only.
                if self.sel_base_needs_ast(expr) {
                    self.bail("bit_sel_base_maps");
                    return None;
                }
                let base = self.compile_expr(expr, 0)?;
                let base_lo = match &expr.kind {
                    ExprKind::Ident(h) => self.declared_low_bound(h),
                    _ => 0,
                };
                if let Some(idx) = self.eval_const_expr(index) {
                    let dest = self.alloc_reg();
                    // Saturate rather than wrap: an out-of-range declared index
                    // is already x-valued, and a negative operand would read as
                    // a huge unsigned bit position.
                    let phys = (idx as i64 - base_lo).max(0) as u32;
                    self.emit(Insn::BitSelectConst(dest, base, phys));
                    return Some(dest);
                }
                let idx = self.compile_expr(index, 0)?;
                let idx = if base_lo != 0 {
                    match &expr.kind {
                        ExprKind::Ident(h) => self.emit_rebased_index(h, idx),
                        _ => idx,
                    }
                } else {
                    idx
                };
                let dest = self.alloc_reg();
                self.emit(Insn::BitSelect(dest, base, idx));
                Some(dest)
            }
            ExprKind::RangeSelect {
                expr,
                left,
                right,
                kind,
                ..
            } => match kind {
                RangeKind::Constant => {
                    // §7.4.1/§11.5.1: ascending or element-of-collection
                    // bases need label mapping — AST path only.
                    if self.sel_base_needs_ast(expr) {
                        self.bail("range_sel_base_maps");
                        return None;
                    }
                    let base = self.compile_expr(expr, 0)?;
                    if let (Some(l), Some(r)) =
                        (self.eval_const_expr(left), self.eval_const_expr(right))
                    {
                        // §7.4.1: on a packed MULTI-D base (`logic [1:0][63:0]`
                        // or a packed array of a struct typedef), a constant
                        // range selects ELEMENTS — `pv[1:0]` is BOTH 64-bit
                        // slices (128 bits), not bits 1..0. Scale the bounds by
                        // the registered element width; a plain vector has no
                        // entry and keeps the historical bit-range meaning.
                        if let ExprKind::Ident(h) = &expr.kind {
                            if let Some(ew) = self.packed_elem_width_of(h).filter(|&w| w > 1) {
                                let dim = self.packed_outer_dim(h);
                                let lsb_l = Self::packed_elem_lsb(dim, l as i64, ew);
                                let lsb_r = Self::packed_elem_lsb(dim, r as i64, ew);
                                let lo = lsb_l.min(lsb_r).max(0) as u32;
                                let hi = (lsb_l.max(lsb_r) + ew as i64 - 1).max(0) as u32;
                                let dest = self.alloc_reg();
                                self.emit(Insn::RangeSelectConst(dest, base, hi, lo));
                                return Some(dest);
                            }
                        }
                        let mut phys_l = l as i64;
                        let mut phys_r = r as i64;
                        if let ExprKind::Ident(h) = &expr.kind {
                            if let Some((dl, dr)) = self.packed_outer_dim(h) {
                                let lo_b = dl.min(dr);
                                if lo_b != 0 {
                                    phys_l -= lo_b;
                                    phys_r -= lo_b;
                                }
                            }
                        }
                        let dest = self.alloc_reg();
                        self.emit(Insn::RangeSelectConst(
                            dest,
                            base,
                            phys_l.max(0) as u32,
                            phys_r.max(0) as u32,
                        ));
                        return Some(dest);
                    }
                    let l = self.compile_expr(left, 0)?;
                    let r = self.compile_expr(right, 0)?;
                    let dest = self.alloc_reg();
                    self.emit(Insn::RangeSelect(dest, base, l, r));
                    Some(dest)
                }
                RangeKind::IndexedUp | RangeKind::IndexedDown => {
                    // §7.4.1/§11.5.1: ascending or element-of-collection
                    // bases need label mapping — AST path only.
                    if self.sel_base_needs_ast(expr) {
                        self.bail("range_sel_base_maps");
                        return None;
                    }
                    // `sig[idx +: W]` / `sig[idx -: W]` — W must be constant.
                    // Emit idx register, then compute hi/lo via Add/Sub with a
                    // const (W-1), and reuse existing RangeSelect insn.
                    let width = match self.eval_const_expr(right) {
                        Some(w) if w > 0 => w,
                        _ => {
                            self.bail("RangeSelect_width_nonconst");
                            return None;
                        }
                    };
                    let base = self.compile_expr(expr, 0)?;
                    let idx = self.compile_expr(left, 0)?;
                    // §7.4.6/§11.5.1: the base index is a DECLARED index, but
                    // `RangeSelect` takes physical bit offsets — rebase it for a
                    // non-zero-based vector exactly as the plain bit select
                    // does. Without this `w[1 +: 2]` on a `logic [3:1] w` read
                    // physical 2:1 (declared 3:2) instead of declared 2:1, and
                    // `w[3 -: 2]` ran off the top of the signal and returned x.
                    let idx = match &expr.kind {
                        ExprKind::Ident(h) => self.emit_rebased_index(h, idx),
                        _ => idx,
                    };
                    let dest = self.alloc_reg();
                    if width == 1 {
                        self.emit(Insn::RangeSelect(dest, base, idx, idx));
                    } else {
                        let delta = self.alloc_reg();
                        self.emit(Insn::LoadConst(
                            delta,
                            Box::new(Value::from_u64((width - 1) as u64, 32)),
                        ));
                        let other = self.alloc_reg();
                        if *kind == RangeKind::IndexedUp {
                            self.emit(Insn::Add(other, idx, delta));
                            self.emit(Insn::RangeSelect(dest, base, other, idx));
                        } else {
                            self.emit(Insn::Sub(other, idx, delta));
                            self.emit(Insn::RangeSelect(dest, base, idx, other));
                        }
                    }
                    Some(dest)
                }
            },
            ExprKind::Replication { count, exprs } => {
                let n = match self.eval_const_expr(count) {
                    Some(val) => val,
                    _ => {
                        self.bail("Replication_nonconst_count");
                        return None;
                    }
                };
                if n == 0 {
                    let dest = self.alloc_reg();
                    self.emit(Insn::LoadConst(dest, Box::new(Value::zero(0))));
                    return Some(dest);
                }
                if n > 10000 {
                    self.bail("Replication_excessive_count");
                    return None;
                }

                // Optimization: use Insn::Replicate if possible
                if exprs.len() == 1 {
                    let r = self.compile_expr(&exprs[0], 0)?;
                    let dest = self.alloc_reg();
                    self.emit(Insn::Replicate(dest, r, n));
                    return Some(dest);
                }

                let mut regs = Vec::with_capacity((exprs.len() * n as usize).max(1));
                for _ in 0..n {
                    for e in exprs {
                        let r = self.compile_expr(e, 0)?;
                        regs.push(r);
                    }
                }
                let dest = self.alloc_reg();
                self.emit(Insn::Concat(dest, Box::new(regs)));
                Some(dest)
            }
            ExprKind::Concatenation(parts) => {
                // LRM §11.4.12 — when any operand is a `string`, `{a, b, …}`
                // is a string concat (byte-level), not a bit-vector concat.
                // The bytecode `Concat` insn bit-concatenates and would
                // shift the bytes (e.g. a 5-char "hello" gets sized to 40
                // bits and aligned wrong), so for any string-valued operand
                // we bail to the AST interpreter which has the special
                // case at `eval_expr_ctx::Concatenation`.
                if parts.iter().any(|p| self.expr_is_string_concat_operand(p)) {
                    // §11.4.12: with every operand statically string-valued,
                    // this is a byte-level join — one native op. A MIXED
                    // concat (some operands of unknown type) keeps the AST
                    // path, whose type knowledge is authoritative.
                    if parts.iter().all(|p| self.expr_is_string_static(p)) {
                        let start = self.insns.len();
                        let start_reg = self.next_reg;
                        let mut regs: Vec<RegId> = Vec::with_capacity(parts.len());
                        let mut ok = true;
                        for p in parts {
                            match self.compile_expr(p, 0) {
                                Some(r) => regs.push(r),
                                None => {
                                    ok = false;
                                    break;
                                }
                            }
                        }
                        if ok {
                            let dst = self.alloc_reg();
                            self.emit(Insn::StrOp(dst, StrOpKind::Concat, Box::new(regs)));
                            return Some(dst);
                        }
                        self.insns.truncate(start);
                        self.next_reg = start_reg;
                    }
                    self.bail("Concat_string");
                    return None;
                }
                let mut regs = Vec::new();
                for p in parts {
                    let r = self.compile_expr(p, 0)?;
                    regs.push(r);
                }
                let dest = self.alloc_reg();
                self.emit(Insn::Concat(dest, Box::new(regs)));
                Some(dest)
            }
            // §11.4.14 streaming concatenation. `{>>{…}}` is exactly the
            // concatenation; `{<<N{…}}` additionally reverses the order of
            // N-bit slices, which with a constant N and a known total width is
            // a FIXED bit permutation — a concat of constant range selects.
            // Previously the whole expression fell to the AST interpreter,
            // measured at ~0.45us per evaluation (a byte-swap written as
            // `{<<8{x}}` ran ~32% slower than the same swap written out by
            // hand).
            ExprKind::StreamOp {
                left_to_right,
                slice_size,
                exprs,
            } => {
                // Widths must be known to place the slices; a part whose width
                // the compiler cannot size keeps the AST path.
                // The LRM self-determined width, cross-checked against
                // `expr_max_width`: the latter is unreliable for an index
                // select (it reports 1 for an element of a packed-struct
                // typedef array), and a wrong total silently permutes the
                // wrong bits. Disagreement means the width is not established
                // well enough to place slices — keep the AST path.
                // Placing the slices needs each part's width to be exactly
                // right — a wrong total silently permutes the wrong bits, and
                // the general width oracles are not trustworthy enough here:
                // BOTH `lrm_self_width` and `expr_max_width` report 1 for an
                // element of a packed-struct typedef array in a submodule, so
                // cross-checking them does not catch it. Accept only shapes
                // whose width is unambiguous here and leave every other
                // spelling on the (correct) AST path.
                let mut widths: Vec<u32> = Vec::with_capacity(exprs.len());
                for e in exprs {
                    let mut inner = e;
                    while let ExprKind::Paren(i) = &inner.kind {
                        inner = i;
                    }
                    let w = match &inner.kind {
                        // A whole signal: the signal table is authoritative.
                        ExprKind::Ident(h) if h.path.iter().all(|s| s.selects.is_empty()) => self
                            .lookup_signal_id(h)
                            .and_then(|id| self.signal_widths.get(id).copied())
                            .unwrap_or(0),
                        // `x[hi:lo]` with constant bounds: hi - lo + 1.
                        ExprKind::RangeSelect {
                            left,
                            right,
                            kind: RangeKind::Constant,
                            ..
                        } => match (self.eval_const_expr(left), self.eval_const_expr(right)) {
                            (Some(hi), Some(lo)) if hi >= lo => hi - lo + 1,
                            (Some(hi), Some(lo)) => lo - hi + 1,
                            _ => 0,
                        },
                        _ => 0,
                    };
                    if w == 0 {
                        self.bail("Stream_operand_shape");
                        return None;
                    }
                    widths.push(w);
                }
                let total_w: u32 = widths.iter().sum();
                let mut regs = Vec::with_capacity(exprs.len());
                for e in exprs {
                    regs.push(self.compile_expr(e, 0)?);
                }
                let src = self.alloc_reg();
                self.emit(Insn::Concat(src, Box::new(regs)));
                if !*left_to_right {
                    return Some(src);
                }
                // `{<<N{…}}`: N must be a constant to know the chunking.
                let slice = match slice_size {
                    None => 1u32,
                    Some(e) => match self.fold_const(e).and_then(|v| v.to_u64()) {
                        Some(n) if n > 0 && n <= u32::MAX as u64 => n as u32,
                        _ => {
                            self.bail("Stream_slice_nonconst");
                            return None;
                        }
                    },
                };
                if slice >= total_w {
                    // One chunk (plus nothing to reverse): identity.
                    return Some(src);
                }
                let full = total_w / slice;
                let rem = total_w - full * slice;
                // Output MSB-first is chunk0, chunk1, … chunk(full-1), then the
                // leftover high bits of the source. Chunk k occupies source
                // bits [k*slice + slice-1 : k*slice].
                let mut parts: Vec<RegId> = Vec::with_capacity(full as usize + 1);
                let mut range = |me: &mut Self, hi: u32, lo: u32| -> RegId {
                    let hr = me.alloc_reg();
                    me.emit(Insn::LoadConst(hr, Box::new(Value::from_u64(hi as u64, 32))));
                    let lr = me.alloc_reg();
                    me.emit(Insn::LoadConst(lr, Box::new(Value::from_u64(lo as u64, 32))));
                    let d = me.alloc_reg();
                    me.emit(Insn::RangeSelect(d, src, hr, lr));
                    d
                };
                for k in 0..full {
                    let lo = k * slice;
                    parts.push(range(self, lo + slice - 1, lo));
                }
                if rem > 0 {
                    parts.push(range(self, total_w - 1, full * slice));
                }
                let dst = self.alloc_reg();
                self.emit(Insn::Concat(dst, Box::new(parts)));
                Some(dst)
            }
            ExprKind::SystemCall { name, args } => match name.as_str() {
                    // §21.3.3 `$sformatf` with a LITERAL template and specs the
                    // native filler covers exactly — parsed once here, filled
                    // from register Values at exec. Anything else (non-literal
                    // fmt, %t/%p/%m/…, arg-count mismatch) keeps the AST path.
                    "$sformatf" | "$psprintf" => {
                        let mut native: Option<(Vec<FmtSeg>, Vec<RegId>)> = None;
                        if let Some(ExprKind::StringLiteral(fmt)) =
                            args.first().map(|a| &a.kind)
                        {
                            if let Some((segs, nargs)) = Self::parse_format_template(fmt) {
                                if nargs == args.len() - 1 {
                                    let start = self.insns.len();
                                    let start_reg = self.next_reg;
                                    let mut arg_regs: Vec<RegId> =
                                        Vec::with_capacity(nargs);
                                    let mut ok = true;
                                    for a in &args[1..] {
                                        match self.compile_expr(a, 0) {
                                            Some(r) => arg_regs.push(r),
                                            None => {
                                                ok = false;
                                                break;
                                            }
                                        }
                                    }
                                    if ok {
                                        native = Some((segs, arg_regs));
                                    } else {
                                        self.insns.truncate(start);
                                        self.next_reg = start_reg;
                                    }
                                }
                            }
                        }
                        let Some((mut segs, arg_regs)) = native else {
                            // Same escape hatch as the `other` arm: one
                            // expression-level fallback, not a whole-stmt bail.
                            if let Some(r) = self.emit_expr_fallback(
                                expr,
                                ctx_width,
                                "SystemCall_sformatf",
                            ) {
                                return Some(r);
                            }
                            self.bail("SystemCall_sformatf");
                            return None;
                        };
                        let mut ai = 0usize;
                        for seg in segs.iter_mut() {
                            if let FmtSeg::Spec { spec, str_valued, .. } = seg {
                                if *spec == 's' {
                                    *str_valued =
                                        self.expr_is_string_static(&args[1 + ai]);
                                }
                                ai += 1;
                            }
                        }
                        let dst = self.alloc_reg();
                        self.emit(Insn::Format(
                            dst,
                            Box::new(FormatData {
                                segs,
                                args: arg_regs,
                            }),
                        ));
                        Some(dst)
                    }
                    "$signed" => {
                        let r = self.compile_expr(args.first()?, 0)?;
                        self.emit(Insn::SetSigned(r));
                        Some(r)
                    }
                    "$unsigned" => {
                        // §6.24.1: reinterpret as unsigned. This was a NO-OP,
                        // so the operand kept its runtime signed flag and the
                        // context Resize SIGN-extended — `unsigned'(sa)` in a
                        // 32-bit context read fffffff4 instead of 000000f4
                        // (the $display path was already correct).
                        let r = self.compile_expr(args.first()?, 0)?;
                        self.emit(Insn::ClearSigned(r));
                        Some(r)
                    }
                    "$__xz_size_cast" => {
                        // §6.24.1 `N'(x)`: evaluate x in context width N,
                        // then resize. N is a literal (parser lowering).
                        let n = match args.first().map(|a| &a.kind) {
                            Some(ExprKind::Number(NumberLiteral::Integer {
                                value, ..
                            })) => value.parse::<u32>().ok(),
                            _ => None,
                        };
                        let Some(n) = n.filter(|&n| n > 0) else {
                            self.bail("SystemCall_size_cast_width");
                            return None;
                        };
                        let r = self.compile_expr(args.get(1)?, n)?;
                        self.emit(Insn::Resize(r, n));
                        Some(r)
                    }
                    // §20.6.2: `$bits` of a statically-known operand is a
                    // COMPILE-TIME constant. Restricted to the shapes the
                    // tables answer exactly: a name in `cast_widths` (typedef
                    // or enum — the `$bits(ibex_mubi_t)` form ibex's unused-
                    // signal reductions use ~4x per cycle) or a plain SIGNAL
                    // (declared width). Anything else — strings, class
                    // handles, unpacked aggregates — keeps the interpreter.
                    "$bits" => {
                        let w: Option<u32> = args.first().and_then(|a| match &a.kind {
                            ExprKind::Ident(h)
                                if h.root.is_none()
                                    && h.path.len() == 1
                                    && h.path[0].selects.is_empty() =>
                            {
                                let nm = &h.path[0].name.name;
                                self.cast_widths
                                    .and_then(|m| m.get(nm).map(|&(w, _)| w))
                                    .or_else(|| {
                                        self.lookup_signal_id(h)
                                            .map(|id| self.signal_widths[id])
                                    })
                            }
                            _ => None,
                        });
                        if let Some(w) = w.filter(|&w| w > 0) {
                            let r = self.alloc_reg();
                            self.emit(Insn::LoadConst(r, Box::new(Value::from_u64(w as u64, 32))));
                            Some(r)
                        } else {
                            if let Some(r) =
                                self.emit_expr_fallback(expr, ctx_width, "SystemCall_bits")
                            {
                                return Some(r);
                            }
                            self.bail("SystemCall_bits");
                            None
                        }
                    }
                    // §6.24.1 named cast, statically resolvable target. The
                    // cast type is the CONTEXT for its operand, so the operand
                    // compiles at the target width, then Resize + sign mark.
                    // A Call operand keeps the interpreter path (it may return
                    // a collection the runtime packs — see the AST handler),
                    // as does a target that is a runtime signal or a real type.
                    // §6.24.1 type cast (`real'(x)`, `int'(x)`) — mirror
                    // the interpreter: self-determined operand, then convert.
                    // A REAL target converts numerically (`emit_to_real`);
                    // an integral target resizes and takes the type's
                    // signedness (a real operand rounds per §10.7 inside
                    // `Value::resize`). Stream operands and exotic targets
                    // keep the AST path.
                    "$__xz_type_cast" => {
                        let dt = match args.first().map(|a| &a.kind) {
                            Some(ExprKind::TypeLiteral(dt)) => dt.clone(),
                            _ => {
                                self.bail("type_cast_shape");
                                return None;
                            }
                        };
                        let inner = args.get(1)?;
                        fn is_stream(e: &Expression) -> bool {
                            match &e.kind {
                                ExprKind::StreamOp { .. } => true,
                                ExprKind::Paren(i) => is_stream(i),
                                _ => false,
                            }
                        }
                        if is_stream(inner) {
                            if let Some(r) =
                                self.emit_expr_fallback(expr, ctx_width, "type_cast_stream")
                            {
                                return Some(r);
                            }
                            self.bail("type_cast_stream");
                            return None;
                        }
                        let src = self.compile_expr(inner, 0)?;
                        let r = self.alloc_reg();
                        self.emit(Insn::Move(r, src));
                        if crate::compiler::elaborate::is_type_real(&dt) {
                            self.emit_to_real(r);
                            return Some(r);
                        }
                        let w = crate::compiler::elaborate::resolve_type_width(
                            &dt,
                            self.params,
                            None,
                        )
                        .max(1);
                        self.emit(Insn::Resize(r, w));
                        if crate::compiler::elaborate::is_type_signed(&dt) {
                            self.emit(Insn::SetSigned(r));
                        } else {
                            self.emit(Insn::ClearSigned(r));
                        }
                        Some(r)
                    }
                    "$__xz_named_cast" => {
                        let target = args.first().and_then(|a| match &a.kind {
                            ExprKind::Ident(h) => {
                                h.path.last().map(|s| s.name.name.clone())
                            }
                            _ => None,
                        });
                        // `8'(x)` — the size is a literal, no name lookup.
                        let literal_w: Option<u32> = args.first().and_then(|a| {
                            if let ExprKind::Number(n) = &a.kind {
                                self.eval_number_static(n)
                                    .and_then(|v| v.to_u64())
                                    .map(|n| (n as u32).max(1))
                            } else {
                                None
                            }
                        });
                        let inner_is_call =
                            matches!(args.get(1).map(|a| &a.kind), Some(ExprKind::Call { .. }));
                        let known = literal_w.map(|w| (w, false)).or_else(|| target.as_ref().and_then(|nm| {
                            self.cast_widths
                                .and_then(|m| m.get(nm).copied())
                                .or_else(|| {
                                    // Parameter-valued SIZE cast: `N'(x)` with
                                    // N a constant parameter.
                                    self.params
                                        .and_then(|p| p.get(nm))
                                        .and_then(|v| v.to_u64())
                                        .map(|n| ((n as u32).max(1), false))
                                })
                        }));
                        if let (Some((w, signed)), false) = (known, inner_is_call) {
                            // Mirror the interpreter EXACTLY: the operand is
                            // evaluated self-determined, then resized. (§6.24.1
                            // arguably makes the cast type the operand's
                            // context, but the interpreter — and the reference
                            // simulator, per the bit-exact ibex traces — do
                            // not widen the operand's intermediate arithmetic.)
                            let src = self.compile_expr(args.get(1)?, 0)?;
                            // NEVER resize `src` in place: for a bare local
                            // (a loop variable, say) compile_expr hands back
                            // the variable's OWN register, and an in-place
                            // Resize would truncate the variable itself —
                            // `NumBitsDeviceSel'(device)` inside ibex's bus
                            // arbiter loop corrupted `device` for the rest of
                            // the loop exactly this way.
                            let r = self.alloc_reg();
                            self.emit(Insn::Move(r, src));
                            self.emit(Insn::Resize(r, w));
                            if signed {
                                self.emit(Insn::SetSigned(r));
                            } else {
                                self.emit(Insn::ClearSigned(r));
                            }
                            Some(r)
                        } else {
                            if let Some(r) = self.emit_expr_fallback(
                                expr,
                                ctx_width,
                                "SystemCall_named_cast",
                            ) {
                                return Some(r);
                            }
                            self.bail("SystemCall_named_cast");
                            None
                        }
                    }
                    other => {
                        let _ = other;
                        if std::env::var_os("XEZIM_PROBE_SYSCALL").is_some() {
                            eprintln!("[SYSCALL_FALLBACK] {}", name);
                        }
                        if let Some(r) =
                            self.emit_expr_fallback(expr, ctx_width, "SystemCall_other")
                        {
                            return Some(r);
                        }
                        self.bail("SystemCall_other");
                        None
                    }
            },
            ExprKind::MemberAccess { expr: base, member } => {
                let member_start = self.insns.len();
                let member_reg = self.next_reg;
                if let Some(dest) = self.compile_indexed_packed_member(base, &member.name) {
                    return Some(dest);
                }
                self.insns.truncate(member_start);
                self.next_reg = member_reg;
                // `b.descriptors[0].region` parses with the element select on
                // the LAST path segment (`b.descriptors[0]` is one identifier):
                // peel that segment into member + index.
                if let ExprKind::Ident(h) = &base.kind
                    && h.root.is_none()
                    && h.path.len() >= 2
                    && h.path.last().is_some_and(|s| s.selects.len() == 1)
                    && h.path[..h.path.len() - 1].iter().all(|s| s.selects.len() <= 1)
                {
                    let mut root_hier = h.clone();
                    let last = root_hier.path.pop().unwrap();
                    let mname = last.name.name.clone();
                    let index = last.selects[0].clone();
                    if let Some(dest) =
                        self.compile_indexed_folded_member(&root_hier, &mname, &index, &member.name)
                    {
                        return Some(dest);
                    }
                }
                self.insns.truncate(member_start);
                self.next_reg = member_reg;

                // Direct packed member (`container.field`). Nested field paths
                // are already flattened in the layout table.
                let direct_start = self.insns.len();
                let direct_reg = self.next_reg;
                if let Some((root, _, _, fields)) = self.compile_packed_struct_value(base)
                    && let Some((_, off, width)) =
                        fields.iter().find(|(name, _, _)| name == &member.name)
                {
                    let dest = self.alloc_reg();
                    self.emit(Insn::RangeSelectConst(
                        dest,
                        root,
                        *off + *width - 1,
                        *off,
                    ));
                    return Some(dest);
                }
                self.insns.truncate(direct_start);
                self.next_reg = direct_reg;
                if let Some(r) = self.emit_expr_fallback(expr, ctx_width, "Expr_MemberAccess") {
                    return Some(r);
                }
                self.bail("Expr_MemberAccess");
                None
            }
            // §13.4: inline a PURE function call — one whose body is a single
            // assignment to the function name (or a single `return`) over input
            // formals. That is the overwhelmingly common combinational-helper
            // shape in RTL (`lfsr32(s)`, `mix(a,b)`), and leaving it to the AST
            // interpreter dragged the whole enclosing block out of bytecode.
            // §10.9.2 assignment pattern with a KNOWN packed-struct target —
            // the assign arms install the destination's layout around the
            // rvalue compile. Without a layout it stays on the AST path.
            ExprKind::AssignmentPattern(items) => {
                if let Some(layout) = self.pattern_layout.take() {
                    let r = self.compile_packed_struct_pattern(items, &layout);
                    self.pattern_layout = Some(layout);
                    if let Some(r) = r {
                        return Some(r);
                    }
                }
                self.bail("Expr_AssignmentPattern");
                None
            }
            // §11.4.13 set membership, restricted to what makes `==?`
            // degenerate to `==`: every member a compile-time constant with
            // no x/z bits. That is the enum-list shape ibex's decoder and CSR
            // logic use (`csr_op inside {CSR_OP_WRITE, ...}`) — ~2.6 such
            // evaluations per cycle ran interpreted. An x in the OPERAND
            // still propagates exactly per LRM: Eq yields x, and x|1 = 1,
            // x|0 = x. Ranges and wildcard members keep the interpreter.
            ExprKind::Inside { expr: e, ranges } => {
                let mut members: Vec<Value> = Vec::with_capacity(ranges.len());
                let mut ok = true;
                for m in ranges {
                    match self.inside_member_const(m) {
                        Some(v) => members.push(v),
                        None => {
                            ok = false;
                            break;
                        }
                    }
                }
                if !ok || members.is_empty() {
                    if let Some(r) = self.emit_expr_fallback(expr, ctx_width, "Expr_Inside") {
                        return Some(r);
                    }
                    self.bail("Expr_Inside");
                    return None;
                }
                let wmax = members
                    .iter()
                    .map(|v| v.width)
                    .fold(self.lrm_self_width(e), u32::max)
                    .max(1);
                let src = self.compile_expr(e, wmax)?;
                // Fresh register: Resize/ClearSigned must not mutate a shared
                // one (same rule as the named cast above).
                let er = self.alloc_reg();
                self.emit(Insn::Move(er, src));
                self.emit(Insn::Resize(er, wmax));
                self.emit(Insn::ClearSigned(er));
                let mut acc: Option<RegId> = None;
                for v in members {
                    let mut c = v.resize(wmax);
                    c.is_signed = false;
                    let cr = self.alloc_reg();
                    self.emit(Insn::LoadConst(cr, Box::new(c)));
                    let t = self.alloc_reg();
                    self.emit(Insn::Eq(t, er, cr));
                    acc = Some(match acc {
                        None => t,
                        Some(a) => {
                            let o = self.alloc_reg();
                            self.emit(Insn::BitOr(o, a, t));
                            o
                        }
                    });
                }
                acc
            }
            ExprKind::Call { func, args } => {
                if let Some(r) = self.compile_string_method(func, args, expr.span) {
                    return Some(r);
                }
                self.compile_pure_call(func, args, ctx_width)
                    .or_else(|| self.emit_expr_fallback(expr, ctx_width, "Expr_Call_impure"))
            }
            other => {
                let n: &'static str = match other {
                    ExprKind::StringLiteral(_) => "Expr_StringLiteral",
                    ExprKind::Replication { .. } => "Expr_Replication",
                    ExprKind::AssignmentPattern(_) => "Expr_AssignmentPattern",
                    ExprKind::Call { .. } => "Expr_Call",
                    ExprKind::Inside { .. } => "Expr_Inside",
                    ExprKind::MemberAccess { expr, member } => {
                        let _ = expr;
                        let _ = member;
                        "Expr_MemberAccess"
                    }
                    ExprKind::Range(..) => "Expr_Range",
                    ExprKind::NamedArg { .. } => "Expr_NamedArg",
                    _ => "Expr_other",
                };
                // Assignment patterns (and named args inside them) spread
                // member-wise at the STATEMENT level on the AST path;
                // evaluating one here to a packed value changes NBA
                // semantics on unpacked structs. Let the statement bail.
                let pattern_like = matches!(
                    other,
                    ExprKind::AssignmentPattern(_) | ExprKind::NamedArg { .. }
                );
                if !pattern_like {
                    if let Some(r) = self.emit_expr_fallback(expr, ctx_width, n) {
                        return Some(r);
                    }
                }
                self.bail(n);
                None
            }
        }
    }

    /// Resolve `arr[i].m` into the operands an array-range store needs:
    /// (array, index reg, hi reg, lo reg, value resized to the member width).
    /// An indexed base keeps the lvalue a `MemberAccess` node — only the bare
    /// `s.m` form collapses to a dotted `Ident` — so both the NBA and the
    /// blocking arm land here. Shared so the two cannot drift apart, which
    /// this member/container pair has done twice.
    ///
    /// Emits the index and constant loads, so a `None` after that point would
    /// leave dead insns behind; every caller bails the whole block on `None`,
    /// which discards them.
    fn packed_array_member_store(
        &mut self,
        base: &Expression,
        member: &str,
        val_reg: RegId,
    ) -> Option<(Box<ArrayOperand>, RegId, RegId, RegId, RegId)> {
        let ExprKind::Index {
            expr: arr_expr,
            index,
        } = &base.kind
        else {
            return None;
        };
        let ExprKind::Ident(hier) = &arr_expr.kind else {
            return None;
        };
        let (_, fields) = self.packed_struct_layout_for_hier(hier)?;
        let &(_, off, mw) = fields.iter().find(|(m, _, _)| m == member)?;
        if mw == 0 {
            return None;
        }
        let name = self.lookup_array_name(hier)?;
        let idx_reg = self.compile_expr(index, 0)?;
        let resized = self.alloc_reg();
        self.emit(Insn::Move(resized, val_reg));
        self.emit(Insn::Resize(resized, mw));
        let hi_reg = self.alloc_reg();
        self.emit(Insn::LoadConst(
            hi_reg,
            Box::new(Value::from_u64((off + mw - 1) as u64, 32)),
        ));
        let lo_reg = self.alloc_reg();
        self.emit(Insn::LoadConst(
            lo_reg,
            Box::new(Value::from_u64(off as u64, 32)),
        ));
        Some((self.array_operand(name), idx_reg, hi_reg, lo_reg, resized))
    }

    fn compile_nba_target(&mut self, lhs: &Expression, val_reg: RegId, width: u32) -> bool {
        match &lhs.kind {
            ExprKind::Ident(hier) => {
                if let Some(id) = self.lookup_signal_id(hier) {
                    self.emit(Insn::NbaAssign(as_sig_id(id), val_reg, width));
                    true
                } else if let Some((base_id, off, mw)) = self.packed_struct_member_target(hier) {
                    // Packed-struct member NBA (`s.m0 <= …`): mirror of the
                    // blocking arm — splice into `[off + mw - 1 : off]` of the
                    // container. Range NBAs compose onto a pending nba_fast
                    // entry, so several members of one container written in the
                    // same cycle each keep their own slice.
                    let resized = self.alloc_reg();
                    self.emit(Insn::Move(resized, val_reg));
                    self.emit(Insn::Resize(resized, mw));
                    self.emit(Insn::NbaAssignRange(
                        as_sig_id(base_id),
                        off + mw - 1,
                        off,
                        resized,
                    ));
                    true
                } else {
                    self.bail("nba_ident_unresolved");
                    false
                }
            }
            ExprKind::Index { expr, index } => {
                // §7.4.1/§11.5.1: ascending or element-of-collection bases
                // need label mapping the stores below do not emit — AST only.
                if self.sel_base_needs_ast(expr) {
                    self.bail("nba_sel_base_maps");
                    return false;
                }
                if let Some(id) = self.const_multi_dim_array_elem_signal_id(lhs) {
                    self.emit(Insn::NbaAssign(as_sig_id(id), val_reg, width));
                    return true;
                }
                if let ExprKind::Ident(hier) = &expr.kind {
                    if self.is_assoc_target(hier) {
                        self.bail("nba_target_assoc");
                        return false;
                    }
                    if self.collection_store_denied(hier) {
                        self.bail("nba_target_collection");
                        return false;
                    }
                    if let Some(name) = self.lookup_array_name(hier) {
                        if let Some(idx_reg) = self.compile_expr(index, 0) {
                            let array = self.array_operand(name);
                            self.emit(Insn::NbaAssignArray(array, idx_reg, val_reg, width));
                            return true;
                        }
                    }
                    if let Some(id) = self.lookup_signal_id(hier) {
                        // Packed multi-D NBA: `mem[i] <= data` must write the
                        // W-bit slice at `i*W +: W`. Mirrors compile_blocking_target.
                        let elem_w = self.packed_elem_width_of(hier);
                        if let Some(elem_w) = elem_w {
                            if let Some(idx_reg) = self.compile_expr(index, 0) {
                                // Normalize the index to a 0-based, LSB-first
                                // slot using the DECLARED outer range.
                                let dim = self.packed_outer_dim(hier);
                                let idx_reg = self.emit_packed_slot_index(dim, idx_reg);
                                let elem_w_reg = self.alloc_reg();
                                self.emit(Insn::LoadConst(
                                    elem_w_reg,
                                    Box::new(Value::from_u64(elem_w as u64, 32)),
                                ));
                                let lo_reg = self.alloc_reg();
                                self.emit(Insn::Mul(lo_reg, idx_reg, elem_w_reg));
                                let em1_reg = self.alloc_reg();
                                self.emit(Insn::LoadConst(
                                    em1_reg,
                                    Box::new(Value::from_u64((elem_w - 1) as u64, 32)),
                                ));
                                let hi_reg = self.alloc_reg();
                                self.emit(Insn::Add(hi_reg, lo_reg, em1_reg));
                                self.emit(Insn::NbaAssignRangeDyn(as_sig_id(id), hi_reg, lo_reg, val_reg));
                                return true;
                            }
                        }
                        if let Some(idx_reg) = self.compile_expr(index, 0) {
                            // §7.4.1: rebase for non-zero-based vectors.
                            let idx_reg = self.emit_rebased_index(hier, idx_reg);
                            self.emit(Insn::NbaAssignBitDyn(as_sig_id(id), idx_reg, val_reg));
                            return true;
                        }
                    }
                }
                if let Some(id) = self.flattened_outer_const_signal_id(expr) {
                    if let Some(idx_reg) = self.compile_expr(index, 0) {
                        self.emit(Insn::NbaAssignBitDyn(as_sig_id(id), idx_reg, val_reg));
                        return true;
                    }
                }
                // `a[i][j] <= v` on a 2-D unpacked array: same row-major
                // addressing the READ path already uses, then an ordinary
                // array store. Elements are materialized contiguously, so the
                // flat index and its out-of-range guard are shared with
                // `compile_2d_array_read`. Without this the whole statement —
                // and any loop containing it — stayed on the AST path, which
                // measured ~24x slower than the 1-D equivalent.
                if let ExprKind::Index {
                    expr: outer,
                    index: j_expr,
                } = &lhs.kind
                    && let ExprKind::Index {
                        expr: base,
                        index: i_expr,
                    } = &outer.kind
                    && let ExprKind::Ident(hier) = &base.kind
                    && let Some((array, flat)) =
                        self.compile_2d_flat_index(hier, i_expr, j_expr)
                {
                    self.emit(Insn::NbaAssignArray(array, flat, val_reg, width));
                    return true;
                }
                self.bail("nba_index_other");
                false
            }
            ExprKind::RangeSelect {
                expr,
                left,
                right,
                kind,
            } => {
                // §7.4.1/§11.5.1: ascending or element-of-collection bases
                // need label mapping the stores below do not emit — AST only.
                if self.sel_base_needs_ast(expr) {
                    self.bail("nba_range_base_maps");
                    return false;
                }
                if let ExprKind::Ident(hier) = &expr.kind {
                    if let Some(id) = self.lookup_signal_id(hier) {
                        // §7.4.1: rebase declared indices to physical offsets
                        // for a non-zero-based vector (see the blocking arm).
                        let base_lo = self.declared_low_bound(hier);
                        match kind {
                            RangeKind::Constant => {
                                if let (Some(hi), Some(lo)) =
                                    (self.eval_const_expr(left), self.eval_const_expr(right))
                                {
                                    let hi = (hi as i64 - base_lo).max(0) as u32;
                                    let lo = (lo as i64 - base_lo).max(0) as u32;
                                    self.emit(Insn::NbaAssignRange(as_sig_id(id), hi, lo, val_reg));
                                    return true;
                                }
                            }
                            RangeKind::IndexedUp | RangeKind::IndexedDown => {
                                let width = match self.eval_const_expr(right) {
                                    Some(w) if w > 0 => w,
                                    _ => {
                                        self.bail("nba_range_width_nonconst");
                                        return false;
                                    }
                                };
                                let resized = self.alloc_reg();
                                self.emit(Insn::Move(resized, val_reg));
                                self.emit(Insn::Resize(resized, width));
                                let Some(idx) = self.compile_expr(left, 0) else {
                                    self.bail("nba_range_base");
                                    return false;
                                };
                                // §7.4.1: rebase for non-zero-based vectors.
                                let idx = self.emit_rebased_index(hier, idx);
                                let (hi_reg, lo_reg) = if width == 1 {
                                    (idx, idx)
                                } else {
                                    let delta = self.alloc_reg();
                                    self.emit(Insn::LoadConst(
                                        delta,
                                        Box::new(Value::from_u64((width - 1) as u64, 32)),
                                    ));
                                    let other = self.alloc_reg();
                                    if *kind == RangeKind::IndexedUp {
                                        self.emit(Insn::Add(other, idx, delta));
                                        (other, idx)
                                    } else {
                                        self.emit(Insn::Sub(other, idx, delta));
                                        (idx, other)
                                    }
                                };
                                self.emit(Insn::NbaAssignRangeDyn(as_sig_id(id), hi_reg, lo_reg, resized));
                                return true;
                            }
                        }
                    }
                }
                if let Some(id) = self.flattened_outer_const_signal_id(expr) {
                    match kind {
                        RangeKind::Constant => {
                            if let (Some(hi), Some(lo)) =
                                (self.eval_const_expr(left), self.eval_const_expr(right))
                            {
                                self.emit(Insn::NbaAssignRange(as_sig_id(id), hi, lo, val_reg));
                                return true;
                            }
                        }
                        RangeKind::IndexedUp | RangeKind::IndexedDown => {
                            let width = match self.eval_const_expr(right) {
                                Some(w) if w > 0 => w,
                                _ => {
                                    self.bail("nba_range_width_nonconst");
                                    return false;
                                }
                            };
                            let resized = self.alloc_reg();
                            self.emit(Insn::Move(resized, val_reg));
                            self.emit(Insn::Resize(resized, width));
                            let Some(idx) = self.compile_expr(left, 0) else {
                                self.bail("nba_range_base");
                                return false;
                            };
                            let (hi_reg, lo_reg) = if width == 1 {
                                (idx, idx)
                            } else {
                                let delta = self.alloc_reg();
                                self.emit(Insn::LoadConst(
                                    delta,
                                    Box::new(Value::from_u64((width - 1) as u64, 32)),
                                ));
                                let other = self.alloc_reg();
                                if *kind == RangeKind::IndexedUp {
                                    self.emit(Insn::Add(other, idx, delta));
                                    (other, idx)
                                } else {
                                    self.emit(Insn::Sub(other, idx, delta));
                                    (idx, other)
                                }
                            };
                            self.emit(Insn::NbaAssignRangeDyn(as_sig_id(id), hi_reg, lo_reg, resized));
                            return true;
                        }
                    }
                }
                if *kind == RangeKind::Constant {
                    if let Some((id, hi, lo)) = self.flattened_const_range_target(expr, left, right)
                    {
                        self.emit(Insn::NbaAssignRange(as_sig_id(id), hi, lo, val_reg));
                        return true;
                    }
                }
                // Handle mem[i][range] <= val — ALL range kinds. This arm
                // used to pass `left`/`right` straight through as (hi, lo),
                // which is only correct for the constant `[hi:lo]` form: for
                // `[base +: W]` they are (base, WIDTH), so a masked byte-lane
                // RAM write (`mem[addr][i*8 +: 8] <= ...`, the lowRISC
                // prim_ram_2p shape) wrote a 9-bit slice at the WRONG offset
                // and corrupted neighbouring lanes. Convert per §11.5.1
                // before emitting.
                if let ExprKind::Index {
                    expr: arr_expr,
                    index,
                } = &expr.kind
                {
                    if let ExprKind::Ident(hier) = &arr_expr.kind {
                        if let Some(name) = self.lookup_array_name(hier) {
                            if let Some(idx_reg) = self.compile_expr(index, 0) {
                                let regs = match kind {
                                    RangeKind::Constant => {
                                        match (
                                            self.compile_expr(left, 0),
                                            self.compile_expr(right, 0),
                                        ) {
                                            (Some(h), Some(l)) => Some((h, l)),
                                            _ => None,
                                        }
                                    }
                                    RangeKind::IndexedUp | RangeKind::IndexedDown => {
                                        let base = self.compile_expr(left, 0);
                                        let w = self
                                            .fold_const(right)
                                            .and_then(|v| v.to_u64())
                                            .filter(|&w| w > 0);
                                        match (base, w) {
                                            (Some(b), Some(w)) => {
                                                let other = self.alloc_reg();
                                                let delta = Value::from_u64(w - 1, 32);
                                                if matches!(kind, RangeKind::IndexedUp) {
                                                    // hi = base + w - 1, lo = base
                                                    self.emit(Insn::BinOpConst(
                                                        other,
                                                        b,
                                                        Box::new(delta),
                                                        BinOpConstKind::Add,
                                                    ));
                                                    Some((other, b))
                                                } else {
                                                    // hi = base, lo = base - w + 1
                                                    let dreg = self.alloc_reg();
                                                    self.emit(Insn::LoadConst(
                                                        dreg,
                                                        Box::new(delta),
                                                    ));
                                                    self.emit(Insn::Sub(other, b, dreg));
                                                    Some((b, other))
                                                }
                                            }
                                            _ => None,
                                        }
                                    }
                                };
                                if let Some((hi_reg, lo_reg)) = regs {
                                    let array = self.array_operand(name);
                                    self.emit(Insn::NbaAssignArrayRange(
                                        array, idx_reg, hi_reg, lo_reg, val_reg,
                                    ));
                                    return true;
                                }
                            }
                        }
                    }
                }
                self.bail("nba_range_unresolved");
                false
            }
            ExprKind::Concatenation(parts) => {
                // {a, b, c} <= value: split value into per-part bit ranges and NBA each part.
                // Concatenation is MSB-first: parts[0] is the highest bits.
                // The RHS may be narrower than the concat width (e.g. $signed of a
                // 12-bit expression assigned to a 32-bit concat LHS). Widen first
                // so the per-part RangeSelects see properly sign/zero-extended bits.
                if width > 0 {
                    self.emit(Insn::Resize(val_reg, width));
                }
                let mut part_widths = Vec::with_capacity(parts.len());
                for p in parts {
                    let w = self.infer_lhs_width(p);
                    part_widths.push(w);
                }
                let mut bit_offset: u32 = 0;
                for (i, p) in parts.iter().enumerate().rev() {
                    let pw = part_widths[i];
                    let lo_reg = self.alloc_reg();
                    self.emit(Insn::LoadConst(
                        lo_reg,
                        Box::new(Value::from_u64(bit_offset as u64, 32)),
                    ));
                    let hi_val = bit_offset + pw - 1;
                    let hi_reg = self.alloc_reg();
                    self.emit(Insn::LoadConst(
                        hi_reg,
                        Box::new(Value::from_u64(hi_val as u64, 32)),
                    ));
                    let part_reg = self.alloc_reg();
                    self.emit(Insn::RangeSelect(part_reg, val_reg, hi_reg, lo_reg));
                    self.emit(Insn::Resize(part_reg, pw));
                    if !self.compile_nba_target(p, part_reg, pw) {
                        return false;
                    }
                    bit_offset += pw;
                }
                true
            }
            ExprKind::MemberAccess { expr, member } => {
                // `arr[i].m <= v`: splice the member's static bit range into
                // the selected ELEMENT, via the same NbaAssignArrayRange the
                // `arr[i][hi:lo]` form uses.
                if let Some((array, idx_reg, hi_reg, lo_reg, resized)) =
                    self.packed_array_member_store(expr, &member.name, val_reg)
                {
                    self.emit(Insn::NbaAssignArrayRange(
                        array, idx_reg, hi_reg, lo_reg, resized,
                    ));
                    return true;
                }
                self.bail("nba_member_access");
                false
            }
            _ => {
                self.bail("nba_other");
                false
            }
        }
    }

    fn compile_blocking_target(&mut self, lhs: &Expression, val_reg: RegId, width: u32) -> bool {
        // Packed element WRITE on a register-backed local (`y[i] = v` on a
        // `u8_vec16_t y` inside an inlined function): mask-splice with plain
        // ALU insns — y = (y & ~(elem_mask << i*ew)) | ((v & elem_mask) << i*ew).
        // 4-state correct via the per-bit AND/OR plane tables (constant mask
        // bits are known, so untouched bits pass through and X in `v` lands
        // as X). An out-of-range index shifts the mask past the register's
        // width and the write becomes a no-op. Assumes [N-1:0] outer bounds
        // (the packed-of-packed typedef shape); other locals have no elem
        // entry and keep the bail-to-AST behavior.
        // Element WRITE of a register-bound LOCAL ARRAY (`row[n] = v`,
        // either parse shape).
        if !self.local_var_array.is_empty() {
            let (aname, idx): (Option<&str>, Option<&Expression>) = match &lhs.kind {
                ExprKind::Index { expr, index } => match &expr.kind {
                    ExprKind::Ident(h)
                        if h.path.len() == 1 && h.path[0].selects.is_empty() =>
                    {
                        (Some(h.path[0].name.name.as_str()), Some(index))
                    }
                    _ => (None, None),
                },
                ExprKind::Ident(h)
                    if h.path.len() == 1 && h.path[0].selects.len() == 1 =>
                {
                    (Some(h.path[0].name.name.as_str()), Some(&h.path[0].selects[0]))
                }
                _ => (None, None),
            };
            if let (Some(n), Some(ix)) = (aname, idx) {
                if self.local_var_array.contains_key(n) {
                    let n = n.to_string();
                    let ix = ix.clone();
                    return match self.compile_local_array_write(&n, &ix, val_reg) {
                        Some(ok) => ok,
                        None => {
                            self.bail("local_array_write");
                            false
                        }
                    };
                }
            }
        }
        // §7.4.1/§11.5.1: ascending or element-of-collection select bases
        // need label mapping the stores below do not emit — AST only.
        if let ExprKind::Index { expr, .. } | ExprKind::RangeSelect { expr, .. } = &lhs.kind {
            if self.sel_base_needs_ast(expr) {
                self.bail("blocking_sel_base_maps");
                return false;
            }
        }
        if let ExprKind::Index { expr, index } = &lhs.kind {
            if let ExprKind::Ident(h) = &expr.kind {
                let raw = Self::hier_raw_name(h);
                if let Some(&ew) = self.local_var_elem.get(&raw).filter(|&&ew| ew <= 64) {
                    if let Some((yreg, yw)) = self.local_var_reg_of(h) {
                        if yw > 0 {
                            let Some(idx_reg) = self.compile_expr(index, 0) else {
                                self.bail("local_elem_idx");
                                return false;
                            };
                            let ew_reg = self.alloc_reg();
                            self.emit(Insn::LoadConst(
                                ew_reg,
                                Box::new(Value::from_u64(ew as u64, 32)),
                            ));
                            let lo_reg = self.alloc_reg();
                            self.emit(Insn::Mul(lo_reg, idx_reg, ew_reg));
                            self.emit(Insn::Resize(lo_reg, 32));
                            // elem mask at the container's width
                            let mask_reg = self.alloc_reg();
                            let mut mv = if ew >= 64 {
                                Value::from_u64(u64::MAX, 64)
                            } else {
                                Value::from_u64((1u64 << ew) - 1, ew)
                            };
                            mv = mv.resize(yw);
                            self.emit(Insn::LoadConst(mask_reg, Box::new(mv)));
                            let shifted_mask = self.alloc_reg();
                            self.emit(Insn::Shl(shifted_mask, mask_reg, lo_reg));
                            let inv = self.alloc_reg();
                            self.emit(Insn::BitNot(inv, shifted_mask));
                            let cleared = self.alloc_reg();
                            self.emit(Insn::BitAnd(cleared, yreg, inv));
                            // value: mask to elem width, widen, shift into place
                            let vex = self.alloc_reg();
                            self.emit(Insn::Move(vex, val_reg));
                            self.emit(Insn::Resize(vex, ew));
                            self.emit(Insn::Resize(vex, yw));
                            let vsh = self.alloc_reg();
                            self.emit(Insn::Shl(vsh, vex, lo_reg));
                            let merged = self.alloc_reg();
                            self.emit(Insn::BitOr(merged, cleared, vsh));
                            self.emit(Insn::Move(yreg, merged));
                            self.emit(Insn::Resize(yreg, yw));
                            return true;
                        }
                    }
                }
            }
        }
        // Register-bank local array element (constant index only).
        if let ExprKind::Index { expr, index } = &lhs.kind {
            if let ExprKind::Ident(h) = &expr.kind {
                if h.root.is_none() && h.path.len() == 1 && h.path[0].selects.is_empty() {
                    if let Some(&(base, ew, len, lo)) =
                        self.local_array_regs.get(&h.path[0].name.name)
                    {
                        let Some(iv) = self.fold_const(index).and_then(|v| v.to_u64()) else {
                            self.bail("local_array_dyn_index");
                            return false;
                        };
                        let slot = (iv as i64) - lo;
                        if slot < 0 || slot as usize >= len {
                            self.bail("local_array_oob");
                            return false;
                        }
                        let dst = base + slot as RegId;
                        self.emit(Insn::Move(dst, val_reg));
                        self.emit(Insn::Resize(dst, ew));
                        return true;
                    }
                }
            }
        }
        // Assignment to a register-backed block local (the loop variable of an
        // enclosing `for (int i = ...)`).
        if let ExprKind::Ident(hier) = &lhs.kind {
            if let Some((dst, w)) = self.local_var_reg_of(hier) {
                let name = &hier.path[0].name.name;
                if self.process_local_names.contains(name) {
                    self.bail("blocking_process_local");
                    return false;
                }
                self.emit(Insn::Move(dst, val_reg));
                if self.local_var_is_real.contains(name.as_str()) {
                    // §13.3.1: a real local/formal/result converts its RHS
                    // numerically; a Resize would bit-truncate instead.
                    self.emit_to_real(dst);
                } else if w > 0 {
                    self.emit(Insn::Resize(dst, w));
                }
                return true;
            }
        }
        match &lhs.kind {
            // Handle `base.field` for unpacked struct member signals.
            // e.g. `a.field1 = Tsum(...).field1;` where `a.field1` is a separate signal.
            ExprKind::MemberAccess { expr, member } => {
                if let ExprKind::Ident(hier) = &expr.kind {
                    if hier.path.len() == 1 {
                        let base_name = hier.path[0].name.name.as_str();
                        let dotted = format!("{}.{}", base_name, member.name);
                        if let Some(id) = self.lookup_signal_id_by_name(&dotted) {
                            self.emit(Insn::BlockingAssign(as_sig_id(id), val_reg, width));
                            return true;
                        }
                    }
                }
                // `arr[i].m = v` — same operands as the NBA arm.
                if let Some((array, idx_reg, hi_reg, lo_reg, resized)) =
                    self.packed_array_member_store(expr, &member.name, val_reg)
                {
                    self.emit(Insn::BlockingAssignArrayRange(
                        array, idx_reg, hi_reg, lo_reg, resized,
                    ));
                    return true;
                }
                // §13.4.1 `fname.member = …`: a write to a member of the
                // function's own RETURN VARIABLE, which an inlined body holds
                // in a register rather than a signal — so neither the signal
                // splice above nor the array path applies. Mask-splice the
                // member's static bit range into that register:
                //   r = (r & ~(mask << off)) | ((v & mask) << off)
                // The layout is registered per function at compile start
                // ("fn ret:<name>"), since the return variable is not a signal
                // and nothing else records one for it.
                if let ExprKind::Ident(bh) = &expr.kind
                    && let Some((slot, rw)) = self.local_var_reg_of(bh)
                    && let Some(fields) = self.packed_struct_fields.and_then(|m| {
                        m.get(format!("fn ret:{}", Self::hier_raw_name(bh)).as_str())
                    })
                    && let Some(&(_, off, mw)) =
                        fields.iter().find(|(n, _, _)| *n == member.name)
                    && mw > 0
                    && rw > 0
                {
                    let fields_w = rw;
                    // value & mask, widened, shifted into place
                    let vex = self.alloc_reg();
                    self.emit(Insn::Move(vex, val_reg));
                    self.emit(Insn::Resize(vex, mw));
                    self.emit(Insn::Resize(vex, fields_w));
                    let sh = self.alloc_reg();
                    self.emit(Insn::LoadConst(
                        sh,
                        Box::new(Value::from_u64(off as u64, 32)),
                    ));
                    let vsh = self.alloc_reg();
                    self.emit(Insn::Shl(vsh, vex, sh));
                    // clear the destination window
                    let mut mask = Value::from_u64(0, fields_w);
                    for b in 0..mw {
                        mask.set_bit((off + b) as usize, xezim_core::value::LogicBit::One);
                    }
                    let mreg = self.alloc_reg();
                    self.emit(Insn::LoadConst(mreg, Box::new(mask)));
                    let inv = self.alloc_reg();
                    self.emit(Insn::BitNot(inv, mreg));
                    let cleared = self.alloc_reg();
                    self.emit(Insn::BitAnd(cleared, slot, inv));
                    let merged = self.alloc_reg();
                    self.emit(Insn::BitOr(merged, cleared, vsh));
                    self.emit(Insn::Move(slot, merged));
                    self.emit(Insn::Resize(slot, fields_w));
                    return true;
                }
                self.bail("blocking_target_member_access");
                false
            }
            ExprKind::Ident(hier) => {
                if let Some(id) = self.lookup_signal_id(hier) {
                    if self.signal_is_string_name(hier) {
                        self.emit(Insn::BlockingAssignString(as_sig_id(id), val_reg));
                    } else {
                        self.emit(Insn::BlockingAssign(as_sig_id(id), val_reg, width));
                    }
                    true
                } else if let Some((base_id, off, mw)) = self.packed_struct_member_target(hier) {
                    // Packed-struct member write (`s.m0 = …`): splice the value
                    // into `[off + mw - 1 : off]` of the container signal.
                    let resized = self.alloc_reg();
                    self.emit(Insn::Move(resized, val_reg));
                    self.emit(Insn::Resize(resized, mw));
                    self.emit(Insn::BlockingAssignRange(
                        as_sig_id(base_id),
                        off + mw - 1,
                        off,
                        resized,
                    ));
                    true
                } else {
                    self.bail("blocking_target");
                    false
                }
            }
            ExprKind::Index { expr, index } => {
                if let Some(id) = self.const_multi_dim_array_elem_signal_id(lhs) {
                    self.emit(Insn::BlockingAssign(as_sig_id(id), val_reg, width));
                    return true;
                }
                if let ExprKind::Ident(hier) = &expr.kind {
                    if self.is_assoc_target(hier) {
                        self.bail("blocking_target_assoc");
                        return false;
                    }
                    if self.collection_store_denied(hier) {
                        self.bail("blocking_target_collection");
                        return false;
                    }
                    if let Some(name) = self.lookup_array_name(hier) {
                        if let Some(idx_reg) = self.compile_expr(index, 0) {
                            let array = self.array_operand(name);
                            self.emit(Insn::BlockingAssignArray(array, idx_reg, val_reg, width));
                            return true;
                        }
                    }
                    if let Some(id) = self.lookup_signal_id(hier) {
                        // Packed multi-D LHS: `mem_n[i] = data_i` for
                        // `logic [N-1:0][W-1:0] mem_n` must write a W-bit
                        // slice at `i*W +: W`, not a single bit. Emit a
                        // RangeDyn write of `(i*W+W-1):(i*W)` instead.
                        let elem_w = self.packed_elem_width_of(hier);
                        if let Some(elem_w) = elem_w {
                            if let Some(idx_reg) = self.compile_expr(index, 0) {
                                // lo = slot * elem_w, where `slot` normalizes
                                // the index against the DECLARED outer range.
                                let dim = self.packed_outer_dim(hier);
                                let idx_reg = self.emit_packed_slot_index(dim, idx_reg);
                                let elem_w_reg = self.alloc_reg();
                                self.emit(Insn::LoadConst(
                                    elem_w_reg,
                                    Box::new(Value::from_u64(elem_w as u64, 32)),
                                ));
                                let lo_reg = self.alloc_reg();
                                self.emit(Insn::Mul(lo_reg, idx_reg, elem_w_reg));
                                // hi = lo + elem_w - 1
                                let em1_reg = self.alloc_reg();
                                self.emit(Insn::LoadConst(
                                    em1_reg,
                                    Box::new(Value::from_u64((elem_w - 1) as u64, 32)),
                                ));
                                let hi_reg = self.alloc_reg();
                                self.emit(Insn::Add(hi_reg, lo_reg, em1_reg));
                                self.emit(Insn::BlockingAssignRangeDyn(
                                    as_sig_id(id), hi_reg, lo_reg, val_reg,
                                ));
                                return true;
                            }
                        }
                        if let Some(idx_reg) = self.compile_expr(index, 0) {
                            // §7.4.1: rebase a declared bit index to a
                            // physical offset on a non-zero-based vector
                            // (`logic [3:1] w; w[3] = …` writes offset 2).
                            let base_lo = self.declared_low_bound(hier);
                            let idx_reg = if base_lo != 0 {
                                let base_reg = self.alloc_reg();
                                self.emit(Insn::LoadConst(
                                    base_reg,
                                    Box::new(Value::from_u64(base_lo as u64, 32)),
                                ));
                                let adj = self.alloc_reg();
                                self.emit(Insn::Sub(adj, idx_reg, base_reg));
                                adj
                            } else {
                                idx_reg
                            };
                            self.emit(Insn::BlockingAssignBitDyn(as_sig_id(id), idx_reg, val_reg));
                            return true;
                        }
                    }
                }
                if let Some(id) = self.flattened_outer_const_signal_id(expr) {
                    if let Some(idx_reg) = self.compile_expr(index, 0) {
                        self.emit(Insn::BlockingAssignBitDyn(as_sig_id(id), idx_reg, val_reg));
                        return true;
                    }
                }
                self.bail("blocking_target");
                false
            }
            ExprKind::RangeSelect {
                expr,
                left,
                right,
                kind,
            } => {
                if let ExprKind::Ident(hier) = &expr.kind {
                    // §7.4.1: a range select on a multi-D PACKED vector
                    // (`logic [1:16][7:0] s; s[1:8] = …`) selects ELEMENTS,
                    // not bits — bail to the interpreter's element-aware path.
                    if self.packed_elem_width_of(hier).is_some() {
                        self.bail("blocking_range_packed_multid");
                        return false;
                    }
                    if let Some(id) = self.lookup_signal_id(hier) {
                        // §7.4.1: declared indices on a non-zero-based vector
                        // (`logic [3:1] w; assign w[2:1] = …`) must be rebased
                        // to physical offsets — the read path already does
                        // this; the write path never did, so the write landed
                        // one position high and the low bit stayed x.
                        let base_lo = self.declared_low_bound(hier);
                        match kind {
                            RangeKind::Constant => {
                                if let (Some(hi), Some(lo)) =
                                    (self.eval_const_expr(left), self.eval_const_expr(right))
                                {
                                    let hi = (hi as i64 - base_lo).max(0) as u32;
                                    let lo = (lo as i64 - base_lo).max(0) as u32;
                                    let (low, high) = if hi >= lo { (lo, hi) } else { (hi, lo) };
                                    if let Some(range_w) =
                                        high.checked_sub(low).and_then(|w| w.checked_add(1))
                                    {
                                        let resized = self.alloc_reg();
                                        self.emit(Insn::Move(resized, val_reg));
                                        self.emit(Insn::Resize(resized, range_w));
                                        self.emit(Insn::BlockingAssignRange(as_sig_id(id), hi, lo, resized));
                                        return true;
                                    }
                                }
                                if let (Some(hi_reg), Some(lo_reg)) =
                                    (self.compile_expr(left, 0), self.compile_expr(right, 0))
                                {
                                    self.emit(Insn::BlockingAssignRangeDyn(
                                        as_sig_id(id), hi_reg, lo_reg, val_reg,
                                    ));
                                    return true;
                                }
                            }
                            RangeKind::IndexedUp | RangeKind::IndexedDown => {
                                let width = match self.eval_const_expr(right) {
                                    Some(w) if w > 0 => w,
                                    _ => {
                                        self.bail("blocking_range_width_nonconst");
                                        return false;
                                    }
                                };
                                let resized = self.alloc_reg();
                                self.emit(Insn::Move(resized, val_reg));
                                self.emit(Insn::Resize(resized, width));
                                let Some(idx) = self.compile_expr(left, 0) else {
                                    self.bail("blocking_range_base");
                                    return false;
                                };
                                // §7.4.1: rebase for non-zero-based vectors.
                                let idx = self.emit_rebased_index(hier, idx);
                                let (hi_reg, lo_reg) = if width == 1 {
                                    (idx, idx)
                                } else {
                                    let delta = self.alloc_reg();
                                    self.emit(Insn::LoadConst(
                                        delta,
                                        Box::new(Value::from_u64((width - 1) as u64, 32)),
                                    ));
                                    let other = self.alloc_reg();
                                    if *kind == RangeKind::IndexedUp {
                                        self.emit(Insn::Add(other, idx, delta));
                                        (other, idx)
                                    } else {
                                        self.emit(Insn::Sub(other, idx, delta));
                                        (idx, other)
                                    }
                                };
                                self.emit(Insn::BlockingAssignRangeDyn(
                                    as_sig_id(id), hi_reg, lo_reg, resized,
                                ));
                                return true;
                            }
                        }
                    }
                }
                if let Some(id) = self.flattened_outer_const_signal_id(expr) {
                    match kind {
                        RangeKind::Constant => {
                            if let (Some(hi), Some(lo)) =
                                (self.eval_const_expr(left), self.eval_const_expr(right))
                            {
                                let (low, high) = if hi >= lo { (lo, hi) } else { (hi, lo) };
                                if let Some(range_w) =
                                    high.checked_sub(low).and_then(|w| w.checked_add(1))
                                {
                                    let resized = self.alloc_reg();
                                    self.emit(Insn::Move(resized, val_reg));
                                    self.emit(Insn::Resize(resized, range_w));
                                    self.emit(Insn::BlockingAssignRange(as_sig_id(id), hi, lo, resized));
                                    return true;
                                }
                            }
                            if let (Some(hi_reg), Some(lo_reg)) =
                                (self.compile_expr(left, 0), self.compile_expr(right, 0))
                            {
                                self.emit(Insn::BlockingAssignRangeDyn(
                                    as_sig_id(id), hi_reg, lo_reg, val_reg,
                                ));
                                return true;
                            }
                        }
                        RangeKind::IndexedUp | RangeKind::IndexedDown => {
                            let width = match self.eval_const_expr(right) {
                                Some(w) if w > 0 => w,
                                _ => {
                                    self.bail("blocking_range_width_nonconst");
                                    return false;
                                }
                            };
                            let resized = self.alloc_reg();
                            self.emit(Insn::Move(resized, val_reg));
                            self.emit(Insn::Resize(resized, width));
                            let Some(idx) = self.compile_expr(left, 0) else {
                                self.bail("blocking_range_base");
                                return false;
                            };
                            let (hi_reg, lo_reg) = if width == 1 {
                                (idx, idx)
                            } else {
                                let delta = self.alloc_reg();
                                self.emit(Insn::LoadConst(
                                    delta,
                                    Box::new(Value::from_u64((width - 1) as u64, 32)),
                                ));
                                let other = self.alloc_reg();
                                if *kind == RangeKind::IndexedUp {
                                    self.emit(Insn::Add(other, idx, delta));
                                    (other, idx)
                                } else {
                                    self.emit(Insn::Sub(other, idx, delta));
                                    (idx, other)
                                }
                            };
                            self.emit(Insn::BlockingAssignRangeDyn(as_sig_id(id), hi_reg, lo_reg, resized));
                            return true;
                        }
                    }
                }
                if *kind == RangeKind::Constant {
                    if let Some((id, hi, lo)) = self.flattened_const_range_target(expr, left, right)
                    {
                        let range_w = hi - lo + 1;
                        let resized = self.alloc_reg();
                        self.emit(Insn::Move(resized, val_reg));
                        self.emit(Insn::Resize(resized, range_w));
                        self.emit(Insn::BlockingAssignRange(as_sig_id(id), hi, lo, resized));
                        return true;
                    }
                }
                // Handle mem[i][hi:lo] = val
                if let ExprKind::Index {
                    expr: arr_expr,
                    index,
                } = &expr.kind
                {
                    if let ExprKind::Ident(hier) = &arr_expr.kind {
                        if let Some(name) = self.lookup_array_name(hier) {
                            if let Some(idx_reg) = self.compile_expr(index, 0) {
                                // §11.5.1: in `[base +: w]` / `[base -: w]` the
                                // RIGHT operand is a WIDTH, not an index. This
                                // arm emitted it as `lo` regardless of `kind`,
                                // so `arr[n][64 +: 32]` became the 33-bit window
                                // `[64:32]`: the payload landed 32 bits low and
                                // the top bit was clipped. The flat-signal arm
                                // above and the NBA array arm both convert —
                                // only this one did not, which wedged the C910
                                // PLIC's prefix-OR chain (plic_hreg_busif.v
                                // builds `mie_lst_read_tmp[n][32*(m+1)+:32]`
                                // from the previous slice, so every stage read
                                // back x/z and the settle never converged).
                                let regs = if *kind == RangeKind::Constant {
                                    match (
                                        self.compile_expr(left, 0),
                                        self.compile_expr(right, 0),
                                    ) {
                                        (Some(h), Some(l)) => Some((h, l)),
                                        _ => None,
                                    }
                                } else {
                                    match self.eval_const_expr(right) {
                                        Some(width) if width > 0 => {
                                            match self.compile_expr(left, 0) {
                                                Some(base) if width == 1 => Some((base, base)),
                                                Some(base) => {
                                                    let delta = self.alloc_reg();
                                                    self.emit(Insn::LoadConst(
                                                        delta,
                                                        Box::new(Value::from_u64(
                                                            (width - 1) as u64,
                                                            32,
                                                        )),
                                                    ));
                                                    let other = self.alloc_reg();
                                                    if *kind == RangeKind::IndexedUp {
                                                        // lo = base, hi = base + w - 1
                                                        self.emit(Insn::Add(other, base, delta));
                                                        Some((other, base))
                                                    } else {
                                                        // hi = base, lo = base - w + 1
                                                        self.emit(Insn::Sub(other, base, delta));
                                                        Some((base, other))
                                                    }
                                                }
                                                None => None,
                                            }
                                        }
                                        _ => None,
                                    }
                                };
                                if let Some((hi_reg, lo_reg)) = regs {
                                    let array = self.array_operand(name);
                                    self.emit(Insn::BlockingAssignArrayRange(
                                        array, idx_reg, hi_reg, lo_reg, val_reg,
                                    ));
                                    return true;
                                }
                            }
                        }
                    }
                }
                self.bail("blocking_target");
                false
            }
            ExprKind::Concatenation(parts) => {
                let mut part_widths = Vec::with_capacity(parts.len());
                for p in parts {
                    let w = self.infer_lhs_width(p);
                    part_widths.push(w);
                }
                let mut bit_offset: u32 = 0;
                for (i, p) in parts.iter().enumerate().rev() {
                    let pw = part_widths[i];
                    let lo_reg = self.alloc_reg();
                    self.emit(Insn::LoadConst(
                        lo_reg,
                        Box::new(Value::from_u64(bit_offset as u64, 32)),
                    ));
                    let hi_val = bit_offset + pw - 1;
                    let hi_reg = self.alloc_reg();
                    self.emit(Insn::LoadConst(
                        hi_reg,
                        Box::new(Value::from_u64(hi_val as u64, 32)),
                    ));
                    let part_reg = self.alloc_reg();
                    self.emit(Insn::RangeSelect(part_reg, val_reg, hi_reg, lo_reg));
                    self.emit(Insn::Resize(part_reg, pw));
                    if !self.compile_blocking_target(p, part_reg, pw) {
                        return false;
                    }
                    bit_offset += pw;
                }
                true
            }
            _ => {
                self.bail("blocking_target");
                false
            }
        }
    }

    pub fn infer_lhs_width_pub(&self, lhs: &Expression) -> u32 {
        self.infer_lhs_width(lhs)
    }

    fn infer_lhs_width(&self, lhs: &Expression) -> u32 {
        match &lhs.kind {
            ExprKind::Ident(hier) => {
                // Register-backed locals FIRST — they shadow any same-named
                // signal, and they are absent from every signal table, so the
                // fallthrough guessed 32 bits: a 128-bit local accumulator in
                // an inlined task truncated on every assignment.
                if let Some((_, w)) = self.local_var_reg_of(hier) {
                    if w > 0 {
                        return w;
                    }
                    // A width-0 STRING local is deliberate: falling through
                    // to the 32-bit default truncated the text to 4 chars.
                    if self
                        .local_var_is_string
                        .contains(&Self::hier_raw_name(hier))
                    {
                        return 0;
                    }
                }
                // Register-bound LOCAL ARRAY element in Ident-with-select
                // shape (`row[0] = …` inside an inlined body).
                if hier.path.len() == 1 && hier.path[0].selects.len() == 1 {
                    if let Some(ab) = self.local_var_array.get(hier.path[0].name.name.as_str()) {
                        return ab.elem_w;
                    }
                }
                if let Some(id) = self.lookup_signal_id(hier) {
                    self.signal_widths[id]
                } else if let Some((_, _, mw)) = self.packed_struct_member_target(hier) {
                    mw
                } else {
                    let raw = Self::hier_raw_name(hier);
                    self.widths.get(&raw).copied().unwrap_or(32)
                }
            }
            ExprKind::Index { expr, .. } => {
                if let ExprKind::Ident(hier) = &expr.kind {
                    // Register-bound LOCAL ARRAY element (inlined pure body):
                    // its declared element width. Missing this returned the
                    // 1-bit default, whose Resize destroyed the value (a REAL
                    // element resized-to-1 collapsed to 1.0).
                    if hier.path.len() == 1 && hier.path[0].selects.is_empty() {
                        if let Some(ab) = self.local_var_array.get(hier.path[0].name.name.as_str()) {
                            return ab.elem_w;
                        }
                    }
                    // Register-bank local array element: the declared element
                    // width. Missing this returned the 1-bit default and a
                    // bank store truncated every value to its LSB.
                    if hier.path.len() == 1 && hier.path[0].selects.is_empty() {
                        if let Some(&(_, ew, _, _)) =
                            self.local_array_regs.get(&hier.path[0].name.name)
                        {
                            return ew;
                        }
                    }
                    // Register-backed PACKED-of-packed local (`u8_vec16_t y`
                    // in an inlined body): `y[i]` selects an ew-bit element,
                    // not one bit. Falling through compiled the RHS at width
                    // 1 and the splice wrote the value's LSB only.
                    if let Some(&ew) =
                        self.local_var_elem.get(&Self::hier_raw_name(hier))
                    {
                        if ew > 1 {
                            return ew;
                        }
                    }
                    if let Some(name) = self.lookup_array_name(hier) {
                        if let Some((_, _, elem_w)) = self.arrays.get(&name) {
                            return *elem_w;
                        }
                    }
                    let raw = Self::hier_raw_name(hier);
                    if let Some((_, _, elem_w)) = self.arrays.get(&raw) {
                        return *elem_w;
                    }
                    // Packed multi-D vector: element is N bits, not 1.
                    if let Some(elem_w) = self.packed_elem_widths.and_then(|m| {
                        m.get(raw.as_str()).copied().or_else(|| {
                                hier.path
                                    .last()
                                    .and_then(|s| m.get(s.name.name.as_str()).copied())
                            })
                    }) {
                        if elem_w > 1 {
                            return elem_w;
                        }
                    }
                    // An ASSOCIATIVE array's element: its width lives in its
                    // own map (assoc elements have no signal-table entry, so
                    // `arrays` does not carry them).
                    if let Some(elem_w) = self.assoc_elem_widths.and_then(|m| {
                        m.get(raw.as_str()).copied().or_else(|| {
                            hier.path
                                .last()
                                .and_then(|s| m.get(s.name.name.as_str()).copied())
                        })
                    }) {
                        if elem_w > 0 {
                            return elem_w;
                        }
                    }
                    // Not an array — bit-select on a plain packed signal; width = 1.
                    1
                } else {
                    32
            }
            }
            ExprKind::RangeSelect {
                left, right, kind, ..
            } => match kind {
                    RangeKind::IndexedUp | RangeKind::IndexedDown => {
                        self.eval_const_expr(right).unwrap_or(32)
                    }
                    RangeKind::Constant => {
                    if let (Some(l), Some(r)) =
                        (self.eval_const_expr(left), self.eval_const_expr(right))
                    {
                            let (hi, lo) = if l >= r { (l, r) } else { (r, l) };
                        hi.checked_sub(lo)
                            .and_then(|w| w.checked_add(1))
                            .unwrap_or(32)
                    } else {
                        32
                }
            }
            },
            ExprKind::Concatenation(parts) => parts.iter().map(|p| self.infer_lhs_width(p)).sum(),
            _ => 32,
        }
    }

    fn eval_const_expr(&self, e: &Expression) -> Option<u32> {
        match &e.kind {
            ExprKind::Number(n) => self.eval_number_static(n)?.to_u64().map(|v| v as u32),
            ExprKind::Paren(inner) => self.eval_const_expr(inner),
            ExprKind::Ident(hier) => {
                if hier.path.len() == 1 && hier.path[0].selects.is_empty() {
                    if let Some(&v) = self.const_var_binds.get(hier.path[0].name.name.as_str()) {
                        return Some(v as u32);
                    }
                }
                self.lookup_param_value(hier)?.to_u64().map(|u| u as u32)
            }
            // Fold simple parameter arithmetic so slice bounds like
            // `[ENTRY_NUM-1:0]` resolve. Without this, expr_max_width on a
            // sliced range returned 1 (unwrap_or(0)), which then clobbered
            // bit-AND operand widths down to 1 via ctx_width propagation,
            // producing wrong results for `|(a[N-1:0] & b[N-1:0])`-shape
            // expressions. (Bug seen on c910 axi_fifo pop_req.)
            ExprKind::Binary { op, left, right } => {
                // LRM §11.4 operator set, evaluated in u64 (then truncated to
                // u32 for the slice-bound use-case). Logical && / || short-
                // circuit on the LHS to match §11.4.7.
                match op {
                    BinaryOp::LogAnd => {
                        let l = self.eval_const_expr(left)? as u64;
                        if l == 0 {
                            return Some(0);
                        }
                        let r = self.eval_const_expr(right)? as u64;
                        return Some(if r != 0 { 1 } else { 0 });
                    }
                    BinaryOp::LogOr => {
                        let l = self.eval_const_expr(left)? as u64;
                        if l != 0 {
                            return Some(1);
                        }
                        let r = self.eval_const_expr(right)? as u64;
                        return Some(if r != 0 { 1 } else { 0 });
                    }
                    _ => {}
                }
                let l = self.eval_const_expr(left)? as u64;
                let r = self.eval_const_expr(right)? as u64;
                let v: u64 = match op {
                    BinaryOp::Add => l.wrapping_add(r),
                    BinaryOp::Sub => l.wrapping_sub(r),
                    BinaryOp::Mul => l.wrapping_mul(r),
                    BinaryOp::Div => {
                        if r == 0 {
                            return None;
                        } else {
                            l / r
                        }
                    }
                    BinaryOp::Mod => {
                        if r == 0 {
                            return None;
                        } else {
                            l % r
                        }
                    }
                    // LRM §11.4.3 power — silently dropped before this fix.
                    BinaryOp::Power => {
                        let e = u32::try_from(r as i64).ok()?;
                        (l as i64).checked_pow(e)? as u64
                    }
                    BinaryOp::ShiftLeft  | BinaryOp::ArithShiftLeft  => l.checked_shl(r as u32)?,
                    BinaryOp::ShiftRight => l.checked_shr(r as u32)?,
                    BinaryOp::ArithShiftRight => ((l as i64).wrapping_shr(r as u32)) as u64,
                    BinaryOp::BitAnd  => l & r,
                    BinaryOp::BitOr   => l | r,
                    BinaryOp::BitXor  => l ^ r,
                    BinaryOp::BitXnor => !(l ^ r),
                    BinaryOp::Eq | BinaryOp::CaseEq => {
                        if l == r {
                            1
                        } else {
                            0
                        }
                    }
                    BinaryOp::Neq | BinaryOp::CaseNeq => {
                        if l != r {
                            1
                        } else {
                            0
                        }
                    }
                    BinaryOp::Lt => {
                        if (l as i64) < (r as i64) {
                            1
                        } else {
                            0
                        }
                    }
                    BinaryOp::Leq => {
                        if (l as i64) <= (r as i64) {
                            1
                        } else {
                            0
                        }
                    }
                    BinaryOp::Gt => {
                        if (l as i64) > (r as i64) {
                            1
                        } else {
                            0
                        }
                    }
                    BinaryOp::Geq => {
                        if (l as i64) >= (r as i64) {
                            1
                        } else {
                            0
                        }
                    }
                    _ => return None,
                };
                Some(v as u32)
            }
            ExprKind::Unary { op, operand } => {
                let v = self.eval_const_expr(operand)? as u64;
                let r: u64 = match op {
                    UnaryOp::Plus    => v,
                    UnaryOp::Minus   => 0u64.wrapping_sub(v),
                    UnaryOp::BitNot  => !v,
                    UnaryOp::LogNot => {
                        if v == 0 {
                            1
                        } else {
                            0
                        }
                    }
                    // LRM §11.4.9 reductions. The unknown bit-width is OK here
                    // since callers use this for sizing/indexing — `|MASK` only
                    // needs to be 1 if MASK has any set bits.
                    UnaryOp::BitAnd => {
                        if v == u64::MAX {
                            1
                        } else {
                            0
                        }
                    }
                    UnaryOp::BitNand => {
                        if v == u64::MAX {
                            0
                        } else {
                            1
                        }
                    }
                    UnaryOp::BitOr => {
                        if v != 0 {
                            1
                        } else {
                            0
                        }
                    }
                    UnaryOp::BitNor => {
                        if v != 0 {
                            0
                        } else {
                            1
                        }
                    }
                    UnaryOp::BitXor  => (v.count_ones() & 1) as u64,
                    UnaryOp::BitXnor => 1 - ((v.count_ones() & 1) as u64),
                    _ => return None,
                };
                Some(r as u32)
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                let c = self.eval_const_expr(condition)?;
                if c != 0 {
                    self.eval_const_expr(then_expr)
                } else {
                    self.eval_const_expr(else_expr)
                }
            }
            _ => None,
        }
    }

    fn eval_number_static(&self, num: &NumberLiteral) -> Option<Value> {
        match num {
            NumberLiteral::Integer {
                size,
                signed,
                base,
                value,
                cached_val,
            } => {
                // §5.7.1 — see `Value::unsized_literal_width`.
                let w = match size {
                    Some(sz) => *sz,
                    None => Value::unsized_literal_width(
                        value,
                        match base {
                            NumberBase::Binary => 2,
                            NumberBase::Octal => 8,
                            NumberBase::Hex => 16,
                            NumberBase::Decimal => 10,
                        },
                    ),
                };
                // §5.7.1: unsized all-x/all-z literal is a FILL (see
                // `Value::unsized_xz_fill_char`) — replicate to context.
                let xz_fill =
                    size.is_none() && Value::unsized_xz_fill_char(value).is_some();
                if let Some((vb, xz, cw)) = cached_val.get() {
                    if cw == w {
                        let mut v = Value::from_inline(vb, xz, w);
                        v.is_signed = *signed;
                        v.is_fill = xz_fill;
                        return Some(v);
                    }
                }
                let r = match base {
                    NumberBase::Binary => 2,
                    NumberBase::Octal => 8,
                    NumberBase::Hex => 16,
                    NumberBase::Decimal => 10,
                };
                // §5.7 (issue #31): compiled-path literals warn here — once
                // per literal string, deduped with the elaboration/AST sites.
                crate::compiler::elaborate::warn_unsized_decimal_wrap(*size, base, value);
                let mut v = Value::from_str_radix(value, r, w);
                v.is_signed = *signed;
                v.is_fill = xz_fill;
                Some(v)
            }
            // A real literal must keep its fractional value as IEEE-754 bits so
            // the VM's real-aware arithmetic sees a real operand. The old
            // `*f as u64` truncated `4.4`→`4` and `5.5`→`5`, turning a comb/
            // cont-assign `(1.0/4.4)*1000.0` into integer `1/4*1000 = 0` (the
            // PLL clamp-mode `vcofbperiod` went to 0 → a #0 vclk livelock).
            NumberLiteral::Real(f) => Some(Value::from_f64(*f)),
            // A time literal's VALUE depends on the active scope's timescale
            // (§5.8: `30000ps` under 1ns/1ps is 30, not 30000 ns ticks) —
            // context this compiler does not carry. The old hardcoded ×1e9
            // silently mis-scaled every non-ns scope; decline instead so the
            // AST path (which scales per scope) stays authoritative.
            NumberLiteral::Time(_) => None,
            // §5.7.1: unbased-unsized literal — a 1-bit FILL value; the Value
            // binary ops and resize replicate it to the consuming context.
            NumberLiteral::UnbasedUnsized(c) => Some(Value::fill_of(*c)),
        }
    }

    /// Compile a continuous assign: evaluate RHS, write to pre-resolved LHS.
    /// Returns true if compiled successfully.
    pub fn compile_cont_assign(&mut self, rhs: &Expression, dst_id: usize, width: u32) -> bool {
        // Verilog context width = max(LHS width, RHS self-determined width).
        // Using just the LHS width truncates intermediates when operands
        // (e.g. 32-bit parameters) are wider than the target wire — but the
        // RHS width must be the LRM §11.6.1 SELF width, not the carry-aware
        // expr_max_width: the inflated context leaked dropped carries back
        // into shift results (`assign r = (a<<4)>>2` on 8-bit r computed the
        // inner shift at 12 bits and read 0x8c for 0x0c — while the IDENTICAL
        // always_comb, compiled with the plain LHS width, was correct).
        let ctx = width.max(self.lrm_self_width(rhs));
        if let Some(val_reg) = self.compile_expr(rhs, ctx) {
            if self.register_overflow {
                self.bail("bytecode_register_limit");
                return false;
            }
            self.emit(Insn::Resize(val_reg, width));
            self.emit(Insn::BlockingAssign(as_sig_id(dst_id), val_reg, width));
            true
        } else {
            false
        }
    }

    /// Compile a continuous assign with bit-select, part-select, or concat LHS:
    /// `assign d[i] = rhs`, `assign d[hi:lo] = rhs`, `assign {a,b} = rhs`.
    /// Reuses compile_blocking_target which emits BlockingAssignBitDyn /
    /// BlockingAssignRange / concat-split insns — same sub-range semantics
    /// as the interpreted assign_value path, but at bytecode speed.
    /// Yosys gate-level netlists emit hundreds of per-bit assigns that used
    /// to fall through to the interpreter on every settle iteration.
    pub fn compile_cont_assign_lhs(
        &mut self,
        lhs: &Expression,
        rhs: &Expression,
        lhs_width: u32,
    ) -> bool {
        let ctx = lhs_width.max(self.expr_max_width(rhs));
        if let Some(val_reg) = self.compile_expr(rhs, ctx) {
            if self.register_overflow {
                self.bail("bytecode_register_limit");
                return false;
            }
            self.emit(Insn::Resize(val_reg, lhs_width));
            self.compile_blocking_target(lhs, val_reg, lhs_width)
        } else {
            false
        }
    }

    /// LRM §11.6.1 SELF-determined width — max-of-operands with NO carry
    /// headroom (expr_max_width deliberately over-reports so temporaries
    /// never truncate; a shift/divide OPERAND must take the LRM width or the
    /// dropped carry returns: `(a<<4)>>2` at 8 bits read 0x8c for 0x0c).
    fn lrm_self_width(&mut self, e: &Expression) -> u32 {
        match &e.kind {
            ExprKind::Paren(i) => self.lrm_self_width(i),
            ExprKind::Number(NumberLiteral::Integer { size: Some(sz), .. }) => *sz,
            ExprKind::Number(NumberLiteral::Integer { size: None, .. }) => 32,
            ExprKind::Number(NumberLiteral::UnbasedUnsized(_)) => 1,
            ExprKind::Unary { op, operand } => match op {
                UnaryOp::Plus | UnaryOp::Minus | UnaryOp::BitNot => self.lrm_self_width(operand),
                _ => 1,
            },
            ExprKind::Binary { op, left, right } => match op {
                BinaryOp::Add
                | BinaryOp::Sub
                | BinaryOp::Mul
                | BinaryOp::Div
                | BinaryOp::Mod
                | BinaryOp::BitAnd
                | BinaryOp::BitOr
                | BinaryOp::BitXor
                | BinaryOp::BitXnor => self.lrm_self_width(left).max(self.lrm_self_width(right)),
                BinaryOp::ShiftLeft
                | BinaryOp::ShiftRight
                | BinaryOp::ArithShiftLeft
                | BinaryOp::ArithShiftRight
                | BinaryOp::Power => self.lrm_self_width(left),
                _ => 1,
            },
            ExprKind::Conditional { then_expr, else_expr, .. } => {
                self.lrm_self_width(then_expr).max(self.lrm_self_width(else_expr))
            }
            _ => self.expr_max_width(e),
        }
    }

    /// Static signedness of an expression operand (§11.8.1), where knowable.
    /// `Some(false)` is the only answer that changes codegen (it forces a
    /// zero-extending widen); anything uncertain returns `None` and keeps the
    /// historical sign-by-value-flag behavior.
    /// True when compiling `e` provably leaves an UNSIGNED value in its
    /// register at RUNTIME — stronger than §11.8.1's static judgment, which
    /// says what the flag SHOULD be, not what the exec arms produce. The
    /// §11.8.1 sites scrub such an operand with `ClearSigned`; when this
    /// returns true that scrub is a no-op and is elided (16.5% of executed
    /// insns on a comb-dense fabric were these scrubs).
    ///
    /// Proof obligations, per accepted shape:
    /// - bit/part select of a plain signal: `Value::bit_select` /
    ///   `range_select` construct `is_signed: false` on every path (fast,
    ///   slow, zext, Wide→Wide — verified in value.rs).
    /// - plain load of an unsigned signal: `LoadSignal` copies the table
    ///   Value, and every table write path re-stamps `is_signed` from
    ///   `signal_signed` before committing.
    /// Idents that name params (compiled as `LoadConst` of the param value)
    /// or arrays (element values keep their own flag) are REJECTED, as is
    /// everything this fn doesn't recognise — a false `false` only keeps a
    /// redundant scrub.
    fn operand_scrub_is_noop(&self, e: &Expression) -> bool {
        let plain_unsigned_signal = |h: &crate::ast::expr::HierarchicalIdentifier| -> bool {
            if !h.path.iter().all(|s| s.selects.is_empty()) {
                return false;
            }
            let name = h.path.last().map(|s| s.name.name.as_str()).unwrap_or("");
            if self.arrays.contains_key(name)
                || self.multi_dim_arrays.is_some_and(|m| m.contains(name))
                || self.assoc_arrays.is_some_and(|m| m.contains_key(name))
            {
                return false;
            }
            if self.lookup_param_value(h).is_some() {
                return false;
            }
            self.lookup_signal_id(h)
                .is_some_and(|id| !self.signal_signed[id])
        };
        // Root of a select/member chain. A select is compiled as a
        // value-select (unsigned result) UNLESS the chain roots in a
        // CONTAINER name — array / assoc / multi-dim — whose element LOAD
        // copies the stored element's own flag. A non-ident root (concat,
        // binary result, call, …) always selects a plain register value.
        fn chain_root(e: &Expression) -> Option<&crate::ast::expr::HierarchicalIdentifier> {
            match &e.kind {
                ExprKind::Paren(inner) => chain_root(inner),
                ExprKind::Index { expr, .. }
                | ExprKind::RangeSelect { expr, .. }
                | ExprKind::MemberAccess { expr, .. } => chain_root(expr),
                ExprKind::Ident(h) => Some(h),
                _ => None,
            }
        }
        let root_is_container = |e: &Expression| -> bool {
            match chain_root(e) {
                None => false,
                Some(h) => {
                    let name = h.path.last().map(|s| s.name.name.as_str()).unwrap_or("");
                    // Path-segment selects mean the flattened key differs
                    // from `name` — refuse rather than mis-key the check.
                    !h.path.iter().all(|s| s.selects.is_empty())
                        || self.arrays.contains_key(name)
                        || self.multi_dim_arrays.is_some_and(|m| m.contains(name))
                        || self.assoc_arrays.is_some_and(|m| m.contains_key(name))
                        || self
                            .string_signals
                            .is_some_and(|m| m.contains(name))
                }
            }
        };
        match &e.kind {
            ExprKind::Paren(inner) => self.operand_scrub_is_noop(inner),
            ExprKind::Index { expr, .. } | ExprKind::RangeSelect { expr, .. } => {
                // Selects are unsigned on every exec path (bit_select /
                // range_select construct is_signed:false) — regardless of
                // the base's own flag — unless the "select" is really a
                // container-element load.
                !root_is_container(expr)
            }
            ExprKind::Ident(h) => plain_unsigned_signal(h),
            _ => false,
        }
    }

    fn expr_signedness(&mut self, e: &Expression) -> Option<bool> {
        match &e.kind {
            ExprKind::Number(NumberLiteral::Integer { signed, .. }) => Some(*signed),
            ExprKind::Paren(i) => self.expr_signedness(i),
            // §20/§21: an `int`/`integer`-valued system function is SIGNED
            // (`$countones(x) - 8 < 0` is a signed compare); `bit`, `time`
            // and the unsigned-int ones are not; `$past`/`$sampled` carry
            // their operand's signedness.
            ExprKind::SystemCall { name, args }
                if matches!(name.as_str(), "$past" | "$sampled") =>
            {
                args.first().and_then(|a| self.expr_signedness(a))
            }
            ExprKind::SystemCall { name, .. } if system_function_result(name).is_some() => {
                system_function_result(name).map(|(_, signed)| signed)
            }
            ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.is_empty() => {
                let id = self.lookup_signal_id(h)?;
                Some(self.signal_signed[id])
            }
            // Part-selects, concatenations and replications are UNSIGNED
            // regardless of their operands (§11.8.1).
            ExprKind::Index { .. }
            | ExprKind::RangeSelect { .. }
            | ExprKind::Concatenation(_)
            | ExprKind::Replication { .. } => Some(false),
            ExprKind::SystemCall { name, args } => match name.as_str() {
                "$signed" => Some(true),
                "$unsigned" => Some(false),
                // §6.24.1: a SIZE cast preserves the operand's signedness.
                "$__xz_size_cast" => args.get(1).and_then(|a| self.expr_signedness(a)),
                _ => None,
            },
            ExprKind::Unary { op, operand } => match op {
                UnaryOp::Plus | UnaryOp::Minus | UnaryOp::BitNot => self.expr_signedness(operand),
                // Reductions and ! are 1-bit unsigned.
                _ => Some(false),
            },
            ExprKind::Binary { op, left, right } => match op {
                BinaryOp::Add
                | BinaryOp::Sub
                | BinaryOp::Mul
                | BinaryOp::Div
                | BinaryOp::Mod
                | BinaryOp::BitAnd
                | BinaryOp::BitOr
                | BinaryOp::BitXor
                | BinaryOp::BitXnor
                | BinaryOp::Power => {
                    match (self.expr_signedness(left), self.expr_signedness(right)) {
                        (Some(true), Some(true)) => Some(true),
                        (Some(false), _) | (_, Some(false)) => Some(false),
                        _ => None,
                    }
                }
                // Shifts take the LEFT operand's signedness.
                BinaryOp::ShiftLeft
                | BinaryOp::ShiftRight
                | BinaryOp::ArithShiftLeft
                | BinaryOp::ArithShiftRight => self.expr_signedness(left),
                // Comparisons / logical ops are 1-bit unsigned.
                _ => Some(false),
            },
            ExprKind::Conditional { then_expr, else_expr, .. } => {
                match (self.expr_signedness(then_expr), self.expr_signedness(else_expr)) {
                    (Some(true), Some(true)) => Some(true),
                    (Some(false), _) | (_, Some(false)) => Some(false),
                    _ => None,
                }
            }
            _ => None,
        }
    }


    /// Self-determined width of a system function's RESULT (IEEE 1800-2017
    /// §20/§21) — see `system_function_result`; `$signed`/`$unsigned`/
    /// `$past`/`$sampled` carry their argument's width. Real- and
    /// string-valued functions report None (no integral width to contribute
    /// to a context).
    fn system_function_width(&self, name: &str, args: &[Expression]) -> Option<u32> {
        if system_function_carries_arg(name) {
            return args.first().map(|a| self.expr_max_width(a));
        }
        system_function_result(name).map(|(w, _)| w)
    }

    fn expr_max_width(&self, expr: &Expression) -> u32 {
        match &expr.kind {
            ExprKind::Ident(hier) => self
                .lookup_signal_id(hier)
                    .map(|id| self.signal_widths[id])
                // A packed-struct MEMBER read (`req.addr`) is not a signal of
                // its own, so the lookup above misses and the old fallback of
                // 0 made every SELF-DETERMINED use of it 1 bit wide: inside a
                // concatenation, `{(req.addr >> 5), lsbs}` resized the shift's
                // operand to 1 bit and the whole term evaluated to 0.
                .or_else(|| self.packed_struct_member_target(hier).map(|(_, _, mw)| mw))
                // A BLOCK-LOCAL declaration — a `for (int i = ...)` header
                // variable or a `begin`-block temp — is a REGISTER, not a
                // signal, so both lookups above miss. The old fallback of 0
                // then made every self-determined use of it 1 bit wide. In an
                // array INDEX (compiled at ctx_width 0) that collapsed a
                // shift's operand width to `max(0, 0).max(1)` = 1, so
                // `arr[(i << 6) + off]` truncated `i` to one bit and the shift
                // masked the result away: every iteration read `arr[off]`.
                // `local_var_regs` already carries the declared width.
                .or_else(|| self.local_var_reg_of(hier).map(|(_, w)| w))
                .unwrap_or(0),
            ExprKind::Number(n) => self.eval_number_static(n).map(|v| v.width).unwrap_or(32),
            ExprKind::Binary { op, left, right } => {
                // Relational, equality, and logical operators always
                // produce a 1-bit result regardless of operand width.
                // Returning operand width here pollutes the ctx_width
                // passed into a sibling bitwise operand of `&&`/`||`,
                // causing it to be resized up and XNOR-then-NOT to
                // produce ~0 in the upper bits — manifests as
                // `(a ^~ b) && (c < d)` returning 1 instead of 0 when
                // a^~b should be 0. (c910 BJU branch_blt_taken bug.)
                if matches!(
                    op,
                    BinaryOp::Eq
                        | BinaryOp::Neq
                        | BinaryOp::CaseEq
                        | BinaryOp::CaseNeq
                        | BinaryOp::WildcardEq
                        | BinaryOp::WildcardNeq
                        | BinaryOp::Lt
                        | BinaryOp::Leq
                        | BinaryOp::Gt
                        | BinaryOp::Geq
                        | BinaryOp::LogAnd
                        | BinaryOp::LogOr
                        | BinaryOp::LogImplies
                        | BinaryOp::LogEquiv
                ) {
                    1
                } else {
                    self.expr_max_width(left).max(self.expr_max_width(right))
                }
            }
            ExprKind::Unary { op, operand } => {
                // Self-determined unary: reductions and logical NOT all
                // produce 1 bit regardless of operand width.
                if matches!(
                    op,
                    UnaryOp::BitAnd
                        | UnaryOp::BitNand
                        | UnaryOp::BitOr
                        | UnaryOp::BitNor
                        | UnaryOp::BitXor
                        | UnaryOp::BitXnor
                        | UnaryOp::LogNot
                ) {
                    1
                } else {
                    self.expr_max_width(operand)
                }
            }
            ExprKind::Paren(inner) => self.expr_max_width(inner),
            ExprKind::Call { args, .. } => args
                .iter()
                .map(|a| self.expr_max_width(a))
                .max()
                .unwrap_or(0),
            // A system FUNCTION has the result type the LRM gives it, not the
            // width of its arguments — and certainly not 0, which the old
            // catch-all returned: `out_step <= $countones(be) >> 3` with a
            // 4-bit target then compiled the shift at the target's width and
            // truncated a count of 24 to 8 before shifting.
            ExprKind::SystemCall { name, args } => {
                self.system_function_width(name, args).unwrap_or(0)
            }
            ExprKind::Conditional {
                then_expr,
                else_expr,
                ..
            } => {
                // Verilog: result of `cond ? then : else` is max(then, else).
                // Condition is self-determined (does NOT contribute to result width).
                self.expr_max_width(then_expr)
                    .max(self.expr_max_width(else_expr))
            }
            ExprKind::Concatenation(parts) => parts.iter().map(|p| self.expr_max_width(p)).sum(),
            ExprKind::RangeSelect {
                expr: base,
                left,
                right,
                kind,
                ..
            } => {
                match kind {
                    RangeKind::Constant => {
                        if let (Some(l), Some(r)) =
                            (self.eval_const_expr(left), self.eval_const_expr(right))
                        {
                            ((l as i64 - r as i64).unsigned_abs() as u32) + 1
                        } else {
                            // Fallback when bounds aren't const-evaluable:
                            // use the base signal's full width. Returning a
                            // tiny value here (the old `unwrap_or(0)` path)
                            // truncated bit-AND operands via ctx_width.
                            self.expr_max_width(base)
                        }
                    }
                    RangeKind::IndexedUp | RangeKind::IndexedDown => self
                        .eval_const_expr(right)
                        .unwrap_or_else(|| self.expr_max_width(base)),
                }
            }
            // A bit-select of a plain vector is 1 bit — but an ELEMENT select
            // of a packed multi-dimensional array is the whole element
            // (`s[1]` on `logic [1:0][11:0] s` is 12 bits). Reporting 1 here
            // under-sized the shift/divide context above, whose whole job is
            // `ctx_width.max(expr_max_width(left))`: the left operand was then
            // compiled at the ASSIGNMENT's width and the source's high bits
            // were truncated BEFORE the shift, so
            // `d[2] = s[1] >> N` (d's element narrower than s's) lost N extra
            // high bits. The procedural interpreter got this right, so the bug
            // only showed inside `always_comb`/compiled blocks.
            ExprKind::Index { expr: base, .. } => match &base.kind {
                // An index select is ONE BIT only when the base is a plain
                // vector. A packed-array element has its element width — and
                // so does an UNPACKED-array element, which this arm used to
                // miss: `(addr_i[0] & mask[0]) == base[0]` sized the compare
                // operands at max(1,1), truncated both sides of the `&` to a
                // single bit, and ibex's bus decoder never matched an address
                // once its enclosing block compiled. The sibling of the
                // `expr_max_width returned 1 for every index select` bug fixed
                // in the simulator earlier — same disease, other table.
                ExprKind::Ident(hier) => self
                    .packed_elem_width_of(hier)
                    .or_else(|| {
                        self.lookup_array_name(hier)
                            .and_then(|n| self.arrays.get(&n).map(|&(_, _, w)| w))
                    })
                    .or_else(|| {
                        self.lookup_array_name(hier).and_then(|n| {
                            self.assoc_elem_widths.and_then(|m| m.get(&n).copied())
                        })
                    })
                    .unwrap_or(1),
                _ => 1,
            },
            ExprKind::Replication { count, exprs } => {
                let n = self.eval_const_expr(count).unwrap_or(0);
                let inner: u32 = exprs.iter().map(|e| self.expr_max_width(e)).sum();
                n * inner
            }
            // `base.member` in MemberAccess spelling — the same packed-struct
            // member width as the dotted-Ident form above.
            ExprKind::MemberAccess { expr: base, member } => {
                let ExprKind::Ident(bh) = &base.kind else { return 0 };
                let mut h = bh.clone();
                h.path.push(crate::ast::expr::HierPathSegment {
                    name: member.clone(),
                    selects: Vec::new(),
                });
                self.packed_struct_member_target(&h)
                    .map(|(_, _, mw)| mw)
                    .unwrap_or(0)
            }
            _ => 0,
        }
    }

    /// Compile a standalone expression and return the register containing its
    /// result. Used by scheduler fast paths that repeatedly evaluate the same
    /// delay expression outside an always-block body.
    pub fn compile_root_expr(&mut self, expr: &Expression) -> Option<RegId> {
        let result = self.compile_expr(expr, 0);
        if self.register_overflow {
            self.bail("bytecode_register_limit");
            None
        } else {
            result
        }
    }

    pub fn finish(mut self) -> CompiledBlock {
        debug_assert!(!self.register_overflow);
        Self::fuse_load_selects(&mut self.insns);
        // After fusion (so the fused `LoadSignalRange`/`LoadSignalBit` count as
        // width-defining) and before `compact_nops` (which removes the `Nop`s
        // this pass leaves behind).
        let signal_widths = self.signal_widths;
        let num_regs = (self.next_reg as usize).min(u16::MAX as usize + 1);
        Self::elide_redundant_resizes(&mut self.insns, signal_widths, self.signal_real, num_regs);
        Self::fold_fill_const_resize(&mut self.insns);
        Self::propagate_copies(&mut self.insns);
        // AFTER resize elision: an about-to-be-deleted `Resize` sitting between
        // the array read and the NBA would otherwise hide the triple. Still
        // before `compact_nops`, which removes the `Nop`s it leaves behind.
        Self::fuse_array_read_nba(&mut self.insns);
        // Also after `elide_redundant_resizes`: a `Resize` that pass deletes
        // is what most often separates a `LoadConst` from its consumer. Its
        // pattern is disjoint from `fuse_array_read_nba`'s, so the order
        // between the two does not matter.
        Self::fuse_binop_const(&mut self.insns);
        // After fuse_binop_const, before fuse_cmp_branch_move_resize: its
        // Move;Resize pattern is disjoint from the Move;Assign one here.
        Self::forward_move_into_assign(&mut self.insns);
        Self::fuse_cmp_branch_move_resize(&mut self.insns);
        // Last, so it sees the final insn stream (fusions above both create
        // and absorb sign scrubs).
        Self::elide_provably_unsigned_scrubs(&mut self.insns, self.signal_signed);
        Self::fuse_addc2(&mut self.insns);
        Self::compact_nops(&mut self.insns);
        // Trim unused capacity. `Vec::push` doubles the backing buffer
        // when it overflows, so a freshly compiled block can sit on
        // up to ~50% slack capacity. With ~100K CompiledBlocks on
        // c910, that slack stacks into double-digit MB; one
        // `shrink_to_fit` per finish reclaims it.
        self.insns.shrink_to_fit();
        let has_fallback = self
            .insns
            .iter()
            .any(|i| matches!(i, Insn::StmtFallback(..)));
        // Any signal written nonblockingly twice — counting the partial forms,
        // since `v[3:0] <= ..; v <= ..` is the same hazard as two whole writes.
        let mut nba_targets: Vec<u32> = Vec::new();
        for i in &self.insns {
            let id = match i {
                Insn::NbaAssign(id, _, _)
                | Insn::NbaAssignConst(id, _, _)
                | Insn::NbaAssignRange(id, _, _, _)
                | Insn::NbaAssignRangeDyn(id, _, _, _)
                | Insn::NbaAssignArrayRead(id, _, _, _)
                | Insn::NbaAssignBitDyn(id, _, _) => Some(*id as u32),
                _ => None,
            };
            if let Some(id) = id {
                nba_targets.push(id);
            }
        }
        nba_targets.sort_unstable();
        // Array-element NBAs resolve their target id at RUN time (dynamic
        // index), so two of them — or one plus a scalar NBA that a constant
        // index folded to an element id — can collide without appearing in
        // `nba_targets`. Any array NBA alongside another NBA marks the block
        // conservatively; the scan it enables is a short rposition over the
        // block's own queue, and single-NBA blocks (the overwhelming
        // majority) still take the plain push path.
        let array_nbas = self
            .insns
            .iter()
            .filter(|i| {
                matches!(i, Insn::NbaAssignArray(..) | Insn::NbaAssignArrayRange(..))
            })
            .count();
        let total_nbas = nba_targets.len() + array_nbas;
        let nba_dup_targets = nba_targets.windows(2).any(|w| w[0] == w[1])
            || (array_nbas >= 1 && total_nbas >= 2);
        CompiledBlock {
            num_regs: self.next_reg,
            instructions: self.insns,
            has_fallback,
            nba_dup_targets,
        }
    }

    /// Does `insn` read register `r`? Conservative: unknown/AST-fallback
    /// instructions report `true`. Used by the fuse peephole's liveness
    /// check — a wrong `false` here would fuse away a load whose register
    /// is still consumed later, so every variant must be enumerated.
    /// (pub(crate): the FSM native generator in `aot.rs` uses this to prove
    /// a Real tick literal feeds only a delay wait.)
    pub(crate) fn insn_reads_reg(insn: &Insn, r: RegId) -> bool {
        match insn {
            Insn::WaitDelayReg(d) => *d == r,
            Insn::WaitEdge(..) => false,
            Insn::CmpBranch(_, l, rr, _, _) => *l == r || *rr == r,
            Insn::MoveResize(_, s, _) => *s == r,
            Insn::CaseLut(_, src, _) => *src == r,
            Insn::CaseJump(src, _) => *src == r,
            Insn::CaseMaskJump(src, _) => *src == r,
            Insn::Format(_, f) => f.args.contains(&r),
            Insn::StrOp(_, _, args) => args.contains(&r),
            Insn::BlockingAssignString(_, v) => *v == r,
            Insn::LoadConst(..)
            | Insn::LoadSignal(..)
            | Insn::LoadSignalSigned(..)
            | Insn::LoadProcessLocal(..)
            | Insn::LoadSignalRange(..)
            | Insn::LoadSignalBit(..)
            | Insn::NbaAssignConst(..)
            | Insn::BranchIfSignalFalse(..)
            // Reads its index straight out of the signal table; no registers.
            | Insn::NbaAssignArrayRead(..)
            | Insn::Jump(..)
            | Insn::Nop => false,
            Insn::BranchUnlessZero(c, _) => *c == r,
            // In-place mutators read their register.
            Insn::Resize(a, _) | Insn::SetSigned(a) | Insn::ClearSigned(a) => *a == r,
            Insn::Pow(_, l, rr)
            | Insn::Add(_, l, rr)
            | Insn::Sub(_, l, rr)
            | Insn::Mul(_, l, rr)
            | Insn::Div(_, l, rr)
            | Insn::Mod(_, l, rr)
            | Insn::BitAnd(_, l, rr)
            | Insn::BitOr(_, l, rr)
            | Insn::BitXor(_, l, rr)
            | Insn::BitXnor(_, l, rr)
            | Insn::LogAnd(_, l, rr)
            | Insn::LogOr(_, l, rr)
            | Insn::Eq(_, l, rr)
            | Insn::Neq(_, l, rr)
            | Insn::CaseEq(_, l, rr)
            | Insn::CasezEq(_, l, rr)
            | Insn::CasexEq(_, l, rr)
            | Insn::Lt(_, l, rr)
            | Insn::Leq(_, l, rr)
            | Insn::Gt(_, l, rr)
            | Insn::Geq(_, l, rr)
            | Insn::Shl(_, l, rr)
            | Insn::Shr(_, l, rr)
            | Insn::AShr(_, l, rr) => *l == r || *rr == r,
            Insn::BitNot(_, s)
            | Insn::LogNot(_, s)
            | Insn::Negate(_, s)
            | Insn::ReduceAnd(_, s)
            | Insn::ReduceOr(_, s)
            | Insn::ReduceXor(_, s)
            | Insn::Move(_, s)
            | Insn::Replicate(_, s, _) => *s == r,
            // Its other operand is the embedded constant, not a register.
            Insn::BinOpConst(_, s, _, _) => *s == r,
            Insn::BinOpConstAdd2(a) => a.s1 == r || a.s2 == r,
            Insn::BitSelect(_, b, i) => *b == r || *i == r,
            Insn::BitSelectConst(_, b, _) => *b == r,
            Insn::RangeSelect(_, b, l, rr) => *b == r || *l == r || *rr == r,
            Insn::RangeSelectConst(_, b, _, _) => *b == r,
            Insn::Concat(_, parts) => parts.contains(&r),
            Insn::BranchIfFalse(c, _) => *c == r,
            Insn::Select(_, c, t, e) => *c == r || *t == r || *e == r,
            Insn::NbaAssign(_, v, _) | Insn::BlockingAssign(_, v, _) => *v == r,
            Insn::NbaAssignRange(_, _, _, v) | Insn::BlockingAssignRange(_, _, _, v) => *v == r,
            Insn::NbaAssignRangeDyn(_, h, l, v) | Insn::BlockingAssignRangeDyn(_, h, l, v) => {
                *h == r || *l == r || *v == r
            }
            Insn::NbaAssignBitDyn(_, i, v) | Insn::BlockingAssignBitDyn(_, i, v) => {
                *i == r || *v == r
            }
            Insn::LoadArrayElem(_, _, i) => *i == r,
            Insn::NbaAssignArray(_, i, v, _) | Insn::BlockingAssignArray(_, i, v, _) => {
                *i == r || *v == r
            }
            Insn::NbaAssignArrayRange(_, i, h, l, v)
            | Insn::BlockingAssignArrayRange(_, i, h, l, v) => {
                *i == r || *h == r || *l == r || *v == r
            }
            // AST fallback can read anything through the interpreter.
            Insn::StmtFallback(..) => true,
            Insn::EvalExprFallback(..) => true,
        }
    }

    /// Peephole: fuse `LoadSignal(t, s); RangeSelectConst(d, t, l, r)` into
    /// `LoadSignalRange(d, s, l, r)` (and the BitSelectConst analogue) when
    /// the loaded register `t` is dead afterwards. The second slot becomes a
    /// `Nop` so every branch target in the block stays valid. Skipped when a
    /// jump lands ON the select (the fused load would then be bypassed), or
    /// when `t` is read again later — unless the select overwrote `t`
    /// itself (d == t), which destroys the raw value anyway.
    /// `XEZIM_FUSE=0` disables the pass (A/B escape hatch).
    fn fuse_load_selects(insns: &mut [Insn]) {
        use std::sync::OnceLock;
        // Bit-set: 1 = load+range, 2 = load+bit, 4 = const-NBA, 8 = branch
        // fusions. Default = all on; named values select one family for A/B
        // bisection.
        static MODE: OnceLock<u8> = OnceLock::new();
        let mode = *MODE.get_or_init(|| match std::env::var("XEZIM_FUSE").as_deref() {
            Ok("0") => 0,
            Ok("range") => 1,
            Ok("bit") => 2,
            Ok("nba") => 4,
            Ok("branch") => 8,
            _ => 0xF,
        });
        if mode == 0 || insns.len() < 2 {
            return;
        }
        // Branch targets: fusing must not change what a jump lands on.
        let mut is_target = vec![false; insns.len() + 1];
        for insn in insns.iter() {
            match insn {
                Insn::CaseJump(_, cj) => {
                    for &t in cj.table.iter().chain(std::iter::once(&cj.default)) {
                        if (t as usize) < is_target.len() {
                            is_target[t as usize] = true;
                        }
                    }
                }
                Insn::CaseMaskJump(_, mj) => {
                    for &t in mj.table.iter().chain(std::iter::once(&mj.xz_path)) {
                        if (t as usize) < is_target.len() {
                            is_target[t as usize] = true;
                        }
                    }
                }
                Insn::BranchIfFalse(_, t)
                | Insn::BranchUnlessZero(_, t)
                | Insn::BranchIfSignalFalse(_, t, _)
                | Insn::CmpBranch(_, _, _, _, t)
                | Insn::Jump(t)
                    if (*t as usize) < is_target.len() => {
                        is_target[*t as usize] = true;
                    }
                _ => {}
            }
        }
        // Second family: pairs whose fused form has NO destination register —
        // the first insn's register must simply be dead everywhere else.
        //   LoadConst K ; NbaAssign(sig, k, w)        → NbaAssignConst
        //   LogNot(d,s) ; BranchIfFalse(d, T)         → BranchUnlessZero(s, T)
        //   LoadSignal(t,s) ; BranchIfFalse(t, T)     → BranchIfSignalFalse(s, T)
        for i in 0..insns.len() - 1 {
            if is_target[i + 1] {
                continue;
            }
            let (dead_reg, repl) = match (&insns[i], &insns[i + 1]) {
                (Insn::LoadConst(c, k), &Insn::NbaAssign(sig, v, w))
                    if v == *c && (mode & 4) != 0 =>
                {
                    // Pre-resize at fuse time — the exec arm then only
                    // compares + clones-on-change, never resizes.
                    (*c, Insn::NbaAssignConst(sig, Box::new(k.resize_for_assign(w)), w))
                }
                (&Insn::LogNot(d, s), &Insn::BranchIfFalse(c, t))
                    if c == d && (mode & 8) != 0 =>
                {
                    (d, Insn::BranchUnlessZero(s, t))
                }
                (&Insn::LoadSignal(r, sig), &Insn::BranchIfFalse(c, t))
                    if c == r && (mode & 8) != 0 =>
                {
                    (r, Insn::BranchIfSignalFalse(sig, t, u32::MAX))
                }
                _ => continue,
            };
            // The fused form never writes `dead_reg`, so ANY other read of it
            // in the block blocks the fusion (no d==t exemption here).
            let consumed = insns
                .iter()
                .enumerate()
                .any(|(j, x)| j != i && j != i + 1 && Self::insn_reads_reg(x, dead_reg));
            if consumed {
                continue;
            }
            insns[i] = repl;
            insns[i + 1] = Insn::Nop;
        }

        // `if (a && b)` guard:
        //   LoadSignal(r1,s1); LoadSignal(r2,s2); LogAnd(d,r1,r2);
        //   BranchIfFalse(d,T)
        //       → BranchIfSignalFalse(s1,T); BranchIfSignalFalse(s2,T)
        // Equivalent under 4-state semantics: the body is skipped whenever
        // `a && b` is not true, and testing the operands in sequence skips in
        // exactly those cases (an X operand skips either way). Both operands
        // are bare signal loads, so dropping the second test on an early skip
        // loses no side effects.
        if (mode & 8) != 0 && insns.len() >= 4 {
            for i in 0..insns.len() - 3 {
                let (&Insn::LoadSignal(r1, s1), &Insn::LoadSignal(r2, s2)) =
                    (&insns[i], &insns[i + 1])
                else {
                    continue;
                };
                let Insn::LogAnd(d, a, b) = insns[i + 2] else {
                    continue;
                };
                if a != r1 || b != r2 {
                    continue;
                }
                let Insn::BranchIfFalse(cnd, t) = insns[i + 3] else {
                    continue;
                };
                if cnd != d {
                    continue;
                }
                if (i + 1..=i + 3).any(|x| is_target[x]) {
                    continue;
                }
                // r1, r2 and d must be dead outside the quad.
                let consumed = insns.iter().enumerate().any(|(x, ins)| {
                    !(i..=i + 3).contains(&x)
                        && (Self::insn_reads_reg(ins, r1)
                            || Self::insn_reads_reg(ins, r2)
                            || Self::insn_reads_reg(ins, d))
                });
                if consumed {
                    continue;
                }
                insns[i] = Insn::BranchIfSignalFalse(s1, t, u32::MAX);
                insns[i + 1] = Insn::BranchIfSignalFalse(s2, t, u32::MAX);
                insns[i + 2] = Insn::Nop;
                insns[i + 3] = Insn::Nop;
            }
        }

        for i in 0..insns.len() - 1 {
            let &Insn::LoadSignal(t, sig) = &insns[i] else {
                continue;
            };
            if is_target[i + 1] {
                continue;
            }
            let fused = match insns[i + 1] {
                Insn::RangeSelectConst(d, b, l, r) if b == t && (mode & 1) != 0 => {
                    Some((d, Insn::LoadSignalRange(d, sig, l, r)))
                }
                Insn::BitSelectConst(d, b, idx) if b == t && (mode & 2) != 0 => {
                    Some((d, Insn::LoadSignalBit(d, sig, idx)))
                }
                _ => None,
            };
            let Some((d, repl)) = fused else { continue };
            // Liveness: the raw loaded value must not be consumed anywhere
            // else in the block. Registers are allocated fresh per value
            // (alloc_reg never reuses ids within a block), so any read of
            // `t` outside the pair consumes THIS load — scan the whole
            // block (not just later pcs) so backward jumps can't smuggle a
            // read of `t` past a suffix-only check. d == t overwrites the
            // raw value in the same pair, making later reads safe.
            if d != t {
                let consumed = insns
                    .iter()
                    .enumerate()
                    .any(|(j, x)| j != i && j != i + 1 && Self::insn_reads_reg(x, t));
                if consumed {
                    continue;
                }
            }
            insns[i] = repl;
            insns[i + 1] = Insn::Nop;
        }

        // Third family — census-driven. The pass above rewrites
        // `LoadSignal;BitSelectConst` into `LoadSignalBit`, leaving a `Nop`
        // where the second instruction was. That newly-created `LoadSignalBit`
        // very often feeds a branch:
        //
        //   LoadSignalBit(d,sig,i) ; [Nop…] ; BranchIfFalse(d,T)
        //       → BranchIfSignalFalse(sig, T, i)
        //
        // On the C906 memcpy census this is the single most frequent adjacent
        // pair (25.4 M, 4.8% of executed instructions) — it is what `if
        // (vec[i])` lowers to. Fusing removes one dispatch and one 32-byte
        // register write per execution. It must run AFTER that pass, since the
        // pair does not exist in the input stream.
        for i in 0..insns.len() {
            let &Insn::LoadSignalBit(d, sig, idx) = &insns[i] else {
                continue;
            };
            if (mode & 8) == 0 {
                continue;
            }
            // Skip the `Nop` placeholders the previous pass just wrote; they
            // are removed by `compact_nops`, so these two really are adjacent
            // in the stream that executes.
            let mut j = i + 1;
            while j < insns.len() && matches!(insns[j], Insn::Nop) {
                j += 1;
            }
            if j >= insns.len() {
                continue;
            }
            // Control must fall through from i to j: nothing in between (nor j
            // itself) may be a branch target, or the fused form would swallow
            // an entry point.
            if (i + 1..=j).any(|k| is_target[k]) {
                continue;
            }
            let Insn::BranchIfFalse(c, t) = insns[j] else {
                continue;
            };
            if c != d {
                continue;
            }
            // The fused form has no destination register, so ANY other read of
            // `d` in the block blocks the fusion.
            let consumed = insns
                .iter()
                .enumerate()
                .any(|(k, x)| k != i && k != j && Self::insn_reads_reg(x, d));
            if consumed {
                continue;
            }
            insns[i] = Insn::BranchIfSignalFalse(sig, t, idx);
            insns[j] = Insn::Nop;
        }
    }

    /// Peephole: collapse the RTL "memory read feeding a flop" idiom
    ///
    ///   LoadSignal(r1, idx_sig)         ; r1 = the array index, from a signal
    ///   LoadArrayElem(r2, array, r1)    ; r2 = array[r1]
    ///   NbaAssign(dst, r2, w)           ; dst <= r2
    ///       → NbaAssignArrayRead(dst, array, idx_sig, w)
    ///
    /// into one instruction, removing two dispatches and two 32-byte VM
    /// register writes per execution.
    ///
    /// The opcode census only proves ADJACENCY; the operand chain is verified
    /// HERE. `LoadArrayElem`'s index register must be exactly the
    /// `LoadSignal`'s destination, `NbaAssign`'s value register exactly the
    /// `LoadArrayElem`'s destination, and — since the fused form writes no
    /// register at all — neither intermediate may be read anywhere else in the
    /// block. Registers are allocated fresh per value (`alloc_reg` never
    /// reuses an id within a block), so any read of `r1`/`r2` outside the
    /// triple consumes THIS chain; the scan covers the whole block, not just
    /// the suffix, so a backward jump cannot smuggle one past it.
    ///
    /// Runs after `elide_redundant_resizes` (a `Resize` that pass is about to
    /// delete would otherwise hide the triple) and before `compact_nops`,
    /// skipping the `Nop` placeholders earlier fusions left behind — those are
    /// removed before execution, so the three really are consecutive in the
    /// stream that runs. `XEZIM_FUSE_ARRNBA=0` disables the pass (A/B escape
    /// hatch).
    /// §5.7.1: `LoadConst` of a FILL literal (`'0`/`'1`/`'x`) immediately
    /// resized to a concrete width becomes one concrete `LoadConst` —
    /// `Value::resize` on a fill replicates the bit and clears `is_fill`.
    /// Fill constants otherwise keep whole blocks off every fast path (the
    /// two-state lowering and both native backends reject them), and the
    /// ibex ALU/controller blocks all start with exactly this pair.
    /// True when any branch in the block targets an earlier pc. Forward-only
    /// blocks (the common case/if lowering) permit LINEAR liveness reasoning:
    /// a register unread after position i is dead there. With a backward
    /// branch, re-entry can reach earlier reads, so callers must fall back to
    /// whole-block read checks.
    fn has_backward_branch(insns: &[Insn]) -> bool {
        insns.iter().enumerate().any(|(i, insn)| match insn {
            Insn::BranchIfFalse(_, t)
            | Insn::BranchUnlessZero(_, t)
            | Insn::BranchIfSignalFalse(_, t, _)
            | Insn::CmpBranch(_, _, _, _, t)
            | Insn::Jump(t) => (*t as usize) <= i,
            Insn::CaseJump(_, cj) => cj
                .table
                .iter()
                .chain(std::iter::once(&cj.default))
                .any(|&t| (t as usize) <= i),
            Insn::CaseMaskJump(_, mj) => mj
                .table
                .iter()
                .chain(std::iter::once(&mj.xz_path))
                .any(|&t| (t as usize) <= i),
            _ => false,
        })
    }

    /// Producer -> `Move` copy propagation: when a pure register-producing
    /// insn is immediately followed by `Move(d, a)` of its destination and
    /// `a` is never read again, retarget the producer to write `d` directly
    /// and drop the Move. `LoadConst -> Move` alone was 4.9% of all executed
    /// insns on the ibex CoreMark census.
    fn propagate_copies(insns: &mut [Insn]) {
        if insns.len() < 2 {
            return;
        }
        let mut is_target = vec![false; insns.len() + 1];
        for insn in insns.iter() {
            match insn {
                Insn::CaseJump(_, cj) => {
                    for &t in cj.table.iter().chain(std::iter::once(&cj.default)) {
                        if (t as usize) < is_target.len() {
                            is_target[t as usize] = true;
                        }
                    }
                }
                Insn::CaseMaskJump(_, mj) => {
                    for &t in mj.table.iter().chain(std::iter::once(&mj.xz_path)) {
                        if (t as usize) < is_target.len() {
                            is_target[t as usize] = true;
                        }
                    }
                }
                Insn::BranchIfFalse(_, t)
                | Insn::BranchUnlessZero(_, t)
                | Insn::BranchIfSignalFalse(_, t, _)
                | Insn::CmpBranch(_, _, _, _, t)
                | Insn::Jump(t)
                    if (*t as usize) < is_target.len() =>
                {
                    is_target[*t as usize] = true;
                }
                _ => {}
            }
        }
        let fwd_only = !Self::has_backward_branch(insns);
        // The producer's destination register, for the pure single-dest
        // shapes this pass retargets. Anything with side effects, in-place
        // semantics (`Resize`, `SetSigned`, ...) or multiple outputs is not
        // listed and never rewritten.
        fn producer_dest(insn: &mut Insn) -> Option<&mut u16> {
            match insn {
                Insn::LoadConst(d, _)
                | Insn::LoadSignal(d, _)
                | Insn::LoadSignalSigned(d, _)
                | Insn::LoadSignalBit(d, _, _)
                | Insn::LoadSignalRange(d, _, _, _)
                | Insn::LoadArrayElem(d, _, _)
                | Insn::Add(d, _, _)
                | Insn::Sub(d, _, _)
                | Insn::Mul(d, _, _)
                | Insn::BitAnd(d, _, _)
                | Insn::BitOr(d, _, _)
                | Insn::BitXor(d, _, _)
                | Insn::BitXnor(d, _, _)
                | Insn::BitNot(d, _)
                | Insn::Negate(d, _)
                | Insn::LogAnd(d, _, _)
                | Insn::LogOr(d, _, _)
                | Insn::LogNot(d, _)
                | Insn::Eq(d, _, _)
                | Insn::Neq(d, _, _)
                | Insn::CaseEq(d, _, _)
                | Insn::CasezEq(d, _, _)
                | Insn::CasexEq(d, _, _)
                | Insn::Lt(d, _, _)
                | Insn::Leq(d, _, _)
                | Insn::Gt(d, _, _)
                | Insn::Geq(d, _, _)
                | Insn::Shl(d, _, _)
                | Insn::Shr(d, _, _)
                | Insn::AShr(d, _, _)
                | Insn::ReduceAnd(d, _)
                | Insn::ReduceOr(d, _)
                | Insn::ReduceXor(d, _)
                | Insn::Select(d, _, _, _)
                | Insn::Concat(d, _)
                | Insn::Replicate(d, _, _)
                | Insn::BinOpConst(d, _, _, _)
                | Insn::BitSelect(d, _, _)
                | Insn::BitSelectConst(d, _, _)
                | Insn::RangeSelect(d, _, _, _)
                | Insn::RangeSelectConst(d, _, _, _)
                | Insn::CaseLut(d, _, _) => Some(d),
                _ => None,
            }
        }
        for i in 0..insns.len() - 1 {
            let Insn::Move(md, ms) = insns[i + 1] else {
                continue;
            };
            if is_target[i + 1] || md == ms {
                continue;
            }
            // The Move must be `ms`'s ONLY reader anywhere in the block —
            // not just after it: a backward branch can re-enter code before
            // the producer, so a linear "dead after" scan is not a liveness
            // proof. The producer must also not read `md` (its dest write
            // would clobber an operand) nor its own dest `ms` (an in-place
            // accumulate shape would change meaning once retargeted).
            if Self::insn_reads_reg(&insns[i], md) || Self::insn_reads_reg(&insns[i], ms) {
                continue;
            }
            let dead = if fwd_only {
                insns[i + 2..].iter().all(|x| !Self::insn_reads_reg(x, ms))
            } else {
                insns
                    .iter()
                    .enumerate()
                    .all(|(k, x)| k == i + 1 || !Self::insn_reads_reg(x, ms))
            };
            if !dead {
                continue;
            }
            let Some(dref) = producer_dest(&mut insns[i]) else {
                continue;
            };
            if *dref != ms {
                continue;
            }
            *dref = md;
            insns[i + 1] = Insn::Nop;
        }
    }

    fn fold_fill_const_resize(insns: &mut [Insn]) {
        if insns.len() < 2 {
            return;
        }
        let mut is_target = vec![false; insns.len() + 1];
        for insn in insns.iter() {
            match insn {
                Insn::CaseJump(_, cj) => {
                    for &t in cj.table.iter().chain(std::iter::once(&cj.default)) {
                        if (t as usize) < is_target.len() {
                            is_target[t as usize] = true;
                        }
                    }
                }
                Insn::CaseMaskJump(_, mj) => {
                    for &t in mj.table.iter().chain(std::iter::once(&mj.xz_path)) {
                        if (t as usize) < is_target.len() {
                            is_target[t as usize] = true;
                        }
                    }
                }
                Insn::BranchIfFalse(_, t)
                | Insn::BranchUnlessZero(_, t)
                | Insn::BranchIfSignalFalse(_, t, _)
                | Insn::CmpBranch(_, _, _, _, t)
                | Insn::Jump(t)
                    if (*t as usize) < is_target.len() =>
                {
                    is_target[*t as usize] = true;
                }
                _ => {}
            }
        }
        for i in 0..insns.len() - 1 {
            let Insn::LoadConst(d, v) = &insns[i] else {
                continue;
            };
            if !v.is_fill || v.width > 64 {
                continue;
            }
            let d = *d;
            // The consumer must be the direct successor and not a jump
            // target (a branch landing on it would see the unfolded
            // constant), and — for the width-adapting folds — the register
            // must have no OTHER reader, since folding fixes its width.
            if is_target[i + 1] {
                continue;
            }
            let single_use = insns[i + 2..]
                .iter()
                .all(|x| !Self::insn_reads_reg(x, d));
            match &insns[i + 1] {
                Insn::Resize(rd, w) if *rd == d && *w <= 64 => {
                    let folded = v.resize(*w);
                    insns[i] = Insn::LoadConst(d, Box::new(folded));
                    insns[i + 1] = Insn::Nop;
                }
                // Fill flowing straight into a store: the store's width IS
                // the §5.7.1 consuming context.
                Insn::BlockingAssign(_, r, w) | Insn::NbaAssign(_, r, w)
                    if *r == d && *w <= 64 && single_use =>
                {
                    let folded = v.resize(*w);
                    insns[i] = Insn::LoadConst(d, Box::new(folded));
                }
                // Reductions read the fill at its own (1-bit) width; the
                // fold just clears is_fill.
                Insn::ReduceXor(_, r) | Insn::ReduceOr(_, r) | Insn::ReduceAnd(_, r)
                    if *r == d && single_use =>
                {
                    let folded = v.resize(v.width.max(1));
                    insns[i] = Insn::LoadConst(d, Box::new(folded));
                }
                _ => {}
            }
        }
    }

    /// Late pair fusions (census-driven; run last, after every fact-based
    /// pass, so no other pass has to understand the fused forms):
    ///   cmp(d,l,r) ; BranchIfFalse(d,T)   -> CmpBranch(kind,l,r,d,T)
    ///   Move(d,s)  ; Resize(d,w)          -> MoveResize(d,s,w)
    ///   Resize(a,w); Move(d,a) [a dead]   -> MoveResize(d,a,w)
    /// `Lt -> BranchIfFalse` alone was 2.7% and `Resize -> Move` 3.1% of all
    /// executed insns on the ibex CoreMark census.
    fn fuse_cmp_branch_move_resize(insns: &mut [Insn]) {
        use std::sync::OnceLock;
        static ON: OnceLock<bool> = OnceLock::new();
        if !*ON.get_or_init(|| {
            !matches!(std::env::var("XEZIM_FUSE").as_deref(), Ok("0"))
        }) || insns.len() < 2
        {
            return;
        }
        let mut is_target = vec![false; insns.len() + 1];
        for insn in insns.iter() {
            match insn {
                Insn::CaseJump(_, cj) => {
                    for &t in cj.table.iter().chain(std::iter::once(&cj.default)) {
                        if (t as usize) < is_target.len() {
                            is_target[t as usize] = true;
                        }
                    }
                }
                Insn::CaseMaskJump(_, mj) => {
                    for &t in mj.table.iter().chain(std::iter::once(&mj.xz_path)) {
                        if (t as usize) < is_target.len() {
                            is_target[t as usize] = true;
                        }
                    }
                }
                Insn::BranchIfFalse(_, t)
                | Insn::BranchUnlessZero(_, t)
                | Insn::BranchIfSignalFalse(_, t, _)
                | Insn::CmpBranch(_, _, _, _, t)
                | Insn::Jump(t)
                    if (*t as usize) < is_target.len() =>
                {
                    is_target[*t as usize] = true;
                }
                _ => {}
            }
        }
        let fwd_only = !Self::has_backward_branch(insns);
        for i in 0..insns.len() - 1 {
            if is_target[i + 1] {
                continue;
            }
            // cmp ; BranchIfFalse — the compare's dest must have no reader
            // other than the branch (backward jumps make a linear liveness
            // scan unsound, so the whole block is checked).
            if let Insn::BranchIfFalse(c, t) = insns[i + 1] {
                let kind = match &insns[i] {
                    Insn::Eq(d, ..) if *d == c => Some(CmpKind::Eq),
                    Insn::Neq(d, ..) if *d == c => Some(CmpKind::Neq),
                    Insn::CaseEq(d, ..) if *d == c => Some(CmpKind::CaseEq),
                    Insn::Lt(d, ..) if *d == c => Some(CmpKind::Lt),
                    Insn::Leq(d, ..) if *d == c => Some(CmpKind::Leq),
                    Insn::Gt(d, ..) if *d == c => Some(CmpKind::Gt),
                    Insn::Geq(d, ..) if *d == c => Some(CmpKind::Geq),
                    _ => None,
                };
                if let Some(kind) = kind {
                    let only_reader = if fwd_only {
                        insns[i + 2..].iter().all(|x| !Self::insn_reads_reg(x, c))
                    } else {
                        insns
                            .iter()
                            .enumerate()
                            .all(|(j, x)| j == i || j == i + 1 || !Self::insn_reads_reg(x, c))
                    };
                    if only_reader {
                        let (l, r) = match &insns[i] {
                            Insn::Eq(_, l, r)
                            | Insn::Neq(_, l, r)
                            | Insn::CaseEq(_, l, r)
                            | Insn::Lt(_, l, r)
                            | Insn::Leq(_, l, r)
                            | Insn::Gt(_, l, r)
                            | Insn::Geq(_, l, r) => (*l, *r),
                            _ => unreachable!(),
                        };
                        insns[i] = Insn::CmpBranch(kind, l, r, c, t);
                        insns[i + 1] = Insn::Nop;
                        continue;
                    }
                }
            }
            // Move ; Resize of the same dest.
            if let (&Insn::Move(d, sr), &Insn::Resize(rd, w)) = (&insns[i], &insns[i + 1]) {
                if rd == d && d != sr {
                    insns[i] = Insn::MoveResize(d, sr, w);
                    insns[i + 1] = Insn::Nop;
                    continue;
                }
            }
            // Resize ; Move where the resized register dies at the Move:
            // the fused form reads the PRE-resize value and resizes it into
            // the Move's dest — identical result, and `a` stays stale-but-dead.
            if let (&Insn::Resize(a, w), &Insn::Move(d, ms)) = (&insns[i], &insns[i + 1]) {
                if ms == a && d != a {
                    let only_reader = if fwd_only {
                        insns[i + 2..].iter().all(|x| !Self::insn_reads_reg(x, a))
                    } else {
                        insns
                            .iter()
                            .enumerate()
                            .all(|(j, x)| j == i + 1 || !Self::insn_reads_reg(x, a))
                    };
                    if only_reader {
                        insns[i] = Insn::MoveResize(d, a, w);
                        insns[i + 1] = Insn::Nop;
                        continue;
                    }
                }
            }
        }
    }

    fn fuse_array_read_nba(insns: &mut [Insn]) {
        use std::sync::OnceLock;
        static ON: OnceLock<bool> = OnceLock::new();
        if !*ON.get_or_init(|| {
            !matches!(std::env::var("XEZIM_FUSE_ARRNBA").as_deref(), Ok("0"))
        }) || insns.len() < 3
        {
            return;
        }
        // Branch targets: fusing must not change what a jump lands on.
        let mut is_target = vec![false; insns.len() + 1];
        for insn in insns.iter() {
            match insn {
                Insn::CaseJump(_, cj) => {
                    for &t in cj.table.iter().chain(std::iter::once(&cj.default)) {
                        if (t as usize) < is_target.len() {
                            is_target[t as usize] = true;
                        }
                    }
                }
                Insn::CaseMaskJump(_, mj) => {
                    for &t in mj.table.iter().chain(std::iter::once(&mj.xz_path)) {
                        if (t as usize) < is_target.len() {
                            is_target[t as usize] = true;
                        }
                    }
                }
                Insn::BranchIfFalse(_, t)
                | Insn::BranchUnlessZero(_, t)
                | Insn::BranchIfSignalFalse(_, t, _)
                | Insn::CmpBranch(_, _, _, _, t)
                | Insn::Jump(t)
                    if (*t as usize) < is_target.len() => {
                        is_target[*t as usize] = true;
                    }
                _ => {}
            }
        }
        // Index of the next instruction that survives `compact_nops`.
        fn next_real(insns: &[Insn], from: usize) -> Option<usize> {
            let mut j = from;
            while j < insns.len() && matches!(insns[j], Insn::Nop) {
                j += 1;
            }
            (j < insns.len()).then_some(j)
        }
        for i in 0..insns.len() - 2 {
            let &Insn::LoadSignal(r1, idx_sig) = &insns[i] else {
                continue;
            };
            let Some(k) = next_real(insns, i + 1) else {
                continue;
            };
            let Insn::LoadArrayElem(r2, _, elem_idx_reg) = &insns[k] else {
                continue;
            };
            let (r2, elem_idx_reg) = (*r2, *elem_idx_reg);
            if elem_idx_reg != r1 {
                continue;
            }
            let Some(j) = next_real(insns, k + 1) else {
                continue;
            };
            let &Insn::NbaAssign(dst, val_reg, width) = &insns[j] else {
                continue;
            };
            if val_reg != r2 {
                continue;
            }
            // Control must fall through i → k → j: no branch may land on
            // anything from i+1 through j, or the fused form would swallow an
            // entry point.
            if (i + 1..=j).any(|x| is_target[x]) {
                continue;
            }
            let consumed = insns.iter().enumerate().any(|(x, ins)| {
                x != i
                    && x != k
                    && x != j
                    && (Self::insn_reads_reg(ins, r1) || Self::insn_reads_reg(ins, r2))
            });
            if consumed {
                continue;
            }
            // Take the boxed operand out of the `LoadArrayElem` rather than
            // cloning its name `String`.
            let Insn::LoadArrayElem(_, array, _) =
                std::mem::replace(&mut insns[k], Insn::Nop)
            else {
                unreachable!("just matched LoadArrayElem")
            };
            insns[i] = Insn::NbaAssignArrayRead(dst, array, idx_sig, width);
            insns[j] = Insn::Nop;
            FUSED_ARRAY_READ_NBA.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// Peephole: absorb a constant load into the ALU op that consumes it.
    ///
    ///   LoadConst(c, K)          ; c = K
    ///   Add|Eq|CaseEq(d, l, c)   ; d = l <op> c
    ///       → BinOpConst(d, l, K, kind)
    ///
    /// `LoadConst` is the #2 opcode on the C906 memcpy census (49.7 M, 12.0%
    /// of executed bytecode) and 32.5 M of those — 7.9% of the whole stream —
    /// feed exactly these three operators. Each fusion removes one dispatch
    /// and one 32-byte VM register write. It also dissolves the `Add;LoadConst`
    /// pairs of an address-increment chain, whose `LoadConst` half is the same
    /// instruction seen from the other side.
    ///
    /// Only the RIGHT operand is fused, and that costs nothing: the compiler
    /// emits the left operand's code, then the right's, then the operator, so
    /// an IMMEDIATELY PRECEDING `LoadConst` is by construction the right
    /// operand. (A left-hand constant has the right operand's code in between
    /// and so is not an adjacent pair at all.) `l == c` — both operands the
    /// same constant register — is rejected, since the fused form no longer
    /// loads `c` for the left side to read.
    ///
    /// The census only proves ADJACENCY; the operand chain is verified HERE:
    /// the operator's right register must be exactly the `LoadConst`'s
    /// destination, and — since the fused form does not write `c` — `c` must
    /// not be read anywhere else in the block. Registers are allocated fresh
    /// per value (`alloc_reg` never reuses an id within a block), so any read
    /// of `c` outside the pair consumes THIS constant; the scan covers the
    /// whole block, not just the suffix, so a backward jump cannot smuggle one
    /// past it.
    ///
    /// Runs after `elide_redundant_resizes` — a `Resize` that pass is about to
    /// delete would otherwise hide the pair, and a `Resize` it KEEPS must
    /// still block the fusion (only `Nop`s are skipped) because the constant
    /// would then need resizing before use. Before `compact_nops`, which
    /// removes the `Nop`s left behind. `XEZIM_FUSE_CONST=0` disables the pass
    /// (A/B escape hatch).
    fn fuse_binop_const(insns: &mut [Insn]) {
        use std::sync::OnceLock;
        static ON: OnceLock<bool> = OnceLock::new();
        if !*ON.get_or_init(|| {
            !matches!(std::env::var("XEZIM_FUSE_CONST").as_deref(), Ok("0"))
        }) || insns.len() < 2
        {
            return;
        }
        // Branch targets: fusing must not change what a jump lands on.
        let mut is_target = vec![false; insns.len() + 1];
        for insn in insns.iter() {
            match insn {
                Insn::CaseJump(_, cj) => {
                    for &t in cj.table.iter().chain(std::iter::once(&cj.default)) {
                        if (t as usize) < is_target.len() {
                            is_target[t as usize] = true;
                        }
                    }
                }
                Insn::CaseMaskJump(_, mj) => {
                    for &t in mj.table.iter().chain(std::iter::once(&mj.xz_path)) {
                        if (t as usize) < is_target.len() {
                            is_target[t as usize] = true;
                        }
                    }
                }
                Insn::BranchIfFalse(_, t)
                | Insn::BranchUnlessZero(_, t)
                | Insn::BranchIfSignalFalse(_, t, _)
                | Insn::CmpBranch(_, _, _, _, t)
                | Insn::Jump(t)
                    if (*t as usize) < is_target.len() =>
                {
                    is_target[*t as usize] = true;
                }
                _ => {}
            }
        }
        for i in 0..insns.len() - 1 {
            let Insn::LoadConst(c, _) = &insns[i] else {
                continue;
            };
            let c = *c;
            // Index of the next instruction that survives `compact_nops`,
            // additionally stepping over `ClearSigned` sign scrubs: a scrub of
            // the CONSTANT's register is absorbed into the boxed constant at
            // fuse time; a scrub of any other register stays in place and
            // still executes before the fused op (which lands at `j`, not `i`).
            let mut j = i + 1;
            let mut const_scrub: Option<usize> = None;
            loop {
                if j >= insns.len() {
                    break;
                }
                match insns[j] {
                    Insn::Nop => j += 1,
                    Insn::ClearSigned(r) => {
                        if r == c {
                            const_scrub = Some(j);
                        }
                        j += 1;
                    }
                    _ => break,
                }
            }
            if j >= insns.len() {
                continue;
            }
            // NOTE (measured 2026-08-25): commutative `l == c` arms were
            // tried here and are UNREACHABLE — operands compile left-first,
            // so a LEFT constant's LoadConst is separated from the op by the
            // right operand's load and never sits in this pass's window. The
            // 76 M adjacent `LoadConst -> CaseEq` census pairs are therefore
            // const-RIGHT shapes rejected by the width conditions below.
            let (d, l, kind) = match insns[j] {
                Insn::Add(d, l, r) if r == c => (d, l, BinOpConstKind::Add),
                Insn::Eq(d, l, r) if r == c => (d, l, BinOpConstKind::Eq),
                Insn::CaseEq(d, l, r) if r == c => (d, l, BinOpConstKind::CaseEq),
                Insn::BitXor(d, l, r) if r == c => (d, l, BinOpConstKind::Xor),
                _ => continue,
            };
            // `op(d, c, c)`: the left operand is the constant register too,
            // and the fused form no longer materialises it.
            if l == c {
                continue;
            }
            // Control must fall through i → j: nothing from i+1 through j may
            // be a branch target, or the fused form would swallow an entry
            // point.
            if (i + 1..=j).any(|x| is_target[x]) {
                continue;
            }
            let consumed = insns.iter().enumerate().any(|(x, ins)| {
                x != i && x != j && Some(x) != const_scrub && Self::insn_reads_reg(ins, c)
            });
            if consumed {
                continue;
            }
            // Take the boxed constant out of the `LoadConst` rather than
            // cloning a possibly-`Wide` `Value`.
            let Insn::LoadConst(_, mut k) = std::mem::replace(&mut insns[i], Insn::Nop) else {
                unreachable!("just matched LoadConst")
            };
            if let Some(sj) = const_scrub {
                k.is_signed = false;
                insns[sj] = Insn::Nop;
            }
            // The fused op replaces the BINOP's slot so any surviving
            // `ClearSigned` of the left operand still runs first.
            insns[j] = Insn::BinOpConst(d, l, k, kind);
            FUSED_BINOP_CONST[kind as usize].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// Static width inference over the emitted stream: delete every
    /// `Resize(r, w)` whose register is already provably `w` bits wide.
    ///
    /// The 27 `emit(Insn::Resize(..))` sites are unconditional — the compiler
    /// knows the target width but never asks whether the register already has
    /// it. On the C906 memcpy census 99.7% of the 243 M executed `Resize`es
    /// (10.8% of all bytecode, the second most frequent opcode) found
    /// `vr.width == w` and fell straight through: pure dispatch.
    ///
    /// The exec arms are `if vm_regs[r].width != w { .. }`, so an instruction
    /// this pass removes is one that would have done LITERALLY nothing —
    /// provided the width really does match. That makes the whole pass rest on
    /// a single invariant, and nothing else: whenever `rw[r]` holds
    /// `Some((w, _))`, at run time `vm_regs[r].width` is exactly `w`.
    ///
    /// Every rule below is justified from the `Value` method the matching exec
    /// arm calls, on EVERY path through it (X-propagation, `Wide` storage, the
    /// §5.7.1 fill widening and the `is_real` special cases each return their
    /// own freshly-built `Value`, and they do not all agree). Where a method
    /// can return a width other than the obvious one — `add` on a real operand
    /// is 64 bits, not `max`; `range_select` past `MAX_WIDTH` is clamped — the
    /// rule is dropped or guarded rather than approximated. **Unknown means
    /// keep the `Resize`**: a wrongly deleted one leaves a value at the wrong
    /// width, which in a 4-state simulator corrupts results silently.
    ///
    /// `plain` on a tracked width means additionally `!is_real && !is_fill`,
    /// which is what the arithmetic and `Select` rules need (see below).
    ///
    /// Control flow: any index a branch or jump can land on is a merge point
    /// where a register's width depends on which path arrived, so the table is
    /// cleared there. That covers backward jumps too (a loop head is a target),
    /// at the cost of giving up the first iteration's worth of knowledge inside
    /// loops. `XEZIM_RESIZE_ELIDE=0` disables the pass (A/B escape hatch).
    fn elide_redundant_resizes(
        insns: &mut [Insn],
        signal_widths: &[u32],
        signal_real: Option<&[bool]>,
        num_regs: usize,
    ) {
        use std::sync::OnceLock;
        static ENABLED: OnceLock<bool> = OnceLock::new();
        if !*ENABLED.get_or_init(|| {
            !matches!(std::env::var("XEZIM_RESIZE_ELIDE").as_deref(), Ok("0"))
        }) {
            return;
        }
        if insns.is_empty() {
            return;
        }

        /// A width is recorded only inside `1..=MAX_WIDTH`. Below that,
        /// `fill_at` rounds a zero width up to one; above it, `cap_width`
        /// clamps — in both cases the constructed `Value` would not have the
        /// width the rule claims.
        fn ok(width: u32) -> Option<u32> {
            (1..=Value::MAX_WIDTH).contains(&width).then_some(width)
        }
        fn fact(rw: &[Option<(u32, bool)>], r: RegId) -> Option<(u32, bool)> {
            rw.get(r as usize).copied().flatten()
        }
        fn store(rw: &mut [Option<(u32, bool)>], r: RegId, f: Option<(u32, bool)>) {
            if let Some(slot) = rw.get_mut(r as usize) {
                *slot = f;
            }
        }

        let mut is_target = vec![false; insns.len() + 1];
        for insn in insns.iter() {
            match insn {
                Insn::CaseJump(_, cj) => {
                    for &t in cj.table.iter().chain(std::iter::once(&cj.default)) {
                        if (t as usize) < is_target.len() {
                            is_target[t as usize] = true;
                        }
                    }
                }
                Insn::CaseMaskJump(_, mj) => {
                    for &t in mj.table.iter().chain(std::iter::once(&mj.xz_path)) {
                        if (t as usize) < is_target.len() {
                            is_target[t as usize] = true;
                        }
                    }
                }
                Insn::BranchIfFalse(_, t)
                | Insn::BranchUnlessZero(_, t)
                | Insn::BranchIfSignalFalse(_, t, _)
                | Insn::CmpBranch(_, _, _, _, t)
                | Insn::Jump(t)
                    if (*t as usize) < is_target.len() =>
                {
                    is_target[*t as usize] = true;
                }
                _ => {}
            }
        }

        let mut rw: Vec<Option<(u32, bool)>> = vec![None; num_regs];
        for i in 0..insns.len() {
            if is_target[i] {
                rw.iter_mut().for_each(|f| *f = None);
            }

            // Handled before the match because this is the only arm that
            // rewrites the instruction it is looking at.
            if let &Insn::Resize(r, width) = &insns[i] {
                let target = ok(width);
                let prev = fact(&rw, r);
                if target.is_some() && prev.map(|(pw, _)| pw) == target {
                    // The exec arm's `vr.width != w` test is already false:
                    // dead. The register keeps exactly the fact it had.
                    insns[i] = Insn::Nop;
                } else {
                    // `Value::resize` clears `is_fill` on every path, and
                    // clears `is_real` on every path but one: a real source
                    // resized to exactly 64 is returned by `self.clone()`.
                    let plain = width != 64 || prev.is_some_and(|(_, p)| p);
                    store(&mut rw, r, target.map(|t| (t, plain)));
                }
                continue;
            }

            match &insns[i] {
                // Handled above.
                Insn::Resize(..) => {}

                // Result width varies per entry; drop tracking for the dest.
                Insn::CaseLut(d, ..) => store(&mut rw, *d, None),
                // Two dests; widths follow operand widths — drop tracking.
                Insn::BinOpConstAdd2(a) => {
                    store(&mut rw, a.d1, None);
                    store(&mut rw, a.d2, None);
                }
                // Control only; defines nothing.
                Insn::CaseJump(..) | Insn::CaseMaskJump(..) => {}
                // Fused compare+branch: the embedded scratch gets the 1-bit
                // compare result; the branch defines nothing else. (This
                // pass runs BEFORE the late fusion pass, so these arms only
                // matter for re-runs over already-fused streams.)
                Insn::CmpBranch(_, _, _, tmp, _) => store(&mut rw, *tmp, Some((1, true))),
                // `dst = resize(src, w)`: width is known, but a same-width
                // source passes through `clone()` (fill/real preserved), so
                // never claim plain-ness — conservative fact, no bad elision.
                Insn::MoveResize(d, _, w) => {
                    store(&mut rw, *d, ok(*w).map(|t| (t, false)));
                }
                // String result: width is the text length, not static.
                Insn::Format(d, ..) => store(&mut rw, *d, None),
                Insn::StrOp(d, ..) => store(&mut rw, *d, None),
                Insn::BlockingAssignString(..) => {}

                // The exec arms clone the boxed `Value` verbatim.
                Insn::LoadConst(d, v) => {
                    let f = ok(v.width).map(|w| (w, !v.is_real && !v.is_fill));
                    store(&mut rw, *d, f);
                }
                // `signal_table[id].clone()`. `is_real` is a property of the
                // signal's DECLARED type, so it is plain exactly when
                // `signal_real[id]` is false. Without that table (no
                // `set_signal_real`) assume possibly-real, which only forgoes
                // elisions. `is_fill` never reaches a stored signal value:
                // `resize`/`resize_for_assign` clear it on the way in.
                Insn::LoadSignal(d, s) | Insn::LoadSignalSigned(d, s) => {
                    let plain = signal_real
                        .and_then(|sr| sr.get(*s as usize).copied())
                        .map(|is_real| !is_real)
                        .unwrap_or(false);
                    let f = signal_widths
                        .get(*s as usize)
                        .copied()
                        .and_then(ok)
                        .map(|w| (w, plain));
                    store(&mut rw, *d, f);
                }
                Insn::LoadProcessLocal(d, _) => store(&mut rw, *d, None),
                // `Value::bit_select` is 1 bit on every path, including the
                // §11.5.1 out-of-range read.
                Insn::LoadSignalBit(d, _, _)
                | Insn::BitSelect(d, _, _)
                | Insn::BitSelectConst(d, _, _) => store(&mut rw, *d, Some((1, true))),
                // `Value::range_select` is `|l-r|+1` on every path — except
                // `range_select_zext`'s guard against an underflowed index,
                // which returns a bounded all-X value instead; `ok` excludes
                // exactly the widths that can reach it.
                Insn::LoadSignalRange(d, _, l, r) | Insn::RangeSelectConst(d, _, l, r) => {
                    let f = l
                        .abs_diff(*r)
                        .checked_add(1)
                        .and_then(ok)
                        .map(|w| (w, true));
                    store(&mut rw, *d, f);
                }

                // Every comparison and logical/reduction operator returns
                // `from_u64(_, 1)` or `new(1)` — 1 bit on every path.
                Insn::Eq(d, ..)
                | Insn::Neq(d, ..)
                | Insn::CaseEq(d, ..)
                | Insn::CasezEq(d, ..)
                | Insn::CasexEq(d, ..)
                | Insn::Lt(d, ..)
                | Insn::Leq(d, ..)
                | Insn::Gt(d, ..)
                | Insn::Geq(d, ..)
                | Insn::LogAnd(d, ..)
                | Insn::LogOr(d, ..)
                | Insn::LogNot(d, _)
                | Insn::ReduceAnd(d, _)
                | Insn::ReduceOr(d, _)
                | Insn::ReduceXor(d, _) => store(&mut rw, *d, Some((1, true))),

                // `vm_regs[d] = vm_regs[s].clone()` / `copy_from`, both of
                // which copy `width` verbatim.
                Insn::Move(d, s) => {
                    let f = fact(&rw, *s);
                    store(&mut rw, *d, f);
                }
                // `bitwise_not` keeps `self.width` for both storage variants.
                Insn::BitNot(d, s) => {
                    let f = fact(&rw, *s).map(|(w, _)| (w, true));
                    store(&mut rw, *d, f);
                }
                // `negate` on a REAL returns a 64-bit `from_f64`, not
                // `self.width`, so the source must be known non-real.
                Insn::Negate(d, s) => {
                    let f = fact(&rw, *s).filter(|(_, p)| *p);
                    store(&mut rw, *d, f);
                }
                // All three shift helpers return `self.width` on every path,
                // real and fill operands included.
                Insn::Shl(d, l, _) | Insn::Shr(d, l, _) | Insn::AShr(d, l, _) => {
                    let f = fact(&rw, *l).map(|(w, _)| (w, true));
                    store(&mut rw, *d, f);
                }

                // `bitwise_*` take `max(width)` on every path: the fast arm,
                // the `Wide` arm (entered only for two equal declared widths),
                // `bitwise_op_slow`, and the §5.7.1 fill widening, which
                // normalises both operands to `max(w).max(1)` before
                // recursing. A real operand is not special-cased at all.
                Insn::BitAnd(d, l, r)
                | Insn::BitOr(d, l, r)
                | Insn::BitXor(d, l, r)
                | Insn::BitXnor(d, l, r) => {
                    let f = match (fact(&rw, *l), fact(&rw, *r)) {
                        (Some((a, _)), Some((b, _))) => ok(a.max(b)).map(|w| (w, true)),
                        _ => None,
                    };
                    store(&mut rw, *d, f);
                }
                // The arithmetic operators DO special-case a real operand
                // (`from_f64`, always 64 bits regardless of the operands'
                // widths), so both operands must be known non-real.
                Insn::Add(d, l, r)
                | Insn::Sub(d, l, r)
                | Insn::Mul(d, l, r)
                | Insn::Div(d, l, r)
                | Insn::Mod(d, l, r) => {
                    let f = match (fact(&rw, *l), fact(&rw, *r)) {
                        (Some((a, true)), Some((b, true))) => {
                            ok(a.max(b)).map(|w| (w, true))
                        }
                        _ => None,
                    };
                    store(&mut rw, *d, f);
                }

                // Same rules as the unfused pair, with the constant's fact
                // read straight off the boxed `Value` instead of looked up —
                // which is strictly MORE inferable than the register form,
                // since `K` can never be unknown. The `Add` kind reuses the
                // arithmetic rule above verbatim (`max` of the operand widths,
                // both operands required non-real because `Value::add`
                // special-cases a real operand into a 64-bit `from_f64`, and
                // non-fill because §5.7.1 widening renormalises them);
                // `Eq`/`CaseEq` land in the 1-bit comparison rule, since
                // `is_equal`/`case_eq` return `from_u64(_, 1)` on every path.
                Insn::BinOpConst(d, s, k, kind) => {
                    let f = match kind {
                        BinOpConstKind::Eq | BinOpConstKind::CaseEq => Some((1, true)),
                        // `bitwise_xor` has no single-width guarantee worth
                        // proving here — leave the register width unknown.
                        BinOpConstKind::Xor => None,
                        BinOpConstKind::Add => {
                            // Identical to the `Insn::LoadConst` arm's fact.
                            let kf = ok(k.width).map(|w| (w, !k.is_real && !k.is_fill));
                            match (fact(&rw, *s), kf) {
                                (Some((a, true)), Some((b, true))) => {
                                    ok(a.max(b)).map(|w| (w, true))
                                }
                                _ => None,
                            }
                        }
                    };
                    store(&mut rw, *d, f);
                }

                // `Select` is the one arm that writes registers it does not
                // name as its destination: it widens a §5.7.1 fill branch to
                // the other branch's width IN PLACE before choosing. Only when
                // both branches are known non-fill do they keep their widths —
                // and then all three outcomes (`merge_unknown`, or a clone of
                // either branch) are `max(tw, ew)` wide, which is a single
                // known width when the two agree. Store `dest` last: it may
                // alias a branch register.
                Insn::Select(d, _, t, e) => {
                    let (ft, fe) = (fact(&rw, *t), fact(&rw, *e));
                    let f = match (ft, fe) {
                        (Some((a, true)), Some((b, true))) if a == b => Some((a, true)),
                        _ => {
                            store(&mut rw, *t, ft.filter(|(_, p)| *p));
                            store(&mut rw, *e, fe.filter(|(_, p)| *p));
                            None
                        }
                    };
                    store(&mut rw, *d, f);
                }

                // `concat_refs` (and the exec arms' inline equivalents)
                // return the SUM of the operand widths. An overflowing sum
                // wraps in the exec arm, and one past `MAX_WIDTH` is clamped;
                // `checked_add` and `ok` between them exclude both.
                Insn::Concat(d, parts) => {
                    let mut sum = Some(0u32);
                    for p in parts.iter() {
                        sum = match (sum, fact(&rw, *p)) {
                            (Some(a), Some((b, _))) => a.checked_add(b),
                            _ => None,
                        };
                    }
                    store(&mut rw, *d, sum.and_then(ok).map(|w| (w, true)));
                }
                // `{1{x}}` hands the source `Value` through untouched (the
                // main exec arm does not even copy it when `d == s`), so it
                // inherits the source's fact exactly, `is_fill` included.
                // `{n{x}}` for n >= 2 concatenates n copies; n == 0 is a
                // zero-width value, which `ok` rejects.
                Insn::Replicate(d, s, n) => {
                    let f = if *n == 1 {
                        fact(&rw, *s)
                    } else {
                        fact(&rw, *s)
                            .and_then(|(w, _)| w.checked_mul(*n))
                            .and_then(ok)
                            .map(|w| (w, true))
                    };
                    store(&mut rw, *d, f);
                }

                // Destination width not established here: the bounds of a
                // dynamic range select are register values, and an array
                // element read that fails to resolve returns a 1-bit X
                // instead of the element.
                Insn::RangeSelect(d, _, _, _) | Insn::LoadArrayElem(d, _, _) => {
                    store(&mut rw, *d, None)
                }

                // Stamp/clear `is_signed`; storage and width are untouched.
                Insn::SetSigned(_) | Insn::ClearSigned(_) => {}

                // Result width is the (runtime) left operand's width — not
                // statically tracked here.
                Insn::Pow(d, _, _) => store(&mut rw, *d, None),

                // No register destination.
                Insn::WaitDelayReg(..)
                | Insn::WaitEdge(..)
                | Insn::BranchIfFalse(..)
                | Insn::BranchUnlessZero(..)
                | Insn::BranchIfSignalFalse(..)
                | Insn::Jump(..)
                | Insn::Nop
                | Insn::NbaAssign(..)
                | Insn::NbaAssignConst(..)
                | Insn::NbaAssignRange(..)
                | Insn::NbaAssignRangeDyn(..)
                | Insn::NbaAssignBitDyn(..)
                | Insn::NbaAssignArray(..)
                | Insn::NbaAssignArrayRange(..)
                | Insn::NbaAssignArrayRead(..)
                | Insn::BlockingAssign(..)
                | Insn::BlockingAssignRange(..)
                | Insn::BlockingAssignRangeDyn(..)
                | Insn::BlockingAssignBitDyn(..)
                | Insn::BlockingAssignArray(..)
                | Insn::BlockingAssignArrayRange(..) => {}

                // The AST interpreter runs with the whole machine in reach.
                Insn::StmtFallback(..) => rw.iter_mut().for_each(|f| *f = None),
                Insn::EvalExprFallback(..) => rw.iter_mut().for_each(|f| *f = None),
            }
        }
    }

    /// Drop `Nop`s left behind by pair fusion above, rewriting branch targets.
    ///
    /// Every fusion in this pass replaces a two-instruction pair with one real
    /// instruction plus a `Nop` placeholder, because collapsing the vector
    /// mid-pass would invalidate the indices the loops and `is_target` use.
    /// Those placeholders were never removed afterwards, so each one cost a
    /// dispatch on EVERY execution for the life of the run. On the C906 SoC
    /// running CoreMark they are 15-20% of the instructions in a compiled
    /// continuous assignment (e.g. the second most-executed RHS shape is
    /// `LoadRng,Nop,Resize,Move,Resize,AssignRng` — one of six).
    ///
    /// A branch target is an index into this vector, so removal must remap it.
    /// A target that pointed AT a removed `Nop` moves to the next surviving
    /// instruction, which is exactly where control would have arrived anyway;
    /// `len` (one past the end, used by loop exits) maps to the new length.
    /// Is any instruction after `j` reading `r`? Conservative liveness for
    /// the census-pair peepholes below: any later read — even one past a
    /// redefinition — counts as live, which can only forgo an optimization,
    /// never miscompile one.
    fn reg_read_after(insns: &[Insn], j: usize, r: RegId) -> bool {
        insns[j + 1..].iter().any(|x| Self::insn_reads_reg(x, r))
    }

    /// Branch-target map shared by the census-pair peepholes: rewriting or
    /// deleting an instruction is only sound when control cannot enter the
    /// pattern's interior from elsewhere.
    fn branch_target_map(insns: &[Insn]) -> Vec<bool> {
        let mut is_target = vec![false; insns.len() + 1];
        for insn in insns.iter() {
            match insn {
                Insn::CaseJump(_, cj) => {
                    for &t in cj.table.iter().chain(std::iter::once(&cj.default)) {
                        if (t as usize) < is_target.len() {
                            is_target[t as usize] = true;
                        }
                    }
                }
                Insn::CaseMaskJump(_, mj) => {
                    for &t in mj.table.iter().chain(std::iter::once(&mj.xz_path)) {
                        if (t as usize) < is_target.len() {
                            is_target[t as usize] = true;
                        }
                    }
                }
                Insn::BranchIfFalse(_, t)
                | Insn::BranchUnlessZero(_, t)
                | Insn::BranchIfSignalFalse(_, t, _)
                | Insn::CmpBranch(_, _, _, _, t)
                | Insn::Jump(t)
                    if (*t as usize) < is_target.len() =>
                {
                    is_target[*t as usize] = true;
                }
                _ => {}
            }
        }
        is_target
    }

    /// `Move(d, s) ; <assign whose VALUE reg is d>` → assign reads `s`
    /// directly, Move deleted.
    ///
    /// `propagate_copies` above folds the PRODUCER side (`producer ; Move`
    /// retargets the producer), but the c906 opcode census showed the
    /// CONSUMER side alive at scale: `Move -> BlockingAssignRange` was 5.3%
    /// of all executed instructions (337 M dynamic pairs on cmark it1) —
    /// the Move's source is produced far earlier, so the producer-side pass
    /// never sees it. Sound when nothing can jump into the pair's interior
    /// and `d` has no later reader.
    fn forward_move_into_assign(insns: &mut [Insn]) {
        use std::sync::OnceLock;
        static ON: OnceLock<bool> = OnceLock::new();
        if !*ON.get_or_init(|| {
            !matches!(std::env::var("XEZIM_FUSE").as_deref(), Ok("0"))
                && !matches!(std::env::var("XEZIM_FUSE_MOVEFWD").as_deref(), Ok("0"))
        }) || insns.len() < 2
            || Self::has_backward_branch(insns)
        {
            return;
        }
        let is_target = Self::branch_target_map(insns);
        for i in 0..insns.len() - 1 {
            let Insn::Move(d, s) = insns[i] else {
                continue;
            };
            if d == s {
                continue;
            }
            // Next surviving instruction; only Nops may sit between, and no
            // branch may land inside (i, j].
            let mut j = i + 1;
            let mut blocked = false;
            while j < insns.len() {
                if is_target[j] {
                    blocked = true;
                    break;
                }
                if !matches!(insns[j], Insn::Nop) {
                    break;
                }
                j += 1;
            }
            if blocked || j >= insns.len() {
                continue;
            }
            let vref = match &mut insns[j] {
                Insn::BlockingAssign(_, v, _)
                | Insn::NbaAssign(_, v, _)
                | Insn::BlockingAssignRange(_, _, _, v)
                | Insn::NbaAssignRange(_, _, _, v) => v,
                _ => continue,
            };
            if *vref != d {
                continue;
            }
            if Self::reg_read_after(insns, j, d) {
                continue;
            }
            match &mut insns[j] {
                Insn::BlockingAssign(_, v, _)
                | Insn::NbaAssign(_, v, _)
                | Insn::BlockingAssignRange(_, _, _, v)
                | Insn::NbaAssignRange(_, _, _, v) => *v = s,
                _ => unreachable!(),
            }
            insns[i] = Insn::Nop;
            FUSED_MOVE_FWD.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// Delete `ClearSigned(r)` when `r` provably already holds an UNSIGNED
    /// value — the scrub is then a no-op and removing it cannot change
    /// anything.
    ///
    /// The c906 opcode census measured ClearSigned at 8.0% of the executed
    /// stream (506 M / run): sign scrubs are emitted defensively at §11.8.1
    /// mixed-signedness sites, but most registers already carry unsigned
    /// values. This is a single forward pass tracking a per-register
    /// "provably unsigned" set, built only from rules VERIFIED against the
    /// `Value` method each executor calls:
    ///
    /// * compares / case-compares / logical ops -> 1-bit unsigned
    /// * `bitwise_and/or/xor`, `range_select`, `bit_select`, `concat_refs`
    ///   -> `is_signed: false` unconditionally
    /// * `add/sub/mul` -> signed only when BOTH operands signed
    /// * `shift_left/right`, `bitwise_not`, replicate, `Move`/`MoveResize`
    ///   -> preserve/propagate the source flag
    /// * `Resize` extends by the value's own flag and keeps it — no change
    ///
    /// Every join point (branch target) wipes the set; any instruction not
    /// understood wipes the set; `SetSigned` un-cleans its register. All
    /// three fallbacks only FORGO deletions, never enable a wrong one.
    fn elide_provably_unsigned_scrubs(insns: &mut [Insn], signal_signed: &[bool]) {
        use std::sync::OnceLock;
        static ON: OnceLock<bool> = OnceLock::new();
        if !*ON.get_or_init(|| {
            !matches!(std::env::var("XEZIM_FUSE").as_deref(), Ok("0"))
                && !matches!(std::env::var("XEZIM_FUSE_SCRUBS").as_deref(), Ok("0"))
        }) || insns.is_empty()
        {
            return;
        }
        let is_target = Self::branch_target_map(insns);
        let mut clean: std::collections::HashSet<RegId> = std::collections::HashSet::new();
        let sig_unsigned =
            |sig: u32| -> bool { signal_signed.get(sig as usize).is_some_and(|s| !*s) };
        for i in 0..insns.len() {
            if is_target[i] {
                clean.clear();
            }
            // set_to(d, c): record the DEST's new cleanliness. Computed before
            // mutation so a dest that is also a source reads its OLD state.
            let mut set_to: Option<(RegId, bool)> = None;
            let mut elide = false;
            match &insns[i] {
                Insn::Nop => {}
                // -- 1-bit unsigned results --
                Insn::Eq(d, _, _)
                | Insn::Neq(d, _, _)
                | Insn::CaseEq(d, _, _)
                | Insn::CasezEq(d, _, _)
                | Insn::CasexEq(d, _, _)
                | Insn::Lt(d, _, _)
                | Insn::Leq(d, _, _)
                | Insn::Gt(d, _, _)
                | Insn::Geq(d, _, _)
                | Insn::LogAnd(d, _, _)
                | Insn::LogOr(d, _, _)
                | Insn::LogNot(d, _) => set_to = Some((*d, true)),
                // -- unconditionally unsigned constructors --
                Insn::BitAnd(d, _, _)
                | Insn::BitOr(d, _, _)
                | Insn::BitXor(d, _, _)
                | Insn::LoadSignalBit(d, _, _)
                | Insn::LoadSignalRange(d, _, _, _)
                | Insn::Concat(d, _) => set_to = Some((*d, true)),
                // -- signed only when BOTH operands are --
                Insn::Add(d, l, r) | Insn::Sub(d, l, r) | Insn::Mul(d, l, r) => {
                    set_to = Some((*d, clean.contains(l) || clean.contains(r)))
                }
                // -- flag preserved / propagated from the source --
                Insn::BitNot(d, sr)
                | Insn::Shl(d, sr, _)
                | Insn::Shr(d, sr, _)
                | Insn::Replicate(d, sr, _)
                | Insn::Move(d, sr)
                | Insn::MoveResize(d, sr, _) => set_to = Some((*d, clean.contains(sr))),
                Insn::Resize(_, _) => {}
                Insn::LoadConst(d, v) => set_to = Some((*d, !v.is_signed && !v.is_real)),
                Insn::LoadSignal(d, sig) => set_to = Some((*d, sig_unsigned(*sig))),
                Insn::LoadSignalSigned(d, _) => set_to = Some((*d, false)),
                Insn::BinOpConst(d, sr, k, kind) => {
                    let c = match kind {
                        BinOpConstKind::Eq | BinOpConstKind::CaseEq | BinOpConstKind::Xor => true,
                        BinOpConstKind::Add => clean.contains(sr) || !k.is_signed,
                    };
                    set_to = Some((*d, c));
                }
                Insn::ClearSigned(r) => {
                    if clean.contains(r) {
                        elide = true;
                    } else {
                        set_to = Some((*r, true));
                    }
                }
                Insn::SetSigned(r) => set_to = Some((*r, false)),
                // -- no register writes: state flows through --
                Insn::Jump(_)
                | Insn::BranchIfFalse(_, _)
                | Insn::BranchUnlessZero(_, _)
                | Insn::BranchIfSignalFalse(_, _, _)
                | Insn::CmpBranch(_, _, _, _, _)
                | Insn::BlockingAssign(_, _, _)
                | Insn::NbaAssign(_, _, _)
                | Insn::BlockingAssignRange(_, _, _, _)
                | Insn::NbaAssignRange(_, _, _, _)
                | Insn::BlockingAssignBitDyn(_, _, _)
                | Insn::NbaAssignBitDyn(_, _, _)
                | Insn::BlockingAssignArray(_, _, _, _)
                | Insn::NbaAssignArray(_, _, _, _)
                | Insn::BlockingAssignArrayRange(_, _, _, _, _)
                | Insn::NbaAssignArrayRange(_, _, _, _, _)
                | Insn::NbaAssignArrayRead(_, _, _, _) => {}
                // -- anything else: unknown effects, forget everything --
                _ => clean.clear(),
            }
            if elide {
                insns[i] = Insn::Nop;
                ELIDED_SCRUBS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                continue;
            }
            if let Some((d, c)) = set_to {
                if c {
                    clean.insert(d);
                } else {
                    clean.remove(&d);
                }
            }
        }
    }

    /// Merge two adjacent constant adds into one `BinOpConstAdd2` dispatch.
    /// Pure dispatch amortization — both adds still execute, in order, so
    /// chained (`s2 == d1`) and independent pairs are equally sound; the only
    /// constraint is that control cannot enter between them. OPT-IN
    /// (`XEZIM_FUSE_ADDC2=1`): the new variant must stay off until every
    /// backend either handles it (interpreter, two-state lowering) or bails
    /// per-block (JIT/AOT return None on unsupported insns) — forming it by
    /// default would silently change which blocks those backends accept.
    fn fuse_addc2(insns: &mut [Insn]) {
        use std::sync::OnceLock;
        static ON: OnceLock<bool> = OnceLock::new();
        if !*ON.get_or_init(|| matches!(std::env::var("XEZIM_FUSE_ADDC2").as_deref(), Ok("1")))
            || insns.len() < 2
        {
            return;
        }
        let is_target = Self::branch_target_map(insns);
        let mut i = 0;
        while i + 1 < insns.len() {
            let Insn::BinOpConst(_, _, _, BinOpConstKind::Add) = insns[i] else {
                i += 1;
                continue;
            };
            let mut j = i + 1;
            let mut blocked = false;
            while j < insns.len() {
                if is_target[j] {
                    blocked = true;
                    break;
                }
                if !matches!(insns[j], Insn::Nop) {
                    break;
                }
                j += 1;
            }
            if blocked || j >= insns.len() {
                i += 1;
                continue;
            }
            let Insn::BinOpConst(_, _, _, BinOpConstKind::Add) = insns[j] else {
                i = j;
                continue;
            };
            let Insn::BinOpConst(d1, s1, k1, _) = std::mem::replace(&mut insns[i], Insn::Nop)
            else {
                unreachable!()
            };
            let Insn::BinOpConst(d2, s2, k2, _) = std::mem::replace(&mut insns[j], Insn::Nop)
            else {
                unreachable!()
            };
            insns[i] = Insn::BinOpConstAdd2(Box::new(AddC2 {
                d1,
                s1,
                k1: *k1,
                d2,
                s2,
                k2: *k2,
            }));
            FUSED_ADDC2.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            i = j + 1;
        }
    }

    fn compact_nops(insns: &mut Vec<Insn>) {
        if !insns.iter().any(|i| matches!(i, Insn::Nop)) {
            return;
        }
        // old index -> new index; `map[len]` is the new one-past-the-end.
        let mut map = vec![0u32; insns.len() + 1];
        let mut new_idx = 0u32;
        for (old, insn) in insns.iter().enumerate() {
            map[old] = new_idx;
            if !matches!(insn, Insn::Nop) {
                new_idx += 1;
            }
        }
        map[insns.len()] = new_idx;
        insns.retain(|i| !matches!(i, Insn::Nop));
        for insn in insns.iter_mut() {
            match insn {
                Insn::CaseJump(_, cj) => {
                    for t in cj.table.iter_mut().chain(std::iter::once(&mut cj.default)) {
                        if let Some(&m) = map.get(*t as usize) {
                            *t = m;
                        }
                    }
                }
                Insn::CaseMaskJump(_, mj) => {
                    for t in mj.table.iter_mut().chain(std::iter::once(&mut mj.xz_path)) {
                        if let Some(&m) = map.get(*t as usize) {
                            *t = m;
                        }
                    }
                }
                Insn::BranchIfFalse(_, t)
                | Insn::BranchUnlessZero(_, t)
                | Insn::BranchIfSignalFalse(_, t, _)
                | Insn::CmpBranch(_, _, _, _, t)
                | Insn::Jump(t) => {
                    if let Some(&m) = map.get(*t as usize) {
                        *t = m;
                    }
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ident_expr(name: &str) -> Expression {
        let span = crate::ast::Span::dummy();
        Expression::new(
            ExprKind::Ident(HierarchicalIdentifier {
                root: None,
                path: vec![HierPathSegment {
                    name: crate::ast::Identifier {
                        name: name.to_owned(),
                        span,
                    },
                    selects: Vec::new(),
                }],
                span,
                cached_signal_id: std::cell::Cell::new(None),
                cached_resolved_name: std::cell::OnceCell::new(),
            }),
            span,
        )
    }

    fn indexed_expr(name: &str, index: char) -> Expression {
        let span = crate::ast::Span::dummy();
        Expression::new(
            ExprKind::Index {
                expr: Box::new(ident_expr(name)),
                index: Box::new(Expression::new(
                    ExprKind::Number(NumberLiteral::UnbasedUnsized(index)),
                    span,
                )),
            },
            span,
        )
    }

    fn nested_indexed_expr(name: &str, outer: char, inner: char) -> Expression {
        let span = crate::ast::Span::dummy();
        Expression::new(
            ExprKind::Index {
                expr: Box::new(indexed_expr(name, outer)),
                index: Box::new(Expression::new(
                    ExprKind::Number(NumberLiteral::UnbasedUnsized(inner)),
                    span,
                )),
            },
            span,
        )
    }

    #[test]
    fn generated_nonzero_outer_index_resolves_to_flattened_signal() {
        let mut signals: HashMap<Arc<str>, usize> = HashMap::default();
        signals.insert(Arc::from("flat"), 0);
        let arrays: HashMap<String, (i64, i64, u32)> = HashMap::default();
        let widths: HashMap<String, u32> = HashMap::default();
        let compiler = BytecodeCompiler::new(&signals, &[false], &[160], &arrays, &widths);

        assert_eq!(
            compiler.flattened_outer_const_signal_id(&indexed_expr("flat", '1')),
            Some(0)
        );
    }

    #[test]
    fn genuine_array_shapes_do_not_resolve_as_flattened_signals() {
        let mut signals: HashMap<Arc<str>, usize> = HashMap::default();
        signals.insert(Arc::from("flat"), 0);
        let widths: HashMap<String, u32> = HashMap::default();
        let expr = indexed_expr("flat", '1');

        let mut arrays: HashMap<String, (i64, i64, u32)> = HashMap::default();
        arrays.insert("flat".to_owned(), (0, 3, 160));
        let compiler = BytecodeCompiler::new(&signals, &[false], &[160], &arrays, &widths);
        assert_eq!(compiler.flattened_outer_const_signal_id(&expr), None);

        let arrays: HashMap<String, (i64, i64, u32)> = HashMap::default();
        let mut packed_elem_widths: HashMap<String, u32> = HashMap::default();
        packed_elem_widths.insert("flat".to_owned(), 32);
        let mut compiler = BytecodeCompiler::new(&signals, &[false], &[160], &arrays, &widths);
        compiler.set_packed_elem_widths(&packed_elem_widths);
        assert_eq!(compiler.flattened_outer_const_signal_id(&expr), None);

        let mut multi_dim_arrays: HashSet<String> = HashSet::default();
        multi_dim_arrays.insert("flat".to_owned());
        let mut compiler = BytecodeCompiler::new(&signals, &[false], &[160], &arrays, &widths);
        compiler.set_multi_dim_arrays(&multi_dim_arrays);
        assert_eq!(compiler.flattened_outer_const_signal_id(&expr), None);
    }

    #[test]
    fn constant_multi_dim_array_element_uses_scalar_bytecode() {
        let mut signals: HashMap<Arc<str>, usize> = HashMap::default();
        signals.insert(Arc::from("m[1][0]"), 0);
        let arrays: HashMap<String, (i64, i64, u32)> = HashMap::default();
        let widths: HashMap<String, u32> = HashMap::default();
        let mut multi_dim_arrays: HashSet<String> = HashSet::default();
        multi_dim_arrays.insert("m".to_owned());
        let lhs = nested_indexed_expr("m", '1', '0');

        let mut compiler = BytecodeCompiler::new(&signals, &[false], &[8], &arrays, &widths);
        compiler.set_multi_dim_arrays(&multi_dim_arrays);
        assert_eq!(
            compiler.const_multi_dim_array_elem_signal_id(&lhs),
            Some(0)
        );
        assert!(compiler.compile_nba_target(&lhs, 0, 8));
        let block = compiler.finish();
        assert!(matches!(
            block.instructions.as_slice(),
            [Insn::NbaAssign(0, 0, 8)]
        ));

        let mut compiler = BytecodeCompiler::new(&signals, &[false], &[8], &arrays, &widths);
        compiler.set_multi_dim_arrays(&multi_dim_arrays);
        assert!(compiler.compile_expr(&lhs, 0).is_some());
        let block = compiler.finish();
        assert!(matches!(
            block.instructions.as_slice(),
            [Insn::LoadSignal(0, 0)]
        ));
    }

    #[test]
    fn register_ids_do_not_wrap_at_u16_limit() {
        let signals: HashMap<Arc<str>, usize> = HashMap::default();
        let arrays: HashMap<String, (i64, i64, u32)> = HashMap::default();
        let widths: HashMap<String, u32> = HashMap::default();
        let mut compiler = BytecodeCompiler::new(&signals, &[], &[], &arrays, &widths);

        let mut last = 0;
        for _ in 0..=u16::MAX {
            last = compiler.alloc_reg();
        }
        compiler.emit(Insn::LoadConst(last, Box::new(Value::zero(1))));

        let block = compiler.finish();
        assert_eq!(last, u16::MAX);
        assert_eq!(block.num_regs, u16::MAX as u32 + 1);
        assert!(matches!(
            block.instructions.last(),
            Some(Insn::LoadConst(reg, _)) if u32::from(*reg) < block.num_regs
        ));

        // The next temporary is the first ID that cannot be represented by
        // the compact instruction encoding. It must request fallback instead
        // of wrapping to register zero as it did in BUG_REPORT.md.
        let mut compiler = BytecodeCompiler::new(&signals, &[], &[], &arrays, &widths);
        for _ in 0..=u16::MAX {
            compiler.alloc_reg();
        }
        let expr = Expression::new(
            ExprKind::Number(NumberLiteral::UnbasedUnsized('0')),
            crate::ast::Span::dummy(),
        );
        assert_eq!(compiler.compile_root_expr(&expr), None);
        assert!(compiler.register_overflow);
        assert_eq!(compiler.next_reg, u16::MAX as u32 + 1);
    }

    #[test]
    fn redundant_resizes_become_nops_and_narrowing_ones_survive() {
        // Signal 0 is 8 bits, so the first resize is already satisfied; the
        // second genuinely narrows; the third is satisfied by the second.
        let mut insns = vec![
            Insn::LoadSignal(0, 0),
            Insn::Resize(0, 8),
            Insn::Resize(0, 4),
            Insn::Resize(0, 4),
        ];
        BytecodeCompiler::elide_redundant_resizes(&mut insns, &[8], None, 1);
        assert!(matches!(
            insns.as_slice(),
            [Insn::LoadSignal(0, 0), Insn::Nop, Insn::Resize(0, 4), Insn::Nop]
        ));
    }

    #[test]
    fn a_resize_that_is_a_branch_target_is_never_removed() {
        // Control can arrive at index 3 without having run index 2, so the
        // width of r0 there depends on the path taken.
        let mut insns = vec![
            Insn::LoadSignal(0, 0),
            Insn::BranchIfFalse(0, 3),
            Insn::Resize(0, 8),
            Insn::Resize(0, 8),
        ];
        BytecodeCompiler::elide_redundant_resizes(&mut insns, &[8], None, 1);
        assert!(matches!(insns[2], Insn::Nop));
        assert!(matches!(insns[3], Insn::Resize(0, 8)));
    }

    #[test]
    fn arithmetic_on_a_possibly_real_operand_keeps_its_resize() {
        // A signal's declared type may be `real`, and `Value::add` then returns
        // a 64-bit `from_f64` instead of `max(width)` — so the result width is
        // not established here even though both operand widths are known.
        // `bitwise_or` has no such special case, so its resize does go.
        let mut insns = vec![
            Insn::LoadSignal(0, 0),
            Insn::LoadSignal(1, 0),
            Insn::Add(2, 0, 1),
            Insn::Resize(2, 8),
            Insn::BitOr(3, 0, 1),
            Insn::Resize(3, 8),
        ];
        BytecodeCompiler::elide_redundant_resizes(&mut insns, &[8], None, 4);
        assert!(matches!(insns[3], Insn::Resize(2, 8)));
        assert!(matches!(insns[5], Insn::Nop));
    }
}

// ---------------------------------------------------------------------------
// Two-state lowering (P5). A comb block whose reads are all X-free at eval
// time can run on plain u64 words: no 4-state masks, no `Value` clones, no
// storage dispatch. `lower_two_state` translates a compiled block's insn
// stream 1:1 into `TsInsn`s with STATICALLY-known widths and masks, bailing
// (returning None) on anything whose 2-state semantics are not proven here:
//
// - signed loads/consts (2-state Resize only zero-extends; every lowered reg
//   is unsigned by construction, so a Resize that widens is a no-op),
// - out-of-STATIC-range selects (4-state reads produce X bits there),
// - fill/real/wide(>64)/X-carrying constants,
// - width-changing or real/wide writebacks,
// - any opcode outside the proven set (branches, fallbacks, arrays, …).
//
// The eval site must additionally check, per evaluation: every signal in
// `read_sigs` currently holds an X-free value (width ≤ 64 was proven at
// lower time, so storage is Inline and `raw_bits` is exact), no forces are
// active, and `warn_x` is off (the 2-state path skips its bookkeeping).
// On any miss it falls back to the 4-state stream, which is always correct.
// ---------------------------------------------------------------------------

/// Payload of the dynamic array-element ops, boxed to keep `TsInsn` at 24
/// bytes (the `i64` bounds pair alone would push every instruction to 40
/// bytes and cost measurable dispatch bandwidth on long comb streams).
#[derive(Debug, Clone)]
pub struct TsElemOp {
    pub first: u32,
    pub lo: i64,
    pub hi: i64,
    pub idx: u16,
    pub s: u16,
    pub w: u32,
    pub mask: u64,
}

#[derive(Debug, Clone)]
pub struct TsNbaFromElem {
    pub dst: u32,
    pub first: u32,
    pub lo: i64,
    pub hi: i64,
    pub idx_sig: u32,
    pub w: u32,
}

/// One two-state instruction. Register file is `u64`; every value is kept
/// masked to its static width by construction. Branch targets are indices
/// into the LOWERED stream (remapped from the 4-state stream's indices).
#[derive(Debug, Clone)]
pub enum TsInsn {
    /// regs[d] = signal_table[sig] (raw value bits; proven X-free by the
    /// eval-site prefilter).
    LoadSig { d: u16, sig: u32 },
    Const { d: u16, v: u64 },
    /// regs[d] = bit `bit` of a ≤64-bit signal (inline raw_bits path).
    SigBit { d: u16, sig: u32, bit: u16 },
    /// regs[d] = (signal >> lo) & mask, ≤64-bit signal.
    SigRange { d: u16, sig: u32, lo: u16, mask: u64 },
    /// Wide-signal (>64) variants — read the addressed slice only.
    SigBitW { d: u16, sig: u32, bit: u16 },
    SigRangeW { d: u16, sig: u32, lo: u16, w: u16, mask: u64 },
    /// regs[d] = bit `bit` of regs[s].
    Bit { d: u16, s: u16, bit: u8 },
    /// regs[d] = (regs[s] >> lo) & mask.
    Range { d: u16, s: u16, lo: u8, mask: u64 },
    Xor { d: u16, a: u16, b: u16 },
    And { d: u16, a: u16, b: u16 },
    Or { d: u16, a: u16, b: u16 },
    /// regs[d] = if regs[c] != 0 { regs[a] } else { regs[b] } — §11.4.11
    /// conditional. Two-state values are X-free by the eval-site prefilter,
    /// so the 4-state arm's unknown-condition bit-merge cannot arise and the
    /// selector reduces to plain SV truthiness (non-zero).
    Sel { d: u16, c: u16, a: u16, b: u16 },
    /// regs[d] = !regs[s] & mask (mask = source width).
    Not { d: u16, s: u16, mask: u64 },
    XorC { d: u16, s: u16, k: u64 },
    /// regs[d] = (regs[s] == k) — 4-state Eq/CaseEq agree on clean values.
    EqC { d: u16, s: u16, k: u64 },
    /// Wrapping two's-complement at max operand width (`mask`); operands are
    /// zero-extended (all lowered registers are unsigned).
    Add { d: u16, a: u16, b: u16, mask: u64 },
    Sub { d: u16, a: u16, b: u16, mask: u64 },
    /// 1-bit comparison results; unsigned compare per §5.5.1 (either operand
    /// unsigned ⇒ unsigned, and every lowered register is unsigned).
    Eq { d: u16, a: u16, b: u16 },
    Neq { d: u16, a: u16, b: u16 },
    Lt { d: u16, a: u16, b: u16 },
    Leq { d: u16, a: u16, b: u16 },
    Gt { d: u16, a: u16, b: u16 },
    Geq { d: u16, a: u16, b: u16 },
    LogNot { d: u16, s: u16 },
    LogAnd { d: u16, a: u16, b: u16 },
    LogOr { d: u16, a: u16, b: u16 },
    /// MSB-first parts, mirroring `Value::concat_refs`.
    Concat { d: u16, parts: Box<[(u16, u8)]> },
    /// In-place truncation (a `Resize` that narrows; widening is free).
    Mask { d: u16, mask: u64 },
    /// Jump to `t` when the signal (or its `bit`, when != u32::MAX) is zero.
    /// The prefilter proved the signal X-free, so `!is_true` = `== 0`.
    BrSigFalse { sig: u32, bit: u32, t: u32 },
    /// Jump to `t` when regs[s] == 0 (4-state `BranchIfFalse` on a clean reg).
    BrFalse { s: u16, t: u32 },
    /// Jump to `t` when regs[s] != 0 (4-state `BranchUnlessZero`).
    BrNz { s: u16, t: u32 },
    Jmp { t: u32 },
    /// Jump-table dispatch (§12.5 `case`). Two-state registers are X-free by
    /// the eval-site prefilter, so the 4-state arm's "any x/z bit matches no
    /// pattern -> default" branch cannot arise and this reduces to a bounds-
    /// checked table index. Targets are LOWERED indices (remapped in the same
    /// fixup pass as the other branches).
    CaseJmp { s: u16, cj: Box<CaseJumpData> },
    /// Bucket-window jump table (§12.5 `case` with wildcard-free windows).
    /// X-free registers make the 4-state `xz_path` (wildcard selector can
    /// match several buckets) unreachable, leaving a plain window index.
    CaseMaskJmp { s: u16, mask: u64, lo: u32, wmask: u64, mj: Box<CaseMaskJumpData> },
    /// regs[d] = (regs[s] != 0) — §11.4.9 reduction OR on an X-free operand.
    RedOr { d: u16, s: u16 },
    /// Wide (65..128-bit) reduction OR: reads the WIDE register file. The
    /// narrow `RedOr` on a wide source read `regs[s]` — a slot the wide load
    /// never wrote, i.e. whatever the PREVIOUS block's evaluation left there.
    WRedOr { d: u16, s: u16 },
    /// Wide reduction AND: value == all-ones over its width
    /// (`mask_hi` masks the high word).
    WRedAnd { d: u16, s: u16, mask_hi: u64 },
    /// regs[d] = (regs[s] == mask) — §11.4.9 reduction AND; `mask` is the
    /// source width, so "all ones" is an equality against it.
    RedAnd { d: u16, s: u16, mask: u64 },
    /// `sig[regs[i]] = regs[s] & 1` (§11.5.1 dynamic bit-select target).
    /// Out-of-range indices are dropped, matching `Value::set_bit`. Splices
    /// through the same plane-level merge as `RangeStore`, so X elsewhere in
    /// the destination survives.
    BitStoreDyn { sig: u32, i: u16, s: u16, w: u32 },
    /// Blocking store of a folded 4-state constant (`y = 'x;`). `v`/`x` are
    /// the raw value/xz planes, already masked to the assigned width.
    ConstStoreX { sig: u32, v: u64, x: u64 },
    /// Partial-range counterpart (`y[hi:lo] = 'x;`).
    RangeStoreX { sig: u32, hi: u32, lo: u32, v: u64, x: u64 },
    /// Write back `regs[s] & mask` to `sig` with change-detect + dirty
    /// marking (the eval site mirrors the 4-state fast-path bookkeeping).
    Store { sig: u32, s: u16, mask: u64 },
    /// Nonblocking write: queue `regs[s] & mask` (width `w`) with §10.4.2
    /// last-write-wins and the eval-time elision, exactly as `NbaAssign`.
    StoreNba { sig: u32, s: u16, w: u32, mask: u64 },
    /// NBA of a compile-time CONSTANT (`q <= 8'h3f;`) — the single largest
    /// two-state bail on c906 (1,189 edge blocks). Non-abortable: the value
    /// was validated 2-state at lowering.
    ConstStoreNba { sig: u32, v: u64, w: u32 },
    /// Partial-bit NBA (`q[hi:lo] <= v`). Mirrors the 4-state executor's
    /// `compose_inline_range_bits` merge against the pending-or-current
    /// value — the composition works on raw bit PLANES, so an X base flows
    /// through instead of aborting; only the SOURCE must be 2-state, and it
    /// is (it lives in a ts register). Non-abortable.
    RangeStoreNba { sig: u32, hi: u32, lo: u32, s: u16, mask: u64 },
    /// Blocking counterpart of `RangeStoreNba`: splice regs[s] into
    /// signal[hi:lo] immediately (§10.4.1), preserving the bits outside the
    /// window. The TS bail census measured `BlockingAssignRange` gating
    /// 133.2M interpreter evaluations on the C906 SoC — 42.9% of them, the
    /// single largest reason two-state lowering gave up.
    RangeStore { sig: u32, hi: u32, lo: u32, s: u16, mask: u64 },
    /// Dynamic array-element read: eid = first + (regs[idx] - lo). ABORTS
    /// the two-state run (caller re-runs 4-state) when the index is out of
    /// range or the element holds X — both produce X in 4-state. Lowering
    /// only admits this before any side-effecting op, so an abort is clean.
    ElemLoad(Box<TsElemOp>),
    /// Dynamic array-element NBA (`mem[waddr] <= v`): out-of-range DROPS
    /// silently (4-state behavior); otherwise mirrors the NbaAssignArray
    /// arm (elide-if-equal, then index+push).
    ElemStoreNba(Box<TsElemOp>),
    /// Dynamic array-element blocking write; out-of-range drops silently.
    ElemStore(Box<TsElemOp>),
    /// Fused array-read-to-NBA (`q <= mem[raddr]`, NbaAssignArrayRead):
    /// index comes from a SIGNAL; aborts on out-of-range or X element
    /// (4-state queues an X value there).
    NbaFromElem(Box<TsNbaFromElem>),
    // ---- WIDE (65..=128-bit) bank: little-endian [u64; 2] registers. ----
    /// wregs[d] = signal words (prefilter proved the signal X-free).
    WLoadSig { d: u16, sig: u32 },
    WConst { d: u16, v: Box<[u64; 2]> },
    WXor { d: u16, a: u16, b: u16 },
    WAnd { d: u16, a: u16, b: u16 },
    WOr { d: u16, a: u16, b: u16 },
    /// wregs[d] = !wregs[s] masked to width (mask_hi masks the high word).
    WNot { d: u16, s: u16, mask_hi: u64 },
    /// Wide→wide slice: wregs[d] = (wregs[s] >> lo) & mask(w), w in 65..=128.
    WRange { d: u16, s: u16, lo: u16, mask_hi: u64 },
    /// Narrow (≤64) slice of a wide register.
    RangeFromW { d: u16, s: u16, lo: u16, mask: u64 },
    BitFromW { d: u16, s: u16, bit: u16 },
    /// MSB-first concat into a wide register; parts may be narrow or wide.
    WConcat { d: u16, parts: Box<[(u16, u8, bool)]> },
    /// Resize truncation within the wide bank.
    WMask { d: u16, mask_hi: u64 },
    /// Bank moves for Resize crossings (same register index).
    WFromN { r: u16 },
    NFromW { r: u16, mask: u64 },
    /// Wide writeback with change-detect + dirty hooks (mask_hi pre-applied
    /// to the register by construction; stored via Value::set_words128).
    WStore { sig: u32, s: u16 },
    WStoreNba { sig: u32, s: u16, w: u32 },
    /// Wrapping multiply at max operand width (both operands unsigned).
    Mul { d: u16, a: u16, b: u16, mask: u64 },
    /// Wide (65..=128-bit) slice of a SIGNAL into the wide bank; the two
    /// halves are prefiltered as ordinary ≤64 slices.
    WSigRange { d: u16, sig: u32, lo: u16, w: u16, mask_hi: u64 },
    /// Logical shifts. `w` is the LEFT operand's width, which is also the
    /// result width (`Value::shift_left`/`shift_right` keep `self.width`);
    /// an amount ≥ w yields 0, matching those helpers exactly. The amount
    /// register is X-free by construction, so the 4-state all-X result for
    /// an X amount cannot arise here.
    /// regs[d] = (regs[s] + k) & mask — the fused constant add. Counters
    /// and address increments make this the most common shape in clocked
    /// bodies, which the islands cover since the edge hook landed.
    AddC { d: u16, s: u16, k: u64, mask: u64 },
    Shl { d: u16, a: u16, b: u16, w: u32, mask: u64 },
    Shr { d: u16, a: u16, b: u16, w: u32 },
    /// Narrow replicate: dst = {count{src}} with count*w ≤ 64.
    Repl { d: u16, s: u16, w: u8, count: u8 },
    /// Wide replicate (result 65..=128 bits) of a NARROW part.
    WRepl { d: u16, s: u16, w: u8, count: u8 },
}

pub struct TwoStateBlock {
    pub insns: Vec<TsInsn>,
    pub num_regs: u32,
    /// Deduped ≤64-bit signals read WHOLE — checked X-free with one
    /// raw_bits call each.
    pub reads_whole: Box<[u32]>,
    /// Deduped (signal, lo, width) slices of WIDE signals — checked via
    /// raw_bits_slice, so unrelated X bits don't force a bail.
    pub reads_slice: Box<[(u32, u16, u16)]>,
    /// Any branch/jump present. Straight-line blocks (the overwhelmingly
    /// common comb shape) run a compact loop without the pc bookkeeping.
    pub has_ctrl: bool,
    /// Any wide (65..=128-bit) op present — routed to the wide executor so
    /// the narrow hot loops keep their code size.
    pub has_wide: bool,
    /// Wide (>64-bit) signals read WHOLE — X-checked via words128_if_clean.
    pub reads_wide: Box<[u32]>,
    /// Signals this block WRITES (sorted, deduped). §9.3.1 force filtering
    /// is per DESTINATION — `write_sig!` drops writes to forced signals and
    /// the two-state stores bypass that check — so an active force only
    /// makes a block unsafe when the block writes a forced signal. Reads of
    /// a forced signal are exact (the forced value lives in the table).
    pub writes: Box<[u32]>,
    /// Array element-store spans `(first_id, count)`: a dynamic element
    /// write can land on any id in the span, so a forced id inside one
    /// disqualifies the block.
    pub writes_span: Box<[(u32, u32)]>,
}

fn ts_mask(w: u32) -> u64 {
    if w >= 64 { u64::MAX } else { (1u64 << w) - 1 }
}

thread_local! {
    /// Opcode the lowering was on when it gave up — the only way to answer
    /// "why is this block not an island?" on a design you cannot read.
    /// Reported by the XZ_TS_DBG dump.
    static TS_BAIL_AT: std::cell::Cell<(usize, &'static str)> =
        const { std::cell::Cell::new((0, "-")) };
}

/// (stream index, opcode) where the last `lower_two_state` call stopped.
pub fn ts_last_bail() -> (usize, &'static str) {
    TS_BAIL_AT.with(|c| c.get())
}

thread_local! {
    /// Fine-grained reason for the last bail, for arms that carry several
    /// distinct gates. The opcode alone was not enough: the gate census
    /// showed `BlockingAssignRange` accounting for 58% of all interpreter
    /// evaluations on the C906 SoC while its DESTINATION passed every
    /// signal-level check, so the deciding condition had to be named from
    /// inside the arm.
    static TS_GATE_WHY: std::cell::Cell<&'static str> =
        const { std::cell::Cell::new("-") };
}

/// Fine-grained gate label for the last `lower_two_state` bail ("-" when the
/// arm does not carry one).
pub fn ts_last_gate() -> &'static str {
    TS_GATE_WHY.with(|c| c.get())
}

pub fn lower_two_state(
    cb: &CompiledBlock,
    signal_widths: &[u32],
    signal_signed: &[bool],
    signal_real: &[bool],
    array_first_id: &HashMap<Arc<str>, (usize, i64, i64)>,
) -> Option<TwoStateBlock> {
    let n = cb.instructions.len();
    let mut out: Vec<TsInsn> = Vec::with_capacity(n);
    // Static width per register. A register REDEFINED with a different width
    // (possible for loop-var slots, which are mutable) bails: stream-order
    // width tracking can't represent it.
    let mut rw: Vec<Option<u32>> = vec![None; cb.num_regs as usize];
    // Constant value per register (from LoadConst), for folding const-index
    // array writes into static element stores. Cleared on any redefinition.
    let mut rc: Vec<Option<u64>> = vec![None; cb.num_regs as usize];
    // Once a side-effecting op is emitted, ABORTABLE ops (dynamic element
    // reads) can no longer be admitted: an abort must leave no trace.
    let mut side_effects = false;
    // Staged with a skip flag: a read of a signal this block has ALREADY
    // STORED (blocking) this evaluation is clean by construction and needs
    // no prefilter entry — but only for STRAIGHT-LINE blocks (a store inside
    // a branch may not execute), so the skip is applied after `has_ctrl` is
    // known. Wide loads BEFORE any side effect are covered by the exec-time
    // abort instead of the prefilter.
    let mut reads_whole: Vec<(u32, bool)> = Vec::new();
    let mut reads_slice: Vec<(u32, u16, u16, bool)> = Vec::new();
    let mut reads_wide: Vec<(u32, bool)> = Vec::new();
    let mut stored: Vec<u32> = Vec::new();
    // idx_map[i] = lowered index of the first lowered insn at-or-after
    // 4-state index i (dropped insns map to their successor). Branch targets
    // are rewritten through it in a fixup pass.
    let mut idx_map: Vec<u32> = Vec::with_capacity(n + 1);
    // ≤64-bit signals are checked whole (one raw_bits each) regardless of
    // which part is read; only WIDE signals get slice entries.
    let mut note_read = |sig: usize,
                         lo: u32,
                         w: u32,
                         narrow: bool,
                         skip: bool,
                         rw_: &mut Vec<(u32, bool)>,
                         rs_: &mut Vec<(u32, u16, u16, bool)>| {
        if narrow {
            let s32 = sig as u32;
            if let Some(e) = rw_.iter_mut().find(|(x, _)| *x == s32) {
                e.1 &= skip;
            } else {
                rw_.push((s32, skip));
            }
        } else {
            let t = (sig as u32, lo as u16, w as u16);
            if let Some(e) = rs_.iter_mut().find(|(a, b, c, _)| (*a, *b, *c) == t) {
                e.3 &= skip;
            } else {
                rs_.push((t.0, t.1, t.2, skip));
            }
        }
    };
    // Whole-value loads need the value in one u64; selects only need the
    // addressed slice, so they accept ANY signal width (the exec reads the
    // slice via raw_bits_slice).
    let sig_ok = |sig: usize| -> bool {
        sig < signal_widths.len() && signal_widths[sig] <= 64 && !signal_real[sig]
    };
    let sig_ok_slice = |sig: usize| -> bool {
        sig < signal_widths.len() && !signal_real[sig]
    };
    let sig_ok_wide = |sig: usize| -> bool {
        sig < signal_widths.len()
            && signal_widths[sig] > 64
            && signal_widths[sig] <= 128
            && !signal_real[sig]
    };
    let wmask_hi = |w: u32| -> u64 { ts_mask(w - 64) };
    // Static array span: (first_id, lo, hi) with elements proven narrow,
    // unsigned, non-real (elements of one array share their declaration;
    // the first element's metadata stands for all).
    let array_span = |a: &ArrayOperand| -> Option<(usize, i64, i64)> {
        let (first, lo, hi) = match a {
            ArrayOperand::Dense { first_id, lo, hi, .. } => (*first_id, *lo, *hi),
            ArrayOperand::Named(name) => {
                let &(first, lo, hi) = array_first_id.get(name.as_str())?;
                (first, lo, hi)
            }
        };
        if hi < lo {
            return None;
        }
        if first >= signal_widths.len()
            || signal_widths[first] > 64
            || signal_signed[first]
            || signal_real[first]
        {
            return None;
        }
        Some((first, lo, hi))
    };
    let clean_const = |k: &Value| -> Option<u64> {
        if k.width > 64 || k.is_real || k.is_fill {
            return None;
        }
        let (v, x) = k.raw_bits();
        if x != 0 {
            return None;
        }
        let v = v & ts_mask(k.width);
        // Bare decimal literals are SIGNED (§5.7.1), so rejecting every
        // signed constant kept `x + 1`, `x >> 3` and `x == 5` — most of the
        // RTL ever written — out of the islands. A NON-NEGATIVE signed
        // constant is admissible: zero- and sign-extension agree on it, and
        // every lowered register is unsigned by construction, so no
        // operation's signedness can flip (§5.5.1). A negative one still
        // bails, since its widening differs.
        if k.is_signed && k.width > 0 && (v >> (k.width - 1)) & 1 == 1 {
            return None;
        }
        Some(v)
    };
    // Stream-order width tracking is EXACT wherever a register's read is
    // reached by exactly ONE of its definitions. Bailing on every
    // differing-width redefinition threw away the `case` shape, where each
    // arm reuses the same temp slot at its own width and every use stays
    // inside its own arm: on the C906 SoC that single rule was the first
    // blocker for 58% of the remaining interpreter-bound evaluations.
    // Instead, allow the redefinition and reject only a read that a BRANCH
    // TARGET separates from its stream-order definition -- the one place a
    // second definition could reach it. `tcount[i]` counts branch targets at
    // indices <= i, so "no target in between" is one integer compare.
    let n_ins = cb.instructions.len();
    let mut tgts: Vec<u32> = Vec::new();
    for insn in cb.instructions.iter() {
        match insn {
            Insn::BranchIfFalse(_, t)
            | Insn::BranchUnlessZero(_, t)
            | Insn::BranchIfSignalFalse(_, t, _)
            | Insn::CmpBranch(_, _, _, _, t)
            | Insn::Jump(t) => tgts.push(*t),
            Insn::CaseJump(_, cj) => {
                tgts.extend(cj.table.iter().copied());
                tgts.push(cj.default);
            }
            Insn::CaseMaskJump(_, mj) => {
                tgts.extend(mj.table.iter().copied());
                tgts.push(mj.xz_path);
            }
            _ => {}
        }
    }
    let mut is_tgt = vec![false; n_ins + 1];
    for t in tgts {
        if (t as usize) <= n_ins {
            is_tgt[t as usize] = true;
        }
    }
    let mut tcount = vec![0u32; n_ins + 1];
    let mut tc_run = 0u32;
    for i in 0..=n_ins {
        if is_tgt[i] {
            tc_run += 1;
        }
        tcount[i] = tc_run;
    }
    // A back edge can re-enter an earlier read from a LATER definition, which
    // the forward-only reasoning above does not model.
    let back_branch = BytecodeCompiler::has_backward_branch(&cb.instructions);
    let mut wconf: Vec<bool> = vec![false; cb.num_regs as usize];
    let mut wconf_list: Vec<RegId> = Vec::new();
    let mut def_tc: Vec<u32> = vec![0; cb.num_regs as usize];
    // Mirror of the loop index, readable from `def!` (a `for` pattern binding
    // is not, due to macro hygiene).
    let mut cur_i: usize = 0;
    // Registers holding a folded 4-state constant: (value plane, xz plane).
    // These registers are NEVER materialized in the u64 file, so any reader
    // outside the fold set would see an unwritten slot -- the guard at the
    // top of the loop rejects the block instead.
    let mut xc: Vec<Option<(u64, u64)>> = vec![None; cb.num_regs as usize];
    let mut xc_live: Vec<RegId> = Vec::new();
    macro_rules! def {
        ($rw:ident, $r:expr, $w:expr) => {{
            let r = $r as usize;
            let w = $w;
            match $rw[r] {
                Some(prev) if prev != w => {
                    if !wconf[r] {
                        wconf[r] = true;
                        wconf_list.push(r as RegId);
                    }
                    $rw[r] = Some(w);
                }
                _ => $rw[r] = Some(w),
            }
            def_tc[r] = tcount[cur_i];
            rc[r] = None;
            xc[r] = None;
        }};
    }
    // Tracking costs one thread-local store per instruction, so it is armed
    // only when the dump is requested.
    static TRACK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let track = *TRACK.get_or_init(|| std::env::var("XEZIM_TS_DBG").is_ok());
    // Runtime opcode bisection: XEZIM_TS_DENY=CaseJump,RedOr,... makes any
    // block containing a listed 4-state opcode bail back to the interpreter,
    // so a suspected two-state miscompile can be isolated without rebuilding.
    static DENY: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    let deny = DENY.get_or_init(|| {
        std::env::var("XEZIM_TS_DENY")
            .map(|v| v.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect())
            .unwrap_or_default()
    });
    // Names the exact condition that rejected an instruction, for arms with
    // more than one gate. Costs nothing unless the dump is armed.
    // A register wider than 64 bits lives in the WIDE plane file, so any
    // TsInsn that reads `regs[r]` must prove the register is narrow first —
    // otherwise it reads whatever the previous evaluation left in the slot.
    macro_rules! narrow_reg {
        ($rw:ident, $r:expr, $why:expr) => {{
            let w = $rw[$r as usize]?;
            if w > 64 {
                gate!($why);
            }
            w
        }};
    }
    macro_rules! gate {
        ($why:expr) => {{
            if track {
                TS_GATE_WHY.with(|c| c.set($why));
            }
            return None;
        }};
    }
    for (ins_i, insn) in cb.instructions.iter().enumerate() {
        if track {
            TS_BAIL_AT.with(|c| c.set((ins_i, insn_opcode_name(insn))));
            TS_GATE_WHY.with(|c| c.set("-"));
        }
        if !deny.is_empty() && deny.iter().any(|d| d == insn_opcode_name(insn)) {
            return None;
        }
        cur_i = ins_i;
        if !wconf_list.is_empty() {
            if back_branch {
                gate!("reg width phi (back edge)");
            }
            let tc = tcount[cur_i];
            if wconf_list
                .iter()
                .any(|&r| tc != def_tc[r as usize]
                    && BytecodeCompiler::insn_reads_reg(insn, r))
            {
                gate!("reg width phi");
            }
        }
        if !xc_live.is_empty()
            && !matches!(
                insn,
                Insn::Replicate(..)
                    | Insn::Move(..)
                    | Insn::BlockingAssign(..)
                    | Insn::BlockingAssignRange(..)
            )
            && xc_live
                .iter()
                .any(|&r| xc[r as usize].is_some() && BytecodeCompiler::insn_reads_reg(insn, r))
        {
            gate!("x-const consumed");
        }
        idx_map.push(out.len() as u32);
        match insn {
            Insn::Nop => {}
            // No-op here: every lowered register is unsigned by construction
            // (signed sources bail below).
            Insn::ClearSigned(_) => {}
            Insn::LoadSignal(d, sig) => {
                let sig = *sig as usize;
                if signal_signed.get(sig).copied().unwrap_or(true) {
                    return None;
                }
                if sig_ok(sig) {
                    note_read(sig, 0, signal_widths[sig], true, stored.contains(&(sig as u32)), &mut reads_whole, &mut reads_slice);
                    def!(rw, *d, signal_widths[sig]);
                    out.push(TsInsn::LoadSig { d: *d as u16, sig: sig as u32 });
                } else if sig_ok_wide(sig) {
                    let skip = !side_effects || stored.contains(&(sig as u32));
                    let s32 = sig as u32;
                    if let Some(e) = reads_wide.iter_mut().find(|(x, _)| *x == s32) {
                        e.1 &= skip;
                    } else {
                        reads_wide.push((s32, skip));
                    }
                    def!(rw, *d, signal_widths[sig]);
                    out.push(TsInsn::WLoadSig { d: *d as u16, sig: sig as u32 });
                } else {
                    return None;
                }
            }
            Insn::LoadConst(d, k) => {
                if k.width > 64 {
                    if k.width > 128 || k.is_signed {
                        return None;
                    }
                    let mut w2 = [0u64; 2];
                    if !k.words128_if_clean(&mut w2) {
                        return None;
                    }
                    def!(rw, *d, k.width);
                    out.push(TsInsn::WConst { d: *d as u16, v: Box::new(w2) });
                } else if k.has_xz() {
                    // Fold rather than reject: admitted only while it stays
                    // a constant all the way to a store (see the loop guard).
                    if k.is_signed || k.is_real || k.is_fill {
                        gate!("x-const signed/real");
                    }
                    let Some((v, x)) = k.inline_bits() else {
                        gate!("x-const not inline");
                    };
                    def!(rw, *d, k.width);
                    let m = ts_mask(k.width);
                    xc[*d as usize] = Some((v & m, x & m));
                    xc_live.push(*d);
                } else {
                    let v = clean_const(k)?;
                    def!(rw, *d, k.width);
                    rc[*d as usize] = Some(v);
                    out.push(TsInsn::Const { d: *d as u16, v });
                }
            }
            Insn::LoadSignalBit(d, sig, idx) => {
                let sig = *sig as usize;
                if !sig_ok_slice(sig) || *idx >= signal_widths[sig] || *idx > u16::MAX as u32 {
                    return None;
                }
                let narrow = signal_widths[sig] <= 64;
                note_read(sig, *idx, 1, narrow, stored.contains(&(sig as u32)), &mut reads_whole, &mut reads_slice);
                def!(rw, *d, 1);
                out.push(if narrow {
                    TsInsn::SigBit { d: *d as u16, sig: sig as u32, bit: *idx as u16 }
                } else {
                    TsInsn::SigBitW { d: *d as u16, sig: sig as u32, bit: *idx as u16 }
                });
            }
            Insn::LoadSignalRange(d, sig, l, r) => {
                let sig = *sig as usize;
                let (hi, lo) = (*l.max(r), *l.min(r));
                if !sig_ok_slice(sig) || hi >= signal_widths[sig] || hi > u16::MAX as u32 {
                    return None;
                }
                let w = hi - lo + 1;
                if w > 64 {
                    if w > 128 {
                        return None;
                    }
                    // Wide slice of a signal: prefilter the two ≤64 halves.
                    note_read(sig, lo, 64, false, stored.contains(&(sig as u32)), &mut reads_whole, &mut reads_slice);
                    note_read(sig, lo + 64, w - 64, false, stored.contains(&(sig as u32)), &mut reads_whole, &mut reads_slice);
                    def!(rw, *d, w);
                    out.push(TsInsn::WSigRange {
                        d: *d as u16,
                        sig: sig as u32,
                        lo: lo as u16,
                        w: w as u16,
                        mask_hi: wmask_hi(w),
                    });
                    continue;
                }
                let narrow = signal_widths[sig] <= 64;
                note_read(sig, lo, w, narrow, stored.contains(&(sig as u32)), &mut reads_whole, &mut reads_slice);
                def!(rw, *d, w);
                out.push(if narrow {
                    TsInsn::SigRange {
                        d: *d as u16,
                        sig: sig as u32,
                        lo: lo as u16,
                        mask: ts_mask(w),
                    }
                } else {
                    TsInsn::SigRangeW {
                        d: *d as u16,
                        sig: sig as u32,
                        lo: lo as u16,
                        w: w as u16,
                        mask: ts_mask(w),
                    }
                });
            }
            Insn::BitSelectConst(d, s, idx) => {
                let sw = rw[*s as usize]?;
                if *idx >= sw || *idx > 127 {
                    return None;
                }
                def!(rw, *d, 1);
                out.push(if sw > 64 {
                    TsInsn::BitFromW { d: *d as u16, s: *s as u16, bit: *idx as u16 }
                } else {
                    TsInsn::Bit { d: *d as u16, s: *s as u16, bit: *idx as u8 }
                });
            }
            Insn::RangeSelectConst(d, s, l, r) => {
                let sw = rw[*s as usize]?;
                let (hi, lo) = (*l.max(r), *l.min(r));
                if hi >= sw || hi > 127 {
                    return None;
                }
                let w = hi - lo + 1;
                def!(rw, *d, w);
                out.push(if sw > 64 {
                    if w > 64 {
                        TsInsn::WRange {
                            d: *d as u16,
                            s: *s as u16,
                            lo: lo as u16,
                            mask_hi: wmask_hi(w),
                        }
                    } else {
                        TsInsn::RangeFromW {
                            d: *d as u16,
                            s: *s as u16,
                            lo: lo as u16,
                            mask: ts_mask(w),
                        }
                    }
                } else {
                    TsInsn::Range {
                        d: *d as u16,
                        s: *s as u16,
                        lo: lo as u8,
                        mask: ts_mask(w),
                    }
                });
            }
            // A plain register copy. Lowered as `d = s | s`, which is exact
            // (`x | x == x`) and needs no new opcode or executor arm — the
            // wide bank has the same identity. Worth an arm at all because
            // the TS bail census measured `Move` gating 43.8M interpreter
            // evaluations on the C906 SoC (14.1% of all of them): the block
            // was not doing anything two-state could not express, it just
            // had no lowering.
            Insn::Move(d, s) => {
                let w = rw[*s as usize]?;
                if let Some(k) = xc[*s as usize] {
                    def!(rw, *d, w);
                    xc[*d as usize] = Some(k);
                    xc_live.push(*d);
                    continue;
                }
                def!(rw, *d, w);
                let (d, s) = (*d as u16, *s as u16);
                out.push(if w > 64 {
                    TsInsn::WOr { d, a: s, b: s }
                } else {
                    TsInsn::Or { d, a: s, b: s }
                });
            }
            // §11.4.11 conditional. The TS bail census measured `Select`
            // gating 57.8M interpreter evaluations (18.6%) — muxes are
            // everywhere in RTL and nothing about them resists two-state.
            // Both branches must share a width: the 4-state arm yields the
            // CHOSEN branch's own width, and a downstream `Not`/`Range` masks
            // by the register width, so adopting the max would change those
            // results. Unequal branches bail (conservative, still covers the
            // ordinary same-width mux).
            Insn::Select(d, c, a, b) => {
                let (wc, wa, wb) = (
                    rw[*c as usize]?,
                    rw[*a as usize]?,
                    rw[*b as usize]?,
                );
                if wa != wb || wa > 64 || wc > 64 {
                    return None;
                }
                def!(rw, *d, wa);
                out.push(TsInsn::Sel {
                    d: *d as u16,
                    c: *c as u16,
                    a: *a as u16,
                    b: *b as u16,
                });
            }
            Insn::BitXor(d, a, b) | Insn::BitAnd(d, a, b) | Insn::BitOr(d, a, b) => {
                let (wa, wb) = (rw[*a as usize]?, rw[*b as usize]?);
                let (aw, bw) = (wa > 64, wb > 64);
                if aw != bw {
                    // Mixed-bank operands — bail rather than model the
                    // zero-extension across banks.
                    return None;
                }
                def!(rw, *d, wa.max(wb));
                let (d, a, b) = (*d as u16, *a as u16, *b as u16);
                out.push(if aw {
                    match insn {
                        Insn::BitXor(..) => TsInsn::WXor { d, a, b },
                        Insn::BitAnd(..) => TsInsn::WAnd { d, a, b },
                        _ => TsInsn::WOr { d, a, b },
                    }
                } else {
                    match insn {
                        Insn::BitXor(..) => TsInsn::Xor { d, a, b },
                        Insn::BitAnd(..) => TsInsn::And { d, a, b },
                        _ => TsInsn::Or { d, a, b },
                    }
                });
            }
            Insn::BitNot(d, s) => {
                let w = rw[*s as usize]?;
                def!(rw, *d, w);
                out.push(if w > 64 {
                    if w > 128 {
                        return None;
                    }
                    TsInsn::WNot { d: *d as u16, s: *s as u16, mask_hi: wmask_hi(w) }
                } else {
                    TsInsn::Not { d: *d as u16, s: *s as u16, mask: ts_mask(w) }
                });
            }
            // Wrapping at max operand width; zero-extension is the correct
            // §11.8.1 widening because both registers are unsigned.
            Insn::Add(d, a, b) | Insn::Sub(d, a, b) => {
                let (wa, wb) = (
                    narrow_reg!(rw, *a, "wide operand (add/sub)"),
                    narrow_reg!(rw, *b, "wide operand (add/sub)"),
                );
                let w = wa.max(wb);
                def!(rw, *d, w);
                let (d, a, b) = (*d as u16, *a as u16, *b as u16);
                out.push(if matches!(insn, Insn::Add(..)) {
                    TsInsn::Add { d, a, b, mask: ts_mask(w) }
                } else {
                    TsInsn::Sub { d, a, b, mask: ts_mask(w) }
                });
            }
            // §11.4.10 logical shifts. Result width = LEFT operand width.
            // AShr is excluded: every lowered register is unsigned, so an
            // arithmetic shift would need sign tracking the bank lacks.
            Insn::Shl(d, a, b) | Insn::Shr(d, a, b) => {
                let wa = narrow_reg!(rw, *a, "wide operand (shift)");
                narrow_reg!(rw, *b, "wide shift amount");
                def!(rw, *d, wa);
                let (d, a, b) = (*d as u16, *a as u16, *b as u16);
                out.push(if matches!(insn, Insn::Shl(..)) {
                    TsInsn::Shl { d, a, b, w: wa, mask: ts_mask(wa) }
                } else {
                    TsInsn::Shr { d, a, b, w: wa }
                });
            }
            // 4-state table semantics (x/z selector -> default) cannot lower
            // to the 2-state pipeline. `CaseJump` is the exception and is
            // handled below, as is `CaseMaskJump`: the eval-site prefilter
            // proves every signal the block reads is X-free, so their selectors
            // cannot carry x/z and the "no pattern matches -> default" /
            // wildcard-chain branches are unreachable, leaving a plain
            // bounds-checked table index.
            Insn::CaseLut(..) => return None,
            Insn::Format(..) => return None,
            Insn::StrOp(..) => return None,
            Insn::BlockingAssignString(..) => return None,
            // 1-bit results. CaseEq/CaseNeq equal Eq/Neq on X-free values.
            Insn::Eq(d, a, b) | Insn::CaseEq(d, a, b) => {
                narrow_reg!(rw, *a, "wide operand (eq)");
                narrow_reg!(rw, *b, "wide operand (eq)");
                def!(rw, *d, 1);
                out.push(TsInsn::Eq { d: *d as u16, a: *a as u16, b: *b as u16 });
            }
            Insn::Neq(d, a, b) => {
                narrow_reg!(rw, *a, "wide operand (neq)");
                narrow_reg!(rw, *b, "wide operand (neq)");
                def!(rw, *d, 1);
                out.push(TsInsn::Neq { d: *d as u16, a: *a as u16, b: *b as u16 });
            }
            // §5.5.1: unsigned compare when either operand is unsigned —
            // always here, since every lowered register is unsigned.
            Insn::Lt(d, a, b) | Insn::Leq(d, a, b) | Insn::Gt(d, a, b)
            | Insn::Geq(d, a, b) => {
                narrow_reg!(rw, *a, "wide operand (cmp)");
                narrow_reg!(rw, *b, "wide operand (cmp)");
                def!(rw, *d, 1);
                let (d, a, b) = (*d as u16, *a as u16, *b as u16);
                out.push(match insn {
                    Insn::Lt(..) => TsInsn::Lt { d, a, b },
                    Insn::Leq(..) => TsInsn::Leq { d, a, b },
                    Insn::Gt(..) => TsInsn::Gt { d, a, b },
                    _ => TsInsn::Geq { d, a, b },
                });
            }
            Insn::LogNot(d, s) => {
                narrow_reg!(rw, *s, "wide operand (lognot)");
                def!(rw, *d, 1);
                out.push(TsInsn::LogNot { d: *d as u16, s: *s as u16 });
            }
            Insn::LogAnd(d, a, b) | Insn::LogOr(d, a, b) => {
                narrow_reg!(rw, *a, "wide operand (logic)");
                narrow_reg!(rw, *b, "wide operand (logic)");
                def!(rw, *d, 1);
                let (d, a, b) = (*d as u16, *a as u16, *b as u16);
                out.push(if matches!(insn, Insn::LogAnd(..)) {
                    TsInsn::LogAnd { d, a, b }
                } else {
                    TsInsn::LogOr { d, a, b }
                });
            }
            Insn::BinOpConstAdd2(a) => {
                // Exactly the two `AddC` lowerings the pair had before
                // merging — in order, so a chained `s2 == d1` sees d1's
                // freshly defined width.
                let w1 = narrow_reg!(rw, a.s1, "wide operand (addc2)");
                let v1 = clean_const(&a.k1)?;
                let wr1 = w1.max(a.k1.width);
                def!(rw, a.d1, wr1);
                out.push(TsInsn::AddC {
                    d: a.d1 as u16,
                    s: a.s1 as u16,
                    k: v1,
                    mask: ts_mask(wr1),
                });
                let w2 = narrow_reg!(rw, a.s2, "wide operand (addc2)");
                let v2 = clean_const(&a.k2)?;
                let wr2 = w2.max(a.k2.width);
                def!(rw, a.d2, wr2);
                out.push(TsInsn::AddC {
                    d: a.d2 as u16,
                    s: a.s2 as u16,
                    k: v2,
                    mask: ts_mask(wr2),
                });
            }
            Insn::BinOpConst(d, s, k, kind) => {
                let w = narrow_reg!(rw, *s, "wide operand (binop-const)");
                let v = clean_const(k)?;
                match kind {
                    BinOpConstKind::Xor => {
                        def!(rw, *d, w.max(k.width));
                        out.push(TsInsn::XorC { d: *d as u16, s: *s as u16, k: v });
                    }
                    BinOpConstKind::Eq | BinOpConstKind::CaseEq => {
                        def!(rw, *d, 1);
                        out.push(TsInsn::EqC { d: *d as u16, s: *s as u16, k: v });
                    }
                    BinOpConstKind::Add => {
                        let wr = w.max(k.width);
                        def!(rw, *d, wr);
                        out.push(TsInsn::AddC {
                            d: *d as u16,
                            s: *s as u16,
                            k: v,
                            mask: ts_mask(wr),
                        });
                    }
                }
            }
            Insn::Concat(d, parts) => {
                let mut total = 0u32;
                let mut any_wide = false;
                let mut lowered: Vec<(u16, u8, bool)> = Vec::with_capacity(parts.len());
                for &p in parts.iter() {
                    let w = rw[p as usize]?;
                    if w > 128 {
                        return None;
                    }
                    total += w;
                    if w > 64 {
                        any_wide = true;
                    }
                    lowered.push((p as u16, w as u8, w > 64));
                }
                if total == 0 || total > 128 {
                    return None;
                }
                def!(rw, *d, total);
                out.push(if total > 64 || any_wide {
                    TsInsn::WConcat { d: *d as u16, parts: lowered.into_boxed_slice() }
                } else {
                    TsInsn::Concat {
                        d: *d as u16,
                        parts: lowered
                            .into_iter()
                            .map(|(r, w, _)| (r, w))
                            .collect::<Vec<_>>()
                            .into_boxed_slice(),
                    }
                });
            }
            Insn::Resize(r, w) => {
                let cur = rw[*r as usize]?;
                if *w > 128 {
                    return None;
                }
                if *w > 64 {
                    if cur <= 64 {
                        out.push(TsInsn::WFromN { r: *r as u16 });
                    } else if *w < cur {
                        out.push(TsInsn::WMask { d: *r as u16, mask_hi: wmask_hi(*w) });
                    }
                    rw[*r as usize] = Some(*w);
                    def_tc[*r as usize] = tcount[cur_i];
                    rc[*r as usize] = None;
                    continue;
                }
                if cur > 64 {
                    out.push(TsInsn::NFromW { r: *r as u16, mask: ts_mask(*w) });
                    rw[*r as usize] = Some(*w);
                    def_tc[*r as usize] = tcount[cur_i];
                    rc[*r as usize] = None;
                    continue;
                }
                if *w < cur {
                    out.push(TsInsn::Mask { d: *r as u16, mask: ts_mask(*w) });
                }
                // Widening zero-extends — free for an unsigned register.
                // Redefinition width-conflict does not apply: Resize is a
                // width CHANGE of the same value, not a second definition.
                // It IS the point the width now dates from, though, so the
                // phi guard measures from here (without marking a conflict,
                // which would newly reject blocks that lower correctly today).
                rw[*r as usize] = Some(*w);
                def_tc[*r as usize] = tcount[cur_i];
                // Keep const knowledge in sync with the (possible) truncation.
                if let Some(k) = rc[*r as usize] {
                    rc[*r as usize] = Some(k & ts_mask(*w));
                }
            }
            Insn::BranchIfSignalFalse(sig, t, bit) => {
                let sig = *sig as usize;
                if *bit == u32::MAX {
                    // Whole-value truthiness needs the value in one word.
                    if !sig_ok(sig) {
                        return None;
                    }
                    note_read(sig, 0, signal_widths[sig], true, stored.contains(&(sig as u32)), &mut reads_whole, &mut reads_slice);
                } else {
                    if !sig_ok_slice(sig)
                        || *bit >= signal_widths[sig]
                        || *bit > u16::MAX as u32
                    {
                        return None;
                    }
                    let narrow = signal_widths[sig] <= 64;
                    note_read(sig, *bit, 1, narrow, stored.contains(&(sig as u32)), &mut reads_whole, &mut reads_slice);
                }
                out.push(TsInsn::BrSigFalse { sig: sig as u32, bit: *bit, t: *t });
            }
            Insn::BranchIfFalse(c, t) => {
                narrow_reg!(rw, *c, "wide branch condition");
                out.push(TsInsn::BrFalse { s: *c as u16, t: *t });
            }
            // Fused compare+branch: decompose to the exact unfused lowering,
            // reusing the embedded dead register as the compare scratch.
            Insn::CmpBranch(kind, a, b, tmp, t) => {
                narrow_reg!(rw, *a, "wide operand (cmpbranch)");
                narrow_reg!(rw, *b, "wide operand (cmpbranch)");
                def!(rw, *tmp, 1);
                let (d, a, b) = (*tmp as u16, *a as u16, *b as u16);
                out.push(match kind {
                    CmpKind::Eq | CmpKind::CaseEq => TsInsn::Eq { d, a, b },
                    CmpKind::Neq => TsInsn::Neq { d, a, b },
                    CmpKind::Lt => TsInsn::Lt { d, a, b },
                    CmpKind::Leq => TsInsn::Leq { d, a, b },
                    CmpKind::Gt => TsInsn::Gt { d, a, b },
                    CmpKind::Geq => TsInsn::Geq { d, a, b },
                });
                out.push(TsInsn::BrFalse { s: d, t: *t });
            }
            Insn::BranchUnlessZero(s, t) => {
                narrow_reg!(rw, *s, "wide branch condition");
                out.push(TsInsn::BrNz { s: *s as u16, t: *t });
            }
            Insn::Jump(t) => {
                out.push(TsInsn::Jmp { t: *t });
            }
            Insn::CaseJump(src, cj) => {
                // The selector must live in the u64 register file: a wider
                // one is kept in the wide plane file, where `regs[s]` is not
                // its value at all.
                let sw = rw[*src as usize]?;
                if sw > 64 {
                    gate!("casejump sel >64b");
                }
                out.push(TsInsn::CaseJmp { s: *src as u16, cj: cj.clone() });
            }
            Insn::CaseMaskJump(src, mj) => {
                let sw = rw[*src as usize]?;
                if sw > 64 {
                    gate!("casemaskjump sel >64b");
                }
                if mj.lo + mj.width > 64 {
                    gate!("casemaskjump window >=64");
                }
                out.push(TsInsn::CaseMaskJmp {
                    s: *src as u16,
                    mask: ts_mask(sw),
                    lo: mj.lo,
                    wmask: ts_mask(mj.width),
                    mj: mj.clone(),
                });
            }
            Insn::ReduceOr(d, src) => {
                // Registers wider than 64 bits live in the WIDE plane file;
                // `regs[s]` for such a register is whatever the previous
                // block's evaluation left in that slot. Emitting the narrow
                // form regardless of width made `|wide_bus` evaluate from
                // stale garbage — on the C910 SoC that wedged the LSU after
                // exactly 250 retired instructions (the first `|128-bit`
                // control reduction on the store path).
                let sw = rw[*src as usize]?;
                def!(rw, *d, 1);
                out.push(if sw > 64 {
                    TsInsn::WRedOr { d: *d as u16, s: *src as u16 }
                } else {
                    TsInsn::RedOr { d: *d as u16, s: *src as u16 }
                });
            }
            Insn::ReduceAnd(d, src) => {
                let sw = rw[*src as usize]?;
                def!(rw, *d, 1);
                out.push(if sw > 64 {
                    TsInsn::WRedAnd {
                        d: *d as u16,
                        s: *src as u16,
                        mask_hi: wmask_hi(sw),
                    }
                } else {
                    TsInsn::RedAnd {
                        d: *d as u16,
                        s: *src as u16,
                        mask: ts_mask(sw),
                    }
                });
            }
            Insn::BlockingAssignBitDyn(sig, idx, r) => {
                let sig = *sig as usize;
                if sig >= signal_widths.len() || signal_real[sig] {
                    gate!("dest oob/real");
                }
                if signal_widths[sig] > 64 {
                    gate!("dest >64b");
                }
                narrow_reg!(rw, *idx, "wide bit index");
                narrow_reg!(rw, *r, "wide store source");
                side_effects = true;
                stored.push(sig as u32);
                out.push(TsInsn::BitStoreDyn {
                    sig: sig as u32,
                    i: *idx as u16,
                    s: *r as u16,
                    w: signal_widths[sig],
                });
            }
            Insn::BlockingAssign(sig, r, w) => {
                let sig = *sig as usize;
                let cw = rw[*r as usize]?;
                if let Some((v, x)) = xc[*r as usize] {
                    // §10.4.1 with a folded constant source. Every lowered
                    // register is unsigned, so a narrow source zero-extends:
                    // masking to the narrower of the two widths IS the resize.
                    if sig >= signal_widths.len()
                        || signal_widths[sig] != *w
                        || signal_real[sig]
                        || *w > 64
                    {
                        gate!("x-const dest shape");
                    }
                    let m = ts_mask(cw.min(*w));
                    side_effects = true;
                    stored.push(sig as u32);
                    out.push(TsInsn::ConstStoreX { sig: sig as u32, v: v & m, x: x & m });
                    continue;
                }
                // Same-width, non-real destination only: the 4-state slow
                // path's fit/resize semantics are not reproduced here.
                if sig >= signal_widths.len() || signal_widths[sig] != *w || signal_real[sig] {
                    return None;
                }
                if *w > 64 {
                    // Wide store: register and destination agree exactly (a
                    // Resize precedes otherwise); signed wide targets bail.
                    if *w > 128 || cw != *w || signal_signed[sig] {
                        return None;
                    }
                    side_effects = true;
                    stored.push(sig as u32);
                    out.push(TsInsn::WStore { sig: sig as u32, s: *r as u16 });
                } else {
                    if cw > 64 {
                        return None;
                    }
                    side_effects = true;
                    stored.push(sig as u32);
                    out.push(TsInsn::Store { sig: sig as u32, s: *r as u16, mask: ts_mask(*w) });
                }
            }
            Insn::NbaAssignConst(sig, k, w) => {
                let sig = *sig as usize;
                if sig >= signal_widths.len() || signal_real[sig] || *w > 64 {
                    return None;
                }
                let v = clean_const(k)?;
                side_effects = true;
                out.push(TsInsn::ConstStoreNba {
                    sig: sig as u32,
                    v: v & ts_mask(*w),
                    w: *w,
                });
            }
            Insn::BlockingAssignRange(sig, hi, lo, r) => {
                let sig = *sig as usize;
                if sig >= signal_widths.len() || signal_real[sig] {
                    gate!("dest oob/real");
                }
                if let Some((v, x)) = xc[*r as usize] {
                    if signal_widths[sig] > 64 {
                        gate!("dest >64b");
                    }
                    let (low, high) = if hi >= lo { (*lo, *hi) } else { (*hi, *lo) };
                    if high >= 64 {
                        gate!("range top >=64");
                    }
                    let m = ts_mask(high - low + 1);
                    side_effects = true;
                    stored.push(sig as u32);
                    out.push(TsInsn::RangeStoreX {
                        sig: sig as u32,
                        hi: high,
                        lo: low,
                        v: v & m,
                        x: x & m,
                    });
                    continue;
                }
                if signal_widths[sig] > 64 {
                    gate!("dest >64b");
                }
                let Some(cw) = rw[*r as usize] else {
                    gate!("src reg width unknown");
                };
                let (low, high) = if hi >= lo { (*lo, *hi) } else { (*hi, *lo) };
                let w = high - low + 1;
                if cw > 64 {
                    gate!("src reg >64b");
                }
                if high >= 64 {
                    gate!("range top >=64");
                }
                side_effects = true;
                stored.push(sig as u32);
                out.push(TsInsn::RangeStore {
                    sig: sig as u32,
                    hi: high,
                    lo: low,
                    s: *r as u16,
                    mask: ts_mask(w),
                });
            }
            Insn::NbaAssignRange(sig, hi, lo, r) => {
                let sig = *sig as usize;
                if sig >= signal_widths.len()
                    || signal_real[sig]
                    || signal_widths[sig] > 64
                {
                    return None;
                }
                let cw = rw[*r as usize]?;
                let (low, high) = if hi >= lo { (*lo, *hi) } else { (*hi, *lo) };
                let w = high - low + 1;
                if cw > 64 || high >= 64 {
                    return None;
                }
                side_effects = true;
                out.push(TsInsn::RangeStoreNba {
                    sig: sig as u32,
                    hi: high,
                    lo: low,
                    s: *r as u16,
                    mask: ts_mask(w),
                });
            }
            Insn::NbaAssign(sig, r, w) => {
                let sig = *sig as usize;
                let cw = rw[*r as usize]?;
                if sig >= signal_widths.len() || signal_real[sig] {
                    return None;
                }
                if *w > 64 {
                    if *w > 128 || cw != *w || signal_signed[sig] {
                        return None;
                    }
                    side_effects = true;
                    out.push(TsInsn::WStoreNba { sig: sig as u32, s: *r as u16, w: *w });
                } else {
                    if cw > 64 {
                        return None;
                    }
                    side_effects = true;
                    out.push(TsInsn::StoreNba {
                        sig: sig as u32,
                        s: *r as u16,
                        w: *w,
                        mask: ts_mask(*w),
                    });
                }
            }
            Insn::LoadArrayElem(d, array, idx_reg) => {
                // Abortable (X/out-of-range element read) — only admissible
                // while nothing side-effecting has run.
                if side_effects {
                    return None;
                }
                let (first, lo, hi) = array_span(array)?;
                narrow_reg!(rw, *idx_reg, "wide array index");
                let w = signal_widths[first];
                def!(rw, *d, w);
                out.push(TsInsn::ElemLoad(Box::new(TsElemOp {
                    first: first as u32,
                    lo,
                    hi,
                    idx: *idx_reg as u16,
                    s: *d as u16,
                    w: 0,
                    mask: 0,
                })));
            }
            Insn::NbaAssignArray(array, idx_reg, val_reg, w) => {
                let (first, lo, hi) = array_span(array)?;
                narrow_reg!(rw, *val_reg, "wide store source");
                if *w > 64 || signal_widths[first] != *w {
                    return None;
                }
                side_effects = true;
                if let Some(k) = rc[*idx_reg as usize] {
                    // Const index folds to a static element target (an
                    // out-of-range constant mirrors the silent 4-state drop
                    // by emitting nothing).
                    let ki = k as i64;
                    if ki >= lo && ki <= hi {
                        let eid = first + (ki - lo) as usize;
                        out.push(TsInsn::StoreNba {
                            sig: eid as u32,
                            s: *val_reg as u16,
                            w: *w,
                            mask: ts_mask(*w),
                        });
                    }
                } else {
                    narrow_reg!(rw, *idx_reg, "wide array index");
                    out.push(TsInsn::ElemStoreNba(Box::new(TsElemOp {
                        first: first as u32,
                        lo,
                        hi,
                        idx: *idx_reg as u16,
                        s: *val_reg as u16,
                        w: *w,
                        mask: ts_mask(*w),
                    })));
                }
            }
            Insn::BlockingAssignArray(array, idx_reg, val_reg, w) => {
                let (first, lo, hi) = array_span(array)?;
                narrow_reg!(rw, *val_reg, "wide store source");
                if *w > 64 || signal_widths[first] != *w {
                    return None;
                }
                side_effects = true;
                if let Some(k) = rc[*idx_reg as usize] {
                    let ki = k as i64;
                    if ki >= lo && ki <= hi {
                        let eid = first + (ki - lo) as usize;
                        out.push(TsInsn::Store {
                            sig: eid as u32,
                            s: *val_reg as u16,
                            mask: ts_mask(*w),
                        });
                    }
                } else {
                    narrow_reg!(rw, *idx_reg, "wide array index");
                    out.push(TsInsn::ElemStore(Box::new(TsElemOp {
                        first: first as u32,
                        lo,
                        hi,
                        idx: *idx_reg as u16,
                        s: *val_reg as u16,
                        w: *w,
                        mask: ts_mask(*w),
                    })));
                }
            }
            Insn::NbaAssignArrayRead(dst, array, idx_sig, w) => {
                let (first, lo, hi) = array_span(array)?;
                let isig = *idx_sig as usize;
                let d = *dst as usize;
                if !sig_ok(isig)
                    || *w > 64
                    || d >= signal_widths.len()
                    || signal_real[d]
                {
                    return None;
                }
                // Historically the read was ABORTABLE (X index, X data,
                // out-of-range), so it had to precede every side effect: a
                // mid-block bail replays the block 4-state, and queued NBAs
                // would double-apply. Two of the three abort sources are now
                // gone — X element DATA is queued as a 4-state Value (the NBA
                // queue holds Values; see the executor), and an X INDEX is
                // impossible when the guard covers `isig`. When the array
                // also spans the index's whole range (RAM with power-of-two
                // depth: lo == 0, hi >= 2^idx_w - 1), out-of-range is
                // statically impossible too — the read is then NON-abortable
                // and may follow side effects. Anything else keeps the old
                // ordering rule. c906: 536 edge blocks bailed here.
                let idx_w = signal_widths[isig];
                let statically_in_range = lo <= 0
                    && idx_w < 63
                    && hi >= (1i64 << idx_w) - 1
                    && !stored.contains(&(isig as u32));
                if side_effects && !statically_in_range {
                    return None;
                }
                note_read(isig, 0, signal_widths[isig], true, stored.contains(&(isig as u32)), &mut reads_whole, &mut reads_slice);
                side_effects = true;
                out.push(TsInsn::NbaFromElem(Box::new(TsNbaFromElem {
                    dst: *dst,
                    first: first as u32,
                    lo,
                    hi,
                    idx_sig: isig as u32,
                    w: *w,
                })));
            }
            Insn::Mul(d, a, b) => {
                let (wa, wb) = (rw[*a as usize]?, rw[*b as usize]?);
                if wa > 64 || wb > 64 {
                    return None;
                }
                let w = wa.max(wb);
                def!(rw, *d, w);
                if let (Some(ka), Some(kb)) = (rc[*a as usize], rc[*b as usize]) {
                    // Both operands are known constants (a genvar expression
                    // the compiler left unfolded): fold at lower time.
                    let v = ka.wrapping_mul(kb) & ts_mask(w);
                    rc[*d as usize] = Some(v);
                    out.push(TsInsn::Const { d: *d as u16, v });
                } else {
                    out.push(TsInsn::Mul {
                        d: *d as u16,
                        a: *a as u16,
                        b: *b as u16,
                        mask: ts_mask(w),
                    });
                }
            }
            Insn::Replicate(d, src, n) => {
                let sw = rw[*src as usize]?;
                let n = *n;
                if n == 0 || sw > 64 || n > 128 {
                    return None;
                }
                let total = sw.saturating_mul(n);
                if let Some((v, x)) = xc[*src as usize] {
                    if total > 64 {
                        gate!("x-const repl >64b");
                    }
                    let (mut rv, mut rx) = (0u64, 0u64);
                    for i in 0..n {
                        rv |= v << (i * sw);
                        rx |= x << (i * sw);
                    }
                    def!(rw, *d, total);
                    xc[*d as usize] = Some((rv, rx));
                    xc_live.push(*d);
                    continue;
                }
                if total == 0 || total > 128 {
                    return None;
                }
                def!(rw, *d, total);
                out.push(if total > 64 {
                    TsInsn::WRepl {
                        d: *d as u16,
                        s: *src as u16,
                        w: sw as u8,
                        count: n as u8,
                    }
                } else {
                    TsInsn::Repl {
                        d: *d as u16,
                        s: *src as u16,
                        w: sw as u8,
                        count: n as u8,
                    }
                });
            }
            _ => return None,
        }
    }
    idx_map.push(out.len() as u32);
    // Fixup: branch targets were recorded as 4-state indices.
    for insn in out.iter_mut() {
        match insn {
            TsInsn::BrSigFalse { t, .. }
            | TsInsn::BrFalse { t, .. }
            | TsInsn::BrNz { t, .. }
            | TsInsn::Jmp { t } => {
                let old = *t as usize;
                if old >= idx_map.len() {
                    return None;
                }
                *t = idx_map[old];
            }
            TsInsn::CaseMaskJmp { mj, .. } => {
                // `xz_path` is unreachable on X-free registers but is still
                // remapped: leaving a 4-state index in a lowered stream would
                // be a live landmine if the invariant ever weakened.
                for t in mj.table.iter_mut().chain(std::iter::once(&mut mj.xz_path)) {
                    let old = *t as usize;
                    if old >= idx_map.len() {
                        return None;
                    }
                    *t = idx_map[old];
                }
            }
            TsInsn::CaseJmp { cj, .. } => {
                for t in cj.table.iter_mut().chain(std::iter::once(&mut cj.default)) {
                    let old = *t as usize;
                    if old >= idx_map.len() {
                        return None;
                    }
                    *t = idx_map[old];
                }
            }
            _ => {}
        }
    }
    // A block that stores nothing is useless to run in 2-state.
    //
    // This list must name EVERY store opcode. It did not: the partial-range
    // and constant NBA stores were absent, so a block whose only side effect
    // was a range store lowered perfectly and was then discarded here as
    // "stores nothing". On the C906 SoC that silently rejected 176.4M
    // interpreter evaluations — 58.4% of all of them — in blocks of exactly
    // the shape `LoadSignalRange(r, wide, 127, 64); BlockingAssignRange(dst,
    // 63, 0, r)` (wide-bus word shuffles). Found by labelling the arm's own
    // gates and seeing that NONE of them fired: the bail was here, after the
    // instruction loop, with the stamp merely naming the last instruction
    // entered.
    if !out
        .iter()
        .any(|i| {
            matches!(
                i,
                TsInsn::Store { .. }
                    | TsInsn::StoreNba { .. }
                    | TsInsn::ConstStoreNba { .. }
                    | TsInsn::RangeStore { .. }
                    | TsInsn::RangeStoreNba { .. }
                    | TsInsn::BitStoreDyn { .. }
                    | TsInsn::ConstStoreX { .. }
                    | TsInsn::RangeStoreX { .. }
                    | TsInsn::ElemStore { .. }
                    | TsInsn::ElemStoreNba { .. }
                    | TsInsn::NbaFromElem { .. }
                    | TsInsn::WStore { .. }
                    | TsInsn::WStoreNba { .. }
            )
        })
    {
        return None;
    }
    let has_wide = out.iter().any(|i| {
        matches!(
            i,
            TsInsn::WLoadSig { .. }
                | TsInsn::WRedOr { .. }
                | TsInsn::WRedAnd { .. }
                | TsInsn::WConst { .. }
                | TsInsn::WXor { .. }
                | TsInsn::WAnd { .. }
                | TsInsn::WOr { .. }
                | TsInsn::WNot { .. }
                | TsInsn::WRange { .. }
                | TsInsn::RangeFromW { .. }
                | TsInsn::BitFromW { .. }
                | TsInsn::WConcat { .. }
                | TsInsn::WMask { .. }
                | TsInsn::WFromN { .. }
                | TsInsn::NFromW { .. }
                | TsInsn::WStore { .. }
                | TsInsn::WStoreNba { .. }
                | TsInsn::WRepl { .. }
                | TsInsn::WSigRange { .. }
        )
    });
    let has_ctrl = out.iter().any(|i| {
        matches!(
            i,
            TsInsn::BrSigFalse { .. }
                | TsInsn::BrFalse { .. }
                | TsInsn::BrNz { .. }
                | TsInsn::Jmp { .. }
                | TsInsn::CaseJmp { .. }
                | TsInsn::CaseMaskJmp { .. }
        )
    });
    let apply_skip = !has_ctrl;
    let reads_whole: Vec<u32> = reads_whole
        .into_iter()
        .filter(|&(_, sk)| !(apply_skip && sk))
        .map(|(x, _)| x)
        .collect();
    let reads_slice: Vec<(u32, u16, u16)> = reads_slice
        .into_iter()
        .filter(|&(_, _, _, sk)| !(apply_skip && sk))
        .map(|(a, b, c, _)| (a, b, c))
        .collect();
    let reads_wide: Vec<u32> = reads_wide
        .into_iter()
        .filter(|&(_, sk)| !(apply_skip && sk))
        .map(|(x, _)| x)
        .collect();
    let mut writes: Vec<u32> = Vec::new();
    let mut writes_span: Vec<(u32, u32)> = Vec::new();
    for i in out.iter() {
        match i {
            TsInsn::Store { sig, .. }
            | TsInsn::StoreNba { sig, .. }
            | TsInsn::WStore { sig, .. }
            | TsInsn::WStoreNba { sig, .. } => writes.push(*sig),
            TsInsn::NbaFromElem(op) => writes.push(op.dst),
            TsInsn::ElemStore(op) | TsInsn::ElemStoreNba(op) => {
                if op.hi >= op.lo {
                    writes_span.push((op.first, (op.hi - op.lo + 1) as u32));
                }
            }
            _ => {}
        }
    }
    writes.sort_unstable();
    writes.dedup();
    writes_span.sort_unstable();
    writes_span.dedup();
    Some(TwoStateBlock {
        insns: out,
        num_regs: cb.num_regs,
        reads_whole: reads_whole.into_boxed_slice(),
        reads_slice: reads_slice.into_boxed_slice(),
        has_ctrl,
        has_wide,
        reads_wide: reads_wide.into_boxed_slice(),
        writes: writes.into_boxed_slice(),
        writes_span: writes_span.into_boxed_slice(),
    })
}

/// IEEE 1800-2017 §20/§21 result type of a system FUNCTION as (width,
/// signed): `int`/`integer` → (32, true); unsigned int → (32, false);
/// `bit` → (1, false); `time` / `$realtobits` → (64, false). Real- and
/// string-valued functions, and the ones that carry their argument's type
/// (`system_function_carries_arg`), are None. Shared by the compiler's width
/// and signedness inference and by the interpreter, so every path agrees.
pub(crate) fn system_function_result(name: &str) -> Option<(u32, bool)> {
    Some(match name {
        "$countones" | "$countbits" | "$clog2" | "$bits" | "$size" | "$dimensions"
        | "$unpacked_dimensions" | "$left" | "$right" | "$low" | "$high" | "$increment"
        | "$rtoi" | "$random" | "$cast" | "$fopen" | "$fgetc" | "$fgets" | "$fscanf"
        | "$sscanf" | "$fread" | "$ftell" | "$feof" | "$ferror" | "$ungetc" | "$fseek"
        | "$rewind" | "$test$plusargs" | "$value$plusargs" | "$dist_uniform"
        | "$dist_normal" | "$dist_exponential" | "$dist_poisson" | "$dist_chi_square"
        | "$dist_t" | "$dist_erlang" | "$coverage_control" | "$coverage_get_max"
        | "$coverage_get" | "$coverage_merge" | "$coverage_save" => (32, true),
        "$urandom" | "$urandom_range" | "$stime" | "$shortrealtobits" => (32, false),
        "$onehot" | "$onehot0" | "$isunknown" | "$isunbounded" | "$rose" | "$fell"
        | "$stable" | "$changed" => (1, false),
        "$time" | "$realtobits" => (64, false),
        _ => return None,
    })
}

/// System functions whose result has the TYPE of their first argument.
pub(crate) fn system_function_carries_arg(name: &str) -> bool {
    matches!(name, "$signed" | "$unsigned" | "$past" | "$sampled")
}
