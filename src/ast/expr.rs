//! SystemVerilog expressions (IEEE 1800-2017 §A.8)

use super::{Identifier, Span};

#[derive(Debug, Clone)]
pub struct Expression {
    pub kind: ExprKind,
    pub span: Span,
}

impl Expression {
    pub fn new(kind: ExprKind, span: Span) -> Self { Self { kind, span } }
}

#[derive(Debug, Clone)]
pub enum ExprKind {
    Number(NumberLiteral),
    StringLiteral(String),
    Ident(HierarchicalIdentifier),
    Unary { op: UnaryOp, operand: Box<Expression> },
    Binary { op: BinaryOp, left: Box<Expression>, right: Box<Expression> },
    Conditional { condition: Box<Expression>, then_expr: Box<Expression>, else_expr: Box<Expression> },
    Concatenation(Vec<Expression>),
    Replication { count: Box<Expression>, exprs: Vec<Expression> },
    AssignmentPattern(Vec<Expression>),
    Call { func: Box<Expression>, args: Vec<Expression> },
    SystemCall { name: String, args: Vec<Expression> },
    MemberAccess { expr: Box<Expression>, member: Identifier },
    Index { expr: Box<Expression>, index: Box<Expression> },
    RangeSelect { expr: Box<Expression>, kind: RangeKind, left: Box<Expression>, right: Box<Expression> },
    Paren(Box<Expression>),
    Dollar,
    Null,
    This,
    Empty,
}

#[derive(Debug, Clone)]
pub enum NumberLiteral {
    Integer { size: Option<u32>, signed: bool, base: NumberBase, value: String },
    Real(f64),
    UnbasedUnsized(char),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberBase { Decimal, Binary, Octal, Hex }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeKind { Constant, IndexedUp, IndexedDown }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Plus, Minus, LogNot, BitNot, BitAnd, BitNand, BitOr, BitNor, BitXor, BitXnor,
    PreIncr, PreDecr, PostIncr, PostDecr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add, Sub, Mul, Div, Mod, Power,
    Eq, Neq, CaseEq, CaseNeq, WildcardEq, WildcardNeq,
    LogAnd, LogOr, LogImplies, LogEquiv,
    Lt, Leq, Gt, Geq,
    BitAnd, BitOr, BitXor, BitXnor,
    ShiftLeft, ShiftRight, ArithShiftLeft, ArithShiftRight,
    Assign,
}

#[derive(Debug, Clone)]
pub struct HierarchicalIdentifier {
    pub root: Option<String>,
    pub path: Vec<HierPathSegment>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct HierPathSegment {
    pub name: Identifier,
    pub selects: Vec<Expression>,
}
