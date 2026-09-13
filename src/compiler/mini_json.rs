//! Minimal dependency-free JSON reader for the trace sidecars.
//!
//! The census and graph sidecars are small, deterministic JSON written by
//! `act_trace`; the cross-language loaders read them back. Full JSON
//! support (comments, unicode escapes beyond \uXXXX, arbitrary nesting) is
//! not needed, so this module parses exactly the subset the sidecars emit:
//! objects, arrays, strings (with `\"`-style escapes), numbers, `true`,
//! `false`, and `null`. It is not a general-purpose JSON parser and does
//! not try to be one.
//!
//! Errors carry a byte offset so a bad sidecar (hand-edited, truncated,
//! written by a different tool) points at the offending character.

/// A JSON value from a trace sidecar.
#[derive(Clone, Debug, PartialEq)]
pub enum JsonValue {
    Null,
    Bool(bool),
    /// Sidecar numbers are either integral signals or times; `f64` covers
    /// both (exact for every value the writers emit).
    Num(f64),
    Str(String),
    Arr(Vec<JsonValue>),
    /// Object members keep declaration order, which the sidecars sort by
    /// path before writing.
    Obj(Vec<(String, JsonValue)>),
}

/// Parser error: `msg` with the byte offset of the offending character.
#[derive(Clone, Debug, PartialEq)]
pub struct JsonError {
    pub msg: String,
    pub offset: usize,
}

/// Parse a complete JSON document (leading/trailing whitespace allowed).
pub fn parse(src: &str) -> Result<JsonValue, JsonError> {
    let mut p = Parser { src, pos: 0 };
    p.skip_ws();
    let v = p.value()?;
    p.skip_ws();
    if p.pos != src.len() {
        return Err(p.err("trailing characters after JSON value"));
    }
    Ok(v)
}

struct Parser<'a> {
    src: &'a str,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<u8> {
        self.src.as_bytes().get(self.pos).copied()
    }

    fn err(&self, msg: impl Into<String>) -> JsonError {
        JsonError {
            msg: msg.into(),
            offset: self.pos,
        }
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if matches!(c, b' ' | b'\t' | b'\r' | b'\n') {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn expect(&mut self, b: u8) -> Result<(), JsonError> {
        if self.peek() == Some(b) {
            self.pos += 1;
            Ok(())
        } else {
            Err(self.err(format!("expected '{}'", b as char)))
        }
    }

    fn value(&mut self) -> Result<JsonValue, JsonError> {
        self.skip_ws();
        match self.peek() {
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => Ok(JsonValue::Str(self._string()?)),
            Some(b't') => self.literal("true", JsonValue::Bool(true)),
            Some(b'f') => self.literal("false", JsonValue::Bool(false)),
            Some(b'n') => self.literal("null", JsonValue::Null),
            Some(b) if b == b'-' || b.is_ascii_digit() => self.number(),
            _ => Err(self.err("unexpected character")),
        }
    }

    fn object(&mut self) -> Result<JsonValue, JsonError> {
        self.expect(b'{')?;
        self.skip_ws();
        let mut members = Vec::new();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(JsonValue::Obj(members));
        }
        loop {
            self.skip_ws();
            if self.peek() != Some(b'"') {
                return Err(self.err("expected string key"));
            }
            let key = self._string()?;
            self.skip_ws();
            self.expect(b':')?;
            let val = self.value()?;
            members.push((key, val));
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                }
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(JsonValue::Obj(members));
                }
                _ => return Err(self.err("expected ',' or '}' in object")),
            }
        }
    }

    fn array(&mut self) -> Result<JsonValue, JsonError> {
        self.expect(b'[')?;
        self.skip_ws();
        let mut items = Vec::new();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(JsonValue::Arr(items));
        }
        loop {
            items.push(self.value()?);
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                }
                Some(b']') => {
                    self.pos += 1;
                    return Ok(JsonValue::Arr(items));
                }
                _ => return Err(self.err("expected ',' or ']' in array")),
            }
        }
    }

    fn literal(&mut self, word: &str, v: JsonValue) -> Result<JsonValue, JsonError> {
        if self.src[self.pos..].starts_with(word) {
            self.pos += word.len();
            Ok(v)
        } else {
            Err(self.err(format!("bad literal, expected '{word}'")))
        }
    }

    fn number(&mut self) -> Result<JsonValue, JsonError> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        if !self.peek().is_some_and(|c| c.is_ascii_digit()) {
            return Err(self.err("malformed number"));
        }
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.pos += 1;
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.pos += 1;
            }
        }
        if matches!(self.peek(), Some(b'e') | Some(b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+') | Some(b'-')) {
                self.pos += 1;
            }
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.pos += 1;
            }
        }
        let raw = &self.src[start..self.pos];
        raw.parse::<f64>()
            .map(JsonValue::Num)
            .map_err(|_| self.err("malformed number"))
    }

    /// Parse a JSON string; the opening quote is consumed, the closing quote
    /// is required. Handles the standard escapes.
    fn _string(&mut self) -> Result<String, JsonError> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            let c = match self.peek() {
                None => return Err(self.err("unterminated string")),
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.pos += 1;
                    match self.peek() {
                        Some(b'"') => '"',
                        Some(b'\\') => '\\',
                        Some(b'/') => '/',
                        Some(b'b') => '\u{0008}',
                        Some(b'f') => '\u{000C}',
                        Some(b'n') => '\n',
                        Some(b'r') => '\r',
                        Some(b't') => '\t',
                        Some(b'u') => {
                            self.pos += 1;
                            if self.pos + 4 <= self.src.len() {
                                let hex = &self.src[self.pos..self.pos + 4];
                                match u32::from_str_radix(hex, 16) {
                                    Ok(code) => {
                                        self.pos += 4;
                                        out.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
                                        continue;
                                    }
                                    Err(_) => return Err(self.err("bad \\u escape")),
                                }
                            }
                            return Err(self.err("truncated \\u escape"));
                        }
                        _ => return Err(self.err("unknown escape")),
                    }
                }
                Some(b) => {
                    if b < 0x20 {
                        return Err(self.err("control character in string"));
                    }
                    char::from(b)
                }
            };
            out.push(c);
            self.pos += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_objects_arrays_and_numbers() {
        let v = parse(
            r#"{"version": 1, "signals": [{"path": "a", "n": -1.5e2}], "ok": true, "z": null}"#,
        )
        .unwrap();
        match v {
            JsonValue::Obj(members) => {
                assert_eq!(members.len(), 4);
                match &members[1].1 {
                    JsonValue::Arr(items) => match &items[0] {
                        JsonValue::Obj(m) => {
                            assert_eq!(m[0], ("path".into(), JsonValue::Str("a".into())));
                            assert_eq!(m[1].1, JsonValue::Num(-150.0));
                        }
                        other => panic!("unexpected {other:?}"),
                    },
                    other => panic!("unexpected {other:?}"),
                }
                assert_eq!(members[2].1, JsonValue::Bool(true));
                assert_eq!(members[3].1, JsonValue::Null);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn string_escapes() {
        assert_eq!(
            parse(r#""a\"b\\c\/d\n""#).unwrap(),
            JsonValue::Str("a\"b\\c/d\n".into())
        );
        // \uXXXX decodes to the raw character; the hex form of 'A' is 41.
        assert_eq!(parse(r#""A""#).unwrap(), JsonValue::Str("A".into()));
        // A broken escape is an error, not silent garbage.
        assert_eq!(parse(r#""\q""#).unwrap_err().msg, "unknown escape");
    }

    #[test]
    fn rejects_garbage() {
        let e = parse("{].nope").unwrap_err();
        assert!(e.msg.contains("expected string key"), "{e:?}");
        let e = parse(r#"{"a": 1"#).unwrap_err();
        assert!(e.msg.contains("',' or '}'"), "{e:?}");
        let e = parse("tru").unwrap_err();
        assert!(e.msg.contains("bad literal"), "{e:?}");
        let e = parse("1 2").unwrap_err();
        assert!(e.msg.contains("trailing"), "{e:?}");
    }

    #[test]
    fn empty_container_forms() {
        assert_eq!(parse("{}").unwrap(), JsonValue::Obj(vec![]));
        assert_eq!(parse("[]").unwrap(), JsonValue::Arr(vec![]));
        assert_eq!(
            parse("  [ 1 , 2 ]  ").unwrap(),
            JsonValue::Arr(vec![JsonValue::Num(1.0), JsonValue::Num(2.0)])
        );
    }
}
