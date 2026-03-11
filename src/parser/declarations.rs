//! Declaration parsing (IEEE 1800-2017 §A.2)

use super::Parser;
use crate::ast::decl::*;
use crate::ast::types::*;
use crate::ast::stmt::VarDeclarator;
use crate::lexer::token::TokenKind;

impl Parser {
    pub(super) fn parse_parameter_port_list(&mut self) -> Vec<ParameterDeclaration> {
        let mut params = Vec::new();
        if self.eat(TokenKind::Hash).is_none() { return params; }
        if self.eat(TokenKind::LParen).is_none() { return params; }
        loop {
            if self.at(TokenKind::RParen) || self.at(TokenKind::Eof) { break; }
            params.push(self.parse_parameter_declaration());
            if self.eat(TokenKind::Comma).is_none() { break; }
        }
        self.expect(TokenKind::RParen);
        params
    }

    pub(super) fn parse_parameter_declaration(&mut self) -> ParameterDeclaration {
        let start = self.current().span.start;
        let local = match self.current_kind() {
            TokenKind::KwParameter => { self.bump(); false }
            TokenKind::KwLocalparam => { self.bump(); true }
            _ => false,
        };
        if self.at(TokenKind::KwType) {
            self.bump();
            let mut assignments = Vec::new();
            let name = self.parse_identifier();
            let init = if self.eat(TokenKind::Assign).is_some() {
                Some(self.parse_data_type())
            } else { None };
            assignments.push(TypeParamAssignment { name, init, span: self.span_from(start) });
            return ParameterDeclaration { local, kind: ParameterKind::Type { assignments }, span: self.span_from(start) };
        }
        // Check if there's an explicit data type keyword or just an implicit type
        // "parameter integer X = ..." has explicit type
        // "parameter WIDTH = ..." has implicit type (identifier followed by =)
        // "parameter [7:0] X = ..." has implicit type with range
        let data_type = if self.is_data_type_keyword() {
            self.parse_data_type()
        } else if self.at(TokenKind::LBracket) {
            // Implicit type with packed dimensions
            let dimensions = self.parse_packed_dimensions();
            DataType::Implicit { signing: None, dimensions, span: self.span_from(start) }
        } else {
            // No explicit type - implicit
            DataType::Implicit { signing: None, dimensions: Vec::new(), span: self.span_from(start) }
        };
        let mut assignments = Vec::new();
        loop {
            let astart = self.current().span.start;
            let name = self.parse_identifier();
            let dimensions = self.parse_unpacked_dimensions();
            let init = if self.eat(TokenKind::Assign).is_some() {
                Some(self.parse_expression())
            } else { None };
            assignments.push(ParamAssignment { name, dimensions, init, span: self.span_from(astart) });
            // Don't consume comma if next token after comma is parameter/localparam
            // (those belong to the parameter port list, not this declaration)
            if self.at(TokenKind::Comma) {
                let next = self.peek_kind();
                if next == TokenKind::KwParameter || next == TokenKind::KwLocalparam {
                    break;
                }
                self.bump(); // consume comma
            } else {
                break;
            }
        }
        ParameterDeclaration { local, kind: ParameterKind::Data { data_type, assignments }, span: self.span_from(start) }
    }

    pub(super) fn parse_parameter_decl_stmt(&mut self) -> ParameterDeclaration {
        let decl = self.parse_parameter_declaration();
        self.expect(TokenKind::Semicolon);
        decl
    }

    pub(super) fn parse_typedef_declaration(&mut self) -> TypedefDeclaration {
        let start = self.current().span.start;
        self.expect(TokenKind::KwTypedef);
        let data_type = self.parse_data_type();
        let name = self.parse_identifier();
        let dimensions = self.parse_unpacked_dimensions();
        self.expect(TokenKind::Semicolon);
        TypedefDeclaration { data_type, name, dimensions, span: self.span_from(start) }
    }

    pub(super) fn parse_import_declaration(&mut self) -> ImportDeclaration {
        let start = self.current().span.start;
        self.expect(TokenKind::KwImport);
        let mut items = Vec::new();
        loop {
            let istart = self.current().span.start;
            let package = self.parse_identifier();
            self.expect(TokenKind::DoubleColon);
            let item = if self.eat(TokenKind::Star).is_some() { None }
            else { Some(self.parse_identifier()) };
            items.push(ImportItem { package, item, span: self.span_from(istart) });
            if self.eat(TokenKind::Comma).is_none() { break; }
        }
        self.expect(TokenKind::Semicolon);
        ImportDeclaration { items, span: self.span_from(start) }
    }

    pub(super) fn parse_timeunits_declaration(&mut self) -> TimeunitsDeclaration {
        let start = self.current().span.start;
        let mut unit = None;
        let mut precision = None;
        if self.eat(TokenKind::KwTimeunit).is_some() {
            unit = Some(self.bump().text.clone());
            if self.eat(TokenKind::Slash).is_some() {
                precision = Some(self.bump().text.clone());
            }
        } else if self.eat(TokenKind::KwTimeprecision).is_some() {
            precision = Some(self.bump().text.clone());
        }
        self.expect(TokenKind::Semicolon);
        TimeunitsDeclaration { unit, precision, span: self.span_from(start) }
    }

    pub(super) fn parse_data_declaration(&mut self) -> DataDeclaration {
        let start = self.current().span.start;
        let const_kw = self.eat(TokenKind::KwConst).is_some();
        let var_kw = self.eat(TokenKind::KwVar).is_some();
        let lifetime = self.parse_optional_lifetime();
        let data_type = self.parse_data_type();
        let declarators = self.parse_var_declarator_list();
        self.expect(TokenKind::Semicolon);
        DataDeclaration { const_kw, var_kw, lifetime, data_type, declarators, span: self.span_from(start) }
    }

    pub(super) fn parse_var_declarator_list(&mut self) -> Vec<VarDeclarator> {
        let mut decls = Vec::new();
        loop {
            let start = self.current().span.start;
            let name = self.parse_identifier();
            let dimensions = self.parse_unpacked_dimensions();
            let init = if self.eat(TokenKind::Assign).is_some() {
                Some(self.parse_expression())
            } else { None };
            decls.push(VarDeclarator { name, dimensions, init, span: self.span_from(start) });
            if self.eat(TokenKind::Comma).is_none() { break; }
        }
        decls
    }

    pub(super) fn parse_function_declaration(&mut self) -> FunctionDeclaration {
        let start = self.current().span.start;
        self.expect(TokenKind::KwFunction);
        let lifetime = self.parse_optional_lifetime();
        let return_type = if self.is_data_type_keyword() || self.at(TokenKind::KwVoid) {
            self.parse_data_type()
        } else {
            DataType::Implicit { signing: None, dimensions: Vec::new(), span: self.span_from(start) }
        };
        let name = self.parse_identifier();
        let ports = self.parse_function_ports();
        self.expect(TokenKind::Semicolon);
        let mut items = Vec::new();
        while !self.at(TokenKind::KwEndfunction) && !self.at(TokenKind::Eof) {
            items.push(self.parse_statement());
        }
        self.expect(TokenKind::KwEndfunction);
        let endlabel = self.parse_end_label();
        FunctionDeclaration { lifetime, return_type, name, ports, items, endlabel, span: self.span_from(start) }
    }

    pub(super) fn parse_task_declaration(&mut self) -> TaskDeclaration {
        let start = self.current().span.start;
        self.expect(TokenKind::KwTask);
        let lifetime = self.parse_optional_lifetime();
        let name = self.parse_identifier();
        let ports = self.parse_function_ports();
        self.expect(TokenKind::Semicolon);
        let mut items = Vec::new();
        while !self.at(TokenKind::KwEndtask) && !self.at(TokenKind::Eof) {
            items.push(self.parse_statement());
        }
        self.expect(TokenKind::KwEndtask);
        let endlabel = self.parse_end_label();
        TaskDeclaration { lifetime, name, ports, items, endlabel, span: self.span_from(start) }
    }

    pub(super) fn parse_function_ports(&mut self) -> Vec<FunctionPort> {
        let mut ports = Vec::new();
        if self.eat(TokenKind::LParen).is_none() { return ports; }
        if self.at(TokenKind::RParen) { self.bump(); return ports; }
        loop {
            if self.at(TokenKind::RParen) || self.at(TokenKind::Eof) { break; }
            let start = self.current().span.start;
            let direction = self.parse_optional_direction().unwrap_or(PortDirection::Input);
            let var_kw = self.eat(TokenKind::KwVar).is_some();
            let data_type = if self.is_data_type_keyword() {
                self.parse_data_type()
            } else {
                DataType::Implicit { signing: None, dimensions: Vec::new(), span: self.span_from(start) }
            };
            let name = self.parse_identifier();
            let dimensions = self.parse_unpacked_dimensions();
            let default = if self.eat(TokenKind::Assign).is_some() {
                Some(self.parse_expression())
            } else { None };
            ports.push(FunctionPort { direction, var_kw, data_type, name, dimensions, default, span: self.span_from(start) });
            if self.eat(TokenKind::Comma).is_none() { break; }
        }
        self.expect(TokenKind::RParen);
        ports
    }

    pub(super) fn parse_package_item(&mut self) -> Option<PackageItem> {
        match self.current_kind() {
            TokenKind::KwParameter => Some(PackageItem::Parameter(self.parse_parameter_decl_stmt())),
            TokenKind::KwLocalparam => Some(PackageItem::Parameter(self.parse_parameter_decl_stmt())),
            TokenKind::KwTypedef => Some(PackageItem::Typedef(self.parse_typedef_declaration())),
            TokenKind::KwFunction => Some(PackageItem::Function(self.parse_function_declaration())),
            TokenKind::KwTask => Some(PackageItem::Task(self.parse_task_declaration())),
            TokenKind::KwImport => Some(PackageItem::Import(self.parse_import_declaration())),
            TokenKind::KwClass | TokenKind::KwVirtual => Some(PackageItem::Class(self.parse_class_declaration())),
            _ if self.is_data_type_keyword() || self.at(TokenKind::KwVar) || self.at(TokenKind::KwConst) =>
                Some(PackageItem::Data(self.parse_data_declaration())),
            TokenKind::Identifier => Some(PackageItem::Data(self.parse_data_declaration())),
            TokenKind::Directive => { self.bump(); self.parse_package_item() }
            TokenKind::Semicolon => { self.bump(); self.parse_package_item() }
            _ => None,
        }
    }
}
