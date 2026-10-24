//! Simulator formatting.

use super::*;

impl Simulator {
    /// §21.2.1.3: widest decimal string the operand's type can produce —
    /// `%d` with no explicit width space-pads to this.
    /// Evaluate one native string op — semantics mirror the AST
    /// interpreter's §6.16 implementations byte-for-byte (same helpers where
    /// they exist: `sv_strcmp`, `sv_ato_value`, the substr highest-bit rule).
    pub(super) fn exec_str_op(
        kind: super::bytecode::StrOpKind,
        args: &[u16],
        regs: &[Value],
    ) -> Value {
        use super::bytecode::StrOpKind as K;
        let v = |i: usize| -> &Value { &regs[args[i] as usize] };
        match kind {
            K::Len => Self::int_count(v(0).sv_string_bytes().len() as u64),
            K::GetC => {
                let idx = v(1).to_u64().unwrap_or(0) as usize;
                let b = v(0).sv_string_bytes().get(idx).copied().unwrap_or(0);
                Self::sv_byte(b)
            }
            K::Substr => {
                // Mirror of the AST rule: length from the highest set bit,
                // inclusive [start, end] byte slice, "" out of range.
                let base = v(0);
                let start = v(1).to_u64().unwrap_or(0) as usize;
                let end = v(2).to_u64().unwrap_or(0) as usize;
                let mut highest_bit = 0usize;
                for i in (0..base.width as usize).rev() {
                    if base.get_bit(i) != LogicBit::Zero {
                        highest_bit = i;
                        break;
                    }
                }
                let actual_len = highest_bit.div_ceil(8);
                if actual_len > 0 && start < actual_len && end < actual_len && start <= end {
                    let l = (actual_len - 1 - start) * 8 + 7;
                    let r = (actual_len - 1 - end) * 8;
                    base.range_select(l, r)
                } else {
                    Value::zero(0)
                }
            }
            K::ToUpper => Value::from_string(&Self::sv_ascii_case(&v(0).to_sv_string(), true)),
            K::ToLower => Value::from_string(&Self::sv_ascii_case(&v(0).to_sv_string(), false)),
            K::Compare | K::ICompare | K::Order => {
                let (a, b) = (v(0).to_sv_string(), v(1).to_sv_string());
                let d = match kind {
                    K::Compare => Self::sv_strcmp(&a, &b, false),
                    K::ICompare => Self::sv_strcmp(&a, &b, true),
                    _ => Self::sv_str_order(&a, &b),
                };
                let mut out = Value::from_u64(d as u64, 32);
                out.is_signed = true;
                out
            }
            K::AToI => sv_ato_value(&v(0).to_sv_string(), 10),
            K::AToHex => sv_ato_value(&v(0).to_sv_string(), 16),
            K::AToOct => sv_ato_value(&v(0).to_sv_string(), 8),
            K::AToBin => sv_ato_value(&v(0).to_sv_string(), 2),
            K::Concat => {
                let mut bytes: Vec<u8> = Vec::new();
                for &r in args {
                    bytes.extend(regs[r as usize].sv_string_bytes());
                }
                let text: String = bytes.into_iter().map(|b| b as char).collect();
                Value::from_string(&text)
            }
            K::PutC => {
                let mut b = v(0).sv_string_bytes();
                let idx = v(1).to_i64().unwrap_or(-1);
                let ch = (v(2).to_u64().unwrap_or(0) & 0xFF) as u8;
                if idx >= 0 && (idx as usize) < b.len() && ch != 0 {
                    b[idx as usize] = ch;
                    Self::sv_string_from_bytes(&b)
                } else {
                    v(0).clone()
                }
            }
            K::IToA => {
                let x = v(0);
                let text = if x.width > 32 {
                    format!("{}", x.to_i64().unwrap_or(0))
                } else {
                    format!("{}", x.to_u64().unwrap_or(0) as u32 as i32)
                };
                Value::from_string(&text)
            }
            K::HexToA => Value::from_string(&format!("{:x}", v(0).to_u64().unwrap_or(0))),
            K::OctToA => Value::from_string(&format!("{:o}", v(0).to_u64().unwrap_or(0))),
            K::BinToA => Value::from_string(&format!("{:b}", v(0).to_u64().unwrap_or(0))),
            K::ToStr => Self::sv_string_cast(v(0)),
        }
    }

    /// Fill a compile-time-parsed `$sformatf` template from register
    /// Values. Byte-identical to the AST formatter for the specs
    /// `parse_format_template` admits — each arm calls the SAME value-level
    /// cores the AST spec arms call.
    pub(super) fn exec_format(f: &super::bytecode::FormatData, regs: &[Value]) -> Value {
        use super::bytecode::FmtSeg;
        let mut out = String::new();
        let mut ai = 0usize;
        for seg in &f.segs {
            match seg {
                FmtSeg::Lit(t) => out.push_str(t),
                FmtSeg::Spec {
                    spec,
                    width,
                    left,
                    str_valued,
                } => {
                    let v = &regs[f.args[ai] as usize];
                    ai += 1;
                    let pad = |out: &mut String, core: String, w: usize, left: bool| {
                        if core.len() >= w {
                            out.push_str(&core);
                        } else if left {
                            out.push_str(&core);
                            for _ in 0..w - core.len() {
                                out.push(' ');
                            }
                        } else {
                            for _ in 0..w - core.len() {
                                out.push(' ');
                            }
                            out.push_str(&core);
                        }
                    };
                    match spec {
                        'd' => {
                            let core = v.to_dec_string();
                            let w = match width {
                                None => Self::dec_default_width(v.width, v.is_signed),
                                Some(n) => *n as usize,
                            };
                            pad(&mut out, core, w, *left);
                        }
                        'b' => {
                            let full = v.to_bin_string();
                            match width {
                                None => out.push_str(&full),
                                Some(n) => Self::push_radix(&mut out, &full, *n as usize, *left),
                            }
                        }
                        'h' | 'x' => {
                            let full = v.to_hex_string();
                            match width {
                                None => out.push_str(&full),
                                Some(n) => Self::push_radix(&mut out, &full, *n as usize, *left),
                            }
                        }
                        'o' => {
                            let full = Self::bin_to_oct_string(&v.to_bin_string());
                            match width {
                                None => out.push_str(&full),
                                Some(n) => Self::push_radix(&mut out, &full, *n as usize, *left),
                            }
                        }
                        's' => {
                            let w = width.unwrap_or(0) as usize;
                            if *str_valued {
                                pad(&mut out, v.to_sv_string(), w, *left);
                            } else {
                                // §21.2.1.3 packed operand: every byte
                                // prints, NUL as space; %0s strips leading
                                // NULs (minimal form).
                                let minimal = matches!(width, Some(0));
                                let nbytes = (v.width as usize).div_ceil(8);
                                let mut bytes: Vec<u8> = Vec::with_capacity(nbytes);
                                for bi in (0..nbytes).rev() {
                                    let mut byte = 0u8;
                                    for b in 0..8usize {
                                        let idx = bi * 8 + b;
                                        if idx < v.width as usize && v.get_bit(idx) == LogicBit::One
                                        {
                                            byte |= 1 << b;
                                        }
                                    }
                                    bytes.push(byte);
                                }
                                if minimal {
                                    while bytes.first() == Some(&0) {
                                        bytes.remove(0);
                                    }
                                }
                                let sres: String = bytes
                                    .iter()
                                    .map(|&b| if b == 0 { ' ' } else { b as char })
                                    .collect();
                                pad(&mut out, sres, w, *left);
                            }
                        }
                        'c' => {
                            let b = (v.to_u64().unwrap_or(0) & 0xff) as u8;
                            pad(
                                &mut out,
                                (b as char).to_string(),
                                width.unwrap_or(0) as usize,
                                *left,
                            );
                        }
                        _ => unreachable!("unsupported spec in compiled template"),
                    }
                }
            }
        }
        Value::from_string(&out)
    }

    pub(super) fn dec_default_width(bits: u32, signed: bool) -> usize {
        if bits == 0 {
            return 1;
        }
        let digits_of =
            |b: u32| (b as f64 * std::f64::consts::LOG2_10.recip()).floor() as usize + 1;
        if signed {
            digits_of(bits.saturating_sub(1).max(1)) + 1
        } else {
            digits_of(bits)
        }
    }

    /// Minimal (leading-zero-trimmed) form of a binary/octal/hex string; x/z
    /// digits stop the trim, and an all-zero value keeps one digit.
    pub(super) fn trim_radix_zeros(s: &str) -> String {
        let t = s.trim_start_matches('0');
        if t.is_empty() {
            return "0".to_string();
        }
        // §21.2.1.3 automatic sizing: a leading RUN of x (or z) digits
        // collapses to one, like leading zeros — `%0h` of 8'hxx is "x",
        // of 16'hxx3f is "x3f". Mixed-case runs (X vs x) keep the first.
        let mut chars = t.chars();
        if let Some(first) = chars.next() {
            if matches!(first, 'x' | 'X' | 'z' | 'Z') {
                let rest: &str = chars.as_str();
                let trimmed = rest.trim_start_matches(|c: char| c.eq_ignore_ascii_case(&first));
                return format!("{}{}", first, trimmed);
            }
        }
        t.to_string()
    }

    /// Full-width octal rendering from a binary string, x/z-aware: a digit is
    /// `x`/`z` when all three bits are, `X`/`Z` when only some are.
    pub(super) fn bin_to_oct_string(bin: &str) -> String {
        let b: Vec<char> = bin.chars().collect();
        let mut digits: Vec<char> = Vec::with_capacity(b.len().div_ceil(3));
        let mut hi = b.len();
        while hi > 0 {
            let lo = hi.saturating_sub(3);
            let grp = &b[lo..hi];
            let n_x = grp.iter().filter(|c| matches!(c, 'x' | 'X')).count();
            let n_z = grp.iter().filter(|c| matches!(c, 'z' | 'Z')).count();
            let d = if n_x == grp.len() {
                'x'
            } else if n_z == grp.len() {
                'z'
            } else if n_x > 0 {
                'X'
            } else if n_z > 0 {
                'Z'
            } else {
                let mut val = 0u32;
                for c in grp {
                    val = val * 2 + if *c == '1' { 1 } else { 0 };
                }
                char::from_digit(val, 8).unwrap_or('0')
            };
            digits.push(d);
            hi = lo;
        }
        digits.reverse();
        digits.into_iter().collect()
    }

    /// C-printf spelling of a non-finite float, or `None` when `x` is finite.
    /// `%f/%e/%g` render `inf`/`nan`; `%F/%E/%G` render `INF`/`NAN`. Only
    /// negative infinity carries a sign — matching a reference simulator and the LRM's
    /// C-printf mapping. (glibc emits `-nan` for `0.0/0.0` because that value
    /// carries a sign bit; a reference simulator normalises it to `nan`, which is what we
    /// reproduce here.) The caller layers on the `+` flag for the positive
    /// forms, exactly as it does for a finite value.
    pub(super) fn nonfinite_float(x: f64, upper: bool) -> Option<String> {
        if x.is_nan() {
            Some(if upper {
                "NAN".to_string()
            } else {
                "nan".to_string()
            })
        } else if x.is_infinite() {
            let body = if upper { "INF" } else { "inf" };
            Some(if x < 0.0 {
                format!("-{}", body)
            } else {
                body.to_string()
            })
        } else {
            None
        }
    }

    /// C-printf-style scientific notation: `1.0e+02` — sign always present,
    /// at least two exponent digits (Rust's `{:e}` writes `1.0e2`).
    pub(super) fn c_style_exp(x: f64, prec: usize, upper: bool) -> String {
        let s = format!("{:.*e}", prec, x);
        let Some(pos) = s.find(['e', 'E']) else {
            return s;
        };
        let (mantissa, exp) = s.split_at(pos);
        let exp = &exp[1..];
        let (sign, digits) = match exp.strip_prefix('-') {
            Some(d) => ('-', d),
            None => ('+', exp.trim_start_matches('+')),
        };
        format!(
            "{}{}{}{:0>2}",
            mantissa,
            if upper { 'E' } else { 'e' },
            sign,
            digits
        )
    }

    /// §21.2.1.2 hex/binary/octal field padding. a reference simulator (and C) pad a bare
    /// `%Nh` with SPACES around the natural full-width form (`8'h0f` -> `  0f`
    /// for `%4h`); the leading-`0` flag both selects the minimal
    /// (leading-zero-trimmed) form AND zero-pads it (`%04h` -> `000f`). A `-`
    /// flag left-justifies with trailing spaces. Passing `full` (not a
    /// pre-trimmed core) lets us honour the flag correctly.
    pub(super) fn push_radix(result: &mut String, full: &str, width: usize, left_align: bool) {
        // The reference simulator's radix-field model, re-measured
        // byte-for-byte (commercial tools disagree here; xezim previously
        // followed the OTHER tool's space-pad-to-natural-width model — the
        // switch is deliberate, see tests/strings/format_sibling_fixes.rs):
        //   * bare `%0<r>`: minimal form — leading zeros AND a leading x/z
        //     run collapse (`%0h` of 16'hxx3f -> "x3f", of 8'hzz -> "z").
        //   * explicit width W: the core is the value with leading ZEROS
        //     trimmed only — an x/z run is kept (`%6h` of 8'hzz ->
        //     "0000zz"). Never truncated below the core (`%2h` of 16'h2a5
        //     -> "2a5"). Right-justified pads with '0' REGARDLESS of the
        //     `0` flag (`%4h` == `%04h` of 8'h0f -> "000f"); `-` pads with
        //     spaces on the right (`%-4h` -> "f   ", `%-08h` of 32'hFF ->
        //     "ff      "). Zero value: `%4h` of 8'h00 -> "0000".
        if width == 0 {
            result.push_str(&Self::trim_radix_zeros(full));
            return;
        }
        let t = full.trim_start_matches('0');
        let core = if t.is_empty() { "0" } else { t };
        if core.len() >= width {
            result.push_str(core);
        } else if left_align {
            result.push_str(core);
            result.push_str(&" ".repeat(width - core.len()));
        } else {
            result.push_str(&"0".repeat(width - core.len()));
            result.push_str(core);
        }
    }

    /// §21.2.1.1 `%g`/`%G`: use `%e` if exponent < -4 or >= precision,
    /// otherwise `%f`; strip trailing zeros. Precision = significant digits
    /// (default 6).
    pub(super) fn format_g(x: f64, prec: usize, upper: bool) -> String {
        if x == 0.0 {
            return "0".to_string();
        }
        let p = prec.max(1);
        // C `%g` decides %f-vs-%e on the exponent of the value ROUNDED to p
        // significant digits, not the raw value. Deriving the exponent from
        // `log10().floor()` picks wrongly at rounding boundaries (999999.5
        // rounds to 1e6 -> exponent 6, so C uses %e) and suffers float error
        // on exact powers of 10. Formatting via %e at precision p-1 performs
        // the rounding for us; its printed exponent is the post-rounding
        // decimal exponent, which is exactly what C's choice is based on.
        let exp: i32 = format!("{:.*e}", p - 1, x)
            .split(['e', 'E'])
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let raw = if exp < -4 || exp >= p as i32 {
            Self::c_style_exp(x, p.saturating_sub(1), upper)
        } else {
            let decimals = (p as i32 - 1 - exp).max(0) as usize;
            format!("{:.*}", decimals, x)
        };
        // Remove trailing zeros from the mantissa (C %g behavior).
        // Works for both plain and exponential forms.
        if let Some(ei) = raw.find(['e', 'E']) {
            let (mant, exp) = raw.split_at(ei);
            let mant = if mant.contains('.') {
                mant.trim_end_matches('0').trim_end_matches('.')
            } else {
                mant
            };
            format!("{}{}", mant, exp)
        } else if raw.contains('.') {
            raw.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            raw
        }
    }

    /// §21.2.1.4 `%u`: the 2-state data (x/z read as 0) as raw little-endian
    /// bytes, padded to whole 32-bit words.
    pub(super) fn unformatted_2state(bin: &str) -> String {
        let bits: Vec<char> = bin.chars().rev().collect(); // LSB first
        let words = bits.len().div_ceil(32);
        let mut out = String::with_capacity(words * 4);
        for byte_idx in 0..words * 4 {
            let mut byte = 0u8;
            for bit in 0..8 {
                if bits.get(byte_idx * 8 + bit) == Some(&'1') {
                    byte |= 1 << bit;
                }
            }
            out.push(byte as char);
        }
        out
    }

    /// §21.2.1.4 `%z`: the 4-state data as raw little-endian bytes — for each
    /// 32-bit word, the bval (x/z mask) word then the aval word. VPI encoding:
    /// 0=(a0,b0), 1=(a1,b0), z=(a0,b1), x=(a1,b1).
    pub(super) fn unformatted_4state(bin: &str) -> String {
        let bits: Vec<char> = bin.chars().rev().collect(); // LSB first
        let words = bits.len().div_ceil(32);
        let mut out = String::with_capacity(words * 8);
        for w in 0..words {
            let word_bytes = |aval: bool, out: &mut String| {
                for byte_in_word in 0..4 {
                    let mut byte = 0u8;
                    for bit in 0..8 {
                        let c = bits.get(w * 32 + byte_in_word * 8 + bit);
                        let set = match c {
                            Some('1') => aval,
                            Some('x') | Some('X') => true,
                            Some('z') | Some('Z') => !aval,
                            _ => false,
                        };
                        if set {
                            byte |= 1 << bit;
                        }
                    }
                    out.push(byte as char);
                }
            };
            word_bytes(false, &mut out); // bval word
            word_bytes(true, &mut out); // aval word
        }
        out
    }

    /// §21.2.1.5 `%v`: strength abbreviation + logic value for a scalar net.
    /// The drive strength comes from the continuous assign that drives it
    /// (`assign (pull1, pull0) n = …`); undriven or unrecorded nets display
    /// as strong. A `z` value displays as HiZ regardless of strength.
    pub(super) fn strength_field(&mut self, arg: &Expression, v: &Value) -> String {
        fn abbrev(tok: &str) -> &'static str {
            if tok.starts_with("supply") {
                "Su"
            } else if tok.starts_with("pull") {
                "Pu"
            } else if tok.starts_with("weak") {
                "We"
            } else if tok.starts_with("highz") {
                "Hi"
            } else if tok.starts_with("small") {
                "Sm"
            } else if tok.starts_with("medium") {
                "Me"
            } else if tok.starts_with("large") {
                "La"
            } else {
                "St"
            }
        }
        // §28.12: a net several drivers resolve reports the strength they
        // resolve to (bit 0, like the value letter below).
        if let ExprKind::Ident(h) = &arg.kind
            && (!self.module.strength_drivers.is_empty() || !self.module.supply_nets.is_empty())
        {
            let name = self.resolve_hier_name(h).into_owned();
            if let Some(drivers) = self.module.strength_drivers.get(&name).cloned() {
                let w = v.width.max(1);
                let ds = self.eval_strength_drivers(
                    w,
                    drivers
                        .iter()
                        .map(|d| (d.s0 * 8 + d.s1, d.ctl.as_ref(), &d.value)),
                );
                let (lo, hi) = Self::strength_bit(&ds, 0).unwrap_or((0, 0));
                return Self::strength_text(lo, hi);
            }
            // §6.6.6: a supply net drives at supply strength.
            if self.module.supply_nets.contains(&name) {
                return match v.get_bit(0) {
                    LogicBit::Zero => "Su0".to_string(),
                    LogicBit::One => "Su1".to_string(),
                    LogicBit::Z => "HiZ".to_string(),
                    LogicBit::X => "SuX".to_string(),
                };
            }
        }
        // §6.6.3: an undriven tri0/tri1 bit is pulled.
        if let ExprKind::Ident(h) = &arg.kind
            && !self.module.resolved_net_kinds.is_empty()
        {
            let name = self.resolve_hier_name(h).into_owned();
            match (self.module.resolved_net_kinds.get(&name), v.get_bit(0)) {
                (Some(super::elaborate::ResolvedNetKind::Tri0), LogicBit::Zero) => {
                    return "Pu0".to_string();
                }
                (Some(super::elaborate::ResolvedNetKind::Tri1), LogicBit::One) => {
                    return "Pu1".to_string();
                }
                _ => {}
            }
        }
        let strengths: Option<(String, String)> = if let ExprKind::Ident(h) = &arg.kind {
            let name = self.resolve_hier_name(h);
            self.module
                .net_strengths
                .get(&*name)
                .or_else(|| {
                    h.path
                        .last()
                        .and_then(|s| self.module.net_strengths.get(&s.name.name))
                })
                .cloned()
        } else {
            None
        };
        let (s1, s0) = strengths.unwrap_or_default();
        let bit = v.to_bin_string().chars().last().unwrap_or('x');
        match bit {
            '1' => format!("{}1", abbrev(&s1)),
            '0' => format!("{}0", abbrev(&s0)),
            'z' | 'Z' => "HiZ".to_string(),
            _ => format!("{}X", abbrev(&s1)),
        }
    }

    /// §28.12: evaluate a net's strength drivers — `(s0, s1, on, value)`
    /// each, `on` the tristate control's truth (`None` when unknown), the
    /// value at the net's width `w` so an unsized fill (`'1`) covers it.
    pub(super) fn eval_strength_drivers<'d>(
        &mut self,
        w: u32,
        drivers: impl Iterator<Item = (u8, Option<&'d Expression>, &'d Expression)>,
    ) -> Vec<(i8, i8, Option<bool>, Value)> {
        let mut out = Vec::new();
        for (code, ctl, value) in drivers {
            let on = match ctl {
                None => Some(true),
                Some(c) => {
                    let cv = self.eval_expr(c);
                    let bits = (0..cv.width.max(1) as usize).map(|k| cv.get_bit(k));
                    if bits.clone().any(|b| b == LogicBit::One) {
                        Some(true)
                    } else if bits.into_iter().all(|b| b == LogicBit::Zero) {
                        Some(false)
                    } else {
                        None
                    }
                }
            };
            if on == Some(false) {
                continue;
            }
            let v = self.eval_expr_ctx(value, w);
            out.push(((code / 8) as i8, (code % 8) as i8, on, v));
        }
        out
    }

    /// §28.12: the strength range bit `i` resolves to from evaluated
    /// drivers, on the scale -7 (Su0) .. 0 (HiZ) .. 7 (Su1): `(lo, hi)`, a
    /// single level for an unambiguous value, a span for an ambiguous one
    /// (`36X` is -3..6). A driver contributes its level for a 0 or 1, the
    /// span of both its levels for an x, nothing for a z, and its value-or-z
    /// (`L`/`H`) when its tristate control is unknown. Drivers combine by
    /// §28.12.2 (see `combine_strength`). `None` when nothing drives it.
    pub(super) fn strength_bit(ds: &[(i8, i8, Option<bool>, Value)], i: usize) -> Option<(i8, i8)> {
        let mut acc: Option<(i8, i8)> = None;
        for (s0, s1, on, v) in ds {
            let b = if i < v.width as usize {
                v.get_bit(i)
            } else {
                LogicBit::Zero
            };
            let mut r = match b {
                LogicBit::Zero => (-*s0, -*s0),
                LogicBit::One => (*s1, *s1),
                LogicBit::X => (-*s0, *s1),
                LogicBit::Z => continue,
            };
            if on.is_none() {
                r = (r.0.min(0), r.1.max(0));
            }
            if r == (0, 0) {
                continue;
            }
            acc = Some(match acc {
                None => r,
                Some(a) => Self::combine_strength(a, r),
            });
        }
        acc
    }

    /// The logic value of a resolved strength range.
    pub(super) fn strength_value_bit(r: Option<(i8, i8)>) -> LogicBit {
        match r {
            None | Some((0, 0)) => LogicBit::Z,
            Some((lo, _)) if lo > 0 => LogicBit::One,
            Some((_, hi)) if hi < 0 => LogicBit::Zero,
            _ => LogicBit::X,
        }
    }

    /// §28.12.2: combine two strength ranges point by point.
    pub(super) fn combine_strength(a: (i8, i8), b: (i8, i8)) -> (i8, i8) {
        let (mut lo, mut hi) = (i8::MAX, i8::MIN);
        for p in a.0..=a.1 {
            for q in b.0..=b.1 {
                let outs: [i8; 2] = if p.abs() > q.abs() {
                    [p, p]
                } else if q.abs() > p.abs() {
                    [q, q]
                } else {
                    [p, q]
                };
                for o in outs {
                    lo = lo.min(o);
                    hi = hi.max(o);
                }
            }
        }
        (lo, hi)
    }

    /// §21.2.1.5: the `%v` text of strength range `(lo, hi)`.
    pub(super) fn strength_text(lo: i8, hi: i8) -> String {
        const NAMES: [&str; 8] = ["Hi", "Sm", "Me", "We", "La", "Pu", "St", "Su"];
        let n = |l: i8| NAMES[l.unsigned_abs().min(7) as usize];
        match (lo, hi) {
            (0, 0) => "HiZ".to_string(),
            (l, h) if l == h && l < 0 => format!("{}0", n(l)),
            (l, h) if l == h => format!("{}1", n(h)),
            (l, h) if l < 0 && h > 0 && -l == h => format!("{}X", n(h)),
            (l, h) if l < 0 && h > 0 => format!("{}{}X", -l, h),
            (l, 0) => format!("{}L", n(l)),
            (0, h) => format!("{}H", n(h)),
            // A one-valued span names its stronger level first (`651`).
            (l, h) if h < 0 => format!("{}{}0", -l, -h),
            (l, h) => format!("{}{}1", h, l),
        }
    }

    /// Re-resolve `prop`'s packed width against the class parameters bound
    /// on the instance `handle` points at.
    ///
    /// Elaboration records ONE width per class — the width under the class's
    /// default parameter values — because `ElaboratedClass` is keyed by
    /// base-class name. For `class box #(parameter int W = 8); bit [W-1:0]
    /// data;` that made `box#(16)` report 8 bits and truncate every store
    /// through `fit_class_prop`. Class value parameters are bound as
    /// INSTANCE PROPERTIES at construction (§8.25, see
    /// `instantiate_class_with_type_args`), so the instance already carries
    /// this specialization's `W` — read the width back off the retained
    /// declared type using those bindings.
    ///
    /// Deliberately narrow: only packed integer-vector declarations whose
    /// range is not already all-literal are re-resolved. Typedef'd, struct,
    /// enum and union properties would need the elaboration-time typedef
    /// table, which is not reachable here, so they keep the elaborated width
    /// rather than risk a wrong one.
    pub(super) fn respec_packed_width(
        &self,
        decl_class: &str,
        handle: usize,
        prop: &str,
    ) -> Option<u32> {
        // Fast reject for the overwhelmingly common case (no dimensions, or a
        // literal range that already elaborated correctly): a borrowed lookup.
        let known = self
            .spec_prop_is_dyn
            .borrow()
            .get(decl_class)
            .and_then(|m| m.get(prop))
            .copied();
        if known == Some(false) {
            return None;
        }
        let note = |v: bool| {
            self.spec_prop_is_dyn
                .borrow_mut()
                .entry(decl_class.to_string())
                .or_default()
                .insert(prop.to_string(), v);
        };
        let cd = self.module.classes.get(decl_class)?;
        let dt = self.class_prop_decl_type(cd, prop);
        let dims = match dt {
            Some(DataType::IntegerVector { dimensions, .. }) if !dimensions.is_empty() => {
                dimensions
            }
            Some(DataType::Implicit { dimensions, .. }) if !dimensions.is_empty() => dimensions,
            _ => {
                note(false);
                return None;
            }
        };
        // Classify once: const-eval'ing the range on every store would defeat
        // the point of caching the width.
        let is_dyn = match known {
            Some(v) => v,
            None => {
                let v = dims.iter().any(|d| {
                    matches!(d, crate::ast::types::PackedDimension::Range { left, right, .. }
                        if crate::elaborate::const_eval_i64_with_params(left, None).is_none()
                            || crate::elaborate::const_eval_i64_with_params(right, None).is_none())
                });
                note(v);
                v
            }
        };
        if !is_dyn {
            return None;
        }
        let dt = dt?;
        let inst = self.heap.get(handle)?.as_ref()?;
        // Gather the parameter bindings this instance carries, leaf class
        // first so a derived class's parameter shadows an ancestor's.
        let mut params: HashMap<String, Value> = HashMap::default();
        let mut fingerprint = String::new();
        let mut cur: Option<&str> = Some(decl_class);
        let mut seen: HashSet<String> = HashSet::default();
        while let Some(cn) = cur {
            if !seen.insert(cn.to_string()) {
                break;
            }
            let Some(c) = self.module.classes.get(cn) else {
                break;
            };
            for (pname, _) in &c.param_defaults {
                if params.contains_key(pname) {
                    continue;
                }
                if let Some(v) = inst.properties.get(pname) {
                    fingerprint.push_str(pname);
                    fingerprint.push('=');
                    fingerprint.push_str(&v.to_dec_string());
                    fingerprint.push(';');
                    params.insert(pname.clone(), v.clone());
                }
            }
            cur = c.extends.as_deref();
        }
        if params.is_empty() {
            return None;
        }
        let key = (decl_class.to_string(), prop.to_string(), fingerprint);
        if let Some(hit) = self.spec_prop_width_cache.borrow().get(&key) {
            return *hit;
        }
        // The instance carries only the CLASS parameters. A range that also
        // references an enclosing-scope parameter (`bit [W+MW-1:0]`) cannot be
        // fully evaluated here, and `resolve_type_width` SKIPS a dimension it
        // cannot evaluate rather than failing — which would silently hand back
        // 1 and undo the elaboration-time width. Only override when every
        // range bound actually resolves against these bindings.
        let all_resolved = dims.iter().all(|d| match d {
            crate::ast::types::PackedDimension::Range { left, right, .. } => {
                crate::elaborate::const_eval_i64_with_params(left, Some(&params)).is_some()
                    && crate::elaborate::const_eval_i64_with_params(right, Some(&params)).is_some()
            }
            _ => true,
        });
        let out = if all_resolved {
            let w = crate::elaborate::resolve_type_width(dt, Some(&params), None);
            Some(w).filter(|w| *w > 0)
        } else {
            None
        };
        self.spec_prop_width_cache.borrow_mut().insert(key, out);
        out
    }

    pub(super) fn class_prop_width(&self, class_name: &str, prop: &str) -> Option<u32> {
        self.class_prop_width_impl(class_name, None, prop)
    }

    /// Declared width of `prop`, walking the `extends` chain to the class that
    /// declares it. When `handle` is given, a property whose packed range
    /// depends on a class parameter is re-resolved against THAT instance's
    /// parameter bindings — see `respec_packed_width`.
    pub(super) fn class_prop_width_impl(
        &self,
        class_name: &str,
        handle: Option<usize>,
        prop: &str,
    ) -> Option<u32> {
        // §8.14: a hidden base property has a qualified storage key.
        let storage_prop = prop;
        let (class_name, prop) = prop.rsplit_once("::").unwrap_or((class_name, prop));
        let memo = self
            .prop_width_memo
            .borrow()
            .get(class_name)
            .and_then(|m| m.get(prop))
            .copied();
        let fixed = match memo {
            Some(v) => v,
            None => {
                let v = self.class_prop_width_fixed(class_name, prop);
                self.prop_width_memo
                    .borrow_mut()
                    .entry(class_name.to_string())
                    .or_default()
                    .insert(prop.to_string(), v);
                v
            }
        };
        if let Some(w) = fixed {
            return w;
        }
        let owners = self.prop_owners(class_name);
        {
            let (cn, cd) = owners.get(prop)?;
            let cn = cn.as_str();
            if let Some(sig) = cd.properties.get(prop) {
                if sig.is_real
                    || sig
                        .type_name
                        .as_ref()
                        .is_some_and(|t| self.module.classes.contains_key(t))
                {
                    return None;
                }
                if cd.string_properties.contains(prop) {
                    return None;
                }
                // A property whose type is a TYPE PARAMETER (`class C#(type T);
                // T value;`) has no width of its own — the width is whatever
                // the parameter is BOUND to on this object. Resolve the binding
                // and clamp to the concrete type; `concrete_type_width` returns
                // None for the cases that must stay unclamped (a `string`,
                // whose leading chars truncation would drop; a class handle; a
                // real; an unresolvable binding), preserving the previous
                // behavior exactly for those.
                //
                // Without a handle there is no instance to read the binding
                // from, so the static path keeps the old conservative answer.
                if sig
                    .type_name
                    .as_ref()
                    .is_some_and(|t| cd.type_param_names.contains(t))
                {
                    let h = handle?;
                    let concrete = self.class_prop_type_name(h, storage_prop)?;
                    let w = self.concrete_type_width(&concrete)?;
                    // §7.4.1: `T [1:0] k;` is a packed array of the bound type.
                    if let Some(DataType::TypeReference { dimensions, .. }) =
                        cd.property_types.get(prop)
                        && !dimensions.is_empty()
                    {
                        let params = self.instance_param_scope(h);
                        let mut n: u64 = 1;
                        for d in dimensions {
                            let crate::ast::types::PackedDimension::Range { left, right, .. } = d
                            else {
                                return Some(w);
                            };
                            let (Some(l), Some(r)) = (
                                crate::elaborate::const_eval_i64_with_params(left, Some(&params)),
                                crate::elaborate::const_eval_i64_with_params(right, Some(&params)),
                            ) else {
                                return Some(w);
                            };
                            n = n.saturating_mul((l - r).unsigned_abs() + 1);
                        }
                        return u32::try_from(u64::from(w).saturating_mul(n)).ok();
                    }
                    return Some(w);
                }
                if let Some(h) = handle {
                    // §8.25/§7.2: a packed struct or enum whose members name
                    // a type parameter is sized by this object's bindings.
                    if Self::agg_names_type_param(cd, prop)
                        && let Some(dt) = cd.property_types.get(prop)
                        && let Some(b) =
                            self.bind_type_params_in(dt, &|n| self.obj_type_param(h, cn, n))
                    {
                        let params = self.instance_param_scope(h);
                        let w = resolve_type_width(&b, Some(&params), Some(&self.module.typedefs));
                        return Some(w).filter(|w| *w > 0);
                    }
                    if let Some(w) = self.respec_packed_width(&cn, h, prop) {
                        return Some(w);
                    }
                }
                return Some(sig.width).filter(|w| *w > 0);
            }
        }
        None
    }

    /// `class_prop_width_impl`'s answer when it cannot depend on the instance
    /// (the class tables are fixed at run time): `Some(answer)`, or `None`
    /// for a type-parameter-typed property or a packed range sized by a
    /// class parameter.
    pub(super) fn class_prop_width_fixed(
        &self,
        class_name: &str,
        prop: &str,
    ) -> Option<Option<u32>> {
        let owners = self.prop_owners(class_name);
        let Some((cn, cd)) = owners.get(prop) else {
            return Some(None);
        };
        let Some(sig) = cd.properties.get(prop) else {
            return Some(None);
        };
        if sig.is_real
            || sig
                .type_name
                .as_ref()
                .is_some_and(|t| self.module.classes.contains_key(t))
            || cd.string_properties.contains(prop)
        {
            return Some(None);
        }
        if sig
            .type_name
            .as_ref()
            .is_some_and(|t| cd.type_param_names.contains(t))
            || Self::agg_names_type_param(cd, prop)
        {
            return None;
        }
        // `respec_packed_width` answers only for a range that is not constant
        // on its own (its classification, without the instance).
        let respec = self.module.classes.get(cn.as_str()).is_none_or(|c| {
            let dims = match self.class_prop_decl_type(c, prop) {
                Some(DataType::IntegerVector { dimensions, .. }) if !dimensions.is_empty() => {
                    dimensions
                }
                Some(DataType::Implicit { dimensions, .. }) if !dimensions.is_empty() => dimensions,
                _ => return false,
            };
            dims.iter().any(|d| {
                matches!(d, crate::ast::types::PackedDimension::Range { left, right, .. }
                    if crate::elaborate::const_eval_i64_with_params(left, None).is_none()
                        || crate::elaborate::const_eval_i64_with_params(right, None).is_none())
            })
        });
        if respec {
            return None;
        }
        Some(Some(sig.width).filter(|w| *w > 0))
    }

    /// Where each property of `class_name`'s `extends` chain is declared:
    /// the first class that declares it, as a chain walk finds it (the walk
    /// stops at a class missing from the table). Built once per class; the
    /// class tables are fixed at run time.
    pub(super) fn prop_owners(&self, class_name: &str) -> std::rc::Rc<PropOwners> {
        if let Some(hit) = self.prop_owner_index.borrow().get(class_name) {
            return hit.clone();
        }
        let mut owners: PropOwners = HashMap::default();
        let mut cur: Option<&str> = Some(class_name);
        while let Some(cn) = cur {
            let Some(cd) = self.module.classes.get(cn) else {
                break;
            };
            for p in cd.properties.keys() {
                owners
                    .entry(p.clone())
                    .or_insert_with(|| (cn.to_string(), cd.clone()));
            }
            cur = cd.extends.as_deref();
        }
        let owners = std::rc::Rc::new(owners);
        self.prop_owner_index
            .borrow_mut()
            .insert(class_name.to_string(), owners.clone());
        owners
    }

    /// Declared width of `prop` on the object `handle` points at, resolved
    /// against the parameters bound on that instance so `box#(16)` and
    /// `box#(8)` each report their own width.
    pub(super) fn heap_prop_width(&self, handle: usize, prop: &str) -> Option<u32> {
        let inst = self.heap.get(handle)?.as_ref()?;
        self.class_prop_width_impl(&inst.class_name, Some(handle), prop)
    }

    /// Clamp `val` to a class property's declared width. Values that already
    /// fit are returned unchanged, so nothing is disturbed for the common case.
    /// §8.10: the storage key a reference to `name` must use when the property
    /// is SHADOWED — declared by more than one class in the current object's
    /// chain. Each declaration is its own variable: a base method's `s` names
    /// the base's copy even when the object's leaf class re-declares `s`. The
    /// viewing class is the executing method's DECLARING class (the context
    /// stack); `from_super` starts one class higher, which is what makes
    /// `super.s` skip the leaf's declaration. `None` keeps the bare-name path:
    /// unshadowed names, external accesses, and the leaf-most declarer's own
    /// methods all store under the bare key.
    pub(super) fn shadowed_prop_key(&self, name: &str, from_super: bool) -> Option<String> {
        if self.shadowed_prop_names.is_empty() || !self.shadowed_prop_names.contains(name) {
            return None;
        }
        let ctx = self.class_context_stack.last().cloned().flatten()?;
        let viewing = if from_super {
            self.module.classes.get(&ctx)?.extends.clone()?
        } else {
            ctx
        };
        // Nearest declarer at or above the viewing class.
        let mut cur = Some(viewing);
        let mut target: Option<String> = None;
        while let Some(cn) = cur {
            let cd = self.module.classes.get(&cn)?;
            if cd.properties.contains_key(name) {
                target = Some(cn);
                break;
            }
            cur = cd.extends.clone();
        }
        let target = target?;
        // The leaf-most declarer for the CURRENT object.
        let handle = self.this_stack.last().copied().flatten()?;
        let leaf = self
            .heap
            .get(handle)
            .and_then(|o| o.as_ref())
            .map(|i| i.class_name.clone())?;
        let mut cur = Some(leaf);
        while let Some(cn) = cur {
            let cd = self.module.classes.get(&cn)?;
            if cd.properties.contains_key(name) {
                return if cn == target {
                    None
                } else {
                    Some(format!("{}::{}", target, name))
                };
            }
            cur = cd.extends.clone();
        }
        None
    }

    pub(super) fn fit_class_prop(&self, handle: usize, prop: &str, val: &Value) -> Value {
        // §10.7: every integral store takes the destination's signedness,
        // including stores whose width already matches the declaration.
        if val.is_real {
            return val.clone();
        }
        let fitted = match self.heap_prop_width(handle, prop) {
            Some(w) => {
                let mut fitted = if w != val.width {
                    val.resize_for_assign(w)
                } else {
                    val.clone()
                };
                fitted.is_signed = self.class_prop_signed_of(handle, prop);
                fitted
            }
            None => val.clone(),
        };
        // §6.11.1/§10.7: a 2-state property drops X/Z on every write.
        if fitted.has_xz() && self.class_prop_two_state(handle, prop) {
            return fitted.to_two_state();
        }
        fitted
    }

    /// Whether property `prop` of the object `handle` is declared with a
    /// 2-state type (`bit`, `int`, a typedef of one, ...).
    pub(super) fn class_prop_two_state(&self, handle: usize, prop: &str) -> bool {
        let Some(Some(inst)) = self.heap.get(handle) else {
            return false;
        };
        let (start, prop) = prop
            .rsplit_once("::")
            .unwrap_or((inst.class_name.as_str(), prop));
        let mut cur = Some(start);
        while let Some(cn) = cur {
            let Some(cd) = self.module.classes.get(cn) else {
                return false;
            };
            if let Some(dt) = cd.property_types.get(prop) {
                // A class-local typedef first, then the design's.
                let dt = match dt {
                    DataType::TypeReference { name, .. } => {
                        cd.typedef_targets.get(&name.name.name).unwrap_or(dt)
                    }
                    _ => dt,
                };
                // §8.25: the state domain of a type parameter belongs to
                // this object, including when another specialization calls it.
                let bound =
                    self.bind_type_params_in(dt, &|name| self.obj_type_param(handle, cn, name));
                return super::elaborate::is_type_two_state_resolved(
                    bound.as_ref().unwrap_or(dt),
                    &self.module.typedef_types,
                );
            }
            if cd.properties.contains_key(prop) {
                return false;
            }
            cur = cd.extends.as_deref();
        }
        false
    }

    /// §8.4: `var.prop` where `var` is a DECLARED class handle currently
    /// null and `prop` is an instance property of that class — a runtime
    /// fatal. Emits the error and terminates; returns true when it fired.
    /// A STATIC property (readable through null) never reaches here — the
    /// static route runs first at every call site.
    pub(super) fn null_deref_fatal(&mut self, vname: &str, prop: &str) -> bool {
        let Some(cn) = self.class_of_var(vname) else {
            return false;
        };
        let base = cn.split('#').next().unwrap_or(&cn).to_string();
        let mut cur = Some(base);
        let mut declares = false;
        let mut guard = 0;
        while let Some(c) = cur {
            guard += 1;
            if guard > 64 {
                break;
            }
            let Some(cd) = self.module.classes.get(&c) else {
                break;
            };
            if cd.properties.contains_key(prop) && !cd.static_properties.contains(prop) {
                declares = true;
                break;
            }
            cur = cd.extends.clone();
        }
        if !declares {
            return false;
        }
        // Inside a class method (deep library code), report ONCE and keep
        // going — an emulation gap can leave a handle null where the
        // reference had one, and terminating there kills whole UVM runs.
        // In module/TB context the deref is the user's bug: fatal.
        let in_method = self.this_stack.last().copied().flatten().is_some()
            || self
                .class_context_stack
                .last()
                .cloned()
                .flatten()
                .is_some()
            // any subroutine frame (package/module function or task) — the
            // fatal is reserved for a deref directly in a process body
            || !self.local_stack.is_empty();
        let key = format!("{}.{}", vname, prop);
        if in_method {
            if self.reported_null_derefs.insert(key.clone()) {
                let m = format!(
                    "[xezim][error] null object dereference: '{}' read through \
                     a null handle (t={})",
                    key, self.time
                );
                self.record_output(m.clone());
                self.stdout_writeln(&m);
            }
            return false;
        }
        let m = format!(
            "[xezim][fatal] null object dereference: '{}' read through a \
             null handle (t={})",
            key, self.time
        );
        self.record_output(m.clone());
        self.stdout_writeln(&m);
        self.abort_run();
        true
    }

    /// See `local_type_stack`: record a frame-local's declared CLASS type
    /// in the innermost frame overlay (falls back to the global map at
    /// module scope).
    pub(super) fn record_local_class_type(&mut self, name: &str, cls: &str) {
        if let Some((c, _)) = self.local_type_stack.last_mut() {
            match c.get_mut(name) {
                Some(v) if v.as_str() == cls => {}
                Some(v) => {
                    v.clear();
                    v.push_str(cls);
                }
                None => {
                    c.insert(name.to_string(), cls.to_string());
                }
            }
        }
    }

    /// See `local_type_stack`: typedef/enum flavor.
    pub(super) fn record_local_typedef_type(&mut self, name: &str, tn: &str) {
        if let Some((_, t)) = self.local_type_stack.last_mut() {
            match t.get_mut(name) {
                Some(v) if v.as_str() == tn => {}
                Some(v) => {
                    v.clear();
                    v.push_str(tn);
                }
                None => {
                    t.insert(name.to_string(), tn.to_string());
                }
            }
        }
    }
}
