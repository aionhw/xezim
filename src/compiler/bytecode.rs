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
type RegId = u16;

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

/// Per-kind count of `LoadConst ; <binop>` pairs collapsed into
/// `Insn::BinOpConst`, indexed by `BinOpConstKind as usize`. See
/// [`BytecodeCompiler::fuse_binop_const`].
static FUSED_BINOP_CONST: [std::sync::atomic::AtomicU64; BinOpConstKind::COUNT] = [
    std::sync::atomic::AtomicU64::new(0),
    std::sync::atomic::AtomicU64::new(0),
    std::sync::atomic::AtomicU64::new(0),
];

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[repr(u8)]
pub enum BinOpConstKind {
    /// `dst = src + K` — same `Value` semantics as [`Insn::Add`].
    Add = 0,
    /// `dst = (src == K)` — same `Value` semantics as [`Insn::Eq`].
    Eq = 1,
    /// `dst = (src === K)` — same `Value` semantics as [`Insn::CaseEq`].
    CaseEq = 2,
}

impl BinOpConstKind {
    /// Number of kinds; sizes the static fusion-count array.
    pub const COUNT: usize = 3;
}

/// Bytecode instruction set. Stack-free, register-based design.
/// Each instruction specifies source and destination registers explicitly,
/// enabling the VM to iterate a flat Vec<Insn> with predictable memory access.
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

/// Opcode name only (no operands) — used by the settle profiler to aggregate
/// continuous-assignment RHS shapes across entries. Operands differ per
/// instance; the SHAPE is what a fused fast path would have to match.
pub fn insn_opcode_name(i: &Insn) -> &'static str {
    match i {
        Insn::LoadConst(..) => "Const",
        Insn::LoadSignal(..) => "Load",
        Insn::LoadSignalSigned(..) => "LoadS",
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
        Insn::BinOpConst(_, _, _, BinOpConstKind::Eq) => "EqC",
        Insn::BinOpConst(_, _, _, BinOpConstKind::CaseEq) => "CaseEqC",
        Insn::StmtFallback(..) => "Fallback",
        Insn::EvalExprFallback(..) => "EvalExpr",
    }
}

/// Compiler state for converting AST → bytecode.
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
    /// Base names of 2D/ND UNPACKED arrays. When a continuous-assign LHS
    /// `m[0][j]` targets one of these, the flattening short-circuit
    /// (`flattened_outer_const_signal_id`) must NOT fire — the
    /// bogus scalar signal `m` would otherwise catch a bit-select write and
    /// silently drop the element. None = no info (older callers); the guard
    /// then only excludes 1D/packed bases as before. Set via
    /// `set_multi_dim_arrays`.
    multi_dim_arrays: Option<&'a HashSet<String>>,
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
            for_loop_var_ids: std::collections::HashMap::default(),
            local_var_regs: std::collections::HashMap::default(),
            reg_var_loop_depth: 0,
            allow_expr_fallback: false,
            tasks: None,
            functions: None,
            inlining_stack: Vec::new(),
            tasks_inlined: 0,
            params: None,
            top_module_name: None,
            packed_elem_widths: None,
            assoc_elem_widths: None,
            assoc_arrays: None,
            packed_full_dims: None,
            loop_break_patches: Vec::new(),
            loop_continue_patches: Vec::new(),
            string_signals: None,
            multi_dim_arrays: None,
            packed_struct_fields: None,
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
        let member = hier.path.last()?.name.name.as_str();
        let base: String = hier.path[..hier.path.len() - 1]
            .iter()
            .map(|s| s.name.name.as_str())
            .collect::<Vec<_>>()
            .join(".");
        // Resolve the container signal id, honoring scope_hint for a bare base.
        let base_id = self.lookup_signal_id_by_name(&base).or_else(|| {
            self.scope_hint
                .as_ref()
                .and_then(|sc| self.lookup_signal_id_by_name(&format!("{}.{}", sc, base)))
        })?;
        // Field layout is keyed by both the bare and scope-qualified base name.
        let fields = fields_map.get(base.as_str()).or_else(|| {
            self.scope_hint
                .as_ref()
                .and_then(|sc| fields_map.get(&format!("{}.{}", sc, base)))
        })?;
        let (_, off, w) = fields.iter().find(|(m, _, _)| m == member)?;
        Some((base_id, *off, *w))
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

    pub fn set_packed_elem_widths(&mut self, w: &'a HashMap<String, u32>) {
        self.packed_elem_widths = Some(w);
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
        let raw = Self::hier_raw_name(hier);
        self.packed_full_dims.and_then(|m| {
            m.get(raw.as_str())
                .or_else(|| hier.path.last().and_then(|s| m.get(s.name.name.as_str())))
                .and_then(|d| d.first())
                .copied()
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
    /// holding its result. Accepts only the shape that cannot observe or
    /// mutate anything: input-only formals, and a body that is a single
    /// assignment to the function name or a single `return <expr>`.
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
        if fd.ports.len() != args.len()
            || fd
                .ports
                .iter()
                .any(|p| !matches!(p.direction, PortDirection::Input) || !p.dimensions.is_empty())
        {
            self.bail("Expr_Call_ports");
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
        if !self.fn_is_pure(&fd) {
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
        let ret_w = crate::compiler::elaborate::resolve_type_width(
            &fd.return_type,
            self.params,
            None,
        );
        // Evaluate the arguments in the CALLER's scope first, then bind them as
        // register-backed locals while compiling the body.
        let mut binds: Vec<(String, (RegId, u32))> = Vec::with_capacity(args.len());
        for (p, a) in fd.ports.iter().zip(args) {
            let w = self.decl_width(&p.data_type);
            let v = self.compile_expr(a, w)?;
            let slot = self.alloc_reg();
            self.emit(Insn::Move(slot, v));
            if w > 0 {
                self.emit(Insn::Resize(slot, w));
            }
            binds.push((p.name.name.clone(), (slot, w)));
        }
        let saved_locals = std::mem::take(&mut self.local_var_regs);
        for (n, b) in binds {
            self.local_var_regs.insert(n, b);
        }
        // The function's own name is its return variable (§13.4.1): give it a
        // register too, so a body that assigns it (possibly across several
        // statements) works exactly like the single-assignment form.
        let ret_slot = self.alloc_reg();
        let ret_init = self.type_default_value(&fd.return_type, ret_w);
        self.emit(Insn::LoadConst(ret_slot, Box::new(ret_init)));
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
        let ok = self.compile_pure_body(&items, ret_slot, ret_w, ctx_width);
        self.allow_ast_fallback = saved_fallback;
        self.inlining_stack.pop();
        self.local_var_regs = saved_locals;
        if !ok {
            return None;
        }
        if ret_w > 0 {
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
                        if !d.dimensions.is_empty() {
                            self.bail("Expr_Call_local_array");
                            return false;
                        }
                        let w = self.decl_width(data_type);
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
                        if w > 0 {
                            self.emit(Insn::Resize(slot, w));
                        }
                        self.local_var_regs.insert(d.name.name.clone(), (slot, w));
                    }
                }
                StatementKind::Return(Some(e)) => {
                    if idx + 1 != items.len() {
                        self.bail("Expr_Call_early_return");
                        return false;
                    }
                    let w = if ret_w > 0 { ret_w } else { ctx_width };
                    let Some(v) = self.compile_expr(e, w) else {
                        return false;
                    };
                    self.emit(Insn::Move(ret_slot, v));
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
        if self.inlining_stack.len() >= MAX_INLINE_DEPTH {
            return false;
        }
        if self.inlining_stack.iter().any(|n| n == task_name) {
            return false;
        }
        let tasks = match self.tasks {
            Some(t) => t,
            None => return false,
        };
        let td = match tasks.get(task_name) {
            Some(t) => t,
            None => return false,
        };
        if !td.ports.is_empty() {
            return false;
        }
        if td.items.iter().any(Self::stmt_is_blocking) {
            return false;
        }
        let body: Vec<Statement> = td.items.clone();
        self.inlining_stack.push(task_name.to_string());
        let mut ok = true;
        for s in &body {
            if !self.compile_stmt(s) {
                ok = false;
                break;
            }
        }
        self.inlining_stack.pop();
        if ok {
            self.tasks_inlined += 1;
        }
        ok
    }

    /// Conservative structural test for loop bodies that the NEW for-loop
    /// compilation capabilities (register-backed `for (int i...)` vars and
    /// signal-backed `i++` steps) are allowed to handle. Nested indexing,
    /// member access, and non-assign statements go through addressing paths
    /// whose register/loop-var handling is not yet audited — those loops
    /// keep the old whole-loop AST fallback. Never regresses: these bodies
    /// always fell back before the new capabilities existed.
    fn for_body_is_simple(stmt: &Statement) -> bool {
        fn expr_simple(e: &Expression) -> bool {
            // no member access anywhere; everything else is fine (reads
            // resolve through compile_expr's normal paths).
            match &e.kind {
                ExprKind::MemberAccess { .. } => false,
                ExprKind::Unary { operand, .. } | ExprKind::Paren(operand) => {
                    expr_simple(operand)
                }
                ExprKind::Binary { left, right, .. } => {
                    expr_simple(left) && expr_simple(right)
                }
                ExprKind::Conditional {
                    condition,
                    then_expr,
                    else_expr,
                } => expr_simple(condition) && expr_simple(then_expr) && expr_simple(else_expr),
                ExprKind::Index { expr, index } => expr_simple(expr) && expr_simple(index),
                ExprKind::RangeSelect { expr, left, right, .. } => {
                    expr_simple(expr) && expr_simple(left) && expr_simple(right)
                }
                ExprKind::SystemCall { args, .. } | ExprKind::Concatenation(args) => {
                    args.iter().all(expr_simple)
                }
                _ => !matches!(&e.kind, ExprKind::Call { .. }),
            }
        }
        fn lv_simple(e: &Expression) -> bool {
            match &e.kind {
                ExprKind::Ident(h) => h.path.iter().all(|s| s.selects.is_empty()),
                ExprKind::Index { expr, index } => {
                    matches!(&expr.kind, ExprKind::Ident(h)
                        if h.path.iter().all(|s| s.selects.is_empty()))
                        && expr_simple(index)
                }
                _ => false,
            }
        }
        fn lv_base_name(e: &Expression) -> Option<&str> {
            match &e.kind {
                ExprKind::Index { expr, .. } => match &expr.kind {
                    ExprKind::Ident(h) => h.path.last().map(|s| s.name.name.as_str()),
                    _ => None,
                },
                _ => None,
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
            StatementKind::NonblockingAssign { lvalue, rvalue, .. }
            | StatementKind::BlockingAssign { lvalue, rvalue } => {
                // A SELF-READING array update (`ptr[i] <= ptr[i] + 1`) is
                // excluded: in an inlined instance the compiled read and
                // write paths can resolve the array through different
                // aliases (port copy vs local), skewing the pre-edge read.
                // Keep such loops on the audited AST path.
                let self_read = lv_base_name(lvalue)
                    .is_some_and(|n| expr_reads_name(rvalue, n));
                lv_simple(lvalue) && expr_simple(rvalue) && !self_read
            }
            StatementKind::SeqBlock { stmts, .. } => {
                stmts.iter().all(Self::for_body_is_simple)
            }
            StatementKind::If {
                condition,
                then_stmt,
                else_stmt,
                ..
            } => {
                expr_simple(condition)
                    && Self::for_body_is_simple(then_stmt)
                    && else_stmt
                        .as_ref()
                        .map(|e| Self::for_body_is_simple(e))
                        .unwrap_or(true)
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
        if !seg.selects.is_empty() || seg.name.name.contains('.') {
            return None;
        }
        self.local_var_regs.get(&seg.name.name).copied()
    }

    /// True when `fd`'s body reads only its formals, its own declared locals
    /// and compile-time constants — i.e. its result depends on nothing but the
    /// arguments. Conservative: any construct not understood here says "no".
    fn fn_is_pure(&self, fd: &FunctionDeclaration) -> bool {
        let mut bound: HashSet<String> = HashSet::default();
        bound.insert(fd.name.name.name.clone());
        for p in &fd.ports {
            bound.insert(p.name.name.clone());
        }
        fn expr_ok(e: &Expression, bound: &HashSet<String>, me: &BytecodeCompiler) -> bool {
            match &e.kind {
                ExprKind::Ident(h) => {
                    if h.path.len() != 1 || h.path[0].name.name.contains('.') {
                        return false;
                    }
                    let n = &h.path[0].name.name;
                    let known = bound.contains(n)
                        || me.params.is_some_and(|p| p.contains_key(n));
                    known && h.path[0].selects.iter().all(|sel| expr_ok(sel, bound, me))
                }
                ExprKind::Number(_) | ExprKind::StringLiteral(_) => true,
                ExprKind::Paren(i) => expr_ok(i, bound, me),
                ExprKind::Unary { operand, .. } => expr_ok(operand, bound, me),
                ExprKind::Binary { left, right, .. } => {
                    expr_ok(left, bound, me) && expr_ok(right, bound, me)
                }
                ExprKind::Conditional {
                    condition,
                    then_expr,
                    else_expr,
                } => {
                    expr_ok(condition, bound, me)
                        && expr_ok(then_expr, bound, me)
                        && expr_ok(else_expr, bound, me)
                }
                ExprKind::Concatenation(parts) => parts.iter().all(|p| expr_ok(p, bound, me)),
                ExprKind::Replication { count, exprs } => {
                    expr_ok(count, bound, me) && exprs.iter().all(|p| expr_ok(p, bound, me))
                }
                ExprKind::Index { expr, index } => {
                    expr_ok(expr, bound, me) && expr_ok(index, bound, me)
                }
                ExprKind::RangeSelect {
                    expr, left, right, ..
                } => {
                    expr_ok(expr, bound, me)
                        && expr_ok(left, bound, me)
                        && expr_ok(right, bound, me)
                }
                _ => false,
            }
        }
        fn stmt_ok(st: &Statement, bound: &mut HashSet<String>, me: &BytecodeCompiler) -> bool {
            match &st.kind {
                StatementKind::Null => true,
                StatementKind::VarDecl {
                    declarators,
                    ..
                } => {
                    for d in declarators {
                        if let Some(e) = &d.init {
                            if !expr_ok(e, bound, me) {
                                return false;
                            }
                        }
                        bound.insert(d.name.name.clone());
                    }
                    true
                }
                StatementKind::BlockingAssign { lvalue, rvalue } => {
                    expr_ok(lvalue, bound, me) && expr_ok(rvalue, bound, me)
                }
                StatementKind::Return(e) => e.as_ref().is_none_or(|e| expr_ok(e, bound, me)),
                StatementKind::SeqBlock { stmts, .. } => {
                    // A block's declarations are visible to the statements that
                    // FOLLOW them, so thread one scope through the sequence.
                    let mut inner = bound.clone();
                    stmts.iter().all(|s| stmt_ok(s, &mut inner, me))
                }
                StatementKind::If {
                    condition,
                    then_stmt,
                    else_stmt,
                    ..
                } => {
                    expr_ok(condition, bound, me)
                        && stmt_ok(then_stmt, &mut bound.clone(), me)
                        && else_stmt
                            .as_ref()
                            .is_none_or(|e| stmt_ok(e, &mut bound.clone(), me))
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
                                if !expr_ok(init, &inner, me) {
                                    return false;
                                }
                                inner.insert(name.name.clone());
                            }
                            ForInit::Assign { lvalue, rvalue } => {
                                if !expr_ok(lvalue, &inner, me) || !expr_ok(rvalue, &inner, me) {
                                    return false;
                                }
                            }
                        }
                    }
                    condition.as_ref().is_none_or(|c| expr_ok(c, &inner, me))
                        && step.iter().all(|e| expr_ok(e, &inner, me))
                        && stmt_ok(body, &mut inner, me)
                }
                _ => false,
            }
        }
        let mut scope = bound;
        fd.items.iter().all(|st| stmt_ok(st, &mut scope, self))
    }

    /// §13.4.1 / §6.8: the initial value of a variable of `dt` — x for a
    /// 4-state type, 0 for a 2-state one. Getting this wrong for an inlined
    /// function's return variable makes a partially-assigned function return 0
    /// where it must return x.
    fn type_default_value(&self, dt: &crate::ast::types::DataType, w: u32) -> Value {
        let w = if w > 0 { w } else { 32 };
        if crate::compiler::elaborate::is_type_two_state(dt) {
            Value::zero(w)
        } else {
            Value::new(w)
        }
    }

    /// Declared width of a block-local variable's data type, resolved against
    /// the module's parameters/typedefs (0 when unknown, meaning "leave as-is").
    fn decl_width(&self, dt: &crate::ast::types::DataType) -> u32 {
        crate::compiler::elaborate::resolve_type_width(dt, self.params, None)
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
        if !raw.contains('.') {
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
        if let Some(scope) = &self.scope_hint {
            let qualified = format!("{}.{}", scope, raw);
            if let Some(&id) = self.signal_name_to_id.get(qualified.as_str()) {
                return Some(id);
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
        let mut found: Option<&Value> = None;
        for (name, value) in params {
            let raw_has_key_suffix = raw.len() >= name.len()
                && raw.ends_with(name.as_str())
                && (raw.len() == name.len() || raw.as_bytes()[raw.len() - name.len() - 1] == b'.');
            let key_has_raw_suffix = name.len() >= raw.len()
                && name.ends_with(raw.as_str())
                && (name.len() == raw.len() || name.as_bytes()[name.len() - raw.len() - 1] == b'.');
            if raw_has_key_suffix || key_has_raw_suffix {
                if found.is_some() {
                    return None;
                }
                found = Some(value);
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
        let raw = Self::hier_raw_name(hier);
        self.packed_full_dims.and_then(|m| {
            m.get(raw.as_str())
                .or_else(|| hier.path.last().and_then(|s| m.get(s.name.name.as_str())))
                .cloned()
        })
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
                m.get(raw.as_str()).copied().or_else(|| {
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
        let raw = Self::hier_raw_name(hier);
        if self.arrays.contains_key(&raw) {
            return Some(raw);
        }
        if let Some(scope) = &self.scope_hint {
            let qualified = format!("{}.{}", scope, raw);
            if self.arrays.contains_key(&qualified) {
                return Some(qualified);
            }
        }
        if hier.path.len() == 1 {
            let leaf = &hier.path[0].name.name;
            if self.arrays.contains_key(leaf) {
                return Some(leaf.clone());
            }
        }
        None
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
                .any(|d| self.signal_name_to_id.contains_key(d.name.name.as_str()))
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
        if self.allow_ast_fallback {
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
            StatementKind::Null => true,
            StatementKind::NonblockingAssign { lvalue, rvalue, .. } => {
                let width = self.infer_lhs_width(lvalue);
                let start = self.insns.len();
                let start_reg = self.next_reg;
                if let Some(val_reg) = self.compile_expr(rvalue, width) {
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
                if let Some(val_reg) = self.compile_expr(rvalue, width) {
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
                if let Some(val_reg) = self.compile_expr(expr, 0) {
                    let mut end_jumps: Vec<usize> = Vec::new();
                    let mut default_item: Option<&Statement> = None;
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
                    }
                    let end = self.insns.len() as u32;
                    for idx in end_jumps {
                        self.insns[idx] = Insn::Jump(end);
                    }
                    true
                } else {
                    false
                }
            }
            StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
                for s in stmts {
                    if !self.compile_stmt(s) {
                        return false;
                    }
                }
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
                        ForInit::VarDecl { data_type, name, init }
                            if Self::for_body_is_simple(body) =>
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
                        #[allow(unreachable_patterns)]
                        ForInit::VarDecl { .. } => {
                            self.for_loop_var_ids = saved_for_vars;
                            self.local_var_regs = saved_locals;
                            self.bail("For_init_vardecl");
                            return false;
                        }
                    }
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
                if !self.compile_stmt(body) {
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
                                    .filter(|_| Self::for_body_is_simple(body))
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
                // A register-backed block local (a for-loop variable) shadows
                // any same-named signal for the duration of its loop.
                if let Some((src, _)) = self.local_var_reg_of(hier) {
                    let r = self.alloc_reg();
                    self.emit(Insn::Move(r, src));
                    return Some(r);
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
                        self.emit(Insn::ClearSigned(l));
                        self.emit(Insn::ClearSigned(r));
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
                            self.emit(Insn::ClearSigned(l));
                            self.emit(Insn::ClearSigned(r));
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
                        self.emit(Insn::ClearSigned(then_reg));
                        self.emit(Insn::ClearSigned(else_reg));
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
                    let raw = Self::hier_raw_name(hier);
                    let elem_w = self
                        .packed_elem_widths
                        .and_then(|m| {
                            m.get(raw.as_str()).copied().or_else(|| {
                                hier.path
                                    .last()
                                    .and_then(|s| m.get(s.name.name.as_str()).copied())
                            })
                        })
                        .filter(|&w| w > 1);
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
            ExprKind::SystemCall { name, args } => match name.as_str() {
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
                    other => {
                        let _ = other;
                        if let Some(r) =
                            self.emit_expr_fallback(expr, ctx_width, "SystemCall_other")
                        {
                            return Some(r);
                        }
                        self.bail("SystemCall_other");
                        None
                    }
            },
            // §13.4: inline a PURE function call — one whose body is a single
            // assignment to the function name (or a single `return`) over input
            // formals. That is the overwhelmingly common combinational-helper
            // shape in RTL (`lfsr32(s)`, `mix(a,b)`), and leaving it to the AST
            // interpreter dragged the whole enclosing block out of bytecode.
            ExprKind::Call { func, args } => self
                .compile_pure_call(func, args, ctx_width)
                .or_else(|| self.emit_expr_fallback(expr, ctx_width, "Expr_Call_impure")),
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

    fn compile_nba_target(&mut self, lhs: &Expression, val_reg: RegId, width: u32) -> bool {
        match &lhs.kind {
            ExprKind::Ident(hier) => {
                if let Some(id) = self.lookup_signal_id(hier) {
                    self.emit(Insn::NbaAssign(as_sig_id(id), val_reg, width));
                    true
                } else {
                    self.bail("nba_ident_unresolved");
                    false
                }
            }
            ExprKind::Index { expr, index } => {
                if let Some(id) = self.const_multi_dim_array_elem_signal_id(lhs) {
                    self.emit(Insn::NbaAssign(as_sig_id(id), val_reg, width));
                    return true;
                }
                if let ExprKind::Ident(hier) = &expr.kind {
                    if self.is_assoc_target(hier) {
                        self.bail("nba_target_assoc");
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
                        let raw = Self::hier_raw_name(hier);
                        let elem_w = self
                            .packed_elem_widths
                            .and_then(|m| {
                                m.get(raw.as_str()).copied().or_else(|| {
                                    hier.path
                                        .last()
                                        .and_then(|s| m.get(s.name.name.as_str()).copied())
                                })
                            })
                            .filter(|&w| w > 1);
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
                self.bail("nba_index_other");
                false
            }
            ExprKind::RangeSelect {
                expr,
                left,
                right,
                kind,
            } => {
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
                // Handle mem[i][hi:lo] <= val
                if let ExprKind::Index {
                    expr: arr_expr,
                    index,
                } = &expr.kind
                {
                    if let ExprKind::Ident(hier) = &arr_expr.kind {
                        if let Some(name) = self.lookup_array_name(hier) {
                            if let Some(idx_reg) = self.compile_expr(index, 0) {
                                if let (Some(hi_reg), Some(lo_reg)) =
                                    (self.compile_expr(left, 0), self.compile_expr(right, 0))
                                {
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
            ExprKind::MemberAccess { .. } => {
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
        // Assignment to a register-backed block local (the loop variable of an
        // enclosing `for (int i = ...)`).
        if let ExprKind::Ident(hier) = &lhs.kind {
            if let Some((dst, w)) = self.local_var_reg_of(hier) {
                self.emit(Insn::Move(dst, val_reg));
                if w > 0 {
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
                self.bail("blocking_target_member_access");
                false
            }
            ExprKind::Ident(hier) => {
                if let Some(id) = self.lookup_signal_id(hier) {
                    self.emit(Insn::BlockingAssign(as_sig_id(id), val_reg, width));
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
                        let raw = Self::hier_raw_name(hier);
                        let elem_w = self
                            .packed_elem_widths
                            .and_then(|m| {
                                m.get(raw.as_str()).copied().or_else(|| {
                                    hier.path
                                        .last()
                                        .and_then(|s| m.get(s.name.name.as_str()).copied())
                                })
                            })
                            .filter(|&w| w > 1);
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
                                if let (Some(hi_reg), Some(lo_reg)) =
                                    (self.compile_expr(left, 0), self.compile_expr(right, 0))
                                {
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
            ExprKind::Ident(hier) => self.lookup_param_value(hier)?.to_u64().map(|u| u as u32),
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
            // Time literal magnitude in tick units (1 ns), matching the
            // interpreter's value-context handling.
            NumberLiteral::Time(s) => Some(Value::from_u64((*s * 1e9) as u64, 64)),
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
    fn expr_signedness(&mut self, e: &Expression) -> Option<bool> {
        match &e.kind {
            ExprKind::Number(NumberLiteral::Integer { signed, .. }) => Some(*signed),
            ExprKind::Paren(i) => self.expr_signedness(i),
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


    fn expr_max_width(&self, expr: &Expression) -> u32 {
        match &expr.kind {
            ExprKind::Ident(hier) => self
                .lookup_signal_id(hier)
                    .map(|id| self.signal_widths[id])
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
            ExprKind::Index { .. } => 1,
            ExprKind::Replication { count, exprs } => {
                let n = self.eval_const_expr(count).unwrap_or(0);
                let inner: u32 = exprs.iter().map(|e| self.expr_max_width(e)).sum();
                n * inner
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
        // AFTER resize elision: an about-to-be-deleted `Resize` sitting between
        // the array read and the NBA would otherwise hide the triple. Still
        // before `compact_nops`, which removes the `Nop`s it leaves behind.
        Self::fuse_array_read_nba(&mut self.insns);
        // Also after `elide_redundant_resizes`: a `Resize` that pass deletes
        // is what most often separates a `LoadConst` from its consumer. Its
        // pattern is disjoint from `fuse_array_read_nba`'s, so the order
        // between the two does not matter.
        Self::fuse_binop_const(&mut self.insns);
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
        let nba_dup_targets = nba_targets.windows(2).any(|w| w[0] == w[1]);
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
    fn insn_reads_reg(insn: &Insn, r: RegId) -> bool {
        match insn {
            Insn::LoadConst(..)
            | Insn::LoadSignal(..)
            | Insn::LoadSignalSigned(..)
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
                Insn::BranchIfFalse(_, t)
                | Insn::BranchUnlessZero(_, t)
                | Insn::BranchIfSignalFalse(_, t, _)
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
                Insn::BranchIfFalse(_, t)
                | Insn::BranchUnlessZero(_, t)
                | Insn::BranchIfSignalFalse(_, t, _)
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
                Insn::BranchIfFalse(_, t)
                | Insn::BranchUnlessZero(_, t)
                | Insn::BranchIfSignalFalse(_, t, _)
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
            // Index of the next instruction that survives `compact_nops`.
            let mut j = i + 1;
            while j < insns.len() && matches!(insns[j], Insn::Nop) {
                j += 1;
            }
            if j >= insns.len() {
                continue;
            }
            let (d, l, kind) = match insns[j] {
                Insn::Add(d, l, r) if r == c => (d, l, BinOpConstKind::Add),
                Insn::Eq(d, l, r) if r == c => (d, l, BinOpConstKind::Eq),
                Insn::CaseEq(d, l, r) if r == c => (d, l, BinOpConstKind::CaseEq),
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
            let consumed = insns
                .iter()
                .enumerate()
                .any(|(x, ins)| x != i && x != j && Self::insn_reads_reg(ins, c));
            if consumed {
                continue;
            }
            // Take the boxed constant out of the `LoadConst` rather than
            // cloning a possibly-`Wide` `Value`.
            let Insn::LoadConst(_, k) = std::mem::replace(&mut insns[i], Insn::Nop) else {
                unreachable!("just matched LoadConst")
            };
            insns[i] = Insn::BinOpConst(d, l, k, kind);
            insns[j] = Insn::Nop;
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
                Insn::BranchIfFalse(_, t)
                | Insn::BranchUnlessZero(_, t)
                | Insn::BranchIfSignalFalse(_, t, _)
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
                Insn::BranchIfFalse(..)
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
                Insn::BranchIfFalse(_, t)
                | Insn::BranchUnlessZero(_, t)
                | Insn::BranchIfSignalFalse(_, t, _)
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
