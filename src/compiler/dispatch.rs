//! Direct threaded dispatch for bytecode VM - Proof of Concept.
//!
//! This module demonstrates infrastructure for optimized instruction dispatch.
//! Full integration with Simulator is left for future work.

use super::bytecode::{BinOpConstKind, Insn};

/// Opcodes for each instruction variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum Opcode {
    LoadConst = 0,
    LoadSignal,
    LoadSignalSigned,
    Resize,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    BitAnd,
    BitOr,
    BitXor,
    BitXnor,
    LogAnd,
    LogOr,
    Eq,
    Neq,
    CaseEq,
    CasezEq,
    CasexEq,
    Lt,
    Leq,
    Gt,
    Geq,
    Shl,
    Shr,
    AShr,
    BitNot,
    LogNot,
    Negate,
    ReduceAnd,
    ReduceOr,
    ReduceXor,
    BitSelect,
    BitSelectConst,
    RangeSelect,
    RangeSelectConst,
    Concat,
    Replicate,
    BranchIfFalse,
    Select,
    Jump,
    NbaAssign,
    NbaAssignRange,
    NbaAssignRangeDyn,
    NbaAssignBitDyn,
    BlockingAssign,
    BlockingAssignRange,
    BlockingAssignRangeDyn,
    BlockingAssignBitDyn,
    LoadArrayElem,
    NbaAssignArray,
    BlockingAssignArray,
    NbaAssignArrayRange,
    BlockingAssignArrayRange,
    Move,
    StmtFallback,
    SetSigned,
    Nop,
    LoadSignalRange,
    LoadSignalBit,
    NbaAssignConst,
    BranchUnlessZero,
    BranchIfSignalFalse,
    ClearSigned,
    Pow,
    NbaAssignArrayRead,
    BinOpConstAdd,
    BinOpConstEq,
    BinOpConstCaseEq,
}

impl Opcode {
    #[inline]
    pub fn from_insn(insn: &Insn) -> Self {
        match insn {
            Insn::LoadConst(_, _) => Self::LoadConst,
            Insn::LoadSignal(_, _) => Self::LoadSignal,
            Insn::LoadSignalSigned(_, _) => Self::LoadSignalSigned,
            Insn::Resize(_, _) => Self::Resize,
            Insn::Add(_, _, _) => Self::Add,
            Insn::Sub(_, _, _) => Self::Sub,
            Insn::Mul(_, _, _) => Self::Mul,
            Insn::Div(_, _, _) => Self::Div,
            Insn::Mod(_, _, _) => Self::Mod,
            Insn::BitAnd(_, _, _) => Self::BitAnd,
            Insn::BitOr(_, _, _) => Self::BitOr,
            Insn::BitXor(_, _, _) => Self::BitXor,
            Insn::BitXnor(_, _, _) => Self::BitXnor,
            Insn::LogAnd(_, _, _) => Self::LogAnd,
            Insn::LogOr(_, _, _) => Self::LogOr,
            Insn::Eq(_, _, _) => Self::Eq,
            Insn::Neq(_, _, _) => Self::Neq,
            Insn::CaseEq(_, _, _) => Self::CaseEq,
            Insn::CasezEq(_, _, _) => Self::CasezEq,
            Insn::CasexEq(_, _, _) => Self::CasexEq,
            Insn::Lt(_, _, _) => Self::Lt,
            Insn::Leq(_, _, _) => Self::Leq,
            Insn::Gt(_, _, _) => Self::Gt,
            Insn::Geq(_, _, _) => Self::Geq,
            Insn::Shl(_, _, _) => Self::Shl,
            Insn::Shr(_, _, _) => Self::Shr,
            Insn::AShr(_, _, _) => Self::AShr,
            Insn::BitNot(_, _) => Self::BitNot,
            Insn::LogNot(_, _) => Self::LogNot,
            Insn::Negate(_, _) => Self::Negate,
            Insn::ReduceAnd(_, _) => Self::ReduceAnd,
            Insn::ReduceOr(_, _) => Self::ReduceOr,
            Insn::ReduceXor(_, _) => Self::ReduceXor,
            Insn::BitSelect(_, _, _) => Self::BitSelect,
            Insn::BitSelectConst(_, _, _) => Self::BitSelectConst,
            Insn::RangeSelect(_, _, _, _) => Self::RangeSelect,
            Insn::RangeSelectConst(_, _, _, _) => Self::RangeSelectConst,
            Insn::Concat(_, _) => Self::Concat,
            Insn::Replicate(_, _, _) => Self::Replicate,
            Insn::BranchIfFalse(_, _) => Self::BranchIfFalse,
            Insn::Select(_, _, _, _) => Self::Select,
            Insn::Jump(_) => Self::Jump,
            Insn::NbaAssign(_, _, _) => Self::NbaAssign,
            Insn::NbaAssignRange(_, _, _, _) => Self::NbaAssignRange,
            Insn::NbaAssignRangeDyn(_, _, _, _) => Self::NbaAssignRangeDyn,
            Insn::NbaAssignBitDyn(_, _, _) => Self::NbaAssignBitDyn,
            Insn::BlockingAssign(_, _, _) => Self::BlockingAssign,
            Insn::BlockingAssignRange(_, _, _, _) => Self::BlockingAssignRange,
            Insn::BlockingAssignRangeDyn(_, _, _, _) => Self::BlockingAssignRangeDyn,
            Insn::BlockingAssignBitDyn(_, _, _) => Self::BlockingAssignBitDyn,
            Insn::LoadArrayElem(_, _, _) => Self::LoadArrayElem,
            Insn::NbaAssignArray(_, _, _, _) => Self::NbaAssignArray,
            Insn::BlockingAssignArray(_, _, _, _) => Self::BlockingAssignArray,
            Insn::NbaAssignArrayRange(_, _, _, _, _) => Self::NbaAssignArrayRange,
            Insn::BlockingAssignArrayRange(_, _, _, _, _) => Self::BlockingAssignArrayRange,
            Insn::Move(_, _) => Self::Move,
            Insn::StmtFallback(_) => Self::StmtFallback,
            Insn::EvalExprFallback(..) => Self::StmtFallback,
            Insn::SetSigned(_) => Self::SetSigned,
            Insn::ClearSigned(_) => Self::ClearSigned,
            Insn::Pow(_, _, _) => Self::Pow,
            Insn::Nop => Self::Nop,
            Insn::LoadSignalRange(_, _, _, _) => Self::LoadSignalRange,
            Insn::LoadSignalBit(_, _, _) => Self::LoadSignalBit,
            Insn::NbaAssignConst(_, _, _) => Self::NbaAssignConst,
            Insn::BranchUnlessZero(_, _) => Self::BranchUnlessZero,
            Insn::BranchIfSignalFalse(_, _, _) => Self::BranchIfSignalFalse,
            Insn::NbaAssignArrayRead(_, _, _, _) => Self::NbaAssignArrayRead,
            // One `Insn` variant, three census buckets: the kind is what
            // makes the pair-census readable, and `Opcode` is census-only.
            Insn::BinOpConst(_, _, _, k) => match k {
                BinOpConstKind::Add => Self::BinOpConstAdd,
                BinOpConstKind::Eq => Self::BinOpConstEq,
                BinOpConstKind::CaseEq => Self::BinOpConstCaseEq,
            },
        }
    }

    #[inline]
    pub fn as_usize(self) -> usize {
        self as usize
    }
}

pub const NUM_OPCODES: usize = 70;

/// Sizes the opcode-census arrays, which are indexed by `Opcode as usize`. A
/// stale value panics at run time under `XEZIM_OPCODE_CENSUS=1`, so pin it to
/// the last discriminant at compile time instead.
const _: () = assert!(NUM_OPCODES == Opcode::BinOpConstCaseEq as usize + 1);

/// Dispatch table - proof of concept.
#[derive(Debug, Clone)]
pub struct DispatchTable {
    pub opcode_count: usize,
}

impl DispatchTable {
    pub fn new() -> Self {
        Self { opcode_count: NUM_OPCODES }
    }

    #[inline]
    pub fn opcode_from_insn(&self, insn: &Insn) -> Opcode {
        Opcode::from_insn(insn)
    }
}

static DISPATCH_TABLE: once_cell::sync::OnceCell<DispatchTable> = once_cell::sync::OnceCell::new();

pub fn get_dispatch_table() -> &'static DispatchTable {
    DISPATCH_TABLE.get_or_init(|| DispatchTable::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_opcode_from_insn() {
        let insn = Insn::Add(0, 1, 2);
        assert_eq!(Opcode::from_insn(&insn), Opcode::Add);
        
        let insn = Insn::Nop;
        assert_eq!(Opcode::from_insn(&insn), Opcode::Nop);
    }

    #[test]
    fn test_dispatch_table() {
        let table = DispatchTable::new();
        assert_eq!(table.opcode_count, NUM_OPCODES);
    }
}
