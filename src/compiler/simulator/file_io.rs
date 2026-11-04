//! Simulator file io.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

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
        self.sv_sscanf_consumed(src, fmt, outs).0
    }

    /// Like `sv_sscanf`, but also returns how many CHARS of `src` were
    /// consumed, so `$fscanf` can push the unread remainder back (§21.3.4.3:
    /// the file position advances only past what the format matched — reading
    /// one `%s` of a two-word line must leave the second word readable, and
    /// `$feof` false).
    pub(super) fn sv_sscanf_consumed(
        &mut self,
        src: &str,
        fmt: &str,
        outs: &[Expression],
    ) -> (i64, usize) {
        let s: Vec<char> = src.chars().collect();
        let mut si = 0usize;
        let mut oi = 0usize;
        let mut assigned = 0i64;
        let mut fi = fmt.chars().peekable();
        fn skip_ws(s: &[char], mut i: usize) -> usize {
            while i < s.len() && s[i].is_whitespace() {
                i += 1;
            }
            i
        }
        while let Some(fc) = fi.next() {
            if fc == '%' {
                let mut width = String::new();
                while fi.peek().is_some_and(|c| c.is_ascii_digit()) {
                    width.push(fi.next().unwrap());
                }
                let w: usize = width.parse().unwrap_or(usize::MAX);
                let Some(spec) = fi.next() else { break };
                if spec == '%' {
                    si = skip_ws(&s, si);
                    if si < s.len() && s[si] == '%' {
                        si += 1;
                        continue;
                    }
                    break;
                }
                if spec == 'c' {
                    if si >= s.len() {
                        break;
                    }
                    let v = Value::from_u64(s[si] as u64, 8);
                    si += 1;
                    if oi < outs.len() {
                        self.assign_value(&outs[oi], &v);
                        oi += 1;
                        assigned += 1;
                    }
                    continue;
                }
                si = skip_ws(&s, si);
                let start = si;
                match spec {
                    's' => {
                        while si < s.len() && !s[si].is_whitespace() && si - start < w {
                            si += 1;
                        }
                        if si == start {
                            break;
                        }
                        let text: String = s[start..si].iter().collect();
                        if oi < outs.len() {
                            self.assign_value(&outs[oi], &Value::from_string(&text));
                            oi += 1;
                            assigned += 1;
                        }
                    }
                    'f' | 'e' | 'g' => {
                        while si < s.len()
                            && (s[si].is_ascii_digit() || "+-.eE".contains(s[si]))
                            && si - start < w
                        {
                            si += 1;
                        }
                        let text: String = s[start..si].iter().collect();
                        match text.parse::<f64>() {
                            Ok(f) => {
                                if oi < outs.len() {
                                    self.assign_value(&outs[oi], &Value::from_f64(f));
                                    oi += 1;
                                    assigned += 1;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                    'd' | 'i' | 'h' | 'x' | 'X' | 'o' | 'b' => {
                        let radix: u32 = match spec {
                            'h' | 'x' | 'X' => 16,
                            'o' => 8,
                            'b' => 2,
                            _ => 10,
                        };
                        let signed = matches!(spec, 'd' | 'i');
                        let mut si2 = si;
                        let mut neg = false;
                        if signed && si2 < s.len() && (s[si2] == '-' || s[si2] == '+') {
                            neg = s[si2] == '-';
                            si2 += 1;
                        }
                        let dstart = si2;
                        while si2 < s.len()
                            && (s[si2].is_digit(radix) || s[si2] == '_')
                            && si2 - dstart < w
                        {
                            si2 += 1;
                        }
                        if si2 == dstart {
                            break;
                        }
                        let text: String = s[dstart..si2].iter().filter(|c| **c != '_').collect();
                        // The scan above admits `char::is_digit`, which is
                        // Unicode-aware, so a non-ASCII digit can land here.
                        // `from_str_radix` would answer X; `$sscanf` has to
                        // report a failed conversion and stop, the way the old
                        // `i64::from_str_radix` returning `Err` did.
                        if text
                            .bytes()
                            .any(|b| !b.is_ascii() || (b as char).to_digit(radix).is_none())
                        {
                            break;
                        }
                        // Parse at the destination's width, not a fixed 32: a
                        // 160-bit field has to keep all 160 bits of the token.
                        let dest_width = if oi < outs.len() {
                            self.infer_lhs_width(&outs[oi]).max(1)
                        } else {
                            32
                        };
                        let mut v = Value::from_str_radix(&text, radix, dest_width);
                        if neg {
                            v = Self::negate_twos_complement(v);
                        } else {
                            v.is_signed = signed;
                        }
                        si = si2;
                        if oi < outs.len() {
                            self.assign_value(&outs[oi], &v);
                            oi += 1;
                            assigned += 1;
                        }
                    }
                    _ => break,
                }
            } else if fc.is_whitespace() {
                si = skip_ws(&s, si);
            } else {
                if si < s.len() && s[si] == fc {
                    si += 1;
                } else {
                    break;
                }
            }
        }
        (assigned, si)
    }

    /// §21.3.4.3 `$fscanf(fd, format, …)`: line-buffered scan of `fd`.
    /// Returns the number of assigned conversions, or -1 at end of file.
    /// Shared by the function form (`n = $fscanf(…)`) and the task form
    /// (`$fscanf(…);`), which discards the count.
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
        // A previous partial read may have pushed back just the tail
        // whitespace of its line ("\n"). For conversions that skip leading
        // whitespace (§21.3.4.3: the numeric/string ones do, across lines) a
        // whitespace-only line is transparent — pull the next. But when the
        // format's first item matches WITHOUT skipping (`%c` reads exactly
        // one character, whitespace included; a literal must match exactly),
        // that line carries the very byte the format must see (issue #262: a
        // file holding one space read as empty, and repeated `%c` lost every
        // '\n' between lines).
        let mut first_line = self.sv_read_line(fd);
        if Self::scanf_leading_item_skips_ws(&fmt) {
            while first_line.as_deref().is_some_and(|l| l.trim().is_empty()) {
                first_line = self.sv_read_line(fd);
            }
        }
        match first_line {
            Some(line) => {
                let (n, consumed) = self.sv_sscanf_consumed(&line, &fmt, &args[2..]);
                // §21.3.4.3: the position advances only past what the format
                // matched. `sv_read_line` pulled a whole line, so push the
                // unread tail back (reversed — the pushback buffer is a LIFO
                // whose top is the next char). Without this,
                // `$fscanf(fd, "%s", w)` on a two-word line dropped the
                // second word and `$feof` reported end-of-file with data
                // left. `sv_read_line` maps bytes onto chars one-to-one
                // (Latin-1), so the reverse mapping `ch as u8` restores the
                // ORIGINAL byte — re-encoding as UTF-8 turned a pushed-back
                // 0xff into the two bytes C3 BF.
                let rest: Vec<char> = line.chars().skip(consumed).collect();
                if !rest.is_empty() {
                    let buf = self.ungetc_buf.entry(fd).or_default();
                    for ch in rest.iter().rev() {
                        buf.push(*ch as u8);
                    }
                    // Data is readable again: an end-of-file flag that the
                    // final line's read raised must come down, the way
                    // `$ungetc` lowers it — `$feof` must not report EOF
                    // while the pushed-back tail is still unread.
                    self.file_eof.remove(&fd);
                }
                n
            }
            None => -1, // EOF (§21.3.4.3)
        }
    }

    /// Does the FIRST item of a scanf format skip leading input whitespace
    /// before it matches? Explicit whitespace in the format does, and so do
    /// the numeric/string conversions (`%d %i %h %x %X %o %b %s %f %e %g`).
    /// `%c` matches any single character — whitespace included
    /// (§21.3.4.3, issue #262) — and `%%` and literal characters must match
    /// exactly, so for those the input's leading whitespace is significant
    /// and the line-buffered reader must not step over it.
    pub(super) fn scanf_leading_item_skips_ws(fmt: &str) -> bool {
        let mut it = fmt.chars().peekable();
        while let Some(c) = it.next() {
            if c.is_whitespace() {
                return true;
            }
            if c != '%' {
                return false; // literal: exact match, no skipping
            }
            while it.peek().is_some_and(|d| d.is_ascii_digit()) {
                it.next(); // field width
            }
            return match it.next() {
                Some('d' | 'i' | 'h' | 'x' | 'X' | 'o' | 'b' | 's' | 'f' | 'e' | 'g') => true,
                _ => false, // %c, %%, unknown: no whitespace skipping
            };
        }
        false
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
        // §8.5: fixed arrays declared as class properties live in the
        // per-object store.  Memory tasks accept those arrays just as they do
        // module arrays, both through `obj.member` and a bare member inside a
        // method.
        if let Some(name) = self.handle_member_array_name(expr) {
            if self.module.arrays.contains_key(&name) {
                return Some(name);
            }
        }
        // §23.6 / §21.4: a hierarchical memory name inside a subroutine body
        // (`r.mem`), or through an instance-array element anywhere
        // (`ra[1].mem`), arrives as a MemberAccess chain rather than a flat
        // Ident. Resolve it as the hierarchical name it spells; without this
        // `$readmemh("f", r.mem)` in a task loaded nothing.
        if matches!(expr.kind, ExprKind::MemberAccess { .. }) {
            if let Some(flat) = self.member_chain_ident(expr) {
                return self.resolve_array_name_from_expr(&flat);
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
            if self.module.arrays.contains_key(&name) {
                return Some(name);
            }
        }
        if self.module.arrays.contains_key(&*resolved) {
            return Some(resolved.to_string());
        }
        if self.module.arrays.contains_key(&raw) {
            return Some(raw);
        }
        if let Some(found) = self.module.arrays.keys().find(|k| {
            *k == &resolved
                || *k == &raw
                || k.ends_with(&format!(".{}", resolved))
                || k.ends_with(&format!(".{}", raw))
        }) {
            return Some(found.clone());
        }
        None
    }

    /// The memory a `$readmem*` / `$writemem*` call names, with its
    /// `(lo, hi, width)`. A target that names no unpacked memory is an error
    /// (the reference simulator rejects the design); report it rather than
    /// skip the call silently.
    pub(super) fn memory_task_target(
        &mut self,
        task: &str,
        arg: &Expression,
    ) -> Option<(String, (i64, i64, u32))> {
        if let Some(name) = self.resolve_array_name_from_expr(arg) {
            if let Some(&dims) = self.module.arrays.get(&name) {
                return Some((name, dims));
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
                cached_signal_id: std::cell::Cell::new(None),
                cached_resolved_name: std::cell::OnceCell::new(),
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
        let Some((mem_name, (lo, hi, width))) = self.memory_task_target(task, &args[1]) else {
            return Value::zero(32);
        };
        let mut addr = if args.len() >= 3 {
            self.eval_expr(&args[2]).to_i64().unwrap_or(lo)
        } else {
            lo
        };
        let end_addr = if args.len() >= 4 {
            self.eval_expr(&args[3]).to_i64().unwrap_or(hi)
        } else {
            hi
        };
        let step: i64 = if addr <= end_addr { 1 } else { -1 };
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
        let dense_array = self.array_first_id.get(mem_name.as_str()).copied();
        'lines: for raw_line in content.lines() {
            let line = raw_line.split("//").next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            for raw_tok in line.split_whitespace() {
                if step > 0 {
                    if addr > end_addr {
                        break 'lines;
                    }
                } else if addr < end_addr {
                    break 'lines;
                }
                if let Some(rest) = raw_tok.strip_prefix('@') {
                    let addr_hex = rest.trim_start_matches("0x").trim_start_matches("0X");
                    if let Ok(a) = i64::from_str_radix(addr_hex, 16) {
                        addr = a;
                    }
                    continue;
                }
                let Some((radix, digits)) = Self::parse_mem_token(raw_tok, default_radix) else {
                    continue;
                };
                if addr >= min_idx && addr <= max_idx {
                    let val = Value::from_str_radix(&digits, radix, width);
                    if let Some((first_id, arr_lo, arr_hi)) = dense_array {
                        if addr >= arr_lo && addr <= arr_hi {
                            let id = first_id + (addr - arr_lo) as usize;
                            if is_packed_id(id) {
                                self.cell_write(id, &val);
                                loaded += 1;
                                addr += step;
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
                addr += step;
            }
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
    /// radix. Each line is `@<addr_hex> <value_in_radix>`, matching the
    /// `$readmem*` format so the file is round-trippable.
    pub(super) fn write_memory_file(&mut self, args: &[Expression], tn: &str) -> Value {
        self.flush_file_writes();
        if args.len() < 2 {
            return Value::zero(32);
        }
        let path = self.system_string_arg(&args[0]);
        if path.is_empty() {
            return Value::zero(32);
        }
        let Some((mem_name, (lo, hi, width))) = self.memory_task_target(tn, &args[1]) else {
            return Value::zero(32);
        };
        let start = if args.len() >= 3 {
            self.eval_expr(&args[2]).to_i64().unwrap_or(lo)
        } else {
            lo
        };
        let end = if args.len() >= 4 {
            self.eval_expr(&args[3]).to_i64().unwrap_or(hi)
        } else {
            hi
        };
        let radix: u32 = if tn.ends_with('b') {
            2
        } else if tn.ends_with('h') {
            16
        } else {
            10
        };
        let step: i64 = if start <= end { 1 } else { -1 };
        let min_idx = lo.min(hi);
        let max_idx = lo.max(hi);
        let dense_array = self.array_first_id.get(mem_name.as_str()).copied();
        let mut out = String::new();
        let mut addr = start;
        while (step > 0 && addr <= end) || (step < 0 && addr >= end) {
            if addr >= min_idx && addr <= max_idx {
                let val = if let Some((first_id, arr_lo, _)) = dense_array {
                    let id = first_id + (addr - arr_lo) as usize;
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
                out.push_str(&format!("@{:x} ", addr));
                let s = match radix {
                    2 => val.to_bin_string(),
                    16 => val.to_hex_string(),
                    _ => val.to_dec_string(),
                };
                out.push_str(&s);
                out.push('\n');
            }
            addr += step;
        }
        let _ = std::fs::write(&path, out);
        Value::zero(32)
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
}
