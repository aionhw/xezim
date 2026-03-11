//! Lexer/Scanner for SystemVerilog (IEEE 1800-2017 §5)

use super::token::{Token, TokenKind, keyword};
use crate::ast::Span;

pub struct Lexer<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a str) -> Self {
        Self { input: source.as_bytes(), pos: 0 }
    }

    pub fn tokenize(mut self) -> Vec<Token> {
        let mut tokens = Vec::new();
        loop {
            self.skip_whitespace();
            if self.pos >= self.input.len() {
                tokens.push(Token::new(TokenKind::Eof, String::new(), Span::new(self.pos, self.pos)));
                break;
            }
            // Skip comments
            if self.pos + 1 < self.input.len() && self.input[self.pos] == b'/' {
                if self.input[self.pos + 1] == b'/' {
                    while self.pos < self.input.len() && self.input[self.pos] != b'\n' { self.pos += 1; }
                    continue;
                }
                if self.input[self.pos + 1] == b'*' {
                    self.pos += 2;
                    while self.pos + 1 < self.input.len() {
                        if self.input[self.pos] == b'*' && self.input[self.pos + 1] == b'/' { self.pos += 2; break; }
                        self.pos += 1;
                    }
                    continue;
                }
            }
            tokens.push(self.scan_token());
        }
        tokens
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos + 1).copied()
    }

    fn skip_whitespace(&mut self) {
        while self.pos < self.input.len() && self.input[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn make_token(&mut self, start: usize, kind: TokenKind) -> Token {
        self.pos += 1;
        let text = String::from_utf8_lossy(&self.input[start..self.pos]).to_string();
        Token::new(kind, text, Span::new(start, self.pos))
    }

    fn scan_token(&mut self) -> Token {
        let start = self.pos;
        let ch = self.input[self.pos];
        match ch {
            // String literal
            b'"' => self.scan_string(start),
            // Compiler directive
            b'`' => self.scan_directive(start),
            // System identifier
            b'$' => {
                if self.peek().map_or(false, |c| c.is_ascii_alphabetic() || c == b'_') {
                    self.scan_system_id(start)
                } else {
                    self.make_token(start, TokenKind::Dollar)
                }
            }
            // Escaped identifier
            b'\\' => self.scan_escaped_id(start),
            // Number starting with apostrophe: 'b, 'h, 'o, 'd, 's, '0, '1, 'x, 'z
            b'\'' => {
                if self.peek().map_or(false, |c| matches!(c, b'{')) {
                    self.pos += 2;
                    Token::new(TokenKind::ApostropheLBrace, "'{".into(), Span::new(start, self.pos))
                } else if self.peek().map_or(false, |c| matches!(c, b'0' | b'1' | b'x' | b'X' | b'z' | b'Z')) {
                    self.pos += 2;
                    let text = String::from_utf8_lossy(&self.input[start..self.pos]).to_string();
                    Token::new(TokenKind::UnbasedUnsizedLiteral, text, Span::new(start, self.pos))
                } else {
                    self.scan_based_number(start)
                }
            }
            // Numbers
            b'0'..=b'9' => self.scan_number(start),
            // Identifiers / keywords
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => self.scan_identifier(start),
            // Operators and punctuation
            b'(' => self.make_token(start, TokenKind::LParen),
            b')' => self.make_token(start, TokenKind::RParen),
            b'[' => self.make_token(start, TokenKind::LBracket),
            b']' => self.make_token(start, TokenKind::RBracket),
            b'{' => self.make_token(start, TokenKind::LBrace),
            b'}' => self.make_token(start, TokenKind::RBrace),
            b';' => self.make_token(start, TokenKind::Semicolon),
            b',' => self.make_token(start, TokenKind::Comma),
            b'.' => self.make_token(start, TokenKind::Dot),
            b'@' => self.make_token(start, TokenKind::At),
            b'?' => self.make_token(start, TokenKind::Question),
            b'~' => {
                self.pos += 1;
                match self.input.get(self.pos) {
                    Some(b'&') => { self.pos += 1; Token::new(TokenKind::BitNand, "~&".into(), Span::new(start, self.pos)) }
                    Some(b'|') => { self.pos += 1; Token::new(TokenKind::BitNor, "~|".into(), Span::new(start, self.pos)) }
                    Some(b'^') => { self.pos += 1; Token::new(TokenKind::BitXnor, "~^".into(), Span::new(start, self.pos)) }
                    _ => Token::new(TokenKind::BitNot, "~".into(), Span::new(start, self.pos))
                }
            }
            b'+' => {
                self.pos += 1;
                match self.input.get(self.pos) {
                    Some(b'+') => { self.pos += 1; Token::new(TokenKind::Increment, "++".into(), Span::new(start, self.pos)) }
                    Some(b'=') => { self.pos += 1; Token::new(TokenKind::PlusAssign, "+=".into(), Span::new(start, self.pos)) }
                    Some(b':') => { self.pos += 1; Token::new(TokenKind::PlusColon, "+:".into(), Span::new(start, self.pos)) }
                    _ => Token::new(TokenKind::Plus, "+".into(), Span::new(start, self.pos))
                }
            }
            b'-' => {
                self.pos += 1;
                match self.input.get(self.pos) {
                    Some(b'-') => { self.pos += 1; Token::new(TokenKind::Decrement, "--".into(), Span::new(start, self.pos)) }
                    Some(b'=') => { self.pos += 1; Token::new(TokenKind::MinusAssign, "-=".into(), Span::new(start, self.pos)) }
                    Some(b'>') => {
                        self.pos += 1;
                        if self.input.get(self.pos) == Some(&b'>') {
                            self.pos += 1; Token::new(TokenKind::DoubleArrow, "->>".into(), Span::new(start, self.pos))
                        } else {
                            Token::new(TokenKind::Arrow, "->".into(), Span::new(start, self.pos))
                        }
                    }
                    Some(b':') => { self.pos += 1; Token::new(TokenKind::MinusColon, "-:".into(), Span::new(start, self.pos)) }
                    _ => Token::new(TokenKind::Minus, "-".into(), Span::new(start, self.pos))
                }
            }
            b'*' => {
                self.pos += 1;
                match self.input.get(self.pos) {
                    Some(b'*') => { self.pos += 1; Token::new(TokenKind::DoubleStar, "**".into(), Span::new(start, self.pos)) }
                    Some(b'=') => { self.pos += 1; Token::new(TokenKind::StarAssign, "*=".into(), Span::new(start, self.pos)) }
                    _ => Token::new(TokenKind::Star, "*".into(), Span::new(start, self.pos))
                }
            }
            b'/' => {
                self.pos += 1;
                match self.input.get(self.pos) {
                    Some(b'=') => { self.pos += 1; Token::new(TokenKind::SlashAssign, "/=".into(), Span::new(start, self.pos)) }
                    _ => Token::new(TokenKind::Slash, "/".into(), Span::new(start, self.pos))
                }
            }
            b'%' => {
                self.pos += 1;
                if self.input.get(self.pos) == Some(&b'=') { self.pos += 1; Token::new(TokenKind::PercentAssign, "%=".into(), Span::new(start, self.pos)) }
                else { Token::new(TokenKind::Percent, "%".into(), Span::new(start, self.pos)) }
            }
            b'!' => {
                self.pos += 1;
                match self.input.get(self.pos) {
                    Some(b'=') => {
                        self.pos += 1;
                        match self.input.get(self.pos) {
                            Some(b'=') => { self.pos += 1; Token::new(TokenKind::CaseNeq, "!==".into(), Span::new(start, self.pos)) }
                            Some(b'?') => { self.pos += 1; Token::new(TokenKind::WildcardNeq, "!=?".into(), Span::new(start, self.pos)) }
                            _ => Token::new(TokenKind::Neq, "!=".into(), Span::new(start, self.pos))
                        }
                    }
                    _ => Token::new(TokenKind::LogNot, "!".into(), Span::new(start, self.pos))
                }
            }
            b'=' => {
                self.pos += 1;
                match self.input.get(self.pos) {
                    Some(b'=') => {
                        self.pos += 1;
                        match self.input.get(self.pos) {
                            Some(b'=') => { self.pos += 1; Token::new(TokenKind::CaseEq, "===".into(), Span::new(start, self.pos)) }
                            Some(b'?') => { self.pos += 1; Token::new(TokenKind::WildcardEq, "==?".into(), Span::new(start, self.pos)) }
                            _ => Token::new(TokenKind::Eq, "==".into(), Span::new(start, self.pos))
                        }
                    }
                    Some(b'>') => { self.pos += 1; Token::new(TokenKind::FatArrow, "=>".into(), Span::new(start, self.pos)) }
                    _ => Token::new(TokenKind::Assign, "=".into(), Span::new(start, self.pos))
                }
            }
            b'<' => {
                self.pos += 1;
                match self.input.get(self.pos) {
                    Some(b'=') => { self.pos += 1; Token::new(TokenKind::Leq, "<=".into(), Span::new(start, self.pos)) }
                    Some(b'<') => {
                        self.pos += 1;
                        match self.input.get(self.pos) {
                            Some(b'<') => { self.pos += 1;
                                if self.input.get(self.pos) == Some(&b'=') { self.pos += 1; Token::new(TokenKind::ArithShiftLeftAssign, "<<<=".into(), Span::new(start, self.pos)) }
                                else { Token::new(TokenKind::ArithShiftLeft, "<<<".into(), Span::new(start, self.pos)) }
                            }
                            Some(b'=') => { self.pos += 1; Token::new(TokenKind::ShiftLeftAssign, "<<=".into(), Span::new(start, self.pos)) }
                            _ => Token::new(TokenKind::ShiftLeft, "<<".into(), Span::new(start, self.pos))
                        }
                    }
                    Some(b'-') => {
                        self.pos += 1;
                        if self.input.get(self.pos) == Some(&b'>') { self.pos += 1; Token::new(TokenKind::LogEquiv, "<->".into(), Span::new(start, self.pos)) }
                        else { self.pos -= 1; Token::new(TokenKind::Lt, "<".into(), Span::new(start, self.pos)) }
                    }
                    _ => Token::new(TokenKind::Lt, "<".into(), Span::new(start, self.pos))
                }
            }
            b'>' => {
                self.pos += 1;
                match self.input.get(self.pos) {
                    Some(b'=') => { self.pos += 1; Token::new(TokenKind::Geq, ">=".into(), Span::new(start, self.pos)) }
                    Some(b'>') => {
                        self.pos += 1;
                        match self.input.get(self.pos) {
                            Some(b'>') => { self.pos += 1;
                                if self.input.get(self.pos) == Some(&b'=') { self.pos += 1; Token::new(TokenKind::ArithShiftRightAssign, ">>>=".into(), Span::new(start, self.pos)) }
                                else { Token::new(TokenKind::ArithShiftRight, ">>>".into(), Span::new(start, self.pos)) }
                            }
                            Some(b'=') => { self.pos += 1; Token::new(TokenKind::ShiftRightAssign, ">>=".into(), Span::new(start, self.pos)) }
                            _ => Token::new(TokenKind::ShiftRight, ">>".into(), Span::new(start, self.pos))
                        }
                    }
                    _ => Token::new(TokenKind::Gt, ">".into(), Span::new(start, self.pos))
                }
            }
            b'&' => {
                self.pos += 1;
                match self.input.get(self.pos) {
                    Some(b'&') => { self.pos += 1; Token::new(TokenKind::LogAnd, "&&".into(), Span::new(start, self.pos)) }
                    Some(b'=') => { self.pos += 1; Token::new(TokenKind::AndAssign, "&=".into(), Span::new(start, self.pos)) }
                    _ => Token::new(TokenKind::BitAnd, "&".into(), Span::new(start, self.pos))
                }
            }
            b'|' => {
                self.pos += 1;
                match self.input.get(self.pos) {
                    Some(b'|') => { self.pos += 1; Token::new(TokenKind::LogOr, "||".into(), Span::new(start, self.pos)) }
                    Some(b'=') => { self.pos += 1; Token::new(TokenKind::OrAssign, "|=".into(), Span::new(start, self.pos)) }
                    _ => Token::new(TokenKind::BitOr, "|".into(), Span::new(start, self.pos))
                }
            }
            b'^' => {
                self.pos += 1;
                match self.input.get(self.pos) {
                    Some(b'~') => { self.pos += 1; Token::new(TokenKind::BitXnor, "^~".into(), Span::new(start, self.pos)) }
                    Some(b'=') => { self.pos += 1; Token::new(TokenKind::XorAssign, "^=".into(), Span::new(start, self.pos)) }
                    _ => Token::new(TokenKind::BitXor, "^".into(), Span::new(start, self.pos))
                }
            }
            b'#' => {
                self.pos += 1;
                if self.input.get(self.pos) == Some(&b'#') {
                    self.pos += 1; Token::new(TokenKind::HashHash, "##".into(), Span::new(start, self.pos))
                } else {
                    Token::new(TokenKind::Hash, "#".into(), Span::new(start, self.pos))
                }
            }
            b':' => {
                self.pos += 1;
                match self.input.get(self.pos) {
                    Some(b':') => { self.pos += 1; Token::new(TokenKind::DoubleColon, "::".into(), Span::new(start, self.pos)) }
                    Some(b'/') => { self.pos += 1; Token::new(TokenKind::ColonSlash, ":/".into(), Span::new(start, self.pos)) }
                    _ => Token::new(TokenKind::Colon, ":".into(), Span::new(start, self.pos))
                }
            }
            _ => self.make_token(start, TokenKind::Unknown),
        }
    }

    fn scan_string(&mut self, start: usize) -> Token {
        self.pos += 1; // skip opening "
        while self.pos < self.input.len() {
            if self.input[self.pos] == b'\\' { self.pos += 2; continue; }
            if self.input[self.pos] == b'"' { self.pos += 1; break; }
            self.pos += 1;
        }
        let text = String::from_utf8_lossy(&self.input[start..self.pos]).to_string();
        Token::new(TokenKind::StringLiteral, text, Span::new(start, self.pos))
    }

    fn scan_directive(&mut self, start: usize) -> Token {
        self.pos += 1; // skip `
        while self.pos < self.input.len() && (self.input[self.pos].is_ascii_alphanumeric() || self.input[self.pos] == b'_') {
            self.pos += 1;
        }
        let text = String::from_utf8_lossy(&self.input[start..self.pos]).to_string();
        Token::new(TokenKind::Directive, text, Span::new(start, self.pos))
    }

    fn scan_system_id(&mut self, start: usize) -> Token {
        self.pos += 1; // skip $
        while self.pos < self.input.len() && (self.input[self.pos].is_ascii_alphanumeric() || self.input[self.pos] == b'_' || self.input[self.pos] == b'$') {
            self.pos += 1;
        }
        let text = String::from_utf8_lossy(&self.input[start..self.pos]).to_string();
        Token::new(TokenKind::SystemIdentifier, text, Span::new(start, self.pos))
    }

    fn scan_escaped_id(&mut self, start: usize) -> Token {
        self.pos += 1; // skip backslash
        while self.pos < self.input.len() && !self.input[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
        let text = String::from_utf8_lossy(&self.input[start..self.pos]).to_string();
        Token::new(TokenKind::EscapedIdentifier, text, Span::new(start, self.pos))
    }

    fn scan_identifier(&mut self, start: usize) -> Token {
        while self.pos < self.input.len() && (self.input[self.pos].is_ascii_alphanumeric() || self.input[self.pos] == b'_' || self.input[self.pos] == b'$') {
            self.pos += 1;
        }
        let text = String::from_utf8_lossy(&self.input[start..self.pos]).to_string();
        let kind = keyword(&text).unwrap_or(TokenKind::Identifier);
        Token::new(kind, text, Span::new(start, self.pos))
    }

    fn scan_number(&mut self, start: usize) -> Token {
        // Consume decimal digits (and underscores)
        while self.pos < self.input.len() && (self.input[self.pos].is_ascii_digit() || self.input[self.pos] == b'_') {
            self.pos += 1;
        }
        // Check for based literal: <size>'<base><value>
        if self.pos < self.input.len() && self.input[self.pos] == b'\'' {
            let next = self.input.get(self.pos + 1).copied().unwrap_or(0);
            if matches!(next, b's' | b'S' | b'b' | b'B' | b'o' | b'O' | b'd' | b'D' | b'h' | b'H') {
                self.pos += 1; // skip '
                if matches!(self.input.get(self.pos), Some(b's' | b'S')) { self.pos += 1; }
                if self.pos < self.input.len() && matches!(self.input[self.pos], b'b' | b'B' | b'o' | b'O' | b'd' | b'D' | b'h' | b'H') {
                    self.pos += 1;
                }
                // IEEE 1800-2017 §5.7.1: whitespace allowed between base and value
                while self.pos < self.input.len() && (self.input[self.pos] == b' ' || self.input[self.pos] == b'\t') {
                    self.pos += 1;
                }
                while self.pos < self.input.len() && (self.input[self.pos].is_ascii_alphanumeric() || self.input[self.pos] == b'_' || self.input[self.pos] == b'?' || self.input[self.pos] == b'x' || self.input[self.pos] == b'X' || self.input[self.pos] == b'z' || self.input[self.pos] == b'Z') {
                    self.pos += 1;
                }
                let text = String::from_utf8_lossy(&self.input[start..self.pos]).to_string();
                return Token::new(TokenKind::IntegerLiteral, text, Span::new(start, self.pos));
            }
        }
        // Check for real literal: digits.digits or digitsEexp
        if self.pos < self.input.len() && self.input[self.pos] == b'.' && self.input.get(self.pos + 1).map_or(false, |c| c.is_ascii_digit()) {
            self.pos += 1;
            while self.pos < self.input.len() && (self.input[self.pos].is_ascii_digit() || self.input[self.pos] == b'_') { self.pos += 1; }
            // Optional exponent
            if self.pos < self.input.len() && matches!(self.input[self.pos], b'e' | b'E') {
                self.pos += 1;
                if self.pos < self.input.len() && matches!(self.input[self.pos], b'+' | b'-') { self.pos += 1; }
                while self.pos < self.input.len() && (self.input[self.pos].is_ascii_digit() || self.input[self.pos] == b'_') { self.pos += 1; }
            }
            let text = String::from_utf8_lossy(&self.input[start..self.pos]).to_string();
            return Token::new(TokenKind::RealLiteral, text, Span::new(start, self.pos));
        }
        // Exponent without decimal point
        if self.pos < self.input.len() && matches!(self.input[self.pos], b'e' | b'E') {
            let saved = self.pos;
            self.pos += 1;
            if self.pos < self.input.len() && matches!(self.input[self.pos], b'+' | b'-') { self.pos += 1; }
            if self.pos < self.input.len() && self.input[self.pos].is_ascii_digit() {
                while self.pos < self.input.len() && (self.input[self.pos].is_ascii_digit() || self.input[self.pos] == b'_') { self.pos += 1; }
                let text = String::from_utf8_lossy(&self.input[start..self.pos]).to_string();
                return Token::new(TokenKind::RealLiteral, text, Span::new(start, self.pos));
            }
            self.pos = saved;
        }
        // Time literal check
        if self.pos + 1 < self.input.len() {
            let rest = &self.input[self.pos..];
            for suffix in &[b"ns" as &[u8], b"us", b"ms", b"ps", b"fs", b"s"] {
                if rest.starts_with(suffix) && !rest.get(suffix.len()).map_or(false, |c| c.is_ascii_alphanumeric()) {
                    self.pos += suffix.len();
                    let text = String::from_utf8_lossy(&self.input[start..self.pos]).to_string();
                    return Token::new(TokenKind::TimeLiteral, text, Span::new(start, self.pos));
                }
            }
        }
        let text = String::from_utf8_lossy(&self.input[start..self.pos]).to_string();
        Token::new(TokenKind::IntegerLiteral, text, Span::new(start, self.pos))
    }

    fn scan_based_number(&mut self, start: usize) -> Token {
        self.pos += 1; // skip '
        if self.pos < self.input.len() && matches!(self.input[self.pos], b's' | b'S') { self.pos += 1; }
        if self.pos < self.input.len() && matches!(self.input[self.pos], b'b' | b'B' | b'o' | b'O' | b'd' | b'D' | b'h' | b'H') {
            self.pos += 1;
        }
        // IEEE 1800-2017 §5.7.1: whitespace allowed between base and value
        while self.pos < self.input.len() && (self.input[self.pos] == b' ' || self.input[self.pos] == b'\t') {
            self.pos += 1;
        }
        while self.pos < self.input.len() && (self.input[self.pos].is_ascii_alphanumeric() || self.input[self.pos] == b'_' || self.input[self.pos] == b'?' || self.input[self.pos] == b'x' || self.input[self.pos] == b'X' || self.input[self.pos] == b'z' || self.input[self.pos] == b'Z') {
            self.pos += 1;
        }
        let text = String::from_utf8_lossy(&self.input[start..self.pos]).to_string();
        Token::new(TokenKind::IntegerLiteral, text, Span::new(start, self.pos))
    }
}
