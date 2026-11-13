//! One startup scan for checkpoint calls, including unreachable syntax.
//! Like checkpoint's address walker, visit Serde's AST structure without an
//! encoding or source-text guesses. Only ExprKind::SystemCall's name counts;
//! strings and ordinary identifiers do not. This covers every serialized AST
//! alternative, including uncalled tasks/functions and class/package bodies.
use super::ElaboratedModule;
use serde::ser::{self, Serialize};

/// Linear in the unfolded serialized size (shared Rc bodies are revisited).
/// Deferred bodies are
/// skipped by artifact serialization, so visit their source syntax explicitly.
/// No expression is evaluated (§13.3/§13.4); dead branches still count.
pub(super) fn in_design(module: &ElaboratedModule) -> bool {
    let mut walk = Walk { call_name: false };
    let scan = (|| -> Result<(), Stop> {
        walk.visit(module)?;
        for pending in &module.pending_initial {
            walk.visit(&pending.source)?;
        }
        for pending in &module.pending_always {
            walk.visit(&pending.source)?;
        }
        for pending in &module.pending_cont_assign {
            walk.visit(&pending.lhs_source)?;
            walk.visit(&pending.rhs_source)?;
            walk.visit(&pending.delay_source)?;
            walk.visit(&pending.delay_fall_source)?;
            walk.visit(&pending.delay_off_source)?;
        }
        Ok(())
    })();
    // Any serialization error conservatively keeps the original cloning path.
    scan.is_err()
}

#[derive(Debug)]
struct Stop;
impl std::fmt::Display for Stop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("checkpoint call or unsupported serialization")
    }
}
impl std::error::Error for Stop {}
impl ser::Error for Stop {
    fn custom<T: std::fmt::Display>(_: T) -> Self {
        Stop
    }
}

struct Walk {
    call_name: bool,
}
impl Walk {
    fn visit<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), Stop> {
        v.serialize(&mut *self)
    }
}
struct Variant<'a> {
    walk: &'a mut Walk,
    system_call: bool,
}

macro_rules! leaf {
    ($($f:ident($t:ty)),*) => {$(
        fn $f(self, _: $t) -> Result<(), Stop> { Ok(()) }
    )*};
}
macro_rules! compound {
    ($tr:ident, $f:ident) => {
        impl ser::$tr for &mut Walk {
            type Ok = ();
            type Error = Stop;
            fn $f<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), Stop> {
                self.visit(v)?;
                Ok(())
            }
            fn end(self) -> Result<(), Stop> {
                Ok(())
            }
        }
    };
}
compound!(SerializeSeq, serialize_element);
compound!(SerializeTuple, serialize_element);
compound!(SerializeTupleStruct, serialize_field);
compound!(SerializeTupleVariant, serialize_field);
impl ser::SerializeMap for &mut Walk {
    type Ok = ();
    type Error = Stop;
    fn serialize_key<T: Serialize + ?Sized>(&mut self, k: &T) -> Result<(), Stop> {
        self.visit(k)?;
        Ok(())
    }
    fn serialize_value<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), Stop> {
        self.visit(v)?;
        Ok(())
    }
    fn end(self) -> Result<(), Stop> {
        Ok(())
    }
}
impl ser::SerializeStruct for &mut Walk {
    type Ok = ();
    type Error = Stop;
    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        _: &'static str,
        v: &T,
    ) -> Result<(), Stop> {
        self.visit(v)?;
        Ok(())
    }
    fn end(self) -> Result<(), Stop> {
        Ok(())
    }
}
impl ser::SerializeStructVariant for Variant<'_> {
    type Ok = ();
    type Error = Stop;
    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        v: &T,
    ) -> Result<(), Stop> {
        self.walk.call_name = self.system_call && key == "name";
        let result = self.walk.visit(v);
        self.walk.call_name = false;
        result
    }
    fn end(self) -> Result<(), Stop> {
        Ok(())
    }
}

impl<'a> ser::Serializer for &'a mut Walk {
    type Ok = ();
    type Error = Stop;
    type SerializeSeq = Self;
    type SerializeTuple = Self;
    type SerializeTupleStruct = Self;
    type SerializeTupleVariant = Self;
    type SerializeMap = Self;
    type SerializeStruct = Self;
    type SerializeStructVariant = Variant<'a>;
    leaf!(
        serialize_bool(bool),
        serialize_i8(i8),
        serialize_i16(i16),
        serialize_i32(i32),
        serialize_i64(i64),
        serialize_i128(i128),
        serialize_u8(u8),
        serialize_u16(u16),
        serialize_u32(u32),
        serialize_u64(u64),
        serialize_u128(u128),
        serialize_f32(f32),
        serialize_f64(f64),
        serialize_char(char),
        serialize_bytes(&[u8]),
        serialize_unit_struct(&'static str)
    );
    fn serialize_str(self, text: &str) -> Result<(), Stop> {
        if self.call_name && matches!(text, "$save" | "$incsave" | "$restart") {
            return Err(Stop);
        }
        Ok(())
    }
    fn serialize_none(self) -> Result<(), Stop> {
        Ok(())
    }
    fn serialize_unit(self) -> Result<(), Stop> {
        Ok(())
    }
    fn serialize_unit_variant(self, _: &'static str, _: u32, _: &'static str) -> Result<(), Stop> {
        Ok(())
    }
    fn serialize_some<T: Serialize + ?Sized>(self, v: &T) -> Result<(), Stop> {
        self.visit(v)?;
        Ok(())
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        v: &T,
    ) -> Result<(), Stop> {
        self.visit(v)?;
        Ok(())
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        v: &T,
    ) -> Result<(), Stop> {
        self.visit(v)?;
        Ok(())
    }
    fn serialize_seq(self, _: Option<usize>) -> Result<Self, Stop> {
        Ok(self)
    }
    fn serialize_tuple(self, _: usize) -> Result<Self, Stop> {
        Ok(self)
    }
    fn serialize_tuple_struct(self, _: &'static str, _: usize) -> Result<Self, Stop> {
        Ok(self)
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self, Stop> {
        Ok(self)
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Self, Stop> {
        Ok(self)
    }
    fn serialize_struct(self, _: &'static str, _: usize) -> Result<Self, Stop> {
        Ok(self)
    }
    fn serialize_struct_variant(
        self,
        name: &'static str,
        _: u32,
        variant: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStructVariant, Stop> {
        Ok(Variant {
            walk: self,
            system_call: name == "ExprKind" && variant == "SystemCall",
        })
    }
}
