//! Statement parsing (IEEE 1800-2017 §A.6)

use super::Parser;
use crate::ast::stmt::*;
use crate::ast::expr::{ExprKind, BinaryOp, Expression};
use crate::ast::types::DataType;
use crate::lexer::token::TokenKind;

impl Parser {
    pub(super) fn parse_statement(&mut self) -> Statement {
        let start = self.current().span.start;

        match self.current_kind() {
            TokenKind::KwBegin => self.parse_seq_block(),
            TokenKind::KwFork => self.parse_par_block(),
            TokenKind::KwIf | TokenKind::KwUnique | TokenKind::KwUnique0 | TokenKind::KwPriority => {
                self.parse_if_or_case()
            }
            TokenKind::KwCase | TokenKind::KwCasex | TokenKind::KwCasez => self.parse_case_statement(),
            TokenKind::KwFor => self.parse_for_statement(),
            TokenKind::KwForeach => self.parse_foreach_statement(),
            TokenKind::KwWhile => self.parse_while_statement(),
            TokenKind::KwDo => self.parse_do_while_statement(),
            TokenKind::KwRepeat => self.parse_repeat_statement(),
            TokenKind::KwForever => {
                self.bump();
                let body = self.parse_statement();
                Statement::new(StatementKind::Forever { body: Box::new(body) }, self.span_from(start))
            }
            TokenKind::KwReturn => {
                self.bump();
                let expr = if !self.at(TokenKind::Semicolon) {
                    Some(self.parse_expression())
                } else { None };
                self.expect(TokenKind::Semicolon);
                Statement::new(StatementKind::Return(expr), self.span_from(start))
            }
            TokenKind::KwBreak => { self.bump(); self.expect(TokenKind::Semicolon); Statement::new(StatementKind::Break, self.span_from(start)) }
            TokenKind::KwContinue => { self.bump(); self.expect(TokenKind::Semicolon); Statement::new(StatementKind::Continue, self.span_from(start)) }
            TokenKind::KwWait => {
                self.bump();
                if self.eat(TokenKind::KwFork).is_some() {
                    self.expect(TokenKind::Semicolon);
                    Statement::new(StatementKind::WaitFork, self.span_from(start))
                } else {
                    self.expect(TokenKind::LParen);
                    let cond = self.parse_expression();
                    self.expect(TokenKind::RParen);
                    let stmt = self.parse_statement();
                    Statement::new(StatementKind::Wait { condition: cond, stmt: Box::new(stmt) }, self.span_from(start))
                }
            }
            TokenKind::KwDisable => {
                self.bump();
                let name = self.parse_identifier();
                self.expect(TokenKind::Semicolon);
                Statement::new(StatementKind::Disable(name), self.span_from(start))
            }
            TokenKind::KwAssert | TokenKind::KwAssume | TokenKind::KwCover => {
                Statement::new(StatementKind::Assertion(self.parse_assertion_statement()), self.span_from(start))
            }
            TokenKind::KwAssign => {
                self.bump();
                let lv = self.parse_expression();
                self.expect(TokenKind::Assign);
                let rv = self.parse_expression();
                self.expect(TokenKind::Semicolon);
                Statement::new(StatementKind::ProceduralContinuous(
                    ProceduralContinuous::Assign { lvalue: lv, rvalue: rv }
                ), self.span_from(start))
            }
            TokenKind::KwForce => {
                self.bump();
                let lv = self.parse_expression();
                self.expect(TokenKind::Assign);
                let rv = self.parse_expression();
                self.expect(TokenKind::Semicolon);
                Statement::new(StatementKind::ProceduralContinuous(
                    ProceduralContinuous::Force { lvalue: lv, rvalue: rv }
                ), self.span_from(start))
            }
            TokenKind::KwDeassign => {
                self.bump();
                let lv = self.parse_expression();
                self.expect(TokenKind::Semicolon);
                Statement::new(StatementKind::ProceduralContinuous(
                    ProceduralContinuous::Deassign(lv)
                ), self.span_from(start))
            }
            TokenKind::KwRelease => {
                self.bump();
                let lv = self.parse_expression();
                self.expect(TokenKind::Semicolon);
                Statement::new(StatementKind::ProceduralContinuous(
                    ProceduralContinuous::Release(lv)
                ), self.span_from(start))
            }
            // Timing control: @
            TokenKind::At => {
                let ctrl = self.parse_event_control();
                let stmt = self.parse_statement();
                Statement::new(StatementKind::TimingControl {
                    control: TimingControl::Event(ctrl),
                    stmt: Box::new(stmt),
                }, self.span_from(start))
            }
            // Delay control: #
            TokenKind::Hash => {
                self.bump();
                let delay = self.parse_expression();
                let stmt = self.parse_statement();
                Statement::new(StatementKind::TimingControl {
                    control: TimingControl::Delay(delay),
                    stmt: Box::new(stmt),
                }, self.span_from(start))
            }
            // Variable declaration (data type keywords)
            k if self.is_data_type_keyword() && k != TokenKind::KwEvent => {
                let data_type = self.parse_data_type();
                let lifetime = None;
                let mut declarators = Vec::new();
                loop {
                    let ds = self.current().span.start;
                    let name = self.parse_identifier();
                    let dimensions = self.parse_unpacked_dimensions();
                    let init = if self.eat(TokenKind::Assign).is_some() {
                        Some(self.parse_expression())
                    } else { None };
                    declarators.push(VarDeclarator { name, dimensions, init, span: self.span_from(ds) });
                    if self.eat(TokenKind::Comma).is_none() { break; }
                }
                self.expect(TokenKind::Semicolon);
                Statement::new(StatementKind::VarDecl { data_type, lifetime, declarators }, self.span_from(start))
            }
            // Null statement
            TokenKind::Semicolon => {
                self.bump();
                Statement::new(StatementKind::Null, self.span_from(start))
            }
            // Expression statement (assignment, call, inc/dec)
            _ => {
                // Parse LHS expression, but stop at <= to allow nonblocking assignment
                let expr = self.parse_lvalue_or_expr();
                // Check for blocking/nonblocking assignment
                if self.at(TokenKind::Assign) || self.at_any(&[
                    TokenKind::PlusAssign, TokenKind::MinusAssign,
                    TokenKind::StarAssign, TokenKind::SlashAssign,
                    TokenKind::PercentAssign, TokenKind::AndAssign,
                    TokenKind::OrAssign, TokenKind::XorAssign,
                    TokenKind::ShiftLeftAssign, TokenKind::ShiftRightAssign,
                ]) {
                    self.bump();
                    let rvalue = self.parse_expression();
                    self.expect(TokenKind::Semicolon);
                    Statement::new(StatementKind::BlockingAssign { lvalue: expr, rvalue }, self.span_from(start))
                } else if self.at(TokenKind::Leq) {
                    // Nonblocking assignment: lvalue <= rvalue
                    self.bump();
                    let rvalue = self.parse_expression();
                    self.expect(TokenKind::Semicolon);
                    Statement::new(StatementKind::NonblockingAssign {
                        lvalue: expr, delay: None, rvalue,
                    }, self.span_from(start))
                } else {
                    self.expect(TokenKind::Semicolon);
                    Statement::new(StatementKind::Expr(expr), self.span_from(start))
                }
            }
        }
    }

    fn parse_seq_block(&mut self) -> Statement {
        let start = self.current().span.start;
        self.expect(TokenKind::KwBegin);
        let name = if self.eat(TokenKind::Colon).is_some() {
            Some(self.parse_identifier())
        } else { None };
        let mut stmts = Vec::new();
        while !self.at(TokenKind::KwEnd) && !self.at(TokenKind::Eof) {
            stmts.push(self.parse_statement());
        }
        self.expect(TokenKind::KwEnd);
        let _ = self.parse_end_label();
        Statement::new(StatementKind::SeqBlock { name, stmts }, self.span_from(start))
    }

    fn parse_par_block(&mut self) -> Statement {
        let start = self.current().span.start;
        self.expect(TokenKind::KwFork);
        let name = if self.eat(TokenKind::Colon).is_some() {
            Some(self.parse_identifier())
        } else { None };
        let mut stmts = Vec::new();
        while !self.at_any(&[TokenKind::KwJoin, TokenKind::KwJoin_any, TokenKind::KwJoin_none, TokenKind::Eof]) {
            stmts.push(self.parse_statement());
        }
        let join_type = match self.current_kind() {
            TokenKind::KwJoin_any => { self.bump(); JoinType::JoinAny }
            TokenKind::KwJoin_none => { self.bump(); JoinType::JoinNone }
            _ => { self.expect(TokenKind::KwJoin); JoinType::Join }
        };
        let _ = self.parse_end_label();
        Statement::new(StatementKind::ParBlock { name, join_type, stmts }, self.span_from(start))
    }

    fn parse_if_or_case(&mut self) -> Statement {
        let up = self.parse_unique_priority();
        if self.at(TokenKind::KwIf) {
            self.parse_if_with_priority(up)
        } else if self.at_any(&[TokenKind::KwCase, TokenKind::KwCasex, TokenKind::KwCasez]) {
            self.parse_case_with_priority(up)
        } else {
            self.parse_if_with_priority(up)
        }
    }

    fn parse_unique_priority(&mut self) -> Option<UniquePriority> {
        match self.current_kind() {
            TokenKind::KwUnique => { self.bump(); Some(UniquePriority::Unique) }
            TokenKind::KwUnique0 => { self.bump(); Some(UniquePriority::Unique0) }
            TokenKind::KwPriority => { self.bump(); Some(UniquePriority::Priority) }
            _ => None,
        }
    }

    fn parse_if_with_priority(&mut self, up: Option<UniquePriority>) -> Statement {
        let start = self.current().span.start;
        self.expect(TokenKind::KwIf);
        self.expect(TokenKind::LParen);
        let condition = self.parse_expression();
        self.expect(TokenKind::RParen);
        let then_stmt = self.parse_statement();
        let else_stmt = if self.eat(TokenKind::KwElse).is_some() {
            Some(Box::new(self.parse_statement()))
        } else { None };
        Statement::new(StatementKind::If {
            condition, then_stmt: Box::new(then_stmt), else_stmt,
            unique_priority: up,
        }, self.span_from(start))
    }

    fn parse_case_statement(&mut self) -> Statement {
        self.parse_case_with_priority(None)
    }

    fn parse_case_with_priority(&mut self, up: Option<UniquePriority>) -> Statement {
        let start = self.current().span.start;
        let kind = match self.bump().kind {
            TokenKind::KwCasex => CaseKind::Casex,
            TokenKind::KwCasez => CaseKind::Casez,
            _ => CaseKind::Case,
        };
        self.expect(TokenKind::LParen);
        let expr = self.parse_expression();
        self.expect(TokenKind::RParen);
        // Check for "inside" keyword
        let kind = if kind == CaseKind::Case && self.eat(TokenKind::KwInside).is_some() {
            CaseKind::CaseInside
        } else { kind };

        let mut items = Vec::new();
        while !self.at(TokenKind::KwEndcase) && !self.at(TokenKind::Eof) {
            let istart = self.current().span.start;
            if self.eat(TokenKind::KwDefault).is_some() {
                self.eat(TokenKind::Colon);
                let stmt = self.parse_statement();
                items.push(CaseItem { patterns: Vec::new(), is_default: true, stmt, span: self.span_from(istart) });
            } else {
                let mut patterns = Vec::new();
                loop {
                    patterns.push(self.parse_expression());
                    if self.eat(TokenKind::Comma).is_none() { break; }
                }
                self.expect(TokenKind::Colon);
                let stmt = self.parse_statement();
                items.push(CaseItem { patterns, is_default: false, stmt, span: self.span_from(istart) });
            }
        }
        self.expect(TokenKind::KwEndcase);
        Statement::new(StatementKind::Case {
            unique_priority: up, kind, expr, items,
        }, self.span_from(start))
    }

    fn parse_for_statement(&mut self) -> Statement {
        let start = self.current().span.start;
        self.expect(TokenKind::KwFor);
        self.expect(TokenKind::LParen);
        // Init
        let mut init = Vec::new();
        if !self.at(TokenKind::Semicolon) {
            if self.is_data_type_keyword() {
                let dt = self.parse_data_type();
                let name = self.parse_identifier();
                self.expect(TokenKind::Assign);
                let val = self.parse_expression();
                init.push(ForInit::VarDecl { data_type: dt, name, init: val });
            } else {
                let lv = self.parse_expression();
                self.expect(TokenKind::Assign);
                let rv = self.parse_expression();
                init.push(ForInit::Assign { lvalue: lv, rvalue: rv });
            }
        }
        self.expect(TokenKind::Semicolon);
        let condition = if !self.at(TokenKind::Semicolon) {
            Some(self.parse_expression())
        } else { None };
        self.expect(TokenKind::Semicolon);
        let mut step = Vec::new();
        if !self.at(TokenKind::RParen) {
            loop {
                // Step can be assignment (i = i + 1) or expression (i++)
                let expr = self.parse_expression();
                if self.eat(TokenKind::Assign).is_some() {
                    let rhs = self.parse_expression();
                    // Wrap as AssignOp expression (lhs = rhs)
                    step.push(Expression::new(
                        ExprKind::Binary { op: BinaryOp::Assign, left: Box::new(expr), right: Box::new(rhs) },
                        crate::ast::Span { start: 0, end: 0 },
                    ));
                } else {
                    step.push(expr);
                }
                if !self.eat(TokenKind::Comma).is_some() { break; }
            }
        }
        self.expect(TokenKind::RParen);
        let body = self.parse_statement();
        Statement::new(StatementKind::For {
            init, condition, step, body: Box::new(body),
        }, self.span_from(start))
    }

    fn parse_foreach_statement(&mut self) -> Statement {
        let start = self.current().span.start;
        self.expect(TokenKind::KwForeach);
        self.expect(TokenKind::LParen);
        let array = self.parse_identifier();
        let array_expr = crate::ast::expr::Expression::new(
            crate::ast::expr::ExprKind::Ident(crate::ast::expr::HierarchicalIdentifier {
                root: None,
                path: vec![crate::ast::expr::HierPathSegment { name: array, selects: Vec::new() }],
                span: self.span_from(start),
            }),
            self.span_from(start),
        );
        self.expect(TokenKind::LBracket);
        let mut vars = Vec::new();
        loop {
            if self.at(TokenKind::RBracket) { break; }
            if self.at(TokenKind::Comma) {
                vars.push(None);
            } else {
                vars.push(Some(self.parse_identifier()));
            }
            if self.eat(TokenKind::Comma).is_none() { break; }
        }
        self.expect(TokenKind::RBracket);
        self.expect(TokenKind::RParen);
        let body = self.parse_statement();
        Statement::new(StatementKind::Foreach {
            array: array_expr, vars, body: Box::new(body),
        }, self.span_from(start))
    }

    fn parse_while_statement(&mut self) -> Statement {
        let start = self.current().span.start;
        self.expect(TokenKind::KwWhile);
        self.expect(TokenKind::LParen);
        let condition = self.parse_expression();
        self.expect(TokenKind::RParen);
        let body = self.parse_statement();
        Statement::new(StatementKind::While { condition, body: Box::new(body) }, self.span_from(start))
    }

    fn parse_do_while_statement(&mut self) -> Statement {
        let start = self.current().span.start;
        self.expect(TokenKind::KwDo);
        let body = self.parse_statement();
        self.expect(TokenKind::KwWhile);
        self.expect(TokenKind::LParen);
        let condition = self.parse_expression();
        self.expect(TokenKind::RParen);
        self.expect(TokenKind::Semicolon);
        Statement::new(StatementKind::DoWhile { body: Box::new(body), condition }, self.span_from(start))
    }

    fn parse_repeat_statement(&mut self) -> Statement {
        let start = self.current().span.start;
        self.expect(TokenKind::KwRepeat);
        self.expect(TokenKind::LParen);
        let count = self.parse_expression();
        self.expect(TokenKind::RParen);
        let body = self.parse_statement();
        Statement::new(StatementKind::Repeat { count, body: Box::new(body) }, self.span_from(start))
    }

    pub(super) fn parse_event_control(&mut self) -> EventControl {
        self.expect(TokenKind::At);
        if self.eat(TokenKind::Star).is_some() {
            return EventControl::Star;
        }
        if self.eat(TokenKind::LParen).is_some() {
            if self.eat(TokenKind::Star).is_some() {
                self.expect(TokenKind::RParen);
                return EventControl::ParenStar;
            }
            let mut events = Vec::new();
            loop {
                let estart = self.current().span.start;
                let edge = match self.current_kind() {
                    TokenKind::KwPosedge => { self.bump(); Some(Edge::Posedge) }
                    TokenKind::KwNegedge => { self.bump(); Some(Edge::Negedge) }
                    TokenKind::KwEdge => { self.bump(); Some(Edge::Edge) }
                    _ => None,
                };
                let expr = self.parse_expression();
                let iff = if self.eat(TokenKind::KwIff).is_some() {
                    Some(self.parse_expression())
                } else { None };
                events.push(EventExpr { edge, expr, iff, span: self.span_from(estart) });
                if self.eat(TokenKind::KwOr).is_some() || self.eat(TokenKind::Comma).is_some() {
                    continue;
                }
                break;
            }
            self.expect(TokenKind::RParen);
            EventControl::EventExpr(events)
        } else {
            let id = self.parse_identifier();
            EventControl::Identifier(id)
        }
    }

    pub(super) fn parse_assertion_statement(&mut self) -> AssertionStatement {
        let start = self.current().span.start;
        let kind = match self.bump().kind {
            TokenKind::KwAssume => AssertionKind::Assume,
            TokenKind::KwCover => AssertionKind::Cover,
            _ => AssertionKind::Assert,
        };
        self.expect(TokenKind::LParen);
        let expr = self.parse_expression();
        self.expect(TokenKind::RParen);
        let action = if !self.at(TokenKind::Semicolon) && !self.at(TokenKind::KwElse) {
            Some(Box::new(self.parse_statement()))
        } else {
            if self.at(TokenKind::Semicolon) { self.bump(); }
            None
        };
        let else_action = if self.eat(TokenKind::KwElse).is_some() {
            Some(Box::new(self.parse_statement()))
        } else { None };
        AssertionStatement { kind, expr, action, else_action, span: self.span_from(start) }
    }
}
