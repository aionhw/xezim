//! Recursive descent parser for SystemVerilog IEEE 1800-2017/2023.

mod helpers;
mod types;
mod expressions;
mod statements;
mod declarations;
mod items;

use crate::ast::*;
use crate::ast::module::*;
use crate::lexer::token::{Token, TokenKind};
use crate::diagnostics::Diagnostic;

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    diagnostics: Vec<Diagnostic>,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0, diagnostics: Vec::new() }
    }

    pub fn diagnostics(&self) -> &[Diagnostic] { &self.diagnostics }

    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(|d| d.severity == crate::diagnostics::Severity::Error)
    }

    /// source_text ::= { description }
    pub fn parse_source_text(&mut self) -> SourceText {
        let start = self.current().span.start;
        let mut descriptions = Vec::new();
        while !self.at(TokenKind::Eof) {
            if let Some(desc) = self.parse_description() {
                descriptions.push(desc);
            } else {
                self.error(format!("unexpected token: {:?}", self.current().text));
                self.bump();
            }
        }
        SourceText { descriptions, span: self.span_from(start) }
    }

    fn parse_description(&mut self) -> Option<Description> {
        match self.current_kind() {
            TokenKind::KwModule | TokenKind::KwMacromodule =>
                Some(Description::Module(self.parse_module_declaration())),
            TokenKind::KwInterface =>
                Some(Description::Interface(self.parse_interface_declaration())),
            TokenKind::KwProgram =>
                Some(Description::Program(self.parse_program_declaration())),
            TokenKind::KwPackage =>
                Some(Description::Package(self.parse_package_declaration())),
            TokenKind::KwTypedef =>
                Some(Description::TypedefDecl(self.parse_typedef_declaration())),
            TokenKind::KwImport =>
                Some(Description::ImportDecl(self.parse_import_declaration())),
            TokenKind::KwTimeunit | TokenKind::KwTimeprecision =>
                Some(Description::TimeunitsDecl(self.parse_timeunits_declaration())),
            TokenKind::Directive => { self.bump(); self.parse_description() }
            _ => None,
        }
    }
}
