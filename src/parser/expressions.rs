//! Expression parsing (IEEE 1800-2017 §A.8) with Pratt precedence climbing.

use super::Parser;
use crate::ast::expr::*;
use crate::ast::Span;
use crate::lexer::token::TokenKind;

impl Parser {
    pub(super) fn parse_expression(&mut self) -> Expression {
        self.parse_expr_bp(0)
    }

    /// Parse an expression that could be an lvalue in a statement context.
    /// Parses only up to identifier/select/concat without consuming `<=` or `=`.
    /// Falls back to full expression if the result doesn't look like an lvalue.
    pub(super) fn parse_lvalue_or_expr(&mut self) -> Expression {
        let save_pos = self.pos;
        // Parse primary + all postfix selects (bit/part/index selects, member access)
        let mut lval = self.parse_prefix();

        // Parse postfix selects: [idx], [l:r], [idx+:w], [idx-:w], .member
        loop {
            if self.at(TokenKind::LBracket) {
                let s = self.current().span.start;
                self.bump();
                let idx = self.parse_expression();
                if self.eat(TokenKind::Colon).is_some() {
                    let right = self.parse_expression();
                    self.expect(TokenKind::RBracket);
                    lval = Expression::new(ExprKind::RangeSelect {
                        expr: Box::new(lval), kind: RangeKind::Constant,
                        left: Box::new(idx), right: Box::new(right),
                    }, self.span_from(s));
                } else if self.eat(TokenKind::PlusColon).is_some() {
                    let width = self.parse_expression();
                    self.expect(TokenKind::RBracket);
                    lval = Expression::new(ExprKind::RangeSelect {
                        expr: Box::new(lval), kind: RangeKind::IndexedUp,
                        left: Box::new(idx), right: Box::new(width),
                    }, self.span_from(s));
                } else if self.eat(TokenKind::MinusColon).is_some() {
                    let width = self.parse_expression();
                    self.expect(TokenKind::RBracket);
                    lval = Expression::new(ExprKind::RangeSelect {
                        expr: Box::new(lval), kind: RangeKind::IndexedDown,
                        left: Box::new(idx), right: Box::new(width),
                    }, self.span_from(s));
                } else {
                    self.expect(TokenKind::RBracket);
                    lval = Expression::new(ExprKind::Index {
                        expr: Box::new(lval), index: Box::new(idx),
                    }, self.span_from(s));
                }
            } else if self.at(TokenKind::Dot) {
                let s = self.current().span.start;
                self.bump();
                let member = self.parse_identifier();
                lval = Expression::new(ExprKind::MemberAccess {
                    expr: Box::new(lval), member,
                }, self.span_from(s));
            } else {
                break;
            }
        }

        // If followed by `<=` or `=` or compound assign, this is likely an lvalue
        if self.at(TokenKind::Leq) || self.at(TokenKind::Assign) || self.at_any(&[
            TokenKind::PlusAssign, TokenKind::MinusAssign,
            TokenKind::StarAssign, TokenKind::SlashAssign,
            TokenKind::PercentAssign, TokenKind::AndAssign,
            TokenKind::OrAssign, TokenKind::XorAssign,
            TokenKind::ShiftLeftAssign, TokenKind::ShiftRightAssign,
        ]) {
            return lval;
        }

        // Otherwise, the prefix alone wasn't enough; rewind and parse as a full expression
        self.pos = save_pos;
        self.parse_expr_bp(0)
    }

    /// Pratt parser: parse expression with minimum binding power.
    fn parse_expr_bp(&mut self, min_bp: u8) -> Expression {
        let start = self.current().span.start;
        let mut lhs = self.parse_prefix();

        loop {
            // Check for postfix: ++ --
            if self.at(TokenKind::Increment) || self.at(TokenKind::Decrement) {
                let op = if self.at(TokenKind::Increment) { UnaryOp::PostIncr } else { UnaryOp::PostDecr };
                let (l_bp, _) = postfix_bp();
                if l_bp < min_bp { break; }
                self.bump();
                lhs = Expression::new(ExprKind::Unary { op, operand: Box::new(lhs) }, self.span_from(start));
                continue;
            }

            // Binary/ternary operators
            if let Some((op, l_bp, r_bp)) = self.infix_bp() {
                if l_bp < min_bp { break; }
                self.bump();

                // Ternary operator
                if op == BinaryOp::Add && self.at(TokenKind::Colon) {
                    // This shouldn't happen here; ternary handled below
                }

                let rhs = self.parse_expr_bp(r_bp);
                lhs = Expression::new(ExprKind::Binary {
                    op, left: Box::new(lhs), right: Box::new(rhs),
                }, self.span_from(start));
                continue;
            }

            // Ternary: ? :
            if self.at(TokenKind::Question) {
                let (l_bp, _) = ternary_bp();
                if l_bp < min_bp { break; }
                self.bump();
                let then_expr = self.parse_expr_bp(0);
                self.expect(TokenKind::Colon);
                let else_expr = self.parse_expr_bp(l_bp);
                lhs = Expression::new(ExprKind::Conditional {
                    condition: Box::new(lhs),
                    then_expr: Box::new(then_expr),
                    else_expr: Box::new(else_expr),
                }, self.span_from(start));
                continue;
            }

            // Member access: .ident
            if self.at(TokenKind::Dot) {
                self.bump();
                let member = self.parse_identifier();
                // Method call: .method(args)
                if self.at(TokenKind::LParen) {
                    let member_expr = Expression::new(ExprKind::MemberAccess {
                        expr: Box::new(lhs), member,
                    }, self.span_from(start));
                    let args = self.parse_call_args();
                    lhs = Expression::new(ExprKind::Call {
                        func: Box::new(member_expr), args,
                    }, self.span_from(start));
                } else {
                    lhs = Expression::new(ExprKind::MemberAccess {
                        expr: Box::new(lhs), member,
                    }, self.span_from(start));
                }
                continue;
            }

            // Index/range select: [expr] or [expr:expr] or [expr+:expr]
            if self.at(TokenKind::LBracket) {
                self.bump();
                let idx = self.parse_expression();
                if self.eat(TokenKind::Colon).is_some() {
                    let right = self.parse_expression();
                    self.expect(TokenKind::RBracket);
                    lhs = Expression::new(ExprKind::RangeSelect {
                        expr: Box::new(lhs),
                        kind: RangeKind::Constant,
                        left: Box::new(idx),
                        right: Box::new(right),
                    }, self.span_from(start));
                } else if self.eat(TokenKind::PlusColon).is_some() {
                    let width = self.parse_expression();
                    self.expect(TokenKind::RBracket);
                    lhs = Expression::new(ExprKind::RangeSelect {
                        expr: Box::new(lhs),
                        kind: RangeKind::IndexedUp,
                        left: Box::new(idx),
                        right: Box::new(width),
                    }, self.span_from(start));
                } else if self.eat(TokenKind::MinusColon).is_some() {
                    let width = self.parse_expression();
                    self.expect(TokenKind::RBracket);
                    lhs = Expression::new(ExprKind::RangeSelect {
                        expr: Box::new(lhs),
                        kind: RangeKind::IndexedDown,
                        left: Box::new(idx),
                        right: Box::new(width),
                    }, self.span_from(start));
                } else {
                    self.expect(TokenKind::RBracket);
                    lhs = Expression::new(ExprKind::Index {
                        expr: Box::new(lhs),
                        index: Box::new(idx),
                    }, self.span_from(start));
                }
                continue;
            }

            break;
        }

        lhs
    }

    /// Parse prefix / primary expression.
    fn parse_prefix(&mut self) -> Expression {
        let start = self.current().span.start;

        match self.current_kind() {
            // Unary operators
            TokenKind::Plus => { self.bump(); let e = self.parse_expr_bp(prefix_bp()); Expression::new(ExprKind::Unary { op: UnaryOp::Plus, operand: Box::new(e) }, self.span_from(start)) }
            TokenKind::Minus => { self.bump(); let e = self.parse_expr_bp(prefix_bp()); Expression::new(ExprKind::Unary { op: UnaryOp::Minus, operand: Box::new(e) }, self.span_from(start)) }
            TokenKind::LogNot => { self.bump(); let e = self.parse_expr_bp(prefix_bp()); Expression::new(ExprKind::Unary { op: UnaryOp::LogNot, operand: Box::new(e) }, self.span_from(start)) }
            TokenKind::BitNot => { self.bump(); let e = self.parse_expr_bp(prefix_bp()); Expression::new(ExprKind::Unary { op: UnaryOp::BitNot, operand: Box::new(e) }, self.span_from(start)) }
            TokenKind::BitAnd => { self.bump(); let e = self.parse_expr_bp(prefix_bp()); Expression::new(ExprKind::Unary { op: UnaryOp::BitAnd, operand: Box::new(e) }, self.span_from(start)) }
            TokenKind::BitOr => { self.bump(); let e = self.parse_expr_bp(prefix_bp()); Expression::new(ExprKind::Unary { op: UnaryOp::BitOr, operand: Box::new(e) }, self.span_from(start)) }
            TokenKind::BitXor => { self.bump(); let e = self.parse_expr_bp(prefix_bp()); Expression::new(ExprKind::Unary { op: UnaryOp::BitXor, operand: Box::new(e) }, self.span_from(start)) }
            TokenKind::BitNand => { self.bump(); let e = self.parse_expr_bp(prefix_bp()); Expression::new(ExprKind::Unary { op: UnaryOp::BitNand, operand: Box::new(e) }, self.span_from(start)) }
            TokenKind::BitNor => { self.bump(); let e = self.parse_expr_bp(prefix_bp()); Expression::new(ExprKind::Unary { op: UnaryOp::BitNor, operand: Box::new(e) }, self.span_from(start)) }
            TokenKind::BitXnor => { self.bump(); let e = self.parse_expr_bp(prefix_bp()); Expression::new(ExprKind::Unary { op: UnaryOp::BitXnor, operand: Box::new(e) }, self.span_from(start)) }
            TokenKind::Increment => { self.bump(); let e = self.parse_expr_bp(prefix_bp()); Expression::new(ExprKind::Unary { op: UnaryOp::PreIncr, operand: Box::new(e) }, self.span_from(start)) }
            TokenKind::Decrement => { self.bump(); let e = self.parse_expr_bp(prefix_bp()); Expression::new(ExprKind::Unary { op: UnaryOp::PreDecr, operand: Box::new(e) }, self.span_from(start)) }

            // Parenthesized expression or mintypmax
            TokenKind::LParen => {
                self.bump();
                let inner = self.parse_expression();
                self.expect(TokenKind::RParen);
                Expression::new(ExprKind::Paren(Box::new(inner)), self.span_from(start))
            }

            // Concatenation / replication: { ... }
            TokenKind::LBrace => self.parse_concatenation(),

            // Assignment pattern: '{ ... }
            TokenKind::ApostropheLBrace => {
                self.bump();
                let mut exprs = Vec::new();
                loop {
                    if self.at(TokenKind::RBrace) || self.at(TokenKind::Eof) { break; }
                    exprs.push(self.parse_expression());
                    if self.eat(TokenKind::Comma).is_none() { break; }
                }
                self.expect(TokenKind::RBrace);
                Expression::new(ExprKind::AssignmentPattern(exprs), self.span_from(start))
            }

            // Number literals
            TokenKind::IntegerLiteral | TokenKind::RealLiteral | TokenKind::TimeLiteral => {
                let tok = self.bump();
                let num = parse_number_literal(&tok.text);
                Expression::new(ExprKind::Number(num), self.span_from(start))
            }
            TokenKind::UnbasedUnsizedLiteral => {
                let tok = self.bump();
                let ch = tok.text.chars().last().unwrap_or('0');
                Expression::new(ExprKind::Number(NumberLiteral::UnbasedUnsized(ch)), self.span_from(start))
            }

            // String literal
            TokenKind::StringLiteral => {
                let tok = self.bump();
                let s = tok.text[1..tok.text.len()-1].to_string();
                Expression::new(ExprKind::StringLiteral(s), self.span_from(start))
            }

            // System call: $display, etc.
            TokenKind::SystemIdentifier => {
                let tok = self.bump();
                let name = tok.text.clone();
                let args = if self.at(TokenKind::LParen) {
                    self.parse_call_args()
                } else { Vec::new() };
                Expression::new(ExprKind::SystemCall { name, args }, self.span_from(start))
            }

            // $
            TokenKind::Dollar => {
                self.bump();
                Expression::new(ExprKind::Dollar, self.span_from(start))
            }

            // null
            TokenKind::KwNull => {
                self.bump();
                Expression::new(ExprKind::Null, self.span_from(start))
            }

            // this
            TokenKind::KwThis => {
                self.bump();
                Expression::new(ExprKind::This, self.span_from(start))
            }

            // Identifier (possibly followed by function call)
            TokenKind::Identifier | TokenKind::EscapedIdentifier => {
                let id = self.parse_identifier();
                let hier = HierarchicalIdentifier {
                    root: None,
                    path: vec![HierPathSegment { name: id, selects: Vec::new() }],
                    span: self.span_from(start),
                };
                let expr = Expression::new(ExprKind::Ident(hier), self.span_from(start));
                // Check for function call
                if self.at(TokenKind::LParen) {
                    let args = self.parse_call_args();
                    Expression::new(ExprKind::Call {
                        func: Box::new(expr), args,
                    }, self.span_from(start))
                } else {
                    expr
                }
            }

            _ => {
                self.error(format!("expected expression, found {:?} '{}'", self.current_kind(), self.current().text));
                self.bump();
                Expression::new(ExprKind::Empty, self.span_from(start))
            }
        }
    }

    fn parse_concatenation(&mut self) -> Expression {
        let start = self.current().span.start;
        self.expect(TokenKind::LBrace);
        if self.at(TokenKind::RBrace) {
            self.bump();
            return Expression::new(ExprKind::Concatenation(Vec::new()), self.span_from(start));
        }
        let first = self.parse_expression();
        // Check for replication: { count { ... } }
        if self.at(TokenKind::LBrace) {
            self.bump();
            let mut exprs = Vec::new();
            loop {
                if self.at(TokenKind::RBrace) || self.at(TokenKind::Eof) { break; }
                exprs.push(self.parse_expression());
                if self.eat(TokenKind::Comma).is_none() { break; }
            }
            self.expect(TokenKind::RBrace);
            self.expect(TokenKind::RBrace);
            return Expression::new(ExprKind::Replication {
                count: Box::new(first), exprs,
            }, self.span_from(start));
        }
        let mut exprs = vec![first];
        while self.eat(TokenKind::Comma).is_some() {
            if self.at(TokenKind::RBrace) || self.at(TokenKind::Eof) { break; }
            exprs.push(self.parse_expression());
        }
        self.expect(TokenKind::RBrace);
        Expression::new(ExprKind::Concatenation(exprs), self.span_from(start))
    }

    pub(super) fn parse_call_args(&mut self) -> Vec<Expression> {
        let mut args = Vec::new();
        self.expect(TokenKind::LParen);
        if self.at(TokenKind::RParen) { self.bump(); return args; }
        loop {
            if self.at(TokenKind::RParen) || self.at(TokenKind::Eof) { break; }
            args.push(self.parse_expression());
            if self.eat(TokenKind::Comma).is_none() { break; }
        }
        self.expect(TokenKind::RParen);
        args
    }

    /// Get infix operator binding power for the current token.
    fn infix_bp(&self) -> Option<(BinaryOp, u8, u8)> {
        let kind = self.current_kind();
        match kind {
            TokenKind::LogOr => Some((BinaryOp::LogOr, 2, 3)),
            TokenKind::LogAnd => Some((BinaryOp::LogAnd, 4, 5)),
            TokenKind::BitOr => Some((BinaryOp::BitOr, 6, 7)),
            TokenKind::BitXor => Some((BinaryOp::BitXor, 8, 9)),
            TokenKind::BitXnor => Some((BinaryOp::BitXnor, 8, 9)),
            TokenKind::BitAnd => Some((BinaryOp::BitAnd, 10, 11)),
            TokenKind::Eq => Some((BinaryOp::Eq, 12, 13)),
            TokenKind::Neq => Some((BinaryOp::Neq, 12, 13)),
            TokenKind::CaseEq => Some((BinaryOp::CaseEq, 12, 13)),
            TokenKind::CaseNeq => Some((BinaryOp::CaseNeq, 12, 13)),
            TokenKind::WildcardEq => Some((BinaryOp::WildcardEq, 12, 13)),
            TokenKind::WildcardNeq => Some((BinaryOp::WildcardNeq, 12, 13)),
            TokenKind::Lt => Some((BinaryOp::Lt, 14, 15)),
            TokenKind::Gt => Some((BinaryOp::Gt, 14, 15)),
            TokenKind::Leq => Some((BinaryOp::Leq, 14, 15)),
            TokenKind::Geq => Some((BinaryOp::Geq, 14, 15)),
            TokenKind::ShiftLeft => Some((BinaryOp::ShiftLeft, 16, 17)),
            TokenKind::ShiftRight => Some((BinaryOp::ShiftRight, 16, 17)),
            TokenKind::ArithShiftLeft => Some((BinaryOp::ArithShiftLeft, 16, 17)),
            TokenKind::ArithShiftRight => Some((BinaryOp::ArithShiftRight, 16, 17)),
            TokenKind::Plus => Some((BinaryOp::Add, 18, 19)),
            TokenKind::Minus => Some((BinaryOp::Sub, 18, 19)),
            TokenKind::Star => Some((BinaryOp::Mul, 20, 21)),
            TokenKind::Slash => Some((BinaryOp::Div, 20, 21)),
            TokenKind::Percent => Some((BinaryOp::Mod, 20, 21)),
            TokenKind::DoubleStar => Some((BinaryOp::Power, 23, 22)), // right-assoc
            _ => None,
        }
    }
}

fn prefix_bp() -> u8 { 25 }
fn postfix_bp() -> (u8, ()) { (27, ()) }
fn ternary_bp() -> (u8, u8) { (1, 1) }

/// Parse a number literal string into our AST representation.
fn parse_number_literal(text: &str) -> NumberLiteral {
    // Try to parse as real
    if text.contains('.') || (text.contains('e') && !text.contains('\'')) || (text.contains('E') && !text.contains('\'')) {
        if let Ok(v) = text.replace('_', "").parse::<f64>() {
            return NumberLiteral::Real(v);
        }
    }
    // Based literal
    if let Some(apos) = text.find('\'') {
        let size_str = &text[..apos];
        let size = if size_str.is_empty() { None } else { size_str.replace('_', "").parse().ok() };
        let rest = &text[apos+1..];
        let (signed, rest) = if rest.starts_with('s') || rest.starts_with('S') {
            (true, &rest[1..])
        } else { (false, rest) };
        let (base, value) = if rest.len() > 1 {
            let b = match rest.as_bytes()[0] {
                b'h' | b'H' => NumberBase::Hex,
                b'b' | b'B' => NumberBase::Binary,
                b'o' | b'O' => NumberBase::Octal,
                b'd' | b'D' => NumberBase::Decimal,
                _ => NumberBase::Decimal,
            };
            (b, rest[1..].to_string())
        } else {
            (NumberBase::Decimal, rest.to_string())
        };
        return NumberLiteral::Integer { size, signed, base, value };
    }
    // Plain decimal — signed per Verilog standard (LRM section 5.7.1)
    NumberLiteral::Integer {
        size: None,
        signed: true,
        base: NumberBase::Decimal,
        value: text.replace('_', ""),
    }
}
