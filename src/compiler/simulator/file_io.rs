//! Simulator file io.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

/// File input advances only as far as the current directive requires. A
/// nonmatching byte stays available to subsequent scans and other file reads.
enum ScanInput<'a> {
    Text(std::iter::Peekable<std::str::Chars<'a>>),
    File(i32),
}

impl ScanInput<'_> {
    fn take_if(&mut self, sim: &mut Simulator, accept: impl FnOnce(char) -> bool) -> Option<char> {
        match self {
            Self::Text(chars) => chars.next_if(|&c| accept(c)),
            Self::File(fd) => {
                let mut byte = [0];
                if sim.fread_bytes(*fd, &mut byte) == 0 {
                    return None;
                }
                let c = char::from(byte[0]);
                if accept(c) {
                    Some(c)
                } else {
                    sim.ungetc_buf.entry(*fd).or_default().push(byte[0]);
                    None
                }
            }
        }
    }

    fn skip_whitespace(&mut self, sim: &mut Simulator) {
        while self.take_if(sim, char::is_whitespace).is_some() {}
    }

    fn token(
        &mut self,
        sim: &mut Simulator,
        width: usize,
        accept: impl Fn(char) -> bool,
    ) -> String {
        (0..width)
            .map_while(|_| self.take_if(sim, &accept))
            .collect()
    }
}

/// The memory a `$readmem*` / `$writemem*` call names (§21.4, §21.5).
pub(super) struct MemoryTaskTarget {
    name: String,
    /// An unpacked memory's declared `(left, right)` address bounds; for an
    /// associative array, the least and greatest address its index type
    /// holds. Wide enough for every address of a 64-bit index type, signed
    /// or unsigned; for an unsigned index type of 128 bits, in `bias` order.
    lo: i128,
    hi: i128,
    /// Element width.
    width: u32,
    /// An associative array with an integral index type, its elements keyed
    /// by address.
    assoc: bool,
    /// The index type's width (at most 128) and signedness, for an
    /// associative array.
    key: (u32, bool),
    /// 2^127 for an unsigned index type of 128 bits, whose keys run past
    /// `i128::MAX`, else 0. The bounds and the addresses a load or a dump
    /// steps through are an address's bits flipped by it ([`key_order`]),
    /// which puts the keys in `i128` order.
    bias: u128,
}

/// An address's bits, as `i128`, to the order of a [`MemoryTaskTarget`]
/// with `bias`, and back: an unsigned 128-bit key `k` is kept as
/// `k - 2^127`, so 0 is `i128::MIN` and `u128::MAX` is `i128::MAX`.
fn key_order(addr: i128, bias: u128) -> i128 {
    (addr as u128 ^ bias) as i128
}

impl Simulator {
    /// Whether `$display`/`$write` output is handed to a background writer.
    ///
    /// Default ON regardless of host parallelism settings: writing the log
    /// is pure I/O, so even a single-threaded simulation benefits from moving
    /// the `write`/`flush` syscalls off the thread that is running the design.
    /// `XEZIM_DUMP_INLINE=1` (the same switch the waveform sinks honour) keeps
    /// it on the simulation thread, which is what you want when interleaving
    /// with stderr has to be exact — the worker writes stdout while diagnostics
    /// still go straight to stderr, so a combined `--log` file can order two
    /// adjacent lines differently. Every `$display` still flushes on its
    /// newline, so the skew is bounded to one line and never reorders stdout
    /// against itself.
    pub(super) fn stdout_threaded(&self) -> bool {
        std::env::var_os("XEZIM_DUMP_INLINE").is_none()
    }

    #[inline]
    pub(super) fn stdout_write(&mut self, s: &str) {
        if self.checkpoint_muted {
            return;
        }
        self.stdout_unsynced = true;
        let threaded = self.stdout_threaded();
        let sink = self.stdout_sink.get_or_insert_with(|| {
            if threaded {
                super::stdout_sink::StdoutSink::threaded()
            } else {
                super::stdout_sink::StdoutSink::inline()
            }
        });
        sink.write_str(s);
    }

    #[inline]
    pub(super) fn stdout_writeln(&mut self, s: &str) {
        if self.checkpoint_muted {
            return;
        }
        self.stdout_unsynced = true;
        let threaded = self.stdout_threaded();
        let sink = self.stdout_sink.get_or_insert_with(|| {
            if threaded {
                super::stdout_sink::StdoutSink::threaded()
            } else {
                super::stdout_sink::StdoutSink::inline()
            }
        });
        sink.writeln_str(s);
    }

    /// Flush `$display`/`$write` output and WAIT for it to reach the file, so
    /// anything the caller writes next is ordered after it.
    pub fn flush_stdout(&mut self) {
        if let Some(s) = self.stdout_sink.as_mut() {
            s.sync();
        }
    }

    #[inline]
    pub(super) fn record_output(&mut self, message: String) {
        if self.capture_output {
            self.output.push(SimOutput {
                time: self.time,
                message,
            });
        }
    }

    #[inline]
    pub(super) fn plusarg_payload(arg: &str) -> &str {
        arg.strip_prefix('+').unwrap_or(arg)
    }

    pub(super) fn test_plusarg(&self, pattern: &str) -> bool {
        if pattern.is_empty() {
            return false;
        }
        let hit = self
            .plusargs
            .iter()
            .any(|a| Self::plusarg_payload(a).starts_with(pattern));
        sim_dbg_eprintln!("[DEBUG] $test$plusargs('{}') -> {}", pattern, hit);
        hit
    }

    pub(super) fn parse_plusarg_format(fmt: &str) -> Option<(&str, char)> {
        let pct = fmt.find('%')?;
        let prefix = &fmt[..pct];
        let mut chars = fmt[pct + 1..].chars().peekable();
        while let Some(c) = chars.peek() {
            if c.is_ascii_digit() || *c == '-' || *c == '+' || *c == '0' || *c == '.' {
                chars.next();
            } else {
                break;
            }
        }
        let spec = chars.next()?;
        Some((prefix, spec.to_ascii_lowercase()))
    }

    /// Two's complement negation at the value's own width: `-x` is `!x + 1`,
    /// so `-1` read into a 160-bit destination is 2^160-1. Callers build the
    /// operand at the destination width first, which is what makes the result
    /// the destination's `-x` rather than a truncated one.
    ///
    /// Neither `Value::negate` nor the `!x + 1` spelling can do this. `negate`
    /// funnels the operand through `to_u64`, and `to_u64` on `Wide` storage
    /// answers with the low word alone; `add` reduces both `Wide` operands
    /// through `u128`, so the carry dies at bit 127 and bits 128 and up come
    /// back 0 — `-1` into 160 bits came out as 2^128-1. `bitwise_not` does
    /// cover the full width, so only the increment has to be walked by hand.
    pub(super) fn negate_twos_complement(v: Value) -> Value {
        // Unknown bits poison an arithmetic result the way `add` and `negate`
        // already answer them.
        if v.has_xz() {
            let mut x = Value::new(v.width);
            x.is_signed = true;
            return x;
        }
        let mut n = v.bitwise_not();
        let w = n.width;
        let mut i = 0u32;
        let mut carry = true;
        while carry && i < w {
            if n.get_bit(i as usize) == LogicBit::Zero {
                n.set_bit(i as usize, LogicBit::One);
                carry = false;
            } else {
                // 1 + 1 wraps to 0 and keeps carrying.
                n.set_bit(i as usize, LogicBit::Zero);
            }
            i += 1;
        }
        n.is_signed = true;
        n
    }

    pub(super) fn parse_plusarg_value(raw: &str, spec: char, width: u32) -> Option<Value> {
        let cleaned: String = raw.chars().filter(|c| *c != '_').collect();
        let w = width.max(1);
        match spec {
            'd' => {
                // Sign first, magnitude after — the split `$sscanf` makes below.
                // The destination decides the two's complement, not the text:
                // reading the magnitude and dropping the `-` here would hand
                // back the same bits for `-1` and `1`.
                let (negative, digits) = match cleaned.strip_prefix('-') {
                    Some(rest) => (true, rest),
                    None => (false, cleaned.strip_prefix('+').unwrap_or(&cleaned)),
                };
                // A payload that isn't a number is no match, and the caller has
                // to move on to the next plusarg. Accumulating while skipping
                // non-digits summed `+N=abc` down to zero, so the register got
                // 0 and $value$plusargs reported a successful match.
                if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                // One decimal parser for every width; core folds digits past
                // u64 into limbs and truncates to `w`.
                let mut v = Value::from_str_radix(digits, 10, w);
                if negative {
                    v = Self::negate_twos_complement(v);
                } else {
                    v.is_signed = true;
                }
                Some(v)
            }
            'h' | 'x' => {
                let s = cleaned
                    .strip_prefix("0x")
                    .or_else(|| cleaned.strip_prefix("0X"))
                    .unwrap_or(&cleaned);
                Some(Value::from_str_radix(s, 16, w))
            }
            'o' => Some(Value::from_str_radix(&cleaned, 8, w)),
            'b' => Some(Value::from_str_radix(&cleaned, 2, w)),
            's' => Some(Value::from_string(raw)),
            'f' | 'e' | 'g' => cleaned.parse::<f64>().ok().map(Value::from_f64),
            _ => None,
        }
    }

    pub(super) fn eval_value_plusargs(&mut self, args: &[Expression]) -> Value {
        if args.len() < 2 {
            return Value::zero(32);
        }
        let fmt = match &args[0].kind {
            ExprKind::StringLiteral(s) => s.clone(),
            _ => self.eval_expr(&args[0]).to_sv_string(),
        };
        let Some((prefix, spec)) = Self::parse_plusarg_format(&fmt) else {
            return Value::zero(32);
        };
        // The destination's width, so a 160-bit field keeps all 160 bits of
        // the token instead of whatever survives a trip through i64.
        let dest_width = self.infer_lhs_width(&args[1]).max(1);

        for arg in &self.plusargs {
            let payload = Self::plusarg_payload(arg);
            if !payload.starts_with(prefix) {
                continue;
            }
            let suffix = &payload[prefix.len()..];
            if let Some(v) = Self::parse_plusarg_value(suffix, spec, dest_width) {
                self.assign_value(&args[1], &v);
                return Value::from_u64(1, 32);
            }
        }
        Value::zero(32)
    }

    pub(super) fn system_string_arg(&mut self, expr: &Expression) -> String {
        match &expr.kind {
            ExprKind::StringLiteral(s) => s.clone(),
            _ => self.eval_expr(expr).to_sv_string(),
        }
    }

    /// Read one line (through '\n', included) from `fd`, honoring the
    /// ungetc pushback buffer. None at EOF.
    pub(super) fn sv_read_line(&mut self, fd: i32) -> Option<String> {
        use std::io::Read;
        let mut out = String::new();
        loop {
            let mut got: Option<u8> = None;
            if let Some(buf) = self.ungetc_buf.get_mut(&fd) {
                if let Some(c) = buf.pop() {
                    got = Some(c);
                }
            }
            if got.is_none() {
                if let Some(f) = self.file_handles.get_mut(&fd) {
                    let mut b = [0u8; 1];
                    if f.read(&mut b).unwrap_or(0) == 1 {
                        got = Some(b[0]);
                    } else {
                        self.file_eof.insert(fd);
                    }
                }
            }
            match got {
                Some(c) => {
                    out.push(c as char);
                    if c == b'\n' {
                        break;
                    }
                }
                None => break,
            }
        }
        if out.is_empty() { None } else { Some(out) }
    }

    /// §21.3.4 formatted input: match `fmt` against `src`, assigning matched
    /// conversions to `outs` in order (C-scanf semantics — stops at the
    /// first mismatch). Returns the number of conversions assigned.
    pub(super) fn sv_sscanf(&mut self, src: &str, fmt: &str, outs: &[Expression]) -> i64 {
        self.scan_format(&mut ScanInput::Text(src.chars().peekable()), fmt, outs)
    }

    fn scan_format(&mut self, input: &mut ScanInput<'_>, fmt: &str, outs: &[Expression]) -> i64 {
        let mut assigned = 0usize;
        let mut completed_conversion = false;
        let mut fi = fmt.chars().peekable();
        while let Some(fc) = fi.next() {
            if fc == '%' {
                let suppress = if fi.peek() == Some(&'*') {
                    fi.next();
                    true
                } else {
                    false
                };
                let mut width = String::new();
                while fi.peek().is_some_and(|c| c.is_ascii_digit()) {
                    width.push(fi.next().unwrap());
                }
                // §21.3.4.3 leaves a maximum field width of 0 undefined; treat 0
                // as "no maximum", matching the reference simulator and the
                // display-side meaning of `%0d` (§21.2.1.3).
                let w: usize = width.parse().ok().filter(|&n| n != 0).unwrap_or(usize::MAX);
                let Some(spec) = fi.next() else { break };
                if spec != 'c' {
                    input.skip_whitespace(self);
                }
                if spec == '%' {
                    if input.take_if(self, |c| c == '%').is_some() {
                        continue;
                    }
                    break;
                }
                let value = match spec {
                    'c' => {
                        let Some(c) = input.take_if(self, |_| true) else {
                            break;
                        };
                        Value::from_u64(c as u64, 8)
                    }
                    's' => {
                        let text = input.token(self, w, |c| !c.is_whitespace());
                        if text.is_empty() {
                            break;
                        }
                        Value::from_string(&text)
                    }
                    'f' | 'e' | 'g' => {
                        let text =
                            input.token(self, w, |c| c.is_ascii_digit() || "+-.eE".contains(c));
                        let Ok(value) = text.parse::<f64>() else {
                            break;
                        };
                        Value::from_f64(value)
                    }
                    'd' | 'i' | 'h' | 'x' | 'X' | 'o' | 'b' => {
                        let radix: u32 = match spec {
                            'h' | 'x' | 'X' => 16,
                            'o' => 8,
                            'b' => 2,
                            _ => 10,
                        };
                        let signed = matches!(spec, 'd' | 'i');
                        let sign = if signed {
                            input.take_if(self, |c| matches!(c, '-' | '+'))
                        } else {
                            None
                        };
                        let remaining = w - usize::from(sign.is_some());
                        let negative = sign == Some('-');
                        let is_four_state = |c| matches!(c, 'x' | 'X' | 'z' | 'Z' | '?');
                        // §21.3.4.3: a decimal x/z represents the entire value.
                        let mut text = if signed && remaining > 0
                            && let Some(digit) = input.take_if(self, is_four_state)
                        {
                            digit.to_string()
                        } else {
                            input.token(self, remaining, |c| {
                                c.is_digit(radix) || c == '_' || (!signed && is_four_state(c))
                            })
                        };
                        text.retain(|c| c != '_');
                        if text.is_empty() {
                            break;
                        }
                        let dest_width = outs
                            .get(assigned)
                            .filter(|_| !suppress)
                            .map_or(1, |out| self.infer_lhs_width(out).max(1));
                        let mut v = Value::from_str_radix(&text, radix, dest_width);
                        if negative {
                            v = Self::negate_twos_complement(v);
                        } else {
                            v.is_signed = signed;
                        }
                        v
                    }
                    _ => break,
                };
                completed_conversion = true;
                if !suppress && assigned < outs.len() {
                    self.assign_value(&outs[assigned], &value);
                    assigned += 1;
                }
            } else if fc.is_whitespace() {
                input.skip_whitespace(self);
            } else if input.take_if(self, |c| c == fc).is_none() {
                break;
            }
        }
        if let ScanInput::File(fd) = input {
            if !completed_conversion && self.file_eof.contains(fd) {
                return -1;
            }
        }
        assigned as i64
    }

    /// §21.3.4.3: both task and function forms scan the file as a byte stream.
    pub(super) fn fscanf_impl(&mut self, args: &[Expression]) -> i64 {
        let fd = args
            .first()
            .map(|a| self.eval_file_handle_arg(a))
            .unwrap_or(0);
        let fmt = args
            .get(1)
            .map(|a| {
                if let ExprKind::StringLiteral(f) = &a.kind {
                    f.clone()
                } else {
                    self.eval_expr(a).to_sv_string()
                }
            })
            .unwrap_or_default();
        if !self.file_handles.contains_key(&fd) {
            return -1;
        }
        self.scan_format(
            &mut ScanInput::File(fd),
            &fmt,
            args.get(2..).unwrap_or_default(),
        )
    }

    pub(super) fn eval_file_handle_arg(&mut self, expr: &Expression) -> i32 {
        let raw = self.eval_expr(expr).to_i64().unwrap_or(0) as i32;
        self.resolve_channel_key(raw)
    }

    /// Map a user-visible file value (MCD or FD) to an internal `file_handles`
    /// table key. Single-channel operations (read, seek, tell, ungetc, …) all
    /// route through here, so resolving once centralizes the MCD/FD split.
    ///
    /// IEEE 1800-2023 §21.3.1 splits the 32-bit value by bit 31:
    ///   * MCD (multichannel descriptor): bit 31 CLEAR. Each set bit selects a
    ///     channel; bit 0 is always stdout. `$fopen(filename)` returns an MCD.
    ///   * FD  (file descriptor):         bit 31 SET. Lower bits are the index;
    ///     STDIN=0x8000_0000, STDOUT=0x8000_0001, STDERR=0x8000_0002.
    ///     `$fopen(filename, type)` returns an FD.
    ///
    /// Internal keying (disjoint ranges, one HashMap):
    ///   * MCD channel  → key = bit number (1..=30). Bit 0 (stdout) → key 0, a
    ///     sentinel absent from the table so reads/seeks on stdout no-op.
    ///   * FD  channel  → key = the raw FD value (0x8000_0000+), a large negative
    ///     i32 disjoint from the MCD range. Pre-opened STDIN/STDOUT/STDERR are
    ///     never inserted, so they naturally miss.
    pub(super) fn resolve_channel_key(&self, raw: i32) -> i32 {
        let rawu = raw as u32;
        if rawu & 0x8000_0000 != 0 {
            raw // FD: the value itself is the table key
        } else if rawu == 0 {
            0 // $fopen-failure / explicit 0 → stdout sentinel
        } else {
            rawu.trailing_zeros() as i32 // MCD: lowest set bit selects a channel
        }
    }

    /// Resolve an output value (MCD or FD) into the list of internal channel
    /// keys that a write/close/flush must touch. MCD bit 0 (stdout) and FD
    /// STDOUT (0x8000_0001) both surface as the stdout sentinel key 0; FD
    /// STDERR (0x8000_0002) surfaces as its own value.
    pub(super) fn resolve_output_keys(&self, raw: i32) -> Vec<i32> {
        let rawu = raw as u32;
        let mut keys = Vec::new();
        if rawu & 0x8000_0000 != 0 {
            keys.push(raw); // FD: single channel
        } else if rawu == 0 {
            keys.push(0); // legacy: mirror to stdout
        } else {
            for b in 0..31u32 {
                if rawu & (1 << b) != 0 {
                    if b == 0 {
                        keys.push(0); // stdout
                    } else {
                        keys.push(b as i32);
                    }
                }
            }
        }
        keys
    }

    pub(super) fn open_file_handle(&mut self, args: &[Expression]) -> Value {
        if args.is_empty() {
            return Value::zero(32);
        }
        let path = self.system_string_arg(&args[0]);
        if path.is_empty() {
            return Value::zero(32);
        }
        // Buffered output reaches its file before this open can truncate or
        // read it.
        self.flush_file_writes();
        // IEEE 1800 §21.3.1: the presence of the `type` argument selects the
        // addressing mode. No type → multichannel descriptor (MCD, bit 31
        // clear, opened for write). With type → file descriptor (FD, bit 31
        // set), opened per the mode string. A failed open returns 0.
        let mcd_mode = args.len() < 2;
        let mode = if mcd_mode {
            "w".to_string()
        } else {
            self.system_string_arg(&args[1])
        };
        self.open_file_path(&path, mcd_mode, &mode)
    }

    /// `$fopen` on an evaluated path: a multichannel descriptor when
    /// `mcd_mode`, else a file descriptor opened with `mode`. Shared with
    /// `vpi_mcd_open`.
    pub(super) fn open_file_path(&mut self, path: &str, mcd_mode: bool, mode: &str) -> Value {
        self.flush_file_writes();
        let mut opts = OpenOptions::new();
        let has_plus = mode.contains('+');
        if mode.contains('r') {
            opts.read(true);
        }
        if mode.contains('w') {
            opts.write(true).create(true).truncate(true);
        }
        if mode.contains('a') {
            opts.append(true).create(true);
        }
        if has_plus {
            opts.read(true).write(true);
        }
        if !mode.contains('r') && !mode.contains('w') && !mode.contains('a') {
            opts.read(true);
        }
        let mut file = match opts.open(path) {
            Ok(f) => sv_file::SvFile::new(f, path, mode),
            Err(_) => return Value::zero(32),
        };
        // A second handle on an open file: both write through, so their
        // writes land in program order.
        if let Some(ident) = file.ident() {
            for other in self.file_handles.values_mut() {
                if other.ident() == Some(ident) {
                    other.set_write_through();
                    file.set_write_through();
                }
            }
        }
        if mcd_mode {
            // Allocate the lowest free bit in 1..=30 (bit 31 reserved/clear,
            // bit 0 is stdout). Return an MCD with that single bit set.
            let bit = (1u32..=30).find(|&b| !self.file_handles.contains_key(&(b as i32)));
            match bit {
                Some(b) => {
                    self.file_handles.insert(b as i32, file);
                    self.file_eof.remove(&(b as i32));
                    self.mcd_names.insert(b as i32, path.to_string());
                    Value::from_u64(1u64 << b, 32)
                }
                None => Value::zero(32), // too many open MCD channels
            }
        } else {
            // Allocate the lowest free FD index >= 3 (0/1/2 are STDIN/STDOUT/
            // STDERR). The table key is the raw FD value (0x8000_0000 | idx).
            let idx = (3u32..).find(|&i| {
                let key = (0x8000_0000u32 | i) as i32;
                !self.file_handles.contains_key(&key)
            });
            match idx {
                Some(i) => {
                    let key = (0x8000_0000u32 | i) as i32;
                    self.file_handles.insert(key, file);
                    self.file_eof.remove(&key);
                    Value::from_u64(0x8000_0000u64 | i as u64, 32)
                }
                None => Value::zero(32),
            }
        }
    }

    pub(super) fn close_file_handle(&mut self, args: &[Expression]) -> Value {
        if args.is_empty() {
            return Value::zero(32);
        }
        let raw = self.eval_expr(&args[0]).to_i64().unwrap_or(0) as i32;
        self.close_channels(raw);
        Value::zero(32)
    }

    /// Close every file channel `raw` (MCD or FD) selects. Shared with
    /// `vpi_mcd_close`.
    pub(super) fn close_channels(&mut self, raw: i32) {
        for k in self.resolve_output_keys(raw) {
            let ku = k as u32;
            // Never close the pre-opened STDIN/STDOUT/STDERR streams.
            if ku == 0 || ku == 0x8000_0000 || ku == 0x8000_0001 || ku == 0x8000_0002 {
                continue;
            }
            if let Some(mut f) = self.file_handles.remove(&k) {
                let _ = f.flush();
            }
            self.ungetc_buf.remove(&k);
            self.mcd_names.remove(&k);
        }
    }

    pub(super) fn write_file_handle_named(
        &mut self,
        args: &[Expression],
        newline: bool,
        tn: &str,
    ) -> Value {
        if args.is_empty() {
            return Value::zero(32);
        }
        let raw = self.eval_expr(&args[0]).to_i64().unwrap_or(0) as i32;
        let mut payload = if args.len() > 1 {
            self.format_args(&args[1..], tn)
        } else {
            String::new()
        };
        if newline {
            payload.push('\n');
        }
        // The count is the number of BYTES written — in the simulator's
        // byte convention (see `sv_payload_bytes`), not the UTF-8 length
        // of the Rust string.
        let nbytes = Self::sv_payload_bytes(&payload).len() as u64;
        self.write_channels(raw, &payload);
        Value::from_u64(nbytes, 32)
    }

    /// Bytes a `$fwrite`/`$fdisplay` payload writes to a file: the mirror of
    /// `sv_read_line`'s byte→char mapping. Chars up to U+00FF are the
    /// simulator's byte representation — a `%c` of 8'hff is the single byte
    /// 0xff, not its two-byte UTF-8 encoding (issue #262's all-256 round
    /// trip) — so each transcodes back to one byte; higher code points
    /// (text from beyond Latin-1) keep their UTF-8 encoding.
    pub(super) fn sv_payload_bytes(payload: &str) -> Vec<u8> {
        let mut out = Vec::with_capacity(payload.len());
        for ch in payload.chars() {
            if (ch as u32) <= 0xff {
                out.push(ch as u8);
            } else {
                let mut tmp = [0u8; 4];
                out.extend_from_slice(ch.encode_utf8(&mut tmp).as_bytes());
            }
        }
        out
    }

    /// Write `payload` to every channel `raw` (MCD or FD) selects. Shared with
    /// `vpi_mcd_printf`.
    pub(super) fn write_channels(&mut self, raw: i32, payload: &str) {
        for k in self.resolve_output_keys(raw) {
            let ku = k as u32;
            if ku == 0 || ku == 0x8000_0001 {
                // stdout (MCD bit 0 sentinel, or FD STDOUT). Mirror
                // $write/$display: record for the test harness too.
                self.record_output(payload.to_string());
                self.stdout_write(payload);
            } else if ku == 0x8000_0002 {
                // FD STDERR
                let mut e = std::io::stderr();
                let _ = e.write_all(&Self::sv_payload_bytes(payload));
                let _ = e.flush();
            } else if let Some(f) = self.file_handles.get_mut(&k) {
                // Byte-exact (Latin-1) so `%c` round-trips every byte value
                // through a file — see `sv_payload_bytes`.
                let _ = f.write_all(&Self::sv_payload_bytes(payload));
                self.file_writes_pending |= f.has_pending();
            }
            // else: closed/unknown fd (e.g. STDIN) → silently drop.
        }
    }

    /// Write out every handle's buffered output. Called wherever the files may
    /// be looked at from outside their own handle (see `sv_file`).
    /// `$fflush(raw)`: flush the channels `raw` selects, or every open file
    /// and stdout when `raw` is 0. Shared with `vpi_mcd_flush`/`vpi_flush`.
    pub(super) fn flush_channels(&mut self, raw: i32) {
        if raw == 0 {
            // $fflush() / $fflush(0): flush every open file and stdout.
            for f in self.file_handles.values_mut() {
                let _ = f.flush();
            }
            self.file_writes_pending = false;
            self.flush_stdout();
        } else {
            for k in self.resolve_output_keys(raw) {
                let ku = k as u32;
                if ku == 0 || ku == 0x8000_0001 {
                    self.flush_stdout();
                } else if ku == 0x8000_0002 {
                    let mut err = std::io::stderr();
                    let _ = err.flush();
                } else if let Some(f) = self.file_handles.get_mut(&k) {
                    let _ = f.flush();
                }
            }
        }
    }

    pub(super) fn flush_file_writes(&mut self) {
        if !self.file_writes_pending {
            return;
        }
        for f in self.file_handles.values_mut() {
            let _ = f.write_out();
        }
        self.file_writes_pending = false;
    }

    /// Read up to `buf.len()` bytes from file descriptor `fd`, looping until
    /// the buffer is full or EOF. Returns the byte count actually read.
    /// Pending pushback (`$ungetc`, or the unread tail of a partially
    /// matched `$fscanf` line) is consumed first, like C stdio.
    pub(super) fn fread_bytes(&mut self, fd: i32, buf: &mut [u8]) -> usize {
        use std::io::Read;
        // Pushback first: bytes `$fscanf` returned to the stream (and
        // `$ungetc`'s) must come back before anything the OS file holds —
        // C stdio reads consume ungetc'd characters first. Without this,
        // `$fread` after a partial `$fscanf` stranded the pushed-back tail
        // in the buffer and read from the wrong stream position.
        let mut got = 0usize;
        while got < buf.len() {
            let Some(b) = self.ungetc_buf.get_mut(&fd).and_then(|b| b.pop()) else {
                break;
            };
            buf[got] = b;
            got += 1;
        }
        if got > 0 {
            // Data was available, so the end-of-file flag a previous read
            // may have raised no longer holds.
            self.file_eof.remove(&fd);
        }
        if got == buf.len() {
            return got; // filled entirely from pushback
        }
        let Some(f) = self.file_handles.get_mut(&fd) else {
            return got;
        };
        let mut hit_end = false;
        while got < buf.len() {
            match f.read(&mut buf[got..]) {
                Ok(0) | Err(_) => {
                    hit_end = true;
                    break;
                }
                Ok(n) => got += n,
            }
        }
        if hit_end {
            self.file_eof.insert(fd);
        }
        got
    }

    /// Big-endian bytes → `Value` of `width` bits. The FIRST byte read is the
    /// most significant (IEEE 1800-2017 §21.3.4.4); on a short (EOF) read the
    /// data is left-justified — the missing low-order bytes read as zero.
    /// Truncation to `width` keeps the low-order bits (16-bit data into a
    /// 12-bit reg keeps bits [11:0], matching a reference simulator).
    pub(super) fn fread_value_from_bytes(bytes: &[u8], nbytes_full: usize, width: u32) -> Value {
        let mut hex = String::with_capacity(nbytes_full * 2);
        for b in bytes {
            hex.push_str(&format!("{:02x}", b));
        }
        for _ in bytes.len()..nbytes_full {
            hex.push_str("00");
        }
        Value::from_str_radix(&hex, 16, width)
    }

    /// §21.3.4.4 `$fread(dest, fd [, start [, count]])` — BINARY load into an
    /// integral variable or an unpacked memory. Returns the number of bytes
    /// read (0 on EOF or a bad descriptor). For a memory, `start` is an
    /// ADDRESS (defaults to the lowest) and `count` a number of elements;
    /// each element consumes ceil(width/8) bytes, big-endian.
    pub(super) fn fread_impl(&mut self, args: &[Expression]) -> Value {
        if args.len() < 2 {
            return Value::zero(32);
        }
        let fd = self.eval_file_handle_arg(&args[1]);
        // Memory destination?
        if let Some(mem_name) = self.resolve_array_name_from_expr(&args[0]) {
            if let Some((lo, hi, width)) = self.module.arrays.get(&mem_name).copied() {
                let min_idx = lo.min(hi);
                let max_idx = lo.max(hi);
                let start = if args.len() >= 3 {
                    self.eval_expr(&args[2]).to_i64().unwrap_or(min_idx)
                } else {
                    min_idx
                };
                let count = if args.len() >= 4 {
                    self.eval_expr(&args[3]).to_i64().unwrap_or(0).max(0)
                } else {
                    (max_idx - start + 1).max(0)
                };
                let nbytes_elem = (width as usize).div_ceil(8);
                let mut total = 0u64;
                let mut buf = vec![0u8; nbytes_elem];
                for k in 0..count {
                    let addr = start + k;
                    if addr < min_idx || addr > max_idx {
                        break;
                    }
                    let got = self.fread_bytes(fd, &mut buf);
                    if got == 0 {
                        break; // EOF: remaining elements left unchanged
                    }
                    let val = Self::fread_value_from_bytes(&buf[..got], nbytes_elem, width);
                    self.fast_signal_write(&format!("{}[{}]", mem_name, addr), val);
                    total += got as u64;
                    if got < nbytes_elem {
                        break; // short element read: EOF reached
                    }
                }
                return Value::from_u64(total, 32);
            }
        }
        // Integral variable destination.
        let w = self.infer_lhs_width(&args[0]).max(1);
        let nbytes = (w as usize).div_ceil(8);
        let mut buf = vec![0u8; nbytes];
        let got = self.fread_bytes(fd, &mut buf);
        if got > 0 {
            let val = Self::fread_value_from_bytes(&buf[..got], nbytes, w);
            self.assign_value(&args[0], &val);
        }
        Value::from_u64(got as u64, 32)
    }

    pub(super) fn resolve_array_name_from_expr(&self, expr: &Expression) -> Option<String> {
        self.resolve_memory_name(expr, MemKind::OneDim)
            .map(|(n, _)| n)
    }

    /// The storage name of the fixed unpacked array `expr` names, of the
    /// `kind` asked for: one-dimensional (`module.arrays`), multidimensional
    /// (`arrays_2d` / `arrays_nd`), or either, with whether it is
    /// multidimensional. Each step of the lookup tries every kind asked for,
    /// so with [`MemKind::Either`] the first array the name reaches wins: a
    /// one-dimensional class property `mem` named in a method is not a
    /// module's two-dimensional `mem`.
    fn resolve_memory_name(&self, expr: &Expression, kind: MemKind) -> Option<(String, bool)> {
        let is_mem = |n: &str| -> Option<bool> {
            let one = kind != MemKind::MultiDim && self.module.arrays.contains_key(n);
            let multi = kind != MemKind::OneDim
                && (self.module.arrays_2d.contains_key(n) || self.module.arrays_nd.contains_key(n));
            if one {
                Some(false)
            } else if multi {
                Some(true)
            } else {
                None
            }
        };
        // §8.5: fixed arrays declared as class properties live in the
        // per-object store.  Memory tasks accept those arrays just as they do
        // module arrays, both through `obj.member` and a bare member inside a
        // method.
        if let Some(name) = self.handle_member_array_name(expr) {
            if let Some(m) = is_mem(&name) {
                return Some((name, m));
            }
        }
        // §23.6 / §21.4: a hierarchical memory name inside a subroutine body
        // (`r.mem`), or through an instance-array element anywhere
        // (`ra[1].mem`), arrives as a MemberAccess chain rather than a flat
        // Ident. Resolve it as the hierarchical name it spells; without this
        // `$readmemh("f", r.mem)` in a task loaded nothing.
        if matches!(expr.kind, ExprKind::MemberAccess { .. }) {
            if let Some(flat) = self.member_chain_ident(expr) {
                return self.resolve_memory_name(&flat, kind);
            }
        }
        let (resolved, raw) = match &expr.kind {
            ExprKind::Ident(hier) => {
                let resolved = self.resolve_hier_name(hier);
                let raw = hier
                    .path
                    .iter()
                    .map(|s| s.name.name.as_str())
                    .collect::<Vec<_>>()
                    .join(".");
                (resolved, raw)
            }
            _ => return None,
        };
        if let Some(name) = self.instance_assoc_member(&raw) {
            if let Some(m) = is_mem(&name) {
                return Some((name, m));
            }
        }
        let ExprKind::Ident(hier) = &expr.kind else {
            return None;
        };
        // Names that belong to the running subroutine or object are not module
        // memories, so these checks come before any array lookup. A resolved
        // name that differs from the spelling and is a registered array is a
        // local array's own storage, which wins over a scalar of that spelling.
        if self.frame_bound_bare_name(hier).is_some() {
            return None;
        }
        // `obj.prop` through a class handle is a property, not a memory (array
        // properties were taken above). Longer chains, such as `obj.vif.mem`,
        // resolve through the handle's members as usual.
        if hier.path.len() == 2
            && self
                .eval_ident_handle(&hier.path[0].name.name)
                .is_some_and(|h| self.heap.get(h).is_some_and(|o| o.is_some()))
        {
            return None;
        }
        let bare = hier.path.len() == 1 && hier.path[0].selects.is_empty();
        if bare
            && !(resolved.as_ref() != raw && is_mem(resolved.as_ref()).is_some())
            && self.scalar_hides_module_coll(hier.path[0].name.name.as_str())
        {
            return None;
        }
        // `resolved` carries the scope or local storage the name refers to
        // here, so it is tried before the spelled path.
        [resolved.as_ref(), raw.as_str()]
            .into_iter()
            .find_map(|n| is_mem(n).map(|m| (n.to_string(), m)))
    }

    /// The memory a `$readmem*` / `$writemem*` call names. A target that
    /// names no memory is an error; report it rather than skip the call
    /// silently.
    pub(super) fn memory_task_target(
        &mut self,
        task: &str,
        arg: &Expression,
    ) -> Option<MemoryTaskTarget> {
        // An associative array is looked up by its exact name first: the
        // unpacked-memory lookup ends in a suffix match that could claim a
        // same-named memory in another scope.
        if let Some(t) = self.assoc_memory_target(arg) {
            return Some(t);
        }
        if let Some(name) = self.resolve_array_name_from_expr(arg) {
            if let Some(&(lo, hi, width)) = self.module.arrays.get(&name) {
                return Some(MemoryTaskTarget {
                    name,
                    lo: lo.into(),
                    hi: hi.into(),
                    width,
                    assoc: false,
                    key: (64, true),
                    bias: 0,
                });
            }
        }
        let flat = Self::member_chain_as_hier_ident(arg);
        let shown = match &flat.as_ref().unwrap_or(arg).kind {
            ExprKind::Ident(h) => h
                .path
                .iter()
                .map(|s| {
                    let mut t = s.name.name.clone();
                    for sel in &s.selects {
                        match self.eval_expr(sel).to_i64() {
                            Some(i) => t.push_str(&format!("[{}]", i)),
                            None => t.push_str("[?]"),
                        }
                    }
                    t
                })
                .collect::<Vec<_>>()
                .join("."),
            _ => "<expression>".to_string(),
        };
        let m = format!(
            "[xezim][error] {}: '{}' does not name an unpacked memory (t={})",
            task, shown, self.time
        );
        self.error_count = self.error_count.saturating_add(1);
        self.record_output(m.clone());
        self.stdout_writeln(&m);
        None
    }

    /// §21.4 / §21.5: the memory may also be an associative array whose
    /// index type is integral; each address is a key. `None` for anything
    /// else, including a string-, class- or wildcard-indexed array.
    fn assoc_memory_target(&mut self, arg: &Expression) -> Option<MemoryTaskTarget> {
        // `r.mem` inside a subroutine body arrives as a member-access chain;
        // resolve it as the hierarchical name it spells, as for an unpacked
        // memory.
        let name = match self.assoc_operand_name(arg) {
            Some(n) => n,
            None if matches!(arg.kind, ExprKind::MemberAccess { .. }) => {
                let flat = self.member_chain_ident(arg)?;
                self.assoc_operand_name(&flat)?
            }
            None => return None,
        };
        if self.is_string_keyed_array(&name) || self.assoc_key_class_of_arr(&name).is_some() {
            return None;
        }
        let (kw, signed) = self.assoc_index_width_for(&name)?;
        let width = self.assoc_elem_width(&name)?;
        // The addresses the index type can hold: all of a `longint` or a
        // `bit [63:0]`, unsigned ones above `i64::MAX` too, and every key of
        // a wider index type up to 128 bits. A wider one still is addressed
        // through its low 128 bits. The unsigned 128-bit keys are all of
        // `u128`, kept in `bias` order.
        let kw = kw.clamp(1, 128);
        let (lo, hi, bias) = if signed {
            (i128::MIN >> (128 - kw), i128::MAX >> (128 - kw), 0)
        } else if kw == 128 {
            (i128::MIN, i128::MAX, 1 << 127)
        } else {
            (0, i128::MAX >> (127 - kw), 0)
        };
        Some(MemoryTaskTarget {
            name,
            lo,
            hi,
            width,
            assoc: true,
            key: (kw, signed),
            bias,
        })
    }

    /// The address a `$readmem*` start or finish argument names: its value
    /// as written, signed or unsigned, so an unsigned 64-bit address above
    /// `i64::MAX` keeps its value, and a wider one (for an associative
    /// array's wide index type) keeps up to 128 bits.
    fn memory_task_address(&mut self, e: &Expression) -> Option<i128> {
        let v = self.eval_expr(e);
        if v.width > 64 {
            if v.has_xz() {
                return None;
            }
            let a = v.to_u128();
            let s = 128 - v.width.min(128);
            return Some(if v.is_signed {
                ((a << s) as i128) >> s
            } else {
                a as i128
            });
        }
        if v.is_signed {
            v.to_i64().map(i128::from)
        } else {
            v.to_u64().map(i128::from)
        }
    }

    /// The address an `@<hex>` entry of a memory file names (§21.4). A
    /// negative address is written as its 64-bit two's complement, so
    /// `@ffffffffffffffff` is -1; for a signed index type the index type's
    /// own two's complement (`@ffffffff` for an `int` index) is -1 too. An
    /// index type wider than 64 bits holds `@ffffffffffffffff` as a key of
    /// its own, so there a negative address is its 128-bit two's complement
    /// or the index type's own. An unsigned 128-bit key above `i128::MAX`
    /// comes back as its bits, which [`key_order`] puts in order.
    fn memory_file_address(hex: &str, key: (u32, bool)) -> Option<i128> {
        let a = u128::from_str_radix(hex, 16).ok()?;
        let (kw, signed) = key;
        let kw = kw.clamp(1, 128);
        Some(
            if signed && kw != 64 && (kw == 128 || a >> kw == 0) && (a >> (kw - 1)) & 1 == 1 {
                let s = 128 - kw;
                ((a << s) as i128) >> s
            } else if a >> 63 == 1 && a >> 64 == 0 && (kw < 64 || (signed && kw == 64)) {
                a as u64 as i64 as i128
            } else {
                a as i128
            },
        )
    }

    /// `addr` as `$writemem*` writes it after `@`: hexadecimal, a negative
    /// address as its 64-bit two's complement, or its 128-bit one for an
    /// index type wider than 64 bits (`wide`) ([`memory_file_address`]
    /// reads each back).
    fn memory_file_address_text(addr: i128, wide: bool) -> String {
        if addr < 0 && !wide {
            format!("{:x}", addr as i64)
        } else {
            format!("{:x}", addr)
        }
    }

    /// §21.4: load `val` into address `addr` of associative memory `name`,
    /// as `name[addr] = val` would: the address converted to the index type
    /// is the key (§7.8), and the value takes the element type. Returns
    /// whether the element changed.
    fn assoc_memory_store(&mut self, name: &str, addr: i128, mut val: Value) -> bool {
        // Two's complement bits of a negative address, and an unsigned one
        // above `i64::MAX` as it is: the key text takes the index type. An
        // index type wider than 64 bits takes all 128 bits of the address,
        // which are an unsigned key's own even with the top one set.
        let (kw, signed) = self.assoc_index_width_for(name).unwrap_or((64, true));
        let mut key = if kw > 64 {
            Value::from_u128(addr as u128, 128)
        } else {
            Value::from_u64(addr as u64, 64)
        };
        key.is_signed = signed && addr < 0;
        let elem = Self::name_with_key(name, &self.assoc_key_str(name, &key));
        if let Some(signed) = self.assoc_elem_signed(name) {
            val.is_signed = signed;
        }
        // A 2-state element (its missing-element default is 0, not x) holds
        // x/z digits as 0.
        if val.has_xz()
            && self
                .assoc_missing_elem_value(name)
                .is_some_and(|d| !d.has_xz())
        {
            val = val.to_two_state();
        }
        let changed = self.signals.get(&elem) != Some(&val);
        self.signals.insert(elem, val);
        changed
    }

    /// [`member_chain_as_hier_ident`] with each select folded into its
    /// segment's name: an instance-array element or a generate block scope
    /// is `ra[1]` / `blk[1]`, the spelling its storage uses. `None` when a
    /// select is not constant.
    pub(super) fn member_chain_ident(&self, e: &Expression) -> Option<Expression> {
        let mut flat = Self::member_chain_as_hier_ident(e)?;
        if let ExprKind::Ident(h) = &mut flat.kind {
            for seg in h.path.iter_mut() {
                for sel in std::mem::take(&mut seg.selects) {
                    let i = self.eval_scalar_self(&sel)?;
                    seg.name.name = format!("{}[{}]", seg.name.name, i);
                }
            }
        }
        Some(flat)
    }

    /// A dotted chain of names and constant-indexed instance-array elements
    /// (`r.mem`, `ra[1].s.mem`, `top.r.mem`) as one hierarchical Ident, the
    /// index expressions kept as segment selects. `None` for other shapes.
    pub(super) fn member_chain_as_hier_ident(e: &Expression) -> Option<Expression> {
        fn collect(e: &Expression, out: &mut Vec<crate::ast::expr::HierPathSegment>) -> bool {
            match &e.kind {
                ExprKind::Ident(h) => {
                    out.extend(h.path.iter().cloned());
                    true
                }
                ExprKind::MemberAccess { expr, member } => {
                    if !collect(expr, out) {
                        return false;
                    }
                    out.push(crate::ast::expr::HierPathSegment {
                        name: member.clone(),
                        selects: Vec::new(),
                    });
                    true
                }
                ExprKind::Index { expr, index } => {
                    if !collect(expr, out) {
                        return false;
                    }
                    match out.last_mut() {
                        Some(seg) => {
                            seg.selects.push((**index).clone());
                            true
                        }
                        None => false,
                    }
                }
                _ => false,
            }
        }
        let mut path = Vec::new();
        if !collect(e, &mut path) || path.len() < 2 {
            return None;
        }
        let root = match &e.kind {
            ExprKind::MemberAccess { .. } => {
                let mut head = e;
                loop {
                    match &head.kind {
                        ExprKind::MemberAccess { expr, .. } | ExprKind::Index { expr, .. } => {
                            head = expr
                        }
                        _ => break,
                    }
                }
                match &head.kind {
                    ExprKind::Ident(h) => h.root.clone(),
                    _ => None,
                }
            }
            _ => None,
        };
        Some(Expression::new(
            ExprKind::Ident(crate::ast::expr::HierarchicalIdentifier {
                root,
                path,
                span: e.span,
                cached_signal_id: crate::ast::cache::SignalCache::new(None),
                cached_resolved_name: crate::ast::cache::NameCache::new(),
                bound: crate::ast::binding::Bound::UNSET,
            }),
            e.span,
        ))
    }

    pub(super) fn parse_mem_token(token: &str, default_radix: u32) -> Option<(u32, String)> {
        let trimmed = token.trim().trim_end_matches(',').trim_end_matches(';');
        if trimmed.is_empty() {
            return None;
        }
        if let Some(pos) = trimmed.find('\'') {
            let rem = &trimmed[pos + 1..];
            let mut chars = rem.chars();
            let base_ch = chars.next()?.to_ascii_lowercase();
            let digits = chars.as_str();
            if digits.is_empty() {
                return None;
            }
            let radix = match base_ch {
                'b' => 2,
                'o' => 8,
                'd' => 10,
                'h' | 'x' => 16,
                _ => default_radix,
            };
            return Some((radix, digits.to_string()));
        }
        Some((default_radix, trimmed.to_string()))
    }

    pub(super) fn read_memory_file(&mut self, args: &[Expression], default_radix: u32) -> Value {
        self.flush_file_writes();
        if args.len() < 2 {
            sim_dbg_eprintln!("[DEBUG] $readmem*: missing arguments");
            return Value::zero(32);
        }
        // §21.4.3: a multidimensional array, or a partially indexed one.
        if let Some(mem) = self.nd_memory_target(&args[1]) {
            return self.read_nd_memory_file(args, default_radix, mem);
        }
        let path = self.system_string_arg(&args[0]);
        if path.is_empty() {
            sim_dbg_eprintln!("[DEBUG] $readmem*: empty path");
            return Value::zero(32);
        }
        let task = match default_radix {
            2 => "$readmemb",
            10 => "$readmemd",
            _ => "$readmemh",
        };
        let Some(MemoryTaskTarget {
            name: mem_name,
            lo,
            hi,
            width,
            assoc,
            key,
            bias,
        }) = self.memory_task_target(task, &args[1])
        else {
            return Value::zero(32);
        };
        // §21.4: with no start address, loading begins at the memory's left
        // bound; an associative array has none and begins at address 0.
        let first = if assoc { key_order(0, bias) } else { lo };
        let start = match args.get(2) {
            Some(a) => self
                .memory_task_address(a)
                .map(|a| key_order(a, bias))
                .unwrap_or(first),
            None => first,
        };
        let end_addr = match args.get(3) {
            Some(a) => self
                .memory_task_address(a)
                .map(|a| key_order(a, bias))
                .unwrap_or(hi),
            None => hi,
        };
        let step: i128 = if start <= end_addr { 1 } else { -1 };
        let min_idx = lo.min(hi);
        let max_idx = lo.max(hi);
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                // §21.4: the memory is left unchanged; say why, as the
                // reference simulator does, instead of loading nothing
                // silently.
                let m = format!(
                    "[xezim][warning] {}: cannot open file \"{}\": {}",
                    task, path, e
                );
                self.record_output(m.clone());
                self.stdout_writeln(&m);
                return Value::zero(32);
            }
        };
        let mut loaded = 0usize;
        let mut next_addr = Some(start);
        let dense_array = if assoc {
            None
        } else {
            self.array_first_id.get(mem_name.as_str()).copied()
        };
        let mut assoc_changed = false;
        for raw_line in content.lines() {
            let line = raw_line.split("//").next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            for raw_tok in line.split_whitespace() {
                if let Some(rest) = raw_tok.strip_prefix('@') {
                    let addr_hex = rest.trim_start_matches("0x").trim_start_matches("0X");
                    if let Some(a) = Self::memory_file_address(addr_hex, key) {
                        next_addr = Some(key_order(a, bias));
                    }
                    continue;
                }
                let Some((radix, digits)) = Self::parse_mem_token(raw_tok, default_radix) else {
                    continue;
                };
                // An explicit address can restart a sparse image after the
                // previous run reached its finish or the index type's limit.
                let Some(addr) = next_addr else { continue };
                if (step > 0 && addr > end_addr) || (step < 0 && addr < end_addr) {
                    continue;
                }
                next_addr = addr.checked_add(step);
                if addr >= min_idx && addr <= max_idx {
                    let val = Value::from_str_radix(&digits, radix, width);
                    if assoc {
                        assoc_changed |=
                            self.assoc_memory_store(&mem_name, key_order(addr, bias), val);
                    } else if let Some((first_id, arr_lo, arr_hi)) = dense_array {
                        let (arr_lo, arr_hi) = (i128::from(arr_lo), i128::from(arr_hi));
                        if addr >= arr_lo && addr <= arr_hi {
                            let id = first_id + (addr - arr_lo) as usize;
                            if is_packed_id(id) {
                                self.cell_write(id, &val);
                                loaded += 1;
                                continue;
                            }
                            let width = self.signal_widths[id];
                            let mut resized = val.resize(width);
                            resized.is_signed = self.signal_signed[id];
                            if self.signal_table[id] != resized {
                                write_sig!(self, id, resized);
                                self.table_modified = true;
                                let has_dependents = id + 1 < self.comb_dep_offsets.len()
                                    && self.comb_dep_offsets[id] != self.comb_dep_offsets[id + 1];
                                if has_dependents || !self.comb_unresolved_idx.is_empty() {
                                    self.mark_dirty_id(id);
                                } else if self.activity_mon {
                                    if self.signal_toggle_counts.len() != self.signal_table.len() {
                                        self.signal_toggle_counts
                                            .resize(self.signal_table.len(), 0);
                                    }
                                    self.signal_toggle_counts[id] += 1;
                                }
                            }
                        }
                    } else {
                        let elem = format!("{}[{}]", mem_name, addr);
                        self.fast_signal_write(&elem, val);
                    }
                    loaded += 1;
                }
            }
        }
        if assoc_changed {
            // Comb readers of the array re-evaluate, as after `mem[k] = v`.
            self.touch_queue(&mem_name);
        }
        sim_dbg_eprintln!(
            "[DEBUG] $readmem*: loaded {} words into '{}' from '{}'",
            loaded,
            mem_name,
            path
        );
        Value::zero(32)
    }

    /// §21.5 `$writememb/h/d`: dump a memory array to a file in the specified
    /// radix, one word per line, in the `$readmem*` format. §21.5.3: an
    /// associative array's words carry their address (`@<addr_hex> <word>`);
    /// an unpacked or dynamic array's are data only, with no `@` address
    /// even when the start address is not the memory's lowest.
    pub(super) fn write_memory_file(&mut self, args: &[Expression], tn: &str) -> Value {
        self.flush_file_writes();
        if args.len() < 2 {
            return Value::zero(32);
        }
        // §21.4.3 / §21.5: a multidimensional array, or a partially indexed
        // one.
        if let Some(mem) = self.nd_memory_target(&args[1]) {
            return self.write_nd_memory_file(args, tn, mem);
        }
        let path = self.system_string_arg(&args[0]);
        if path.is_empty() {
            return Value::zero(32);
        }
        let Some(MemoryTaskTarget {
            name: mem_name,
            lo,
            hi,
            width,
            assoc,
            bias,
            ..
        }) = self.memory_task_target(tn, &args[1])
        else {
            return Value::zero(32);
        };
        // A dynamic array or queue holds the elements its size counts, not
        // the storage set aside for it (§7.5, §7.10).
        let hi = if !assoc && self.module.dynamic_arrays.contains(&mem_name) {
            let size = self.get_queue_size(&mem_name);
            if size == 0 {
                let _ = std::fs::write(&path, "");
                return Value::zero(32);
            }
            lo + i128::from(size) - 1
        } else {
            hi
        };
        let start = match args.get(2) {
            Some(a) => self
                .memory_task_address(a)
                .map(|a| key_order(a, bias))
                .unwrap_or(lo),
            None => lo,
        };
        let end = match args.get(3) {
            Some(a) => self
                .memory_task_address(a)
                .map(|a| key_order(a, bias))
                .unwrap_or(hi),
            None => hi,
        };
        let radix: u32 = if tn.ends_with('b') {
            2
        } else if tn.ends_with('h') {
            16
        } else {
            10
        };
        if assoc {
            // §21.5: an associative array writes the elements that exist,
            // in index order, between the start and finish addresses.
            let (from, to) = (start.min(end), start.max(end));
            // A key's text is its decimal value (hex digits for an index
            // wider than 64 bits), the full value of an unsigned 64-bit key.
            let wide_key = self
                .assoc_index_width_for(&mem_name)
                .filter(|&(kw, _)| kw > 64);
            let mut words: Vec<(i128, Value)> = Vec::new();
            for key in self.assoc_top_level_keys_shared(&mem_name).iter() {
                // The hex digits of a wide key are its index type's bits.
                let parsed = match wide_key {
                    Some(kt) => Self::memory_file_address(key, kt).map(|a| key_order(a, bias)),
                    None => key.parse::<i128>().ok(),
                };
                let Some(addr) = parsed else {
                    continue;
                };
                if addr < from || addr > to {
                    continue;
                }
                if let Some(val) = self.lookup_signal_value(&Self::name_with_key(&mem_name, key)) {
                    words.push((addr, val));
                }
            }
            // Index order: a signed wide key's text sorts as its unsigned bits.
            words.sort_by_key(|(addr, _)| *addr);
            if start > end {
                words.reverse();
            }
            let mut out = String::new();
            for (addr, val) in &words {
                let addr = key_order(*addr, bias);
                Self::push_memory_word(&mut out, addr, val, radix, wide_key.is_some());
            }
            let _ = std::fs::write(&path, out);
            return Value::zero(32);
        }
        let step: i128 = if start <= end { 1 } else { -1 };
        let min_idx = lo.min(hi);
        let max_idx = lo.max(hi);
        let dense_array = self.array_first_id.get(mem_name.as_str()).copied();
        let mut out = String::new();
        let (mut addr, last) = if step > 0 {
            (start.max(min_idx), end.min(max_idx))
        } else {
            (start.min(max_idx), end.max(min_idx))
        };
        while (step > 0 && addr <= last) || (step < 0 && addr >= last) {
            let val = if let Some((first_id, arr_lo, _)) = dense_array {
                let id = first_id + (addr - i128::from(arr_lo)) as usize;
                if is_packed_id(id) {
                    self.packed.read(id)
                } else {
                    self.signal_table
                        .get(id)
                        .cloned()
                        .unwrap_or_else(|| Value::zero(width))
                }
            } else {
                Value::zero(width)
            };
            Self::push_memory_data(&mut out, &val, radix);
            addr += step;
        }
        let _ = std::fs::write(&path, out);
        Value::zero(32)
    }

    /// One `$writemem*` line of an associative array, `@<addr_hex>
    /// <value_in_radix>`; `wide` as for [`memory_file_address_text`].
    fn push_memory_word(out: &mut String, addr: i128, val: &Value, radix: u32, wide: bool) {
        out.push('@');
        out.push_str(&Self::memory_file_address_text(addr, wide));
        out.push(' ');
        Self::push_memory_data(out, val, radix);
    }

    /// One `$writemem*` line holding only the word, in the task's radix:
    /// §21.5.3 writes no address for an unpacked or dynamic array.
    fn push_memory_data(out: &mut String, val: &Value, radix: u32) {
        let s = match radix {
            2 => val.to_bin_string(),
            16 => val.to_hex_string(),
            _ => val.to_dec_string(),
        };
        out.push_str(&s);
        out.push('\n');
    }

    pub(super) fn base_ident_name(expr: &Expression) -> Option<String> {
        match &expr.kind {
            ExprKind::Ident(hier) => Some(
                hier.path
                    .iter()
                    .map(|s| s.name.name.as_str())
                    .collect::<Vec<_>>()
                    .join("."),
            ),
            ExprKind::Index { expr, .. } | ExprKind::RangeSelect { expr, .. } => {
                Self::base_ident_name(expr)
            }
            ExprKind::Paren(inner) => Self::base_ident_name(inner),
            _ => None,
        }
    }

    /// The memory of a `$readmem*` / `$writemem*` call when it is a
    /// multidimensional fixed unpacked array, or one partially indexed down
    /// to a lesser-dimensioned subarray (`ram`, `ram[1]` of
    /// `ram [4][2][3:0]`; §21.4.3). `None` for anything else, which the
    /// one-dimensional path then takes or reports.
    fn nd_memory_target(&mut self, arg: &Expression) -> Option<NdMemory> {
        if self.module.arrays_2d.is_empty() && self.module.arrays_nd.is_empty() {
            return None;
        }
        // The subarray selects, outermost first. `ram[1]` is an index
        // expression on `ram`; an index followed by another lands in the
        // identifier's last segment (`r3[0][1]` is `[1]` on `r3` with
        // select `[0]`).
        let mut selects: Vec<&Expression> = Vec::new();
        let mut cur = arg;
        while let ExprKind::Index { expr, index } = &cur.kind {
            selects.push(index);
            cur = expr;
        }
        selects.reverse();
        if let ExprKind::Ident(h) = &cur.kind
            && h.path.last().is_some_and(|s| !s.selects.is_empty())
        {
            let mut path = h.path.clone();
            let outer = std::mem::take(&mut path.last_mut()?.selects);
            let stripped = Expression::new(
                ExprKind::Ident(crate::ast::expr::HierarchicalIdentifier {
                    root: h.root.clone(),
                    path,
                    span: h.span,
                    cached_signal_id: crate::ast::cache::SignalCache::new(None),
                    cached_resolved_name: crate::ast::cache::NameCache::new(),
                    bound: crate::ast::binding::Bound::UNSET,
                }),
                cur.span,
            );
            let all: Vec<&Expression> = outer.iter().chain(selects).collect();
            return self.nd_memory_from(&stripped, &all);
        }
        self.nd_memory_from(cur, &selects)
    }

    /// [`nd_memory_target`] for the array `base_expr` names, indexed by
    /// `selects`.
    fn nd_memory_from(
        &mut self,
        base_expr: &Expression,
        selects: &[&Expression],
    ) -> Option<NdMemory> {
        // The array the name reaches first, of either kind: one that is a
        // one-dimensional memory stays on that path.
        let (base, multi) = self.resolve_memory_name(base_expr, MemKind::Either)?;
        if !multi {
            return None;
        }
        let dims = self.multi_dim_memory_dims(&base)?;
        let width = match self.module.arrays_2d.get(&base) {
            Some(&(_, _, w)) => w,
            None => self.module.arrays_nd.get(&base)?.1,
        };
        if selects.len() >= dims.len() {
            return None;
        }
        let mut prefix = Vec::with_capacity(selects.len());
        for (sel, &(l, r)) in selects.iter().zip(&dims) {
            let i = self.eval_expr(sel).to_i64()?;
            if i < l.min(r) || i > l.max(r) {
                return None;
            }
            prefix.push(i);
        }
        let dims = dims[prefix.len()..]
            .iter()
            .map(|&(l, r)| (l.min(r), l.max(r)))
            .collect();
        Some(NdMemory {
            base,
            prefix,
            dims,
            width,
        })
    }

    /// Declared bounds of the multidimensional array `name`, outermost
    /// first, when it can be a memory: a fixed unpacked array whose element
    /// is a vector, a packed struct among them (§21.4.1), and not an
    /// unpacked struct.
    fn multi_dim_memory_dims(&self, name: &str) -> Option<Vec<(i64, i64)>> {
        if self.module.dynamic_arrays.contains(name) || self.is_associative_array(name) {
            return None;
        }
        if let Some(dt) = self.module.var_decl_types.get(name)
            && matches!(self.resolve_dt(dt), DataType::Struct(su) if !su.packed)
        {
            return None;
        }
        if let Some(&(r0, r1, _)) = self.module.arrays_2d.get(name) {
            return Some(match self.module.unpacked_decl_dims.get(name) {
                Some(d) if d.len() == 2 => d.clone(),
                _ => vec![r0, r1],
            });
        }
        let (shape, _) = self.module.arrays_nd.get(name)?;
        Some(match self.module.unpacked_decl_dims.get(name) {
            Some(d) if d.len() == shape.len() => d.clone(),
            _ => shape.clone(),
        })
    }

    /// `$readmemb` / `$readmemh` / `$readmemd` into a multidimensional
    /// array: [`read_memory_file`] with one address per element, in the
    /// order [`NdMemory`] describes.
    fn read_nd_memory_file(
        &mut self,
        args: &[Expression],
        default_radix: u32,
        mem: NdMemory,
    ) -> Value {
        let task = match default_radix {
            2 => "$readmemb",
            10 => "$readmemd",
            _ => "$readmemh",
        };
        let path = self.system_string_arg(&args[0]);
        if path.is_empty() {
            return Value::zero(32);
        }
        let (lo, hi) = mem.addr_range();
        let start = match args.get(2) {
            Some(a) => self.eval_expr(a).to_i64().unwrap_or(lo),
            None => lo,
        };
        let end_addr = match args.get(3) {
            Some(a) => self.eval_expr(a).to_i64().unwrap_or(hi),
            None => hi,
        };
        let step: i64 = if start <= end_addr { 1 } else { -1 };
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                let m = format!(
                    "[xezim][warning] {}: cannot open file \"{}\": {}",
                    task, path, e
                );
                self.record_output(m.clone());
                self.stdout_writeln(&m);
                return Value::zero(32);
            }
        };
        let mut loaded = 0usize;
        let mut next_addr = Some(start);
        for raw_line in content.lines() {
            let line = raw_line.split("//").next().unwrap_or("").trim();
            for raw_tok in line.split_whitespace() {
                if let Some(rest) = raw_tok.strip_prefix('@') {
                    let addr_hex = rest.trim_start_matches("0x").trim_start_matches("0X");
                    // A negative address is written as its 64-bit two's
                    // complement (`{:x}` of an `i64`).
                    if let Ok(a) = u64::from_str_radix(addr_hex, 16) {
                        next_addr = Some(a as i64);
                    }
                    continue;
                }
                let Some((radix, digits)) = Self::parse_mem_token(raw_tok, default_radix) else {
                    continue;
                };
                // An explicit address can restart a sparse image after the
                // previous run reached its finish or the index type's limit.
                let Some(addr) = next_addr else { continue };
                if (step > 0 && addr > end_addr) || (step < 0 && addr < end_addr) {
                    continue;
                }
                next_addr = addr.checked_add(step);
                if addr >= lo && addr <= hi {
                    let idx = mem.indices(addr);
                    let val = Value::from_str_radix(&digits, radix, mem.width);
                    match self.elem_cell(&mem.base, &idx) {
                        Some(id) => {
                            self.fast_signal_write_id(id, &val);
                        }
                        // A class property's elements have no cell; they
                        // are stored by name, and a 2-state property drops
                        // X/Z as any write to it does (§6.11.1).
                        None => {
                            let val = match mem.base.split_once('#') {
                                Some((h, prop))
                                    if val.has_xz()
                                        && h.parse::<usize>()
                                            .is_ok_and(|h| self.class_prop_two_state(h, prop)) =>
                                {
                                    val.to_two_state()
                                }
                                _ => val,
                            };
                            self.fast_signal_write(&Self::elem_name(&mem.base, &idx), val);
                        }
                    }
                    loaded += 1;
                }
            }
        }
        sim_dbg_eprintln!(
            "[DEBUG] $readmem*: loaded {} words into '{}' from '{}'",
            loaded,
            mem.base,
            path
        );
        Value::zero(32)
    }

    /// `$writememb` / `$writememh` / `$writememd` of a multidimensional
    /// array (§21.5): [`write_memory_file`] one element per line, data only
    /// (§21.5.3), in the order [`NdMemory`] describes (§21.4.3), so
    /// `$readmem*` reads it back.
    fn write_nd_memory_file(&mut self, args: &[Expression], tn: &str, mem: NdMemory) -> Value {
        let path = self.system_string_arg(&args[0]);
        if path.is_empty() {
            return Value::zero(32);
        }
        let (lo, hi) = mem.addr_range();
        let start = match args.get(2) {
            Some(a) => self.eval_expr(a).to_i64().unwrap_or(lo),
            None => lo,
        };
        let end = match args.get(3) {
            Some(a) => self.eval_expr(a).to_i64().unwrap_or(hi),
            None => hi,
        };
        let radix: u32 = if tn.ends_with('b') {
            2
        } else if tn.ends_with('h') {
            16
        } else {
            10
        };
        // The addresses between start and finish that the array holds, in
        // the direction start to finish.
        let addrs: Box<dyn Iterator<Item = i64>> = if start <= end {
            Box::new(start.max(lo)..=end.min(hi))
        } else {
            Box::new((end.max(lo)..=start.min(hi)).rev())
        };
        let mut out = String::new();
        for addr in addrs {
            let idx = mem.indices(addr);
            let val = match self.elem_cell(&mem.base, &idx) {
                Some(id) => self.cell_read(id),
                // A class property's elements are stored by name.
                None => self
                    .lookup_signal_value(&Self::elem_name(&mem.base, &idx))
                    .unwrap_or_else(|| Value::zero(mem.width)),
            };
            Self::push_memory_data(&mut out, &val, radix);
        }
        let _ = std::fs::write(&path, out);
        Value::zero(32)
    }
}

/// A multidimensional fixed unpacked array, or a partially indexed one, as
/// the memory of a `$readmem*` / `$writemem*` call.
///
/// §21.4.3 lays its words out in row-major order: the rightmost dimension
/// varies fastest, and every dimension runs from its low index to its high
/// one whichever way it is declared. Each element has one address, counted
/// in that order from the low index of the leftmost dimension, and the
/// start and finish arguments and the file's `@` entries are such addresses.
/// The LRM text has `@` entries and the address arguments name words of the
/// leftmost dimension instead, but Verilator and Icarus both linearize the
/// array, and real images depend on it: a
/// byte-wide `ram [DEPTH][NB]` is loaded from a byte-addressed hex file with
/// finish address `DEPTH*NB-1`.
pub(super) struct NdMemory {
    /// Storage name of the whole array.
    base: String,
    /// The indices that select the subarray, outermost first.
    prefix: Vec<i64>,
    /// `(lo, hi)` of each remaining dimension, outermost first.
    dims: Vec<(i64, i64)>,
    /// Element width.
    width: u32,
}

impl NdMemory {
    /// The first and last address: the low index of the leftmost remaining
    /// dimension, and that plus the element count less one.
    fn addr_range(&self) -> (i64, i64) {
        let count = self
            .dims
            .iter()
            .fold(1i64, |n, &(lo, hi)| n.saturating_mul(hi - lo + 1));
        (self.dims[0].0, self.dims[0].0.saturating_add(count - 1))
    }

    /// The full index list of the element at `addr`, which lies in
    /// [`addr_range`](Self::addr_range).
    fn indices(&self, addr: i64) -> Vec<i64> {
        let mut k = addr - self.dims[0].0;
        let mut tail = vec![0i64; self.dims.len()];
        for (slot, &(lo, hi)) in tail.iter_mut().zip(&self.dims).rev() {
            let size = hi - lo + 1;
            *slot = lo + k % size;
            k /= size;
        }
        let mut idx = self.prefix.clone();
        idx.extend(tail);
        idx
    }
}

/// Which fixed unpacked arrays [`Simulator::resolve_memory_name`] looks for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum MemKind {
    OneDim,
    MultiDim,
    Either,
}
