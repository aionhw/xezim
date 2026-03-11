//! Value types for SystemVerilog simulation.
//! Supports 4-state logic (0, 1, X, Z) with arbitrary-width bit vectors.

use std::fmt;

/// A single 4-state logic bit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

/// An arbitrary-width 4-state logic value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Value {
    pub bits: Vec<LogicBit>,  // LSB first: bits[0] is bit 0
    pub width: u32,
    pub is_signed: bool,
}

impl Value {
    pub fn new(width: u32) -> Self {
        Self {
            bits: vec![LogicBit::X; width as usize],
            width,
            is_signed: false,
        }
    }

    pub fn zero(width: u32) -> Self {
        Self {
            bits: vec![LogicBit::Zero; width as usize],
            width,
            is_signed: false,
        }
    }

    pub fn ones(width: u32) -> Self {
        Self {
            bits: vec![LogicBit::One; width as usize],
            width,
            is_signed: false,
        }
    }

    pub fn from_u64(val: u64, width: u32) -> Self {
        let mut bits = Vec::with_capacity(width as usize);
        for i in 0..width {
            if i < 64 && (val >> i) & 1 == 1 {
                bits.push(LogicBit::One);
            } else {
                bits.push(LogicBit::Zero);
            }
        }
        Self { bits, width, is_signed: false }
    }

    pub fn from_i64(val: i64, width: u32) -> Self {
        let mut v = Self::from_u64(val as u64, width);
        v.is_signed = true;
        v
    }

    pub fn from_str_radix(s: &str, radix: u32, width: u32) -> Self {
        let clean: String = s.chars().filter(|c| *c != '_').collect();
        match radix {
            2 => {
                let mut bits = Vec::new();
                for ch in clean.chars().rev() {
                    bits.push(LogicBit::from_char(ch));
                }
                // Pad with MSB value: if MSB is x/z, extend with same
                let pad = bits.last().copied().unwrap_or(LogicBit::Zero);
                let pad = if pad == LogicBit::One { LogicBit::Zero } else { pad };
                while bits.len() < width as usize { bits.push(pad); }
                bits.truncate(width as usize);
                Self { bits, width, is_signed: false }
            }
            16 => {
                let mut bits = Vec::new();
                for ch in clean.chars().rev() {
                    if ch == 'x' || ch == 'X' {
                        for _ in 0..4 { bits.push(LogicBit::X); }
                    } else if ch == 'z' || ch == 'Z' {
                        for _ in 0..4 { bits.push(LogicBit::Z); }
                    } else if let Some(d) = ch.to_digit(16) {
                        for i in 0..4 {
                            bits.push(if (d >> i) & 1 == 1 { LogicBit::One } else { LogicBit::Zero });
                        }
                    }
                }
                let pad = bits.last().copied().unwrap_or(LogicBit::Zero);
                let pad = if pad == LogicBit::One { LogicBit::Zero } else { pad };
                while bits.len() < width as usize { bits.push(pad); }
                bits.truncate(width as usize);
                Self { bits, width, is_signed: false }
            }
            8 => {
                let mut bits = Vec::new();
                for ch in clean.chars().rev() {
                    if let Some(d) = ch.to_digit(8) {
                        for i in 0..3 {
                            bits.push(if (d >> i) & 1 == 1 { LogicBit::One } else { LogicBit::Zero });
                        }
                    }
                }
                while bits.len() < width as usize { bits.push(LogicBit::Zero); }
                bits.truncate(width as usize);
                Self { bits, width, is_signed: false }
            }
            _ => {
                // Decimal
                if let Ok(v) = clean.parse::<u64>() {
                    Self::from_u64(v, width)
                } else {
                    Self::new(width) // X on parse failure
                }
            }
        }
    }

    /// Convert to u64 (returns None if any bit is X or Z).
    pub fn to_u64(&self) -> Option<u64> {
        let mut val: u64 = 0;
        for (i, bit) in self.bits.iter().enumerate() {
            match bit {
                LogicBit::One => {
                    if i < 64 { val |= 1u64 << i; }
                }
                LogicBit::Zero => {}
                _ => return None,
            }
        }
        Some(val)
    }

    /// Convert to i64 (sign-extends based on MSB).
    pub fn to_i64(&self) -> Option<i64> {
        let u = self.to_u64()?;
        if self.is_signed && self.width > 0 && self.width < 64 {
            let sign_bit = 1u64 << (self.width - 1);
            if u & sign_bit != 0 {
                // Sign extend
                let mask = !((1u64 << self.width) - 1);
                return Some((u | mask) as i64);
            }
        }
        Some(u as i64)
    }

    pub fn is_true(&self) -> bool {
        self.bits.iter().any(|b| *b == LogicBit::One)
    }

    pub fn is_zero(&self) -> bool {
        self.bits.iter().all(|b| *b == LogicBit::Zero)
    }

    pub fn has_unknown(&self) -> bool {
        self.bits.iter().any(|b| !b.is_known())
    }

    /// Resize to a new width (zero-extend or truncate).
    pub fn resize(&self, new_width: u32) -> Self {
        let mut bits = self.bits.clone();
        if new_width as usize > bits.len() {
            let ext = if self.is_signed {
                *bits.last().unwrap_or(&LogicBit::Zero)
            } else {
                LogicBit::Zero
            };
            bits.resize(new_width as usize, ext);
        } else {
            bits.truncate(new_width as usize);
        }
        Self { bits, width: new_width, is_signed: self.is_signed }
    }

    /// Bit select: returns single bit as a 1-bit value.
    pub fn bit_select(&self, idx: usize) -> Self {
        let bit = self.bits.get(idx).copied().unwrap_or(LogicBit::X);
        Self { bits: vec![bit], width: 1, is_signed: false }
    }

    /// Range select [msb:lsb].
    pub fn range_select(&self, msb: usize, lsb: usize) -> Self {
        if msb < lsb { return Self::new(0); }
        let w = (msb - lsb + 1) as u32;
        let mut bits = Vec::with_capacity(w as usize);
        for i in lsb..=msb {
            bits.push(self.bits.get(i).copied().unwrap_or(LogicBit::X));
        }
        Self { bits, width: w, is_signed: false }
    }

    /// Format as binary string.
    pub fn to_bin_string(&self) -> String {
        self.bits.iter().rev().map(|b| format!("{}", b)).collect()
    }

    /// Format as hex string.
    pub fn to_hex_string(&self) -> String {
        let mut s = String::new();
        let nibbles = (self.width as usize + 3) / 4;
        for n in (0..nibbles).rev() {
            let base = n * 4;
            let mut has_x = false;
            let mut has_z = false;
            let mut val = 0u8;
            for i in 0..4 {
                let bit = self.bits.get(base + i).copied().unwrap_or(LogicBit::Zero);
                match bit {
                    LogicBit::One => val |= 1 << i,
                    LogicBit::X => has_x = true,
                    LogicBit::Z => has_z = true,
                    _ => {}
                }
            }
            if has_x { s.push('x'); }
            else if has_z { s.push('z'); }
            else { s.push(char::from_digit(val as u32, 16).unwrap()); }
        }
        s
    }

    /// Format as decimal string (if all bits known).
    pub fn to_dec_string(&self) -> String {
        if self.has_unknown() {
            return format!("{}'{}", self.width, self.to_bin_string());
        }
        if self.is_signed {
            if let Some(v) = self.to_i64() { return format!("{}", v); }
        }
        if let Some(v) = self.to_u64() { return format!("{}", v); }
        "X".to_string()
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.to_dec_string())
    }
}

// ═══════════════════════════════════════════════════════════════════
// Arithmetic and logic operations on Values
// ═══════════════════════════════════════════════════════════════════

impl Value {
    pub fn bitwise_not(&self) -> Self {
        let bits = self.bits.iter().map(|b| match b {
            LogicBit::Zero => LogicBit::One,
            LogicBit::One => LogicBit::Zero,
            _ => LogicBit::X,
        }).collect();
        Self { bits, width: self.width, is_signed: self.is_signed }
    }

    pub fn bitwise_and(&self, other: &Self) -> Self {
        let w = self.width.max(other.width);
        let a = self.resize(w);
        let b = other.resize(w);
        let bits = a.bits.iter().zip(b.bits.iter()).map(|(x, y)| match (x, y) {
            (LogicBit::Zero, _) | (_, LogicBit::Zero) => LogicBit::Zero,
            (LogicBit::One, LogicBit::One) => LogicBit::One,
            _ => LogicBit::X,
        }).collect();
        Self { bits, width: w, is_signed: false }
    }

    pub fn bitwise_or(&self, other: &Self) -> Self {
        let w = self.width.max(other.width);
        let a = self.resize(w);
        let b = other.resize(w);
        let bits = a.bits.iter().zip(b.bits.iter()).map(|(x, y)| match (x, y) {
            (LogicBit::One, _) | (_, LogicBit::One) => LogicBit::One,
            (LogicBit::Zero, LogicBit::Zero) => LogicBit::Zero,
            _ => LogicBit::X,
        }).collect();
        Self { bits, width: w, is_signed: false }
    }

    pub fn bitwise_xor(&self, other: &Self) -> Self {
        let w = self.width.max(other.width);
        let a = self.resize(w);
        let b = other.resize(w);
        let bits = a.bits.iter().zip(b.bits.iter()).map(|(x, y)| match (x, y) {
            (LogicBit::Zero, LogicBit::Zero) | (LogicBit::One, LogicBit::One) => LogicBit::Zero,
            (LogicBit::Zero, LogicBit::One) | (LogicBit::One, LogicBit::Zero) => LogicBit::One,
            _ => LogicBit::X,
        }).collect();
        Self { bits, width: w, is_signed: false }
    }

    pub fn logic_not(&self) -> Self {
        if self.has_unknown() { return Self { bits: vec![LogicBit::X], width: 1, is_signed: false }; }
        if self.is_zero() { Self::from_u64(1, 1) } else { Self::from_u64(0, 1) }
    }

    pub fn logic_and(&self, other: &Self) -> Self {
        if self.has_unknown() || other.has_unknown() { return Self { bits: vec![LogicBit::X], width: 1, is_signed: false }; }
        Self::from_u64(if self.is_true() && other.is_true() { 1 } else { 0 }, 1)
    }

    pub fn logic_or(&self, other: &Self) -> Self {
        if self.is_true() || other.is_true() { return Self::from_u64(1, 1); }
        if self.has_unknown() || other.has_unknown() { return Self { bits: vec![LogicBit::X], width: 1, is_signed: false }; }
        Self::from_u64(0, 1)
    }

    pub fn add(&self, other: &Self) -> Self {
        let w = self.width.max(other.width);
        let signed = self.is_signed && other.is_signed;
        if signed {
            let a = self.resize(w);
            let b = other.resize(w);
            match (a.to_i64(), b.to_i64()) {
                (Some(av), Some(bv)) => {
                    let mut v = Self::from_i64(av.wrapping_add(bv), w + 1);
                    v.is_signed = true; v
                }
                _ => Self::new(w),
            }
        } else {
            match (self.to_u64(), other.to_u64()) {
                (Some(a), Some(b)) => Self::from_u64(a.wrapping_add(b), w + 1),
                _ => Self::new(w),
            }
        }
    }

    pub fn sub(&self, other: &Self) -> Self {
        let w = self.width.max(other.width);
        let signed = self.is_signed && other.is_signed;
        if signed {
            let a = self.resize(w);
            let b = other.resize(w);
            match (a.to_i64(), b.to_i64()) {
                (Some(av), Some(bv)) => {
                    let mut v = Self::from_i64(av.wrapping_sub(bv), w + 1);
                    v.is_signed = true; v
                }
                _ => Self::new(w),
            }
        } else {
            match (self.to_u64(), other.to_u64()) {
                (Some(a), Some(b)) => Self::from_u64(a.wrapping_sub(b), w + 1),
                _ => Self::new(w),
            }
        }
    }

    pub fn mul(&self, other: &Self) -> Self {
        let w = self.width.max(other.width);
        let signed = self.is_signed || other.is_signed;
        if signed {
            let a_resized = self.resize(w);
            let b_resized = other.resize(w);
            match (a_resized.to_i64(), b_resized.to_i64()) {
                (Some(a), Some(b)) => {
                    let mut v = Self::from_i64(a.wrapping_mul(b), w);
                    v.is_signed = true;
                    v
                }
                _ => Self::new(w),
            }
        } else {
            match (self.to_u64(), other.to_u64()) {
                (Some(a), Some(b)) => Self::from_u64(a.wrapping_mul(b), w),
                _ => Self::new(w),
            }
        }
    }

    pub fn div(&self, other: &Self) -> Self {
        let w = self.width.max(other.width);
        match (self.to_u64(), other.to_u64()) {
            (Some(_), Some(0)) => Self::new(w),
            (Some(a), Some(b)) => Self::from_u64(a / b, w),
            _ => Self::new(w),
        }
    }

    pub fn modulo(&self, other: &Self) -> Self {
        let w = self.width.max(other.width);
        match (self.to_u64(), other.to_u64()) {
            (Some(_), Some(0)) => Self::new(w),
            (Some(a), Some(b)) => Self::from_u64(a % b, w),
            _ => Self::new(w),
        }
    }

    pub fn shift_left(&self, amount: &Self) -> Self {
        match amount.to_u64() {
            Some(n) if n < 64 => {
                match self.to_u64() {
                    Some(v) => Self::from_u64(v << n, self.width),
                    None => Self::new(self.width),
                }
            }
            _ => Self::zero(self.width),
        }
    }

    pub fn shift_right(&self, amount: &Self) -> Self {
        match amount.to_u64() {
            Some(n) if n < 64 => {
                match self.to_u64() {
                    Some(v) => Self::from_u64(v >> n, self.width),
                    None => Self::new(self.width),
                }
            }
            _ => Self::zero(self.width),
        }
    }

    pub fn arith_shift_right(&self, amount: &Self) -> Self {
        if !self.is_signed { return self.shift_right(amount); }
        match (self.to_i64(), amount.to_u64()) {
            (Some(v), Some(n)) if n < 64 => Self::from_i64(v >> n, self.width),
            _ => Self::new(self.width),
        }
    }

    pub fn eq(&self, other: &Self) -> Self {
        if self.has_unknown() || other.has_unknown() { return Value { bits: vec![LogicBit::X], width: 1, is_signed: false }; }
        let w = self.width.max(other.width);
        let a = self.resize(w);
        let b = other.resize(w);
        Self::from_u64(if a.bits == b.bits { 1 } else { 0 }, 1)
    }

    pub fn neq(&self, other: &Self) -> Self {
        let e = self.eq(other);
        if e.has_unknown() { e } else { e.logic_not() }
    }

    pub fn case_eq(&self, other: &Self) -> Self {
        let w = self.width.max(other.width);
        let a = self.resize(w);
        let b = other.resize(w);
        Self::from_u64(if a.bits == b.bits { 1 } else { 0 }, 1)
    }

    pub fn lt(&self, other: &Self) -> Self {
        if self.has_unknown() || other.has_unknown() { return Value { bits: vec![LogicBit::X], width: 1, is_signed: false }; }
        if self.is_signed && other.is_signed {
            match (self.to_i64(), other.to_i64()) {
                (Some(a), Some(b)) => Self::from_u64(if a < b { 1 } else { 0 }, 1),
                _ => Value { bits: vec![LogicBit::X], width: 1, is_signed: false },
            }
        } else {
            match (self.to_u64(), other.to_u64()) {
                (Some(a), Some(b)) => Self::from_u64(if a < b { 1 } else { 0 }, 1),
                _ => Value { bits: vec![LogicBit::X], width: 1, is_signed: false },
            }
        }
    }

    pub fn leq(&self, other: &Self) -> Self {
        if self.has_unknown() || other.has_unknown() { return Value { bits: vec![LogicBit::X], width: 1, is_signed: false }; }
        if self.is_signed && other.is_signed {
            match (self.to_i64(), other.to_i64()) {
                (Some(a), Some(b)) => Self::from_u64(if a <= b { 1 } else { 0 }, 1),
                _ => Value { bits: vec![LogicBit::X], width: 1, is_signed: false },
            }
        } else {
            match (self.to_u64(), other.to_u64()) {
                (Some(a), Some(b)) => Self::from_u64(if a <= b { 1 } else { 0 }, 1),
                _ => Value { bits: vec![LogicBit::X], width: 1, is_signed: false },
            }
        }
    }

    pub fn gt(&self, other: &Self) -> Self {
        if self.has_unknown() || other.has_unknown() { return Value { bits: vec![LogicBit::X], width: 1, is_signed: false }; }
        if self.is_signed && other.is_signed {
            match (self.to_i64(), other.to_i64()) {
                (Some(a), Some(b)) => Self::from_u64(if a > b { 1 } else { 0 }, 1),
                _ => Value { bits: vec![LogicBit::X], width: 1, is_signed: false },
            }
        } else {
            match (self.to_u64(), other.to_u64()) {
                (Some(a), Some(b)) => Self::from_u64(if a > b { 1 } else { 0 }, 1),
                _ => Value { bits: vec![LogicBit::X], width: 1, is_signed: false },
            }
        }
    }

    pub fn geq(&self, other: &Self) -> Self {
        if self.has_unknown() || other.has_unknown() { return Value { bits: vec![LogicBit::X], width: 1, is_signed: false }; }
        if self.is_signed && other.is_signed {
            match (self.to_i64(), other.to_i64()) {
                (Some(a), Some(b)) => Self::from_u64(if a >= b { 1 } else { 0 }, 1),
                _ => Value { bits: vec![LogicBit::X], width: 1, is_signed: false },
            }
        } else {
            match (self.to_u64(), other.to_u64()) {
                (Some(a), Some(b)) => Self::from_u64(if a >= b { 1 } else { 0 }, 1),
                _ => Value { bits: vec![LogicBit::X], width: 1, is_signed: false },
            }
        }
    }

    pub fn power(&self, exp: &Self) -> Self {
        match (self.to_u64(), exp.to_u64()) {
            (Some(base), Some(e)) => Self::from_u64(base.wrapping_pow(e as u32), self.width),
            _ => Self::new(self.width),
        }
    }

    /// Reduction AND
    pub fn reduce_and(&self) -> Self {
        if self.has_unknown() { return Value { bits: vec![LogicBit::X], width: 1, is_signed: false }; }
        Self::from_u64(if self.bits.iter().all(|b| *b == LogicBit::One) { 1 } else { 0 }, 1)
    }

    /// Reduction OR
    pub fn reduce_or(&self) -> Self {
        if self.bits.iter().any(|b| *b == LogicBit::One) { return Self::from_u64(1, 1); }
        if self.has_unknown() { return Value { bits: vec![LogicBit::X], width: 1, is_signed: false }; }
        Self::from_u64(0, 1)
    }

    /// Reduction XOR
    pub fn reduce_xor(&self) -> Self {
        if self.has_unknown() { return Value { bits: vec![LogicBit::X], width: 1, is_signed: false }; }
        let count = self.bits.iter().filter(|b| **b == LogicBit::One).count();
        Self::from_u64(if count % 2 == 1 { 1 } else { 0 }, 1)
    }

    /// Concatenate values: {self, other} where self is the MSB part.
    pub fn concat(&self, other: &Self) -> Self {
        let w = self.width + other.width;
        let mut bits = other.bits.clone();
        bits.extend_from_slice(&self.bits);
        Self { bits, width: w, is_signed: false }
    }
}
