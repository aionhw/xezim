//! Value types for SystemVerilog simulation.
//! Supports 4-state logic (0, 1, X, Z) with arbitrary-width bit vectors.
//!
//! Optimized representation: values ≤64 bits use inline u64 storage,
//! avoiding heap allocation entirely. Wider values fall back to Vec<LogicBit>.

use std::fmt;
use serde::{Serialize, Deserialize};

/// A single 4-state logic bit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LogicBit {
    Zero,
    One,
    X,
    Z,
}

impl LogicBit {
    pub fn from_char(c: char) -> Self {
        match c {
            '0' => Self::Zero,
            '1' => Self::One,
            'x' | 'X' => Self::X,
            'z' | 'Z' | '?' => Self::Z,
            _ => Self::X,
        }
    }

    pub fn to_bool(self) -> bool {
        matches!(self, Self::One)
    }

    pub fn is_known(self) -> bool {
        matches!(self, Self::Zero | Self::One)
    }
}

impl fmt::Display for LogicBit {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Zero => write!(f, "0"),
            Self::One => write!(f, "1"),
            Self::X => write!(f, "x"),
            Self::Z => write!(f, "z"),
        }
    }
}

/// Storage for value bits. Values ≤64 bits use inline u64 pair.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
enum ValueStorage {
    /// Packed: val_bits holds 0/1, xz_bits marks X/Z.
    /// bit i: val=bit i of val_bits, xz=bit i of xz_bits
    /// 0: val=0,xz=0  1: val=1,xz=0  X: val=0,xz=1  Z: val=1,xz=1
    Inline { val_bits: u64, xz_bits: u64 },
    /// Fallback for width > 64.
    Wide(Vec<LogicBit>),
}

/// An arbitrary-width 4-state logic value.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Value {
    storage: ValueStorage,
    pub width: u32,
    pub is_signed: bool,
    /// When true, the inline val_bits hold f64 bits (IEEE 754).
    pub is_real: bool,
}

impl Value {
    /// Bit mask for the valid bits of an inline value.
    #[inline(always)]
    fn mask(width: u32) -> u64 {
        if width >= 64 { u64::MAX } else { (1u64 << width) - 1 }
    }

    pub fn new(width: u32) -> Self {
        if width <= 64 {
            // All X: xz_bits = all 1s for width bits, val_bits = 0
            Self {
                storage: ValueStorage::Inline { val_bits: 0, xz_bits: Self::mask(width) },
                width,
                is_signed: false, is_real: false,
            }
        } else {
            Self {
                storage: ValueStorage::Wide(vec![LogicBit::X; width as usize]),
                width,
                is_signed: false, is_real: false,
            }
        }
    }

    pub fn zero(width: u32) -> Self {
        if width <= 64 {
            Self { storage: ValueStorage::Inline { val_bits: 0, xz_bits: 0 }, width, is_signed: false, is_real: false }
        } else {
            Self { storage: ValueStorage::Wide(vec![LogicBit::Zero; width as usize]), width, is_signed: false, is_real: false }
        }
    }

    pub fn from_u64(val: u64, width: u32) -> Self {
        if width <= 64 {
            let mask = Self::mask(width);
            Self { storage: ValueStorage::Inline { val_bits: val & mask, xz_bits: 0 }, width, is_signed: false, is_real: false }
        } else {
            let mut bits = vec![LogicBit::Zero; width as usize];
            for i in 0..64.min(width as usize) {
                if (val >> i) & 1 == 1 { bits[i] = LogicBit::One; }
            }
            Self { storage: ValueStorage::Wide(bits), width, is_signed: false, is_real: false }
        }
    }

    /// Create a Value from pre-computed inline bits (for cached number literals).
    #[inline]
    pub fn from_inline(val_bits: u64, xz_bits: u64, width: u32) -> Self {
        Self { storage: ValueStorage::Inline { val_bits, xz_bits }, width, is_signed: false, is_real: false }
    }

    /// Create a Value holding an f64 (stored as its IEEE 754 bit pattern in a 64-bit inline).
    pub fn from_f64(f: f64) -> Self {
        Self { storage: ValueStorage::Inline { val_bits: f.to_bits(), xz_bits: 0 }, width: 64, is_signed: false, is_real: true }
    }

    pub fn from_string(s: &str) -> Self {
        let bytes = s.as_bytes();
        let width = (bytes.len() * 8) as u32;
        if width <= 64 {
            let mut val_bits = 0u64;
            for (i, &b) in bytes.iter().rev().enumerate() {
                val_bits |= (b as u64) << (i * 8);
            }
            Self { storage: ValueStorage::Inline { val_bits, xz_bits: 0 }, width, is_signed: false, is_real: false }
        } else {
            let mut bits = Vec::with_capacity(width as usize);
            for &b in bytes.iter().rev() {
                for i in 0..8 {
                    bits.push(if (b >> i) & 1 == 1 { LogicBit::One } else { LogicBit::Zero });
                }
            }
            Self { storage: ValueStorage::Wide(bits), width, is_signed: false, is_real: false }
        }
    }

    /// Extract f64 from a real-typed value.
    pub fn to_f64(&self) -> f64 {
        if self.is_real {
            match &self.storage {
                ValueStorage::Inline { val_bits, .. } => f64::from_bits(*val_bits),
                _ => 0.0,
            }
        } else {
            if self.is_signed {
                self.to_i64().unwrap_or(0) as f64
            } else {
                self.to_u64().unwrap_or(0) as f64
            }
        }
    }

    /// Extract inline bits for caching. Returns None for Wide values.
    #[inline]
    pub fn inline_bits(&self) -> Option<(u64, u64)> {
        match &self.storage {
            ValueStorage::Inline { val_bits, xz_bits } => Some((*val_bits, *xz_bits)),
            _ => None,
        }
    }

    pub fn raw_bits(&self) -> (u64, u64) {
        match &self.storage {
            ValueStorage::Inline { val_bits, xz_bits } => (*val_bits, *xz_bits),
            ValueStorage::Wide(bits) => {
                let mut v = 0u64;
                let mut x = 0u64;
                for (i, &b) in bits.iter().take(64).enumerate() {
                    match b {
                        LogicBit::One => v |= 1u64 << i,
                        LogicBit::X => x |= 1u64 << i,
                        LogicBit::Z => { v |= 1u64 << i; x |= 1u64 << i; },
                        LogicBit::Zero => {},
                    }
                }
                (v, x)
            }
        }
    }

    /// Access the bits field (compatibility layer for existing code).
    /// Returns a temporary Vec for wide values, or constructs from inline.
    pub fn get_bits(&self) -> BitsRef<'_> {
        BitsRef { value: self }
    }

    #[inline(always)]
    fn inline_vals(&self) -> Option<(u64, u64)> {
        match &self.storage {
            ValueStorage::Inline { val_bits, xz_bits } => Some((*val_bits, *xz_bits)),
            _ => None,
        }
    }

    #[inline(always)]
    pub fn has_xz(&self) -> bool {
        match &self.storage {
            ValueStorage::Inline { xz_bits, .. } => *xz_bits != 0,
            ValueStorage::Wide(bits) => bits.iter().any(|b| matches!(b, LogicBit::X | LogicBit::Z)),
        }
    }

    /// Get bit at position i.
    #[inline]
    pub fn get_bit(&self, i: usize) -> LogicBit {
        if i as u32 >= self.width { return LogicBit::Zero; }
        match &self.storage {
            ValueStorage::Inline { val_bits, xz_bits } => {
                let v = (*val_bits >> i) & 1;
                let x = (*xz_bits >> i) & 1;
                match (v, x) {
                    (0, 0) => LogicBit::Zero,
                    (1, 0) => LogicBit::One,
                    (0, 1) => LogicBit::X,
                    (_, _) => LogicBit::Z,
                }
            }
            ValueStorage::Wide(bits) => bits.get(i).copied().unwrap_or(LogicBit::Zero),
        }
    }

    /// Set bit at position i.
    #[inline]
    pub fn set_bit(&mut self, i: usize, bit: LogicBit) {
        if i as u32 >= self.width { return; }
        match &mut self.storage {
            ValueStorage::Inline { val_bits, xz_bits } => {
                let mask = 1u64 << i;
                match bit {
                    LogicBit::Zero => { *val_bits &= !mask; *xz_bits &= !mask; }
                    LogicBit::One  => { *val_bits |= mask;  *xz_bits &= !mask; }
                    LogicBit::X    => { *val_bits &= !mask; *xz_bits |= mask; }
                    LogicBit::Z    => { *val_bits |= mask;  *xz_bits |= mask; }
                }
            }
            ValueStorage::Wide(bits) => {
                if let Some(b) = bits.get_mut(i) { *b = bit; }
            }
        }
    }

    /// Convert to u64, treating X/Z as 0.
    pub fn to_u64(&self) -> Option<u64> {
        match &self.storage {
            ValueStorage::Inline { val_bits, xz_bits } => Some(*val_bits & !*xz_bits),
            ValueStorage::Wide(bits) => {
                let mut result: u64 = 0;
                for (i, bit) in bits.iter().enumerate() {
                    if i >= 64 { break; }
                    if *bit == LogicBit::One { result |= 1u64 << i; }
                }
                Some(result)
            }
        }
    }

    /// Convert to i64 (sign-extended if is_signed).
    pub fn to_i64(&self) -> Option<i64> {
        let raw = self.to_u64()?;
        if self.is_signed && self.width > 0 && self.width < 64 {
            let sign_bit = 1u64 << (self.width - 1);
            if raw & sign_bit != 0 {
                Some((raw | !Self::mask(self.width)) as i64)
            } else {
                Some(raw as i64)
            }
        } else {
            Some(raw as i64)
        }
    }

    /// Resize to target width. If narrowing, truncate. If widening, zero/sign-extend.
    pub fn resize(&self, target: u32) -> Value {
        if target == 0 { return Value::zero(0); }
        if self.is_real {
            if target == 64 { return self.clone(); }
            // convert the real value to an integer (rounding to nearest)
            let f = self.to_f64();
            return Value::from_u64(f.round() as u64, target);
        }
        if target == self.width {
            return self.clone();
        }
        match &self.storage {
            ValueStorage::Inline { val_bits, xz_bits } if target <= 64 => {
                let mask = Self::mask(target);
                if target < self.width {
                    // Truncate
                    Value {
                        storage: ValueStorage::Inline { val_bits: *val_bits & mask, xz_bits: *xz_bits & mask },
                        width: target, is_signed: self.is_signed, is_real: false,
                    }
                } else {
                    // Widen
                    if self.is_signed && self.width > 0 {
                        let sign_bit = if self.width <= 64 { (*xz_bits >> (self.width - 1)) & 1 == 0 && (*val_bits >> (self.width - 1)) & 1 == 1 } else { false };
                        if sign_bit {
                            let ext_mask = mask & !Self::mask(self.width);
                            Value {
                                storage: ValueStorage::Inline { val_bits: *val_bits | ext_mask, xz_bits: *xz_bits },
                                width: target, is_signed: self.is_signed, is_real: false,
                            }
                        } else {
                            Value {
                                storage: ValueStorage::Inline { val_bits: *val_bits, xz_bits: *xz_bits },
                                width: target, is_signed: self.is_signed, is_real: false,
                            }
                        }
                    } else {
                        Value {
                            storage: ValueStorage::Inline { val_bits: *val_bits, xz_bits: *xz_bits },
                            width: target, is_signed: self.is_signed, is_real: false,
                        }
                    }
                }
            }
            _ => {
                // Fall back to bit-by-bit
                let mut result = if self.is_signed {
                    let sign = self.get_bit(self.width.saturating_sub(1) as usize);
                    let fill = if sign == LogicBit::One { LogicBit::One } else { LogicBit::Zero };
                    Value { storage: if target <= 64 {
                        let fill_val = if fill == LogicBit::One { Self::mask(target) } else { 0 };
                        ValueStorage::Inline { val_bits: fill_val, xz_bits: 0 }
                    } else {
                        ValueStorage::Wide(vec![fill; target as usize])
                    }, width: target, is_signed: self.is_signed , is_real: false }
                } else {
                    Value::zero(target)
                };
                result.is_signed = self.is_signed;
                let copy_bits = self.width.min(target) as usize;
                for i in 0..copy_bits {
                    result.set_bit(i, self.get_bit(i));
                }
                result
            }
        }
    }

    // === Arithmetic ===

    pub fn negate(&self) -> Value {
        if self.is_real {
            return Value::from_f64(-self.to_f64());
        }
        if self.has_xz() {
            return Value::new(self.width);
        }
        let w = self.width;
        let v = self.to_u64().unwrap_or(0);
        let mut r = Value::from_u64(v.wrapping_neg(), w);
        r.is_signed = true;
        r
    }

    /// IEEE 1800-2017 §10.7 assignment-padding resize. When widening, if the MSB
    /// of the source is X or Z the extension bits are X or Z respectively;
    /// otherwise behaves like `resize` (zero- or sign-extension). Used when padding
    /// a nonblocking/blocking assignment RHS to the LHS width.
    pub fn resize_for_assign(&self, target: u32) -> Value {
        if target == self.width || self.width == 0 || self.is_real {
            return self.resize(target);
        }
        if target < self.width {
            return self.resize(target);
        }
        let msb = self.get_bit(self.width.saturating_sub(1) as usize);
        if msb != LogicBit::X && msb != LogicBit::Z {
            return self.resize(target);
        }
        // X/Z extend
        match &self.storage {
            ValueStorage::Inline { val_bits, xz_bits } if target <= 64 => {
                let mask = Self::mask(target);
                let ext_mask = mask & !Self::mask(self.width);
                let (new_val, new_xz) = if msb == LogicBit::Z {
                    (*val_bits | ext_mask, *xz_bits | ext_mask)
                } else {
                    (*val_bits, *xz_bits | ext_mask)
                };
                Value {
                    storage: ValueStorage::Inline { val_bits: new_val, xz_bits: new_xz },
                    width: target, is_signed: self.is_signed, is_real: false,
                }
            }
            _ => {
                let mut result = self.resize(target);
                for i in self.width as usize..target as usize {
                    result.set_bit(i, msb);
                }
                result
            }
        }
    }

    pub fn add(&self, other: &Value) -> Value {
        if self.is_real || other.is_real {
            return Value::from_f64(self.to_f64() + other.to_f64());
        }
        if self.has_xz() || other.has_xz() {
            return Value::new(self.width.max(other.width));
        }
        let w = self.width.max(other.width);
        let result_signed = self.is_signed && other.is_signed;
        let a = self.to_u64().unwrap_or(0);
        let b = other.to_u64().unwrap_or(0);
        let mut v = Value::from_u64(a.wrapping_add(b), w);
        v.is_signed = result_signed;
        v
    }

    pub fn sub(&self, other: &Value) -> Value {
        if self.is_real || other.is_real {
            return Value::from_f64(self.to_f64() - other.to_f64());
        }
        if self.has_xz() || other.has_xz() {
            return Value::new(self.width.max(other.width));
        }
        let w = self.width.max(other.width);
        let result_signed = self.is_signed && other.is_signed;
        let a = self.to_u64().unwrap_or(0);
        let b = other.to_u64().unwrap_or(0);
        let mut v = Value::from_u64(a.wrapping_sub(b), w);
        v.is_signed = result_signed;
        v
    }

    pub fn mul(&self, other: &Value) -> Value {
        if self.is_real || other.is_real {
            return Value::from_f64(self.to_f64() * other.to_f64());
        }
        if self.has_xz() || other.has_xz() { return Value::new(self.width.max(other.width)); }
        let w = self.width.max(other.width);
        let result_signed = self.is_signed && other.is_signed;
        let a = self.to_u64().unwrap_or(0);
        let b = other.to_u64().unwrap_or(0);
        let mut v = Value::from_u64(a.wrapping_mul(b), w);
        v.is_signed = result_signed;
        v
    }

    pub fn div(&self, other: &Value) -> Value {
        if self.is_real || other.is_real {
            return Value::from_f64(self.to_f64() / other.to_f64());
        }
        if self.has_xz() || other.has_xz() { return Value::new(self.width.max(other.width)); }
        let w = self.width.max(other.width);
        let a = self.to_u64().unwrap_or(0);
        let b = other.to_u64().unwrap_or(0);
        if b == 0 { return Value::new(w); } // X for divide by zero
        if self.is_signed || other.is_signed {
            let sa = self.to_i64().unwrap_or(0);
            let sb = other.to_i64().unwrap_or(0);
            if sb == 0 { return Value::new(w); }
            Value::from_u64(sa.wrapping_div(sb) as u64, w)
        } else {
            Value::from_u64(a / b, w)
        }
    }

    pub fn modulo(&self, other: &Value) -> Value {
        if self.is_real || other.is_real {
            return Value::from_f64(self.to_f64() % other.to_f64());
        }
        if self.has_xz() || other.has_xz() { return Value::new(self.width.max(other.width)); }
        let w = self.width.max(other.width);
        let b = other.to_u64().unwrap_or(0);
        if b == 0 { return Value::new(w); }
        if self.is_signed || other.is_signed {
            let sa = self.to_i64().unwrap_or(0);
            let sb = other.to_i64().unwrap_or(0);
            if sb == 0 { return Value::new(w); }
            Value::from_u64(sa.wrapping_rem(sb) as u64, w)
        } else {
            let a = self.to_u64().unwrap_or(0);
            Value::from_u64(a % b, w)
        }
    }

    pub fn power(&self, other: &Value) -> Value {
        if self.is_real || other.is_real {
            return Value::from_f64(self.to_f64().powf(other.to_f64()));
        }
        if self.has_xz() || other.has_xz() { return Value::new(self.width); }
        let base = self.to_u64().unwrap_or(0);
        let exp = other.to_u64().unwrap_or(0);
        let mut result: u64 = 1;
        for _ in 0..exp.min(64) { result = result.wrapping_mul(base); }
        Value::from_u64(result, self.width)
    }

    // === Bitwise ===

    pub fn bitwise_and(&self, other: &Value) -> Value {
        let w = self.width.max(other.width);
        match (&self.storage, &other.storage) {
            (ValueStorage::Inline { val_bits: av, xz_bits: ax }, ValueStorage::Inline { val_bits: bv, xz_bits: bx }) => {
                if *ax == 0 && *bx == 0 {
                    // Fast path: no X/Z
                    Value { storage: ValueStorage::Inline { val_bits: av & bv, xz_bits: 0 }, width: w, is_signed: false, is_real: false }
                } else {
                    // X propagation for AND: 0 & X = 0, 1 & X = X
                    let any_xz = ax | bx;
                    let result_val = av & bv & !any_xz;
                    let result_xz = any_xz & !((!av & !ax) | (!bv & !bx)); // known 0 kills X
                    Value { storage: ValueStorage::Inline { val_bits: result_val, xz_bits: result_xz & Self::mask(w) }, width: w, is_signed: false, is_real: false }
                }
            }
            _ => self.bitwise_op_slow(other, |a, b| match (a, b) {
                (LogicBit::Zero, _) | (_, LogicBit::Zero) => LogicBit::Zero,
                (LogicBit::One, LogicBit::One) => LogicBit::One,
                _ => LogicBit::X,
            }),
        }
    }

    pub fn bitwise_or(&self, other: &Value) -> Value {
        let w = self.width.max(other.width);
        match (&self.storage, &other.storage) {
            (ValueStorage::Inline { val_bits: av, xz_bits: ax }, ValueStorage::Inline { val_bits: bv, xz_bits: bx }) => {
                if *ax == 0 && *bx == 0 {
                    Value { storage: ValueStorage::Inline { val_bits: av | bv, xz_bits: 0 }, width: w, is_signed: false, is_real: false }
                } else {
                    let any_xz = ax | bx;
                    let result_val = (av | bv) & !any_xz;
                    let result_xz = any_xz & !((av & !ax) | (bv & !bx)); // known 1 kills X
                    Value { storage: ValueStorage::Inline { val_bits: result_val | ((av & !ax) | (bv & !bx)), xz_bits: result_xz & Self::mask(w) }, width: w, is_signed: false, is_real: false }
                }
            }
            _ => self.bitwise_op_slow(other, |a, b| match (a, b) {
                (LogicBit::One, _) | (_, LogicBit::One) => LogicBit::One,
                (LogicBit::Zero, LogicBit::Zero) => LogicBit::Zero,
                _ => LogicBit::X,
            }),
        }
    }

    pub fn bitwise_xor(&self, other: &Value) -> Value {
        let w = self.width.max(other.width);
        match (&self.storage, &other.storage) {
            (ValueStorage::Inline { val_bits: av, xz_bits: ax }, ValueStorage::Inline { val_bits: bv, xz_bits: bx }) => {
                let any_xz = ax | bx;
                let result_val = (av ^ bv) & !any_xz;
                Value { storage: ValueStorage::Inline { val_bits: result_val, xz_bits: any_xz & Self::mask(w) }, width: w, is_signed: false, is_real: false }
            }
            _ => self.bitwise_op_slow(other, |a, b| match (a, b) {
                (LogicBit::Zero, LogicBit::Zero) | (LogicBit::One, LogicBit::One) => LogicBit::Zero,
                (LogicBit::Zero, LogicBit::One) | (LogicBit::One, LogicBit::Zero) => LogicBit::One,
                _ => LogicBit::X,
            }),
        }
    }

    pub fn bitwise_xnor(&self, other: &Value) -> Value {
        let r = self.bitwise_xor(other);
        r.bitwise_not()
    }

    pub fn bitwise_not(&self) -> Value {
        match &self.storage {
            ValueStorage::Inline { val_bits, xz_bits } => {
                let mask = Self::mask(self.width);
                Value {
                    storage: ValueStorage::Inline { val_bits: (!val_bits & !xz_bits) & mask, xz_bits: *xz_bits },
                    width: self.width, is_signed: self.is_signed, is_real: false,
                }
            }
            ValueStorage::Wide(bits) => {
                let new_bits: Vec<LogicBit> = bits.iter().map(|b| match b {
                    LogicBit::Zero => LogicBit::One,
                    LogicBit::One => LogicBit::Zero,
                    _ => LogicBit::X,
                }).collect();
                Value { storage: ValueStorage::Wide(new_bits), width: self.width, is_signed: self.is_signed , is_real: false }
            }
        }
    }

    fn bitwise_op_slow(&self, other: &Value, op: impl Fn(LogicBit, LogicBit) -> LogicBit) -> Value {
        let w = self.width.max(other.width) as usize;
        let mut result = Value::zero(w as u32);
        for i in 0..w {
            let a = self.get_bit(i);
            let b = other.get_bit(i);
            result.set_bit(i, op(a, b));
        }
        result
    }

    /// Per-bit merge following IEEE 1800 §11.4.11 Table 11-21: a bit is known
    /// only where `self` and `other` agree; every other bit becomes X. Used by
    /// the `?:` operator when the condition is X/Z: both branches are evaluated
    /// and combined bitwise.
    pub fn merge_unknown(&self, other: &Value) -> Value {
        let w = self.width.max(other.width);
        match (&self.storage, &other.storage) {
            (ValueStorage::Inline { val_bits: av, xz_bits: ax },
             ValueStorage::Inline { val_bits: bv, xz_bits: bx }) if w <= 64 => {
                let mask = Self::mask(w);
                let ax = *ax & mask;
                let bx = *bx & mask;
                let av = *av & mask;
                let bv = *bv & mask;
                // Bit is known iff both sides are known and equal.
                let both_known = !ax & !bx & mask;
                let agree = both_known & !(av ^ bv);
                let xz_bits = mask & !agree;
                let val_bits = av & agree;
                Value {
                    storage: ValueStorage::Inline { val_bits, xz_bits },
                    width: w, is_signed: self.is_signed && other.is_signed, is_real: false,
                }
            }
            _ => {
                let mut result = Value::new(w);
                for i in 0..w as usize {
                    let a = if i < self.width as usize { self.get_bit(i) } else { LogicBit::Zero };
                    let b = if i < other.width as usize { other.get_bit(i) } else { LogicBit::Zero };
                    let bit = match (a, b) {
                        (LogicBit::Zero, LogicBit::Zero) => LogicBit::Zero,
                        (LogicBit::One, LogicBit::One) => LogicBit::One,
                        _ => LogicBit::X,
                    };
                    result.set_bit(i, bit);
                }
                result
            }
        }
    }

    // === Shifts ===

    pub fn shift_left(&self, amount: &Value) -> Value {
        let amt = amount.to_u64().unwrap_or(0) as u32;
        if amount.has_xz() { return Value::new(self.width); }
        match &self.storage {
            ValueStorage::Inline { val_bits, xz_bits } => {
                let mask = Self::mask(self.width);
                if amt >= self.width { return Value::zero(self.width); }
                Value {
                    storage: ValueStorage::Inline {
                        val_bits: (val_bits << amt) & mask,
                        xz_bits: (xz_bits << amt) & mask,
                    },
                    width: self.width, is_signed: self.is_signed, is_real: false,
                }
            }
            _ => {
                let mut result = Value::zero(self.width);
                for i in 0..self.width as usize {
                    let src = (i as u32).checked_sub(amt);
                    if let Some(s) = src {
                        result.set_bit(i, self.get_bit(s as usize));
                    }
                }
                result
            }
        }
    }

    pub fn shift_right(&self, amount: &Value) -> Value {
        let amt = amount.to_u64().unwrap_or(0) as u32;
        if amount.has_xz() { return Value::new(self.width); }
        match &self.storage {
            ValueStorage::Inline { val_bits, xz_bits } => {
                if amt >= self.width { return Value::zero(self.width); }
                Value {
                    storage: ValueStorage::Inline {
                        val_bits: val_bits >> amt,
                        xz_bits: xz_bits >> amt,
                    },
                    width: self.width, is_signed: self.is_signed, is_real: false,
                }
            }
            _ => {
                let mut result = Value::zero(self.width);
                for i in 0..self.width as usize {
                    let src = i + amt as usize;
                    if src < self.width as usize {
                        result.set_bit(i, self.get_bit(src));
                    }
                }
                result
            }
        }
    }

    pub fn arith_shift_right(&self, amount: &Value) -> Value {
        let amt = amount.to_u64().unwrap_or(0) as u32;
        if amount.has_xz() { return Value::new(self.width); }
        let sign = self.get_bit(self.width.saturating_sub(1) as usize);
        match &self.storage {
            ValueStorage::Inline { val_bits, xz_bits } => {
                if amt >= self.width {
                    return if sign == LogicBit::One {
                        let mask = Self::mask(self.width);
                        Value { storage: ValueStorage::Inline { val_bits: mask, xz_bits: 0 }, width: self.width, is_signed: true , is_real: false }
                    } else { Value::zero(self.width) };
                }
                let shifted_val = val_bits >> amt;
                let shifted_xz = xz_bits >> amt;
                if sign == LogicBit::One && self.width > 0 {
                    let mask = Self::mask(self.width);
                    let ext = mask & !Self::mask(self.width - amt);
                    Value {
                        storage: ValueStorage::Inline { val_bits: shifted_val | ext, xz_bits: shifted_xz },
                        width: self.width, is_signed: true, is_real: false,
                    }
                } else {
                    Value {
                        storage: ValueStorage::Inline { val_bits: shifted_val, xz_bits: shifted_xz },
                        width: self.width, is_signed: self.is_signed, is_real: false,
                    }
                }
            }
            _ => {
                let mut result = Value::zero(self.width);
                for i in 0..self.width as usize {
                    let src = i + amt as usize;
                    let bit = if src < self.width as usize { self.get_bit(src) } else { sign };
                    result.set_bit(i, bit);
                }
                result.is_signed = true;
                result
            }
        }
    }

    // === Comparison ===

    pub fn is_equal(&self, other: &Value) -> Value {
        if self.is_real || other.is_real {
            return Value::from_u64((self.to_f64() == other.to_f64()) as u64, 1);
        }
        if self.has_xz() || other.has_xz() {
            // IEEE 1800: == returns X only when ambiguous.
            // If any position has both bits known and they differ -> 0.
            let w = self.width.max(other.width) as usize;
            let sign_a = self.is_signed && (self.width as usize) < w;
            let sign_b = other.is_signed && (other.width as usize) < w;
            let top_a = if self.width > 0 { self.get_bit((self.width - 1) as usize) } else { LogicBit::Zero };
            let top_b = if other.width > 0 { other.get_bit((other.width - 1) as usize) } else { LogicBit::Zero };
            for i in 0..w {
                let a = if i < self.width as usize { self.get_bit(i) } else if sign_a { top_a } else { LogicBit::Zero };
                let b = if i < other.width as usize { other.get_bit(i) } else if sign_b { top_b } else { LogicBit::Zero };
                let a_known = matches!(a, LogicBit::Zero | LogicBit::One);
                let b_known = matches!(b, LogicBit::Zero | LogicBit::One);
                if a_known && b_known && a != b {
                    return Value::from_u64(0, 1);
                }
            }
            return Value::new(1);
        }
        // IEEE 1800: if either operand is signed, sign-extend both to max width
        if (self.is_signed || other.is_signed) && self.width != other.width {
            let w = self.width.max(other.width);
            let a = self.resize(w).to_u64().unwrap_or(0);
            let b = other.resize(w).to_u64().unwrap_or(0);
            return Value::from_u64((a == b) as u64, 1);
        }
        let eq = self.to_u64().unwrap_or(0) == other.to_u64().unwrap_or(0);
        Value::from_u64(eq as u64, 1)
    }

    pub fn is_not_equal(&self, other: &Value) -> Value {
        let eq = self.is_equal(other);
        match eq.get_bit(0) {
            LogicBit::Zero => Value::from_u64(1, 1),
            LogicBit::One => Value::from_u64(0, 1),
            _ => Value::new(1),
        }
    }

    pub fn case_eq(&self, other: &Value) -> Value {
        // === operator: compares including X/Z
        let w = self.width.max(other.width) as usize;
        for i in 0..w {
            if self.get_bit(i) != other.get_bit(i) { return Value::from_u64(0, 1); }
        }
        Value::from_u64(1, 1)
    }

    pub fn case_neq(&self, other: &Value) -> Value {
        let eq = self.case_eq(other);
        if eq.to_u64() == Some(1) { Value::from_u64(0, 1) } else { Value::from_u64(1, 1) }
    }

    pub fn less_than(&self, other: &Value) -> Value {
        if self.has_xz() || other.has_xz() { return Value::new(1); }
        if self.is_real || other.is_real {
            return Value::from_u64((self.to_f64() < other.to_f64()) as u64, 1);
        }
        if self.is_signed || other.is_signed {
            let a = self.to_i64().unwrap_or(0);
            let b = other.to_i64().unwrap_or(0);
            Value::from_u64((a < b) as u64, 1)
        } else {
            let a = self.to_u64().unwrap_or(0);
            let b = other.to_u64().unwrap_or(0);
            Value::from_u64((a < b) as u64, 1)
        }
    }

    pub fn less_equal(&self, other: &Value) -> Value {
        if self.has_xz() || other.has_xz() { return Value::new(1); }
        if self.is_real || other.is_real {
            return Value::from_u64((self.to_f64() <= other.to_f64()) as u64, 1);
        }
        if self.is_signed || other.is_signed {
            Value::from_u64((self.to_i64().unwrap_or(0) <= other.to_i64().unwrap_or(0)) as u64, 1)
        } else {
            Value::from_u64((self.to_u64().unwrap_or(0) <= other.to_u64().unwrap_or(0)) as u64, 1)
        }
    }

    pub fn greater_than(&self, other: &Value) -> Value { other.less_than(self) }
    pub fn greater_equal(&self, other: &Value) -> Value { other.less_equal(self) }

    // === Logic ===

    pub fn logic_and(&self, other: &Value) -> Value {
        let a = self.is_nonzero();
        let b = other.is_nonzero();
        match (a, b) {
            (Some(true), Some(true)) => Value::from_u64(1, 1),
            (Some(false), _) | (_, Some(false)) => Value::from_u64(0, 1),
            _ => Value::new(1),
        }
    }

    pub fn logic_or(&self, other: &Value) -> Value {
        let a = self.is_nonzero();
        let b = other.is_nonzero();
        match (a, b) {
            (Some(true), _) | (_, Some(true)) => Value::from_u64(1, 1),
            (Some(false), Some(false)) => Value::from_u64(0, 1),
            _ => Value::new(1),
        }
    }

    pub fn logic_not(&self) -> Value {
        match self.is_nonzero() {
            Some(true) => Value::from_u64(0, 1),
            Some(false) => Value::from_u64(1, 1),
            None => Value::new(1),
        }
    }

    /// Returns Some(true) if nonzero, Some(false) if zero, None if contains X/Z.
    pub fn is_nonzero(&self) -> Option<bool> {
        if self.is_real {
            return Some(self.to_f64() != 0.0);
        }
        match &self.storage {
            ValueStorage::Inline { val_bits, xz_bits } => {
                if *xz_bits != 0 { None }
                else { Some(*val_bits != 0) }
            }
            ValueStorage::Wide(bits) => {
                let has_xz = bits.iter().any(|b| matches!(b, LogicBit::X | LogicBit::Z));
                if has_xz { None }
                else { Some(bits.iter().any(|b| *b == LogicBit::One)) }
            }
        }
    }

    // === Reduction ===

    pub fn reduce_and(&self) -> Value {
        match &self.storage {
            ValueStorage::Inline { val_bits, xz_bits } => {
                let mask = Self::mask(self.width);
                if *xz_bits & mask != 0 { Value::new(1) }
                else { Value::from_u64(if *val_bits & mask == mask { 1 } else { 0 }, 1) }
            }
            ValueStorage::Wide(bits) => {
                if bits.iter().any(|b| !b.is_known()) { Value::new(1) }
                else { Value::from_u64(if bits.iter().all(|b| *b == LogicBit::One) { 1 } else { 0 }, 1) }
            }
        }
    }

    pub fn reduce_or(&self) -> Value {
        match &self.storage {
            ValueStorage::Inline { val_bits, xz_bits } => {
                let mask = Self::mask(self.width);
                if (*val_bits & !xz_bits & mask) != 0 { Value::from_u64(1, 1) }
                else if *xz_bits & mask != 0 { Value::new(1) }
                else { Value::from_u64(0, 1) }
            }
            ValueStorage::Wide(bits) => {
                if bits.iter().any(|b| *b == LogicBit::One) { Value::from_u64(1, 1) }
                else if bits.iter().any(|b| !b.is_known()) { Value::new(1) }
                else { Value::from_u64(0, 1) }
            }
        }
    }

    pub fn reduce_xor(&self) -> Value {
        if self.has_xz() { return Value::new(1); }
        let v = self.to_u64().unwrap_or(0);
        Value::from_u64(v.count_ones() as u64 % 2, 1)
    }

    // === Concatenation ===

    pub fn concat(values: &[Value]) -> Value {
        // values[0] is leftmost (MSB)
        let total_width: u32 = values.iter().map(|v| v.width).sum();
        let mut result = Value::zero(total_width);
        let mut offset = 0u32;
        for val in values.iter().rev() {
            for i in 0..val.width as usize {
                result.set_bit((offset as usize) + i, val.get_bit(i));
            }
            offset += val.width;
        }
        result
    }

    /// Format as hex string.
    pub fn to_hex(&self) -> String {
        if self.width == 0 { return "0".into(); }
        let ndigits = ((self.width + 3) / 4) as usize;
        let mut s = String::with_capacity(ndigits);
        for d in (0..ndigits).rev() {
            let mut digit = 0u8;
            let mut has_x = false;
            for b in 0..4 {
                let bit_idx = d * 4 + b;
                match self.get_bit(bit_idx) {
                    LogicBit::One => digit |= 1 << b,
                    LogicBit::X | LogicBit::Z => has_x = true,
                    _ => {}
                }
            }
            if has_x { s.push('x'); } else { s.push(char::from_digit(digit as u32, 16).unwrap()); }
        }
        s
    }

    /// Format as binary string.
    pub fn to_bin(&self) -> String {
        let mut s = String::with_capacity(self.width as usize);
        for i in (0..self.width as usize).rev() {
            s.push(match self.get_bit(i) {
                LogicBit::Zero => '0',
                LogicBit::One => '1',
                LogicBit::X => 'x',
                LogicBit::Z => 'z',
            });
        }
        if s.is_empty() { s.push('0'); }
        s
    }

    /// Compatibility: access bits as a slice-like interface.
    /// This is for existing code that uses value.bits[i] or value.bits.first().
    pub fn bits_first(&self) -> LogicBit {
        self.get_bit(0)
    }

    /// Extract string content from bit vector.
    pub fn to_string(&self) -> String {
        let mut s = Vec::new();
        let bytes = self.width / 8;
        for b in 0..bytes {
            let mut byte_val = 0u8;
            for bit in 0..8 {
                if self.get_bit((b * 8 + bit) as usize) == LogicBit::One { byte_val |= 1 << bit; }
            }
            if byte_val == 0 { break; }
            s.push(byte_val);
        }
        // SV strings are MSB-first, so byte 0 is the LAST character.
        s.reverse();
        String::from_utf8_lossy(&s).into_owned()
    }
}

/// A reference wrapper for accessing bits, providing compatibility with
/// code that uses `value.bits`.
pub struct BitsRef<'a> {
    value: &'a Value,
}

impl<'a> BitsRef<'a> {
    pub fn first(&self) -> Option<LogicBit> {
        if self.value.width > 0 { Some(self.value.get_bit(0)) } else { None }
    }

    pub fn get(&self, i: usize) -> Option<LogicBit> {
        if (i as u32) < self.value.width { Some(self.value.get_bit(i)) } else { None }
    }

    pub fn len(&self) -> usize {
        self.value.width as usize
    }

    pub fn iter(&self) -> BitsIter<'a> {
        BitsIter { value: self.value, pos: 0 }
    }
}

impl<'a> PartialEq for BitsRef<'a> {
    fn eq(&self, other: &Self) -> bool {
        if self.value.width != other.value.width { return false; }
        for i in 0..self.value.width as usize {
            if self.value.get_bit(i) != other.value.get_bit(i) { return false; }
        }
        true
    }
}

pub struct BitsIter<'a> {
    value: &'a Value,
    pos: usize,
}

impl<'a> Iterator for BitsIter<'a> {
    type Item = LogicBit;
    fn next(&mut self) -> Option<Self::Item> {
        if (self.pos as u32) < self.value.width {
            let bit = self.value.get_bit(self.pos);
            self.pos += 1;
            Some(bit)
        } else {
            None
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}'", self.width)?;
        if self.has_xz() {
            write!(f, "b{}", self.to_bin())
        } else {
            write!(f, "h{}", self.to_hex())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_ops() {
        let a = Value::from_u64(5, 8);
        let b = Value::from_u64(3, 8);
        assert_eq!(a.add(&b).to_u64(), Some(8));
        assert_eq!(a.sub(&b).to_u64(), Some(2));
        assert_eq!(a.bitwise_and(&b).to_u64(), Some(1));
        assert_eq!(a.bitwise_or(&b).to_u64(), Some(7));
    }

    #[test]
    fn test_shifts() {
        let v = Value::from_u64(0x0F, 8);
        assert_eq!(v.shift_left(&Value::from_u64(4, 8)).to_u64(), Some(0xF0));
        assert_eq!(v.shift_right(&Value::from_u64(2, 8)).to_u64(), Some(3));
    }

    #[test]
    fn test_x_propagation() {
        let x = Value::new(8); // all X
        let one = Value::from_u64(1, 8);
        assert!(x.add(&one).has_xz());
        assert!(x.is_equal(&one).has_xz());
    }
}

// Compatibility shims for the simulator
impl Value {
    /// Check if the value represents a nonzero / true condition
    pub fn is_true(&self) -> bool {
        self.is_nonzero().unwrap_or(false)
    }

    /// Check if the value has any unknown (X/Z) bits
    pub fn has_unknown(&self) -> bool {
        match &self.storage {
            ValueStorage::Inline { xz_bits, .. } => *xz_bits != 0,
            ValueStorage::Wide(bits) => bits.iter().any(|b| matches!(b, LogicBit::X | LogicBit::Z)),
        }
    }

    /// Create a value with all bits set to 1
    pub fn ones(width: u32) -> Self {
        if width <= 64 {
            Self::from_u64(Self::mask(width), width)
        } else {
            let bits = vec![LogicBit::One; width as usize];
            Self { storage: ValueStorage::Wide(bits), width, is_signed: false, is_real: false }
        }
    }

    /// Decimal string representation
    pub fn to_dec_string(&self) -> String {
        if self.is_real {
            return format!("{:?}", self.to_f64());
        }
        if self.has_unknown() {
            return "x".to_string();
        }
        if self.width <= 64 {
            if self.is_signed {
                if let Some(v) = self.to_i64() {
                    return format!("{}", v);
                }
            }
            if let Some(v) = self.to_u64() {
                return format!("{}", v);
            }
        }
        // Wide value: compute from bits
        let mut result = 0u128;
        for i in (0..self.width as usize).rev() {
            result = result * 2 + if self.get_bit(i) == LogicBit::One { 1 } else { 0 };
        }
        if self.is_signed && self.get_bit(self.width as usize - 1) == LogicBit::One {
            // Negative: 2's complement
            let max = 1u128 << self.width;
            format!("-{}", max - result)
        } else {
            format!("{}", result)
        }
    }

    /// Convert packed bytes to a SystemVerilog-style string.
    /// Interprets the value as big-endian bytes (MSB first) and
    /// trims leading NUL bytes introduced by widening.
    pub fn to_sv_string(&self) -> String {
        let num_bytes = ((self.width + 7) / 8) as usize;
        if num_bytes == 0 {
            return String::new();
        }
        let mut out: Vec<u8> = Vec::new();
        for bi in (0..num_bytes).rev() {
            let mut byte = 0u8;
            for b in 0..8usize {
                let bit_idx = bi * 8 + b;
                if bit_idx >= self.width as usize {
                    break;
                }
                if self.get_bit(bit_idx) == LogicBit::One {
                    byte |= 1u8 << b;
                }
            }
            if byte != 0 {
                out.push(byte);
            }
        }
        String::from_utf8_lossy(&out).to_string()
    }

    /// Hex string representation
    pub fn to_hex_string(&self) -> String {
        self.to_hex()
    }

    /// Binary string representation  
    pub fn to_bin_string(&self) -> String {
        self.to_bin()
    }

    /// Parse from a string with given radix (2, 8, 10, 16)
    pub fn from_str_radix(s: &str, radix: u32, width: u32) -> Self {
        let s = s.trim().replace("_", "");
        if s.contains('x') || s.contains('X') || s.contains('z') || s.contains('Z') {
            // Parse with unknown bits
            let mut val = Self::zero(width);
            let bits_per_digit = match radix {
                2 => 1, 8 => 3, 16 => 4,
                _ => { 
                    // For decimal, can't have x/z
                    return Self::new(width);
                }
            };
            for (i, ch) in s.chars().rev().enumerate() {
                let bit_pos = i * bits_per_digit;
                match ch {
                    'x' | 'X' => {
                        for b in 0..bits_per_digit {
                            if bit_pos + b < width as usize {
                                val.set_bit(bit_pos + b, LogicBit::X);
                            }
                        }
                    }
                    'z' | 'Z' | '?' => {
                        for b in 0..bits_per_digit {
                            if bit_pos + b < width as usize {
                                val.set_bit(bit_pos + b, LogicBit::Z);
                            }
                        }
                    }
                    _ => {
                        if let Some(digit) = ch.to_digit(radix) {
                            for b in 0..bits_per_digit {
                                if bit_pos + b < width as usize {
                                    val.set_bit(bit_pos + b, if (digit >> b) & 1 == 1 { LogicBit::One } else { LogicBit::Zero });
                                }
                            }
                        }
                    }
                }
            }
            // IEEE §5.7.1: If the MSB digit is x, upper bits fill with x.
            // If the MSB digit is z, upper bits fill with z.
            // Otherwise, upper bits fill with 0.
            let specified_bits = s.chars().count() * bits_per_digit;
            if specified_bits < width as usize {
                let msb_char = s.chars().next().unwrap_or('0');
                let fill = match msb_char {
                    'x' | 'X' => LogicBit::X,
                    'z' | 'Z' | '?' => LogicBit::Z,
                    _ => LogicBit::Zero,
                };
                if fill != LogicBit::Zero {
                    for b in specified_bits..width as usize {
                        val.set_bit(b, fill);
                    }
                }
            }
            val
        } else {
            // Pure numeric
            if let Ok(v) = u64::from_str_radix(&s, radix) {
                return Self::from_u64(v, width);
            }
            // Wide value: parse digit-by-digit for radices that are powers of 2.
            let bits_per_digit = match radix { 2 => 1, 8 => 3, 16 => 4, _ => 0 };
            if bits_per_digit == 0 {
                // Decimal wide number not supported here; fall back to zero.
                return Self::zero(width);
            }
            let mut val = Self::zero(width);
            for (i, ch) in s.chars().rev().enumerate() {
                let bit_pos = i * bits_per_digit;
                if let Some(digit) = ch.to_digit(radix) {
                    for b in 0..bits_per_digit {
                        if bit_pos + b < width as usize {
                            val.set_bit(bit_pos + b, if (digit >> b) & 1 == 1 { LogicBit::One } else { LogicBit::Zero });
                        }
                    }
                }
            }
            val
        }
    }

    /// Select a single bit
    pub fn bit_select(&self, index: usize) -> Value {
        let bit = self.get_bit(index);
        let mut v = Value::zero(1);
        v.set_bit(0, bit);
        v
    }

    /// Select a range of bits [left:right]
    pub fn range_select(&self, left: usize, right: usize) -> Value {
        let width = if left >= right { left - right + 1 } else { right - left + 1 };
        let lo = left.min(right);
        let mut result = Value::zero(width as u32);
        for i in 0..width {
            result.set_bit(i, self.get_bit(lo + i));
        }
        result
    }

    /// Not-equal comparison
    pub fn neq(&self, other: &Value) -> Value {
        self.is_not_equal(other)
    }

    /// Less-or-equal comparison
    pub fn leq(&self, other: &Value) -> Value {
        self.less_equal(other)
    }

    /// Greater-or-equal comparison
    pub fn geq(&self, other: &Value) -> Value {
        self.greater_equal(other)
    }
}

impl Value {
    /// Copy the storage from another value (used in NBA apply)
    pub fn copy_from(&mut self, other: &Value) {
        self.storage = other.storage.clone();
        self.width = other.width;
        // Preserve is_signed from self
    }
}

impl Value {
    /// Instance method concat: self ++ other (self is MSB side)
    pub fn concat_with(&self, other: &Value) -> Value {
        Value::concat(&[self.clone(), other.clone()])
    }
}

impl Value {
    /// Create a value with all bits set to Z
    pub fn all_z(width: u32) -> Self {
        if width <= 64 {
            // For inline: xz_bits = all 1s (marks X/Z), val_bits = all 1s (Z vs X)
            let mask = Self::mask(width);
            Self {
                storage: ValueStorage::Inline { val_bits: mask, xz_bits: mask },
                width,
                is_signed: false, is_real: false,
            }
        } else {
            Self {
                storage: ValueStorage::Wide(vec![LogicBit::Z; width as usize]),
                width,
                is_signed: false, is_real: false,
            }
        }
    }
}
