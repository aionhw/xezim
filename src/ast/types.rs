//! SystemVerilog data types (IEEE 1800-2017 §6, §7)

use super::{Identifier, Span, Spanned, expr};

/// Data type AST node.
#[derive(Debug, Clone)]
pub enum DataType {
    IntegerVector { kind: IntegerVectorType, signing: Option<Signing>, dimensions: Vec<PackedDimension>, span: Span },
    IntegerAtom { kind: IntegerAtomType, signing: Option<Signing>, span: Span },
    Real { kind: RealType, span: Span },
    Simple { kind: SimpleType, span: Span },
    Struct(StructUnionType),
    Enum(EnumType),
    Void(Span),
    TypeReference { name: TypeName, dimensions: Vec<PackedDimension>, span: Span },
    Implicit { signing: Option<Signing>, dimensions: Vec<PackedDimension>, span: Span },
}

#[derive(Debug, Clone)]
pub struct TypeName {
    pub scope: Option<Identifier>,
    pub name: Identifier,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegerVectorType { Bit, Logic, Reg }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegerAtomType { Byte, ShortInt, Int, LongInt, Integer, Time }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RealType { Real, ShortReal, RealTime }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimpleType { String, Chandle, Event }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signing { Signed, Unsigned }

#[derive(Debug, Clone)]
pub enum PackedDimension {
    Range { left: Box<expr::Expression>, right: Box<expr::Expression>, span: Span },
    Unsized(Span),
}

#[derive(Debug, Clone)]
pub enum UnpackedDimension {
    Range { left: Box<expr::Expression>, right: Box<expr::Expression>, span: Span },
    Expression { expr: Box<expr::Expression>, span: Span },
    Unsized(Span),
    Queue { max_size: Option<Box<expr::Expression>>, span: Span },
}

/// struct/union type
#[derive(Debug, Clone)]
pub struct StructUnionType {
    pub kind: StructUnionKind,
    pub packed: bool,
    pub signing: Option<Signing>,
    pub members: Vec<StructMember>,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructUnionKind { Struct, Union }

#[derive(Debug, Clone)]
pub struct StructMember {
    pub rand_qualifier: Option<RandQualifier>,
    pub data_type: DataType,
    pub declarators: Vec<StructDeclarator>,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RandQualifier { Rand, Randc }

#[derive(Debug, Clone)]
pub struct StructDeclarator {
    pub name: Identifier,
    pub dimensions: Vec<UnpackedDimension>,
    pub init: Option<expr::Expression>,
    pub span: Span,
}

/// enum type
#[derive(Debug, Clone)]
pub struct EnumType {
    pub base_type: Option<Box<DataType>>,
    pub members: Vec<EnumMember>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct EnumMember {
    pub name: Identifier,
    pub range: Option<(expr::Expression, expr::Expression)>,
    pub init: Option<expr::Expression>,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetType { Wire, Tri, Wand, Wor, TriAnd, TriOr, Tri0, Tri1, Supply0, Supply1, TriReg, Uwire }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifetime { Static, Automatic }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortDirection { Input, Output, Inout, Ref }
