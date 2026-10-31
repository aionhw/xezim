//! Conservative tracing of values and captured syntax for class reachability.
//!
//! This visits Serde's structural representation without building an encoding.
//! In particular it covers AST alternatives omitted by ad-hoc expression walkers.
//! It may retain an integer that happens to equal a handle. It never evaluates SV
//! expressions or treats a missing/unknown type as proof that a value is scalar.
//! The collector's root inventory and sweep are separate from this visitor.

use serde::{ser, Serialize, Serializer};

#[derive(Debug)]
pub(super) struct TraceError;
impl std::fmt::Display for TraceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("class reachability serialization failed")
    }
}
impl std::error::Error for TraceError {}
impl ser::Error for TraceError {
    fn custom<T: std::fmt::Display>(_: T) -> Self {
        Self
    }
}

/// Candidates only: callers must intersect these with allocated heap slots.
/// False positives retain objects; a serialization error must abort collection.
pub(super) struct HandleTrace {
    pub candidates: Vec<bool>,
    found: Vec<usize>,
    value_words: Vec<u64>,
    xz_words: Vec<u64>,
    capture_words: u8,
}
impl HandleTrace {
    pub fn new(heap_len: usize) -> Self {
        Self {
            candidates: vec![false; heap_len],
            found: Vec::new(),
            value_words: Vec::new(),
            xz_words: Vec::new(),
            capture_words: 0,
        }
    }
    pub fn candidate(&mut self, n: u64) {
        if n != 0
            && let Ok(n) = usize::try_from(n)
            && let Some(mark) = self.candidates.get_mut(n)
        {
            if !*mark {
                self.found.push(n);
            }
            *mark = true;
        }
    }
    pub fn drain_candidates(&mut self) -> Vec<usize> {
        let found = std::mem::take(&mut self.found);
        for &h in &found {
            self.candidates[h] = false;
        }
        found
    }
    pub fn trace<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), TraceError> {
        value.serialize(&mut *self)
    }
    fn text(&mut self, text: &str) {
        if let Some(rest) = text.strip_prefix("\u{2}inst")
            && let Some((owner, _)) = rest.split_once('#')
            && let Ok(handle) = owner.parse::<u64>()
        {
            self.candidate(handle);
        }

        // Captured handle literals are decimal strings. Other radices remain
        // conservative too; an AST integer carries its base in another field.
        let digits = text.replace('_', "");
        for radix in [2, 8, 10, 16] {
            if let Ok(n) = u64::from_str_radix(&digits, radix) {
                self.candidate(n);
            }
        }
        // Object-owned storage and class-handle associative keys can be the
        // only references in a continuation. Include numeric indices as well:
        // without the owner's key type, excluding them would be unsound.
        for piece in text.split(['.', '[', ']', ':']) {
            let number = piece
                .split_once('#')
                .map(|(owner, _)| owner)
                .unwrap_or(piece);
            if let Ok(n) = number.parse::<u64>() {
                self.candidate(n);
            }
        }
    }
    fn finish_value(&mut self) {
        // SV event queues encode the owning object in a packed string. Match
        // the ASCII prefix as bytes: event names can contain non-UTF-8 content.
        // to_sv_string clears X/Z bits, so decode that same value plane.
        let words: Vec<u64> = self
            .value_words
            .iter()
            .enumerate()
            .map(|(i, value)| value & !self.xz_words.get(i).copied().unwrap_or(0))
            .collect();
        if !words.iter().any(|w| w.to_be_bytes().contains(&2)) {
            return;
        }
        let bytes: Vec<u8> = words.iter().rev().flat_map(|w| w.to_be_bytes()).collect();
        let start = bytes.iter().position(|&b| b != 0).unwrap_or(bytes.len());
        let Some(rest) = bytes[start..].strip_prefix(b"\x02inst") else {
            return;
        };
        let mut handle = 0u64;
        let mut digits = 0;
        for &byte in rest {
            if byte == b'#' && digits > 0 {
                self.candidate(handle);
                return;
            }
            if !byte.is_ascii_digit() {
                return;
            }
            let Some(next) = handle
                .checked_mul(10)
                .and_then(|h| h.checked_add((byte - b'0') as u64))
            else {
                return;
            };
            handle = next;
            digits += 1;
        }
    }
}

pub(super) struct Fields<'a> {
    trace: &'a mut HandleTrace,
    skip: bool,
    kind: &'static str,
}
impl<'a> Serializer for &'a mut HandleTrace {
    type Ok = ();
    type Error = TraceError;
    type SerializeSeq = Fields<'a>;
    type SerializeTuple = Fields<'a>;
    type SerializeTupleStruct = Fields<'a>;
    type SerializeTupleVariant = Fields<'a>;
    type SerializeMap = Fields<'a>;
    type SerializeStruct = Fields<'a>;
    type SerializeStructVariant = Fields<'a>;
    fn serialize_i8(self, v: i8) -> Result<(), TraceError> {
        if let Ok(n) = u64::try_from(v) {
            self.candidate(n);
        }
        Ok(())
    }
    fn serialize_i16(self, v: i16) -> Result<(), TraceError> {
        if let Ok(n) = u64::try_from(v) {
            self.candidate(n);
        }
        Ok(())
    }
    fn serialize_i32(self, v: i32) -> Result<(), TraceError> {
        if let Ok(n) = u64::try_from(v) {
            self.candidate(n);
        }
        Ok(())
    }
    fn serialize_i64(self, v: i64) -> Result<(), TraceError> {
        if let Ok(n) = u64::try_from(v) {
            self.candidate(n);
        }
        Ok(())
    }
    fn serialize_i128(self, v: i128) -> Result<(), TraceError> {
        if let Ok(n) = u64::try_from(v) {
            self.candidate(n);
        }
        Ok(())
    }
    fn serialize_u8(self, v: u8) -> Result<(), TraceError> {
        self.candidate(v as u64);
        Ok(())
    }
    fn serialize_u16(self, v: u16) -> Result<(), TraceError> {
        self.candidate(v as u64);
        Ok(())
    }
    fn serialize_u32(self, v: u32) -> Result<(), TraceError> {
        self.candidate(v as u64);
        Ok(())
    }
    fn serialize_u64(self, v: u64) -> Result<(), TraceError> {
        match self.capture_words {
            1 => self.value_words.push(v),
            2 => self.xz_words.push(v),
            _ => {}
        }
        self.candidate(v as u64);
        Ok(())
    }
    fn serialize_u128(self, v: u128) -> Result<(), TraceError> {
        if let Ok(n) = u64::try_from(v) {
            self.candidate(n);
        }
        Ok(())
    }
    fn serialize_bool(self, _: bool) -> Result<(), TraceError> {
        Ok(())
    }
    fn serialize_f32(self, _: f32) -> Result<(), TraceError> {
        Ok(())
    }
    fn serialize_f64(self, _: f64) -> Result<(), TraceError> {
        Ok(())
    }
    fn serialize_char(self, _: char) -> Result<(), TraceError> {
        Ok(())
    }
    fn serialize_str(self, v: &str) -> Result<(), TraceError> {
        self.text(v);
        Ok(())
    }
    fn serialize_bytes(self, v: &[u8]) -> Result<(), TraceError> {
        // Serde AST/Value do not encode handles as bytes. Trace each aligned
        // word conservatively if a future serializable runtime state does.
        for bytes in v.windows(8) {
            let bytes: [u8; 8] = bytes.try_into().unwrap();
            self.candidate(u64::from_le_bytes(bytes));
            self.candidate(u64::from_be_bytes(bytes));
        }
        Ok(())
    }
    fn serialize_none(self) -> Result<(), TraceError> {
        Ok(())
    }
    fn serialize_some<T: Serialize + ?Sized>(self, v: &T) -> Result<(), TraceError> {
        v.serialize(self)
    }
    fn serialize_unit(self) -> Result<(), TraceError> {
        Ok(())
    }
    fn serialize_unit_struct(self, _: &'static str) -> Result<(), TraceError> {
        Ok(())
    }
    fn serialize_unit_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
    ) -> Result<(), TraceError> {
        Ok(())
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        v: &T,
    ) -> Result<(), TraceError> {
        v.serialize(self)
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        v: &T,
    ) -> Result<(), TraceError> {
        v.serialize(self)
    }
    fn serialize_seq(self, _: Option<usize>) -> Result<Self::SerializeSeq, TraceError> {
        Ok(Fields {
            trace: self,
            skip: false,
            kind: "",
        })
    }
    fn serialize_tuple(self, _: usize) -> Result<Self::SerializeTuple, TraceError> {
        Ok(Fields {
            trace: self,
            skip: false,
            kind: "",
        })
    }
    fn serialize_tuple_struct(
        self,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleStruct, TraceError> {
        Ok(Fields {
            trace: self,
            skip: false,
            kind: "",
        })
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleVariant, TraceError> {
        Ok(Fields {
            trace: self,
            skip: false,
            kind: "",
        })
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Self::SerializeMap, TraceError> {
        Ok(Fields {
            trace: self,
            skip: false,
            kind: "",
        })
    }
    fn serialize_struct(
        self,
        name: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStruct, TraceError> {
        if name == "Value" {
            self.value_words.clear();
            self.xz_words.clear();
        }
        Ok(Fields {
            trace: self,
            skip: name == "Span",
            kind: name,
        })
    }
    fn serialize_struct_variant(
        self,
        name: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStructVariant, TraceError> {
        Ok(Fields {
            trace: self,
            skip: false,
            kind: name,
        })
    }
}
impl ser::SerializeSeq for Fields<'_> {
    type Ok = ();
    type Error = TraceError;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), TraceError> {
        self.trace.trace(value)
    }
    fn end(self) -> Result<(), TraceError> {
        Ok(())
    }
}
impl ser::SerializeTuple for Fields<'_> {
    type Ok = ();
    type Error = TraceError;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), TraceError> {
        self.trace.trace(value)
    }
    fn end(self) -> Result<(), TraceError> {
        Ok(())
    }
}
impl ser::SerializeTupleStruct for Fields<'_> {
    type Ok = ();
    type Error = TraceError;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), TraceError> {
        self.trace.trace(value)
    }
    fn end(self) -> Result<(), TraceError> {
        Ok(())
    }
}
impl ser::SerializeTupleVariant for Fields<'_> {
    type Ok = ();
    type Error = TraceError;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), TraceError> {
        self.trace.trace(value)
    }
    fn end(self) -> Result<(), TraceError> {
        Ok(())
    }
}
impl ser::SerializeMap for Fields<'_> {
    type Ok = ();
    type Error = TraceError;
    fn serialize_key<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), TraceError> {
        self.trace.trace(value)
    }
    fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), TraceError> {
        self.trace.trace(value)
    }
    fn end(self) -> Result<(), TraceError> {
        Ok(())
    }
}
impl ser::SerializeStruct for Fields<'_> {
    type Ok = ();
    type Error = TraceError;
    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), TraceError> {
        if !self.skip {
            let old = self.trace.capture_words;
            if (self.kind == "ValueStorage" && key == "val_bits")
                || (self.kind == "WidePlanes" && key == "val")
            {
                self.trace.capture_words = 1;
            }
            if (self.kind == "ValueStorage" && key == "xz_bits")
                || (self.kind == "WidePlanes" && key == "xz")
            {
                self.trace.capture_words = 2;
            }
            let result = self.trace.trace(value);
            self.trace.capture_words = old;
            result?;
        }
        Ok(())
    }
    fn end(self) -> Result<(), TraceError> {
        if self.kind == "Value" {
            self.trace.finish_value();
        }
        Ok(())
    }
}
impl ser::SerializeStructVariant for Fields<'_> {
    type Ok = ();
    type Error = TraceError;
    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), TraceError> {
        if !self.skip {
            let old = self.trace.capture_words;
            if (self.kind == "ValueStorage" && key == "val_bits")
                || (self.kind == "WidePlanes" && key == "val")
            {
                self.trace.capture_words = 1;
            }
            if (self.kind == "ValueStorage" && key == "xz_bits")
                || (self.kind == "WidePlanes" && key == "xz")
            {
                self.trace.capture_words = 2;
            }
            let result = self.trace.trace(value);
            self.trace.capture_words = old;
            result?;
        }
        Ok(())
    }
    fn end(self) -> Result<(), TraceError> {
        if self.kind == "Value" {
            self.trace.finish_value();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{
        decl::ConstraintItem,
        expr::{AssignmentPatternItem, ExprKind, Expression, NumberBase, NumberLiteral},
        stmt::Pattern,
        Span,
    };
    use std::cell::Cell;
    fn lit(n: u64) -> Expression {
        Expression::new(
            ExprKind::Number(NumberLiteral::Integer {
                size: Some(64),
                signed: false,
                base: NumberBase::Decimal,
                value: n.to_string(),
                cached_val: Cell::new(None),
            }),
            Span::new(600, 601),
        )
    }
    #[test]
    fn traces_captures_in_nested_ast_alternatives_without_evaluation() {
        let expr = Expression::new(
            ExprKind::RandomizeWith {
                call: Box::new(lit(101)),
                constraints: vec![ConstraintItem::Expr(Expression::new(
                    ExprKind::Matches {
                        expr: Box::new(Expression::new(
                            ExprKind::WithClause {
                                expr: Box::new(lit(102)),
                                filter: Box::new(lit(103)),
                            },
                            Span::dummy(),
                        )),
                        pattern: Box::new(Pattern::Expr(Expression::new(
                            ExprKind::AssignmentPattern(vec![AssignmentPatternItem::Keyed(
                                lit(104),
                                Expression::new(
                                    ExprKind::SvaClocked {
                                        events: vec![],
                                        body: Box::new(lit(105)),
                                    },
                                    Span::dummy(),
                                ),
                            )]),
                            Span::dummy(),
                        ))),
                    },
                    Span::dummy(),
                ))],
            },
            Span::dummy(),
        );
        let mut trace = HandleTrace::new(1000);
        trace.trace(&expr).unwrap();
        for h in 101..=105 {
            assert!(trace.candidates[h], "missing captured handle {h}");
        }
        assert!(
            !trace.candidates[600] && !trace.candidates[601],
            "source spans are not references"
        );
    }
    #[test]
    fn traces_owned_names_associative_keys_and_wide_values() {
        let mut trace = HandleTrace::new(1000);
        trace.trace(&["123#members[456]", "top.map[789]"]).unwrap();
        trace
            .trace(&crate::compiler::simulator::Value::from_u64(321, 128))
            .unwrap();
        trace
            .trace(&crate::compiler::simulator::Value::from_string(
                "\u{2}inst654#event",
            ))
            .unwrap();
        trace.trace("\u{2}inst655#event").unwrap();
        trace
            .trace(&crate::compiler::simulator::Value::from_string(
                "\u{2}inst656#é",
            ))
            .unwrap();
        for h in [123, 456, 789, 321, 654, 655, 656] {
            assert!(trace.candidates[h]);
        }
    }
}

// A suspended loop can rewind before its current cursor. Visit its whole
// immutable frame, and walk continuation links iteratively to avoid stack growth.
impl Serialize for super::ProcCont {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use ser::SerializeSeq;
        let mut seq = serializer.serialize_seq(None)?;
        let mut frame = Some(self);
        while let Some(cont) = frame {
            seq.serialize_element(&cont.stmts)?;
            frame = cont.next.as_deref();
        }
        seq.end()
    }
}
