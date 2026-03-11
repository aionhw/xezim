//! Abstract Syntax Tree definitions for SystemVerilog IEEE 1800-2017/2023.

pub mod types;
pub mod expr;
pub mod stmt;
pub mod decl;
pub mod module;

/// A span of source text identified by byte offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self { Self { start, end } }
    pub fn dummy() -> Self { Self { start: 0, end: 0 } }
}

/// Trait for AST nodes that have a source span.
pub trait Spanned {
    fn span(&self) -> Span;
}

/// An identifier with its source location.
#[derive(Debug, Clone)]
pub struct Identifier {
    pub name: String,
    pub span: Span,
}

/// An attribute instance: (* attr_spec { , attr_spec } *)
#[derive(Debug, Clone)]
pub struct AttributeInstance {
    pub attrs: Vec<(Identifier, Option<expr::Expression>)>,
    pub span: Span,
}

/// Top-level source text: a sequence of descriptions.
#[derive(Debug)]
pub struct SourceText {
    pub descriptions: Vec<Description>,
    pub span: Span,
}

/// A top-level description item.
#[derive(Debug)]
pub enum Description {
    Module(module::ModuleDeclaration),
    Interface(module::InterfaceDeclaration),
    Program(module::ProgramDeclaration),
    Package(module::PackageDeclaration),
    TypedefDecl(decl::TypedefDeclaration),
    ImportDecl(decl::ImportDeclaration),
    TimeunitsDecl(decl::TimeunitsDeclaration),
}
