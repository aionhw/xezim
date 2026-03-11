//! Module, interface, program, and package declarations.

use super::{Identifier, AttributeInstance, Span};
use super::expr::Expression;
use super::types::*;
use super::decl::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleKind { Module, Macromodule }

#[derive(Debug, Clone)]
pub struct ModuleDeclaration {
    pub attrs: Vec<AttributeInstance>,
    pub kind: ModuleKind,
    pub lifetime: Option<Lifetime>,
    pub name: Identifier,
    pub params: Vec<ParameterDeclaration>,
    pub ports: PortList,
    pub items: Vec<ModuleItem>,
    pub endlabel: Option<Identifier>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct InterfaceDeclaration {
    pub attrs: Vec<AttributeInstance>,
    pub lifetime: Option<Lifetime>,
    pub name: Identifier,
    pub params: Vec<ParameterDeclaration>,
    pub ports: PortList,
    pub items: Vec<ModuleItem>,
    pub endlabel: Option<Identifier>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ProgramDeclaration {
    pub attrs: Vec<AttributeInstance>,
    pub lifetime: Option<Lifetime>,
    pub name: Identifier,
    pub params: Vec<ParameterDeclaration>,
    pub ports: PortList,
    pub items: Vec<ModuleItem>,
    pub endlabel: Option<Identifier>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct PackageDeclaration {
    pub attrs: Vec<AttributeInstance>,
    pub lifetime: Option<Lifetime>,
    pub name: Identifier,
    pub items: Vec<PackageItem>,
    pub endlabel: Option<Identifier>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum PortList {
    Empty,
    Ansi(Vec<AnsiPort>),
    NonAnsi(Vec<Identifier>),
}

#[derive(Debug, Clone)]
pub struct AnsiPort {
    pub attrs: Vec<AttributeInstance>,
    pub direction: Option<PortDirection>,
    pub net_type: Option<NetType>,
    pub var_kw: bool,
    pub data_type: Option<DataType>,
    pub name: Identifier,
    pub dimensions: Vec<UnpackedDimension>,
    pub default: Option<Expression>,
    pub span: Span,
}
