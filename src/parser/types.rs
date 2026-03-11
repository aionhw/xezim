//! Data type parsing (IEEE 1800-2017 §A.2.2)

use super::Parser;
use crate::ast::types::*;
use crate::lexer::token::TokenKind;

impl Parser {
    pub(super) fn is_data_type_keyword(&self) -> bool {
        matches!(self.current_kind(),
            TokenKind::KwBit | TokenKind::KwLogic | TokenKind::KwReg |
            TokenKind::KwByte | TokenKind::KwShortint | TokenKind::KwInt |
            TokenKind::KwLongint | TokenKind::KwInteger | TokenKind::KwTime |
            TokenKind::KwReal | TokenKind::KwShortreal | TokenKind::KwRealtime |
            TokenKind::KwString | TokenKind::KwChandle | TokenKind::KwEvent |
            TokenKind::KwVoid | TokenKind::KwStruct | TokenKind::KwUnion |
            TokenKind::KwEnum | TokenKind::KwSigned | TokenKind::KwUnsigned
        )
    }

    pub(super) fn is_type_start(&self) -> bool {
        self.is_data_type_keyword() || self.at(TokenKind::Identifier)
    }

    pub(super) fn parse_data_type(&mut self) -> DataType {
        let start = self.current().span.start;
        match self.current_kind() {
            TokenKind::KwBit | TokenKind::KwLogic | TokenKind::KwReg => {
                let kind = match self.bump().kind {
                    TokenKind::KwBit => IntegerVectorType::Bit,
                    TokenKind::KwLogic => IntegerVectorType::Logic,
                    _ => IntegerVectorType::Reg,
                };
                let signing = self.parse_optional_signing();
                let dimensions = self.parse_packed_dimensions();
                DataType::IntegerVector { kind, signing, dimensions, span: self.span_from(start) }
            }
            TokenKind::KwByte | TokenKind::KwShortint | TokenKind::KwInt |
            TokenKind::KwLongint | TokenKind::KwInteger | TokenKind::KwTime => {
                let kind = match self.bump().kind {
                    TokenKind::KwByte => IntegerAtomType::Byte,
                    TokenKind::KwShortint => IntegerAtomType::ShortInt,
                    TokenKind::KwInt => IntegerAtomType::Int,
                    TokenKind::KwLongint => IntegerAtomType::LongInt,
                    TokenKind::KwInteger => IntegerAtomType::Integer,
                    _ => IntegerAtomType::Time,
                };
                let signing = self.parse_optional_signing();
                DataType::IntegerAtom { kind, signing, span: self.span_from(start) }
            }
            TokenKind::KwReal => { self.bump(); DataType::Real { kind: RealType::Real, span: self.span_from(start) } }
            TokenKind::KwShortreal => { self.bump(); DataType::Real { kind: RealType::ShortReal, span: self.span_from(start) } }
            TokenKind::KwRealtime => { self.bump(); DataType::Real { kind: RealType::RealTime, span: self.span_from(start) } }
            TokenKind::KwString => { self.bump(); DataType::Simple { kind: SimpleType::String, span: self.span_from(start) } }
            TokenKind::KwChandle => { self.bump(); DataType::Simple { kind: SimpleType::Chandle, span: self.span_from(start) } }
            TokenKind::KwEvent => { self.bump(); DataType::Simple { kind: SimpleType::Event, span: self.span_from(start) } }
            TokenKind::KwVoid => { self.bump(); DataType::Void(self.span_from(start)) }
            TokenKind::KwEnum => self.parse_enum_type(),
            TokenKind::KwStruct | TokenKind::KwUnion => self.parse_struct_type(),
            TokenKind::KwSigned | TokenKind::KwUnsigned => {
                let signing = self.parse_optional_signing();
                let dimensions = self.parse_packed_dimensions();
                DataType::Implicit { signing, dimensions, span: self.span_from(start) }
            }
            TokenKind::Identifier => {
                let name = self.parse_type_name();
                let dimensions = self.parse_packed_dimensions();
                DataType::TypeReference { name, dimensions, span: self.span_from(start) }
            }
            _ => DataType::Implicit { signing: None, dimensions: Vec::new(), span: self.span_from(start) }
        }
    }

    pub(super) fn parse_type_name(&mut self) -> TypeName {
        let start = self.current().span.start;
        let first = self.parse_identifier();
        if self.at(TokenKind::DoubleColon) {
            self.bump();
            let second = self.parse_identifier();
            TypeName { scope: Some(first), name: second, span: self.span_from(start) }
        } else {
            TypeName { scope: None, name: first, span: self.span_from(start) }
        }
    }

    pub(super) fn parse_optional_signing(&mut self) -> Option<Signing> {
        match self.current_kind() {
            TokenKind::KwSigned => { self.bump(); Some(Signing::Signed) }
            TokenKind::KwUnsigned => { self.bump(); Some(Signing::Unsigned) }
            _ => None,
        }
    }

    pub(super) fn parse_optional_lifetime(&mut self) -> Option<Lifetime> {
        match self.current_kind() {
            TokenKind::KwStatic => { self.bump(); Some(Lifetime::Static) }
            TokenKind::KwAutomatic => { self.bump(); Some(Lifetime::Automatic) }
            _ => None,
        }
    }

    pub(super) fn parse_packed_dimensions(&mut self) -> Vec<PackedDimension> {
        let mut dims = Vec::new();
        while self.at(TokenKind::LBracket) {
            let start = self.current().span.start;
            self.bump();
            if self.at(TokenKind::RBracket) {
                self.bump();
                dims.push(PackedDimension::Unsized(self.span_from(start)));
            } else {
                let left = self.parse_expression();
                self.expect(TokenKind::Colon);
                let right = self.parse_expression();
                self.expect(TokenKind::RBracket);
                dims.push(PackedDimension::Range {
                    left: Box::new(left), right: Box::new(right),
                    span: self.span_from(start),
                });
            }
        }
        dims
    }

    pub(super) fn parse_unpacked_dimensions(&mut self) -> Vec<UnpackedDimension> {
        let mut dims = Vec::new();
        while self.at(TokenKind::LBracket) {
            let start = self.current().span.start;
            self.bump();
            if self.at(TokenKind::RBracket) {
                self.bump();
                dims.push(UnpackedDimension::Unsized(self.span_from(start)));
            } else if self.at(TokenKind::Dollar) {
                self.bump();
                self.expect(TokenKind::RBracket);
                dims.push(UnpackedDimension::Queue { max_size: None, span: self.span_from(start) });
            } else {
                let expr = self.parse_expression();
                if self.eat(TokenKind::Colon).is_some() {
                    let right = self.parse_expression();
                    self.expect(TokenKind::RBracket);
                    dims.push(UnpackedDimension::Range {
                        left: Box::new(expr), right: Box::new(right),
                        span: self.span_from(start),
                    });
                } else {
                    self.expect(TokenKind::RBracket);
                    dims.push(UnpackedDimension::Expression {
                        expr: Box::new(expr), span: self.span_from(start),
                    });
                }
            }
        }
        dims
    }

    fn parse_enum_type(&mut self) -> DataType {
        let start = self.current().span.start;
        self.expect(TokenKind::KwEnum);
        let base_type = if self.is_data_type_keyword() {
            Some(Box::new(self.parse_data_type()))
        } else { None };
        self.expect(TokenKind::LBrace);
        let mut members = Vec::new();
        loop {
            if self.at(TokenKind::RBrace) || self.at(TokenKind::Eof) { break; }
            let mstart = self.current().span.start;
            let name = self.parse_identifier();
            let init = if self.eat(TokenKind::Assign).is_some() {
                Some(self.parse_expression())
            } else { None };
            members.push(crate::ast::types::EnumMember {
                name, range: None, init, span: self.span_from(mstart),
            });
            if self.eat(TokenKind::Comma).is_none() { break; }
        }
        self.expect(TokenKind::RBrace);
        DataType::Enum(crate::ast::types::EnumType {
            base_type, members, span: self.span_from(start),
        })
    }

    fn parse_struct_type(&mut self) -> DataType {
        let start = self.current().span.start;
        let kind = if self.eat(TokenKind::KwUnion).is_some() {
            StructUnionKind::Union
        } else {
            self.expect(TokenKind::KwStruct);
            StructUnionKind::Struct
        };
        let packed = self.eat(TokenKind::KwPacked).is_some();
        let signing = self.parse_optional_signing();
        self.expect(TokenKind::LBrace);
        let mut members = Vec::new();
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            let mstart = self.current().span.start;
            let rand_qualifier = match self.current_kind() {
                TokenKind::KwRand => { self.bump(); Some(RandQualifier::Rand) }
                TokenKind::KwRandc => { self.bump(); Some(RandQualifier::Randc) }
                _ => None,
            };
            let data_type = self.parse_data_type();
            let mut declarators = Vec::new();
            loop {
                let dstart = self.current().span.start;
                let name = self.parse_identifier();
                let dimensions = self.parse_unpacked_dimensions();
                let init = if self.eat(TokenKind::Assign).is_some() {
                    Some(self.parse_expression())
                } else { None };
                declarators.push(StructDeclarator { name, dimensions, init, span: self.span_from(dstart) });
                if self.eat(TokenKind::Comma).is_none() { break; }
            }
            self.expect(TokenKind::Semicolon);
            members.push(StructMember { rand_qualifier, data_type, declarators, span: self.span_from(mstart) });
        }
        self.expect(TokenKind::RBrace);
        DataType::Struct(StructUnionType { kind, packed, signing, members, span: self.span_from(start) })
    }

    pub(super) fn parse_optional_direction(&mut self) -> Option<PortDirection> {
        match self.current_kind() {
            TokenKind::KwInput => { self.bump(); Some(PortDirection::Input) }
            TokenKind::KwOutput => { self.bump(); Some(PortDirection::Output) }
            TokenKind::KwInout => { self.bump(); Some(PortDirection::Inout) }
            TokenKind::KwRef => { self.bump(); Some(PortDirection::Ref) }
            _ => None,
        }
    }

    pub(super) fn parse_optional_net_type(&mut self) -> Option<NetType> {
        match self.current_kind() {
            TokenKind::KwWire => { self.bump(); Some(NetType::Wire) }
            TokenKind::KwTri => { self.bump(); Some(NetType::Tri) }
            TokenKind::KwWand => { self.bump(); Some(NetType::Wand) }
            TokenKind::KwWor => { self.bump(); Some(NetType::Wor) }
            TokenKind::KwTriand => { self.bump(); Some(NetType::TriAnd) }
            TokenKind::KwTrior => { self.bump(); Some(NetType::TriOr) }
            TokenKind::KwTri0 => { self.bump(); Some(NetType::Tri0) }
            TokenKind::KwTri1 => { self.bump(); Some(NetType::Tri1) }
            TokenKind::KwSupply0 => { self.bump(); Some(NetType::Supply0) }
            TokenKind::KwSupply1 => { self.bump(); Some(NetType::Supply1) }
            TokenKind::KwTrireg => { self.bump(); Some(NetType::TriReg) }
            TokenKind::KwUwire => { self.bump(); Some(NetType::Uwire) }
            _ => None,
        }
    }

    pub(super) fn is_port_direction(&self) -> bool {
        matches!(self.current_kind(),
            TokenKind::KwInput | TokenKind::KwOutput | TokenKind::KwInout | TokenKind::KwRef)
    }
}
