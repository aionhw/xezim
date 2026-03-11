//! Module-level item parsing (IEEE 1800-2017 §A.1)

use super::Parser;
use crate::ast::decl::*;
use crate::ast::module::*;
use crate::ast::types::*;
use crate::lexer::token::TokenKind;

impl Parser {
    pub(super) fn parse_module_declaration(&mut self) -> ModuleDeclaration {
        let start = self.current().span.start;
        let kind_tok = self.bump();
        let kind = if kind_tok.kind == TokenKind::KwMacromodule { ModuleKind::Macromodule } else { ModuleKind::Module };
        let lifetime = self.parse_optional_lifetime();
        let name = self.parse_identifier();
        let params = self.parse_parameter_port_list();
        let ports = self.parse_port_list();
        self.expect(TokenKind::Semicolon);
        let items = self.parse_module_items();
        self.expect(TokenKind::KwEndmodule);
        let endlabel = self.parse_end_label();
        ModuleDeclaration { attrs: Vec::new(), kind, lifetime, name, params, ports, items, endlabel, span: self.span_from(start) }
    }

    pub(super) fn parse_interface_declaration(&mut self) -> InterfaceDeclaration {
        let start = self.current().span.start;
        self.expect(TokenKind::KwInterface);
        let lifetime = self.parse_optional_lifetime();
        let name = self.parse_identifier();
        let params = self.parse_parameter_port_list();
        let ports = self.parse_port_list();
        self.expect(TokenKind::Semicolon);
        let items = self.parse_module_items();
        self.expect(TokenKind::KwEndinterface);
        let endlabel = self.parse_end_label();
        InterfaceDeclaration { attrs: Vec::new(), lifetime, name, params, ports, items, endlabel, span: self.span_from(start) }
    }

    pub(super) fn parse_program_declaration(&mut self) -> ProgramDeclaration {
        let start = self.current().span.start;
        self.expect(TokenKind::KwProgram);
        let lifetime = self.parse_optional_lifetime();
        let name = self.parse_identifier();
        let params = self.parse_parameter_port_list();
        let ports = self.parse_port_list();
        self.expect(TokenKind::Semicolon);
        let items = self.parse_module_items();
        self.expect(TokenKind::KwEndprogram);
        let endlabel = self.parse_end_label();
        ProgramDeclaration { attrs: Vec::new(), lifetime, name, params, ports, items, endlabel, span: self.span_from(start) }
    }

    pub(super) fn parse_package_declaration(&mut self) -> PackageDeclaration {
        let start = self.current().span.start;
        self.expect(TokenKind::KwPackage);
        let lifetime = self.parse_optional_lifetime();
        let name = self.parse_identifier();
        self.expect(TokenKind::Semicolon);
        let mut items = Vec::new();
        while !self.at(TokenKind::KwEndpackage) && !self.at(TokenKind::Eof) {
            if let Some(item) = self.parse_package_item() { items.push(item); }
            else { self.error("unexpected token in package"); self.bump(); }
        }
        self.expect(TokenKind::KwEndpackage);
        let endlabel = self.parse_end_label();
        PackageDeclaration { attrs: Vec::new(), lifetime, name, items, endlabel, span: self.span_from(start) }
    }

    pub(super) fn parse_port_list(&mut self) -> PortList {
        if self.eat(TokenKind::LParen).is_none() { return PortList::Empty; }
        if self.at(TokenKind::RParen) { self.bump(); return PortList::Empty; }
        if self.is_port_direction() || self.is_data_type_keyword() || self.at(TokenKind::KwVar) {
            let mut ports = Vec::new();
            let mut last_direction: Option<PortDirection> = None;
            let mut last_data_type: Option<DataType> = None;
            let mut last_net_type: Option<NetType> = None;
            loop {
                if self.at(TokenKind::RParen) || self.at(TokenKind::Eof) { break; }
                let mut port = self.parse_ansi_port();
                // IEEE 1800-2017 §23.2.2.3: inherit direction and type from previous port
                // Data type is only inherited when direction is also omitted.
                // If a new direction is explicitly given, data_type resets to default (1-bit).
                let direction_was_explicit = port.direction.is_some();
                if port.direction.is_none() && last_direction.is_some() {
                    port.direction = last_direction;
                }
                if port.data_type.is_none() && last_data_type.is_some() && !direction_was_explicit {
                    port.data_type = last_data_type.clone();
                }
                if port.net_type.is_none() && last_net_type.is_some() && !direction_was_explicit {
                    port.net_type = last_net_type;
                }
                // Update last values
                if port.direction.is_some() { last_direction = port.direction; }
                if port.data_type.is_some() { last_data_type = port.data_type.clone(); }
                if port.net_type.is_some() { last_net_type = port.net_type; }
                ports.push(port);
                if self.eat(TokenKind::Comma).is_none() { break; }
            }
            self.expect(TokenKind::RParen);
            PortList::Ansi(ports)
        } else {
            let mut names = Vec::new();
            loop {
                if self.at(TokenKind::RParen) || self.at(TokenKind::Eof) { break; }
                names.push(self.parse_identifier());
                if self.eat(TokenKind::Comma).is_none() { break; }
            }
            self.expect(TokenKind::RParen);
            PortList::NonAnsi(names)
        }
    }

    fn parse_ansi_port(&mut self) -> AnsiPort {
        let start = self.current().span.start;
        let direction = self.parse_optional_direction();
        let net_type = self.parse_optional_net_type();
        let var_kw = self.eat(TokenKind::KwVar).is_some();
        let data_type = if self.is_data_type_keyword() {
            Some(self.parse_data_type())
        } else if self.at(TokenKind::LBracket) {
            // Implicit type with packed dimensions: input [7:0] a
            let dimensions = self.parse_packed_dimensions();
            Some(DataType::Implicit { signing: None, dimensions, span: self.span_from(start) })
        } else { None };
        let name = self.parse_identifier();
        let dimensions = self.parse_unpacked_dimensions();
        let default = if self.eat(TokenKind::Assign).is_some() { Some(self.parse_expression()) } else { None };
        AnsiPort { attrs: Vec::new(), direction, net_type, var_kw, data_type, name, dimensions, default, span: self.span_from(start) }
    }

    pub(super) fn parse_module_items(&mut self) -> Vec<ModuleItem> {
        let end_tokens = [TokenKind::KwEndmodule, TokenKind::KwEndinterface, TokenKind::KwEndprogram, TokenKind::Eof];
        let mut items = Vec::new();
        while !self.at_any(&end_tokens) {
            if let Some(item) = self.parse_module_item() { items.push(item); }
            else { self.error(format!("unexpected: {:?}", self.current().text)); self.bump(); }
        }
        items
    }

    pub(super) fn parse_module_item(&mut self) -> Option<ModuleItem> {
        match self.current_kind() {
            TokenKind::KwInput | TokenKind::KwOutput | TokenKind::KwInout | TokenKind::KwRef => {
                let start = self.current().span.start;
                let dir = self.parse_optional_direction().unwrap_or(PortDirection::Input);
                let nt = self.parse_optional_net_type();
                let dt = if self.is_data_type_keyword() { self.parse_data_type() }
                    else if self.at(TokenKind::LBracket) {
                        let dimensions = self.parse_packed_dimensions();
                        DataType::Implicit { signing: None, dimensions, span: self.span_from(start) }
                    }
                    else { DataType::Implicit { signing: None, dimensions: Vec::new(), span: self.span_from(start) } };
                let decls = self.parse_var_declarator_list();
                self.expect(TokenKind::Semicolon);
                Some(ModuleItem::PortDeclaration(PortDeclaration { direction: dir, net_type: nt, data_type: dt, declarators: decls, span: self.span_from(start) }))
            }
            TokenKind::KwWire | TokenKind::KwTri | TokenKind::KwWand | TokenKind::KwWor |
            TokenKind::KwSupply0 | TokenKind::KwSupply1 | TokenKind::KwTriand | TokenKind::KwTrior |
            TokenKind::KwTri0 | TokenKind::KwTri1 | TokenKind::KwTrireg | TokenKind::KwUwire =>
                Some(ModuleItem::NetDeclaration(self.parse_net_declaration())),
            _ if self.is_data_type_keyword() =>
                Some(ModuleItem::DataDeclaration(self.parse_data_declaration())),
            TokenKind::KwVar | TokenKind::KwConst =>
                Some(ModuleItem::DataDeclaration(self.parse_data_declaration())),
            TokenKind::KwParameter =>
                Some(ModuleItem::ParameterDeclaration(self.parse_parameter_decl_stmt())),
            TokenKind::KwLocalparam =>
                Some(ModuleItem::LocalparamDeclaration(self.parse_parameter_decl_stmt())),
            TokenKind::KwTypedef =>
                Some(ModuleItem::TypedefDeclaration(self.parse_typedef_declaration())),
            TokenKind::KwAlways | TokenKind::KwAlways_comb | TokenKind::KwAlways_ff | TokenKind::KwAlways_latch => {
                let start = self.current().span.start;
                let kind = match self.bump().kind {
                    TokenKind::KwAlways_comb => AlwaysKind::AlwaysComb,
                    TokenKind::KwAlways_ff => AlwaysKind::AlwaysFf,
                    TokenKind::KwAlways_latch => AlwaysKind::AlwaysLatch,
                    _ => AlwaysKind::Always,
                };
                let stmt = self.parse_statement();
                Some(ModuleItem::AlwaysConstruct(AlwaysConstruct { kind, stmt, span: self.span_from(start) }))
            }
            TokenKind::KwInitial => { let s = self.current().span.start; self.bump(); let st = self.parse_statement();
                Some(ModuleItem::InitialConstruct(InitialConstruct { stmt: st, span: self.span_from(s) })) }
            TokenKind::KwFinal => { let s = self.current().span.start; self.bump(); let st = self.parse_statement();
                Some(ModuleItem::FinalConstruct(FinalConstruct { stmt: st, span: self.span_from(s) })) }
            TokenKind::KwAssign => {
                let start = self.current().span.start; self.bump();
                let mut asgns = Vec::new();
                loop { let l = self.parse_expression(); self.expect(TokenKind::Assign); let r = self.parse_expression();
                    asgns.push((l, r)); if self.eat(TokenKind::Comma).is_none() { break; } }
                self.expect(TokenKind::Semicolon);
                Some(ModuleItem::ContinuousAssign(ContinuousAssign { strength: None, delay: None, assignments: asgns, span: self.span_from(start) }))
            }
            TokenKind::KwGenerate => {
                let s = self.current().span.start; self.bump();
                let items = self.parse_module_items_until(TokenKind::KwEndgenerate);
                self.expect(TokenKind::KwEndgenerate);
                Some(ModuleItem::GenerateRegion(GenerateRegion { items, span: self.span_from(s) }))
            }
            TokenKind::KwGenvar => {
                let s = self.current().span.start; self.bump();
                let mut names = Vec::new();
                loop { names.push(self.parse_identifier()); if self.eat(TokenKind::Comma).is_none() { break; } }
                self.expect(TokenKind::Semicolon);
                Some(ModuleItem::GenvarDeclaration(GenvarDeclaration { names, span: self.span_from(s) }))
            }
            TokenKind::KwFunction => Some(ModuleItem::FunctionDeclaration(self.parse_function_declaration())),
            TokenKind::KwTask => Some(ModuleItem::TaskDeclaration(self.parse_task_declaration())),
            TokenKind::KwImport => Some(ModuleItem::ImportDeclaration(self.parse_import_declaration())),
            TokenKind::KwClass | TokenKind::KwVirtual => Some(ModuleItem::ClassDeclaration(self.parse_class_declaration())),
            TokenKind::KwAssert | TokenKind::KwAssume | TokenKind::KwCover =>
                Some(ModuleItem::AssertionItem(self.parse_assertion_statement())),
            // Generate-if: if (...) begin...end [else if (...) begin...end] [else begin...end]
            TokenKind::KwIf => {
                let s = self.current().span.start;
                Some(self.parse_generate_if(s))
            }
            // Generate-for: for (...) begin...end
            TokenKind::KwFor => {
                let s = self.current().span.start;
                self.bump(); // skip 'for'
                self.expect(TokenKind::LParen);
                // Skip for-init
                while !self.at(TokenKind::Semicolon) && !self.at(TokenKind::Eof) { self.bump(); }
                self.expect(TokenKind::Semicolon);
                // Skip condition
                while !self.at(TokenKind::Semicolon) && !self.at(TokenKind::Eof) { self.bump(); }
                self.expect(TokenKind::Semicolon);
                // Skip step
                while !self.at(TokenKind::RParen) && !self.at(TokenKind::Eof) { self.bump(); }
                self.expect(TokenKind::RParen);
                if self.eat(TokenKind::KwBegin).is_some() {
                    let _label = self.parse_end_label();
                    let items = self.parse_module_items_until(TokenKind::KwEnd);
                    self.expect(TokenKind::KwEnd);
                    let _ = self.parse_end_label();
                    Some(ModuleItem::GenerateRegion(GenerateRegion { items, span: self.span_from(s) }))
                } else {
                    let item = self.parse_module_item();
                    Some(ModuleItem::GenerateRegion(GenerateRegion {
                        items: item.into_iter().collect(),
                        span: self.span_from(s),
                    }))
                }
            }
            TokenKind::Identifier => Some(self.parse_identifier_starting_item()),
            TokenKind::Semicolon => { self.bump(); Some(ModuleItem::Null) }
            TokenKind::Directive => { self.bump(); self.parse_module_item() }
            // Generate-if: if (expr) begin ... end [else ...]
            TokenKind::KwIf => {
                let s = self.current().span.start;
                Some(self.parse_generate_if(s))
            }
            // Generate-for: for (...) begin ... end
            TokenKind::KwFor => {
                let s = self.current().span.start;
                let stmt = self.parse_statement();
                Some(ModuleItem::GenerateRegion(GenerateRegion {
                    items: vec![ModuleItem::InitialConstruct(InitialConstruct { stmt, span: self.span_from(s) })],
                    span: self.span_from(s),
                }))
            }
            // begin/end blocks at module level (inside generate)
            TokenKind::KwBegin => {
                let s = self.current().span.start;
                let stmt = self.parse_statement();
                Some(ModuleItem::GenerateRegion(GenerateRegion {
                    items: vec![ModuleItem::InitialConstruct(InitialConstruct { stmt, span: self.span_from(s) })],
                    span: self.span_from(s),
                }))
            }
            _ => None,
        }
    }

    /// Parse a generate-if construct: if (cond) begin items end [else if (cond) begin items end]* [else begin items end]
    fn parse_generate_if(&mut self, start: usize) -> ModuleItem {
        let mut branches = Vec::new();

        // Parse first 'if (cond) branch'
        self.bump(); // skip 'if'
        self.expect(TokenKind::LParen);
        let cond = self.parse_expression();
        self.expect(TokenKind::RParen);
        let items = self.parse_generate_branch_items();
        branches.push((Some(cond), items));

        // Parse else-if / else chain
        while self.eat(TokenKind::KwElse).is_some() {
            if self.at(TokenKind::KwIf) {
                self.bump();
                self.expect(TokenKind::LParen);
                let c = self.parse_expression();
                self.expect(TokenKind::RParen);
                let items = self.parse_generate_branch_items();
                branches.push((Some(c), items));
            } else {
                // Plain else
                let items = self.parse_generate_branch_items();
                branches.push((None, items));
                break;
            }
        }

        ModuleItem::GenerateIf(GenerateIf { branches, span: self.span_from(start) })
    }

    /// Parse a generate branch body: either begin...end block of items, or a single item
    fn parse_generate_branch_items(&mut self) -> Vec<ModuleItem> {
        if self.eat(TokenKind::KwBegin).is_some() {
            let _ = self.parse_end_label();
            let items = self.parse_module_items_until(TokenKind::KwEnd);
            self.expect(TokenKind::KwEnd);
            let _ = self.parse_end_label();
            items
        } else {
            self.parse_module_item().into_iter().collect()
        }
    }

    fn parse_identifier_starting_item(&mut self) -> ModuleItem {
        let start = self.current().span.start;
        let first_name = self.parse_identifier();
        let params = if self.at(TokenKind::Hash) {
            self.bump();
            if self.eat(TokenKind::LParen).is_some() {
                let mut p = Vec::new();
                while !self.at(TokenKind::RParen) && !self.at(TokenKind::Eof) {
                    if self.at(TokenKind::Dot) {
                        self.bump();
                        let pn = self.parse_identifier();
                        self.expect(TokenKind::LParen);
                        let pv = if !self.at(TokenKind::RParen) { Some(self.parse_expression()) } else { None };
                        self.expect(TokenKind::RParen);
                        p.push(ParamConnection::Named { name: pn, value: pv });
                    } else { p.push(ParamConnection::Ordered(Some(self.parse_expression()))); }
                    if self.eat(TokenKind::Comma).is_none() { break; }
                }
                self.expect(TokenKind::RParen);
                Some(p)
            } else { None }
        } else { None };

        if self.at(TokenKind::Identifier) {
            let mut instances = Vec::new();
            loop {
                let is = self.current().span.start;
                let iname = self.parse_identifier();
                let dims = self.parse_unpacked_dimensions();
                let conns = self.parse_port_connections();
                instances.push(HierarchicalInstance { name: iname, dimensions: dims, connections: conns, span: self.span_from(is) });
                if self.eat(TokenKind::Comma).is_none() { break; }
            }
            self.expect(TokenKind::Semicolon);
            ModuleItem::ModuleInstantiation(ModuleInstantiation { module_name: first_name, params, instances, span: self.span_from(start) })
        } else {
            let dt = DataType::TypeReference {
                name: TypeName { scope: None, name: first_name, span: self.span_from(start) },
                dimensions: Vec::new(), span: self.span_from(start),
            };
            let decls = self.parse_var_declarator_list();
            self.expect(TokenKind::Semicolon);
            ModuleItem::DataDeclaration(DataDeclaration { const_kw: false, var_kw: false, lifetime: None, data_type: dt, declarators: decls, span: self.span_from(start) })
        }
    }

    fn parse_port_connections(&mut self) -> Vec<PortConnection> {
        let mut conns = Vec::new();
        if self.eat(TokenKind::LParen).is_none() { return conns; }
        if self.at(TokenKind::RParen) { self.bump(); return conns; }
        loop {
            if self.at(TokenKind::RParen) || self.at(TokenKind::Eof) { break; }
            if self.at(TokenKind::Dot) {
                self.bump();
                if self.at(TokenKind::Star) { self.bump(); conns.push(PortConnection::Wildcard); }
                else {
                    let nm = self.parse_identifier();
                    let ex = if self.eat(TokenKind::LParen).is_some() {
                        let e = if !self.at(TokenKind::RParen) { Some(self.parse_expression()) } else { None };
                        self.expect(TokenKind::RParen); e
                    } else { None };
                    conns.push(PortConnection::Named { name: nm, expr: ex });
                }
            } else { conns.push(PortConnection::Ordered(Some(self.parse_expression()))); }
            if self.eat(TokenKind::Comma).is_none() { break; }
        }
        self.expect(TokenKind::RParen);
        conns
    }

    fn parse_module_items_until(&mut self, end: TokenKind) -> Vec<ModuleItem> {
        let mut items = Vec::new();
        while !self.at(end) && !self.at(TokenKind::Eof) {
            if let Some(item) = self.parse_module_item() { items.push(item); }
            else { self.bump(); }
        }
        items
    }

    fn parse_net_declaration(&mut self) -> NetDeclaration {
        let start = self.current().span.start;
        let nt = self.parse_optional_net_type().unwrap_or(NetType::Wire);
        let dt = if self.is_data_type_keyword() { self.parse_data_type() }
            else if self.at(TokenKind::LBracket) {
                // Implicit type with packed dimensions: wire [7:0] a;
                let signing = None;
                let dimensions = self.parse_packed_dimensions();
                DataType::Implicit { signing, dimensions, span: self.span_from(start) }
            }
            else { DataType::Implicit { signing: None, dimensions: Vec::new(), span: self.span_from(start) } };
        let mut decls = Vec::new();
        loop {
            let ds = self.current().span.start;
            let nm = self.parse_identifier();
            let dims = self.parse_unpacked_dimensions();
            let init = if self.eat(TokenKind::Assign).is_some() { Some(self.parse_expression()) } else { None };
            decls.push(NetDeclarator { name: nm, dimensions: dims, init, span: self.span_from(ds) });
            if self.eat(TokenKind::Comma).is_none() { break; }
        }
        self.expect(TokenKind::Semicolon);
        NetDeclaration { net_type: nt, strength: None, data_type: dt, delay: None, declarators: decls, span: self.span_from(start) }
    }

    pub(super) fn parse_class_declaration(&mut self) -> ClassDeclaration {
        let start = self.current().span.start;
        let virt = self.eat(TokenKind::KwVirtual).is_some();
        self.expect(TokenKind::KwClass);
        let name = self.parse_identifier();
        // Skip to semicolon (may have extends/implements)
        while !self.at(TokenKind::Semicolon) && !self.at(TokenKind::Eof) { self.bump(); }
        self.expect(TokenKind::Semicolon);
        while !self.at(TokenKind::KwEndclass) && !self.at(TokenKind::Eof) { self.bump(); }
        self.expect(TokenKind::KwEndclass);
        let endlabel = self.parse_end_label();
        ClassDeclaration { virtual_kw: virt, name, params: Vec::new(), extends: None,
            implements: Vec::new(), items: Vec::new(), endlabel, span: self.span_from(start) }
    }
}
