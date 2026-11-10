//! Built-in UVM DPI-C library.
//!
//! UVM compiled without `+define+UVM_NO_DPI` imports C helpers from its
//! `src/dpi` directory: regular expressions (`uvm_regex.cc`), the
//! command-line walk (`uvm_svcmd_dpi.c`) and HDL backdoor access
//! (`uvm_hdl_*.c`). When no `-sv_lib` library defines one of those C
//! symbols, `exec_dpi_import_call` serves it from here with the C sources'
//! semantics:
//!
//! * regex: libc `regcomp`/`regexec` with `REG_EXTENDED`, the engine the C
//!   code calls, so matching is POSIX extended. `uvm_re_match` strips one
//!   pair of surrounding `/`. `uvm_glob_to_re` is a character-for-character
//!   port, including the `/.../` passthrough and the per-release result for
//!   an empty glob (`""` up to UVM 1.2, `/^$/` from 1800.2 on). Compiled
//!   patterns are cached; an invalid one is still reported on every call.
//! * cmdline: `uvm_dpi_get_next_arg_c` returns the simulator's argv (tool
//!   name, then the plusargs) one entry per call.
//! * hdl: read, deposit, force and release by full hierarchical path, with
//!   the bit-select (`sig[3]`) and part-select (`sig[7:4]`) forms the C
//!   code expands bit by bit, memory words (`mem[5]`) and packed-struct
//!   members.
//!
//! * polling (1800.2-2020.3, `uvm_hdl_polling.c`): `uvm_polling_create`
//!   probes a signal, `uvm_polling_set_enable_callback` hooks its value
//!   changes (the VPI value-change callback path), a change toggles the
//!   notifier bit named by `uvm_polling_setup_notifier`, and
//!   `uvm_polling_process_changelist` calls the exported
//!   `uvm_polling_value_change_notify` once per changed probe, newest
//!   first. `uvm_hdl_signal_size` is the width at a path.
//!
//! Errors are reported through UVM's exported `m__uvm_report_dpi` with the C
//! code's ids and texts. Requests without a faithful mapping (forcing part
//! of a signal or a packed-arena memory cell, a select into a
//! multi-dimensional packed vector) fail the call with a one-time `[DPI]`
//! error.
use super::*;

const M_UVM_INFO: i64 = 0;
const M_UVM_ERROR: i64 = 2;
const M_UVM_FATAL: i64 = 3;
const M_UVM_NONE: i64 = 0;
const M_UVM_LOW: i64 = 100;
const M_UVM_MEDIUM: i64 = 200;
/// `UVM_REGEX_MAX_LENGTH` in uvm_regex.cc.
const UVM_REGEX_MAX_LENGTH: usize = 2048;
/// `uvm_re_match` compile-cache bound; the cache is dropped when full.
const RE_CACHE_MAX: usize = 4096;

/// A compiled POSIX regular expression.
pub(super) struct PosixRe(Box<libc::regex_t>);

impl PosixRe {
    /// `Err((code, text))` carries regcomp's error code and regerror's text.
    fn compile(pat: &str, flags: libc::c_int) -> Result<PosixRe, (i32, String)> {
        let Ok(c_pat) = CString::new(pat) else {
            return Err((libc::REG_BADPAT, "pattern contains a NUL byte".to_string()));
        };
        // SAFETY: `re` is a zeroed regex_t owned by the Box; regcomp
        // initializes it and regfree releases it (also after a failed
        // compile, as uvm_regex.cc does).
        unsafe {
            let mut re: Box<libc::regex_t> = Box::new(std::mem::zeroed());
            let err = libc::regcomp(re.as_mut(), c_pat.as_ptr(), flags);
            if err != 0 {
                let mut buf = [0 as libc::c_char; 256];
                libc::regerror(err, re.as_ref(), buf.as_mut_ptr(), buf.len());
                libc::regfree(re.as_mut());
                let text = CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned();
                return Err((err, text));
            }
            Ok(PosixRe(re))
        }
    }

    /// regexec: 0 on a match, `REG_NOMATCH` otherwise.
    fn exec(&self, s: &str) -> i32 {
        let Ok(c_s) = CString::new(s) else {
            return libc::REG_NOMATCH;
        };
        // SAFETY: the regex_t was initialized by a successful regcomp.
        unsafe { libc::regexec(self.0.as_ref(), c_s.as_ptr(), 0, std::ptr::null_mut(), 0) }
    }
}

impl Drop for PosixRe {
    fn drop(&mut self) {
        // SAFETY: constructed only from a successful regcomp.
        unsafe { libc::regfree(self.0.as_mut()) }
    }
}

#[derive(Default)]
pub(super) struct UvmDpiState {
    /// Cursor of the argv walk (a C static in uvm_svcmd_dpi.c).
    argv_idx: usize,
    /// `uvm_re_match` patterns compiled so far, failures included.
    re_cache: HashMap<String, Result<PosixRe, (i32, String)>>,
    /// Live handles for `uvm_re_compexecfree`'s (pattern, deglob) cache —
    /// those chandles never escape to SV, so they can persist.
    re_exec_handles: HashMap<(String, bool), u64>,
    /// Compiled patterns behind the chandles `uvm_dpi_regcomp` and
    /// `uvm_re_comp` returned.
    handles: HashMap<u64, PosixRe>,
    next_handle: u64,
    /// uvm_regex.cc's static `uvm_re` buffer, as `uvm_re_buffer` returns it.
    re_buffer: String,
    /// One-time diagnostics already printed.
    noted: HashSet<String>,
    /// Whether the design imports `uvm_dump_re_cache` (UVM 1.1d/1.2).
    legacy_regex: Option<bool>,
    /// Probes `uvm_polling_create` made, by chandle.
    polls: HashMap<u64, PollProbe>,
    /// Probes with an unserviced value change, oldest first.
    poll_changes: Vec<u64>,
    /// Signal-table slot of the notifier bit.
    poll_notifier: Option<usize>,
}

/// One `uvm_polling_create` probe.
pub(super) struct PollProbe {
    target: HdlTarget,
    /// Slot whose writes carry the value-change callback.
    slot: usize,
    sv_key: i64,
    enabled: bool,
    /// The probed bits as last seen, to report changes only.
    last: Value,
}

/// Value-change callback of an enabled probe; `user_data` is its chandle.
extern "C" fn uvm_polling_value_change(cb: *mut s_cb_data) -> libc::c_int {
    if cb.is_null() {
        return 0;
    }
    let h = unsafe { (*cb).user_data } as u64;
    let _ = try_active_sim("uvm_polling", |sim| sim.uvm_polling_changed(h));
    0
}

/// Storage behind a `uvm_hdl_*` path.
#[derive(Clone)]
enum HdlObj {
    /// A signal-table slot.
    Slot(usize),
    /// A runtime-map signal.
    Named(String),
}

#[derive(Clone)]
struct HdlTarget {
    obj: HdlObj,
    /// The signal's name, as force/release bookkeeping keys it.
    key: String,
    /// Bits addressed (value bit `i` is signal bit `bits[i]`); `None` is the
    /// whole signal.
    bits: Option<Vec<u32>>,
}

enum HdlLookup {
    Found(HdlTarget),
    /// A module instance: it exists, but has no value.
    Scope,
    Missing,
    /// Names storage the backdoor cannot address faithfully.
    Unsupported(String),
}

/// uvm_re_match's bracket rule: one pair of surrounding `/` is dropped.
fn strip_re_brackets(re: &str) -> &str {
    let b = re.as_bytes();
    if b.len() > 1 && b[0] == b'/' && b[b.len() - 1] == b'/' {
        &re[1..re.len() - 1]
    } else {
        re
    }
}

/// uvm_glob_to_re's translation of a non-empty glob. A glob already
/// bracketed as `/.../` is a regex and passes through; `with_brackets`
/// false drops the brackets (1800.2-2020's `uvm_re_deglobbed`).
fn deglob(glob: &str, with_brackets: bool) -> String {
    let b = glob.as_bytes();
    if b.len() > 1 && b[0] == b'/' && b[b.len() - 1] == b'/' {
        return if with_brackets {
            glob.to_string()
        } else {
            strip_re_brackets(glob).to_string()
        };
    }
    let mut re: Vec<u8> = Vec::with_capacity(b.len() * 2 + 4);
    if b.first() != Some(&b'^') {
        re.push(b'^');
    }
    for &c in b {
        match c {
            b'*' => re.extend_from_slice(b".*"),
            b'+' => re.extend_from_slice(b".+"),
            b'.' => re.extend_from_slice(b"\\."),
            b'?' => re.push(b'.'),
            b'[' => re.extend_from_slice(b"\\["),
            b']' => re.extend_from_slice(b"\\]"),
            b'(' => re.extend_from_slice(b"\\("),
            b')' => re.extend_from_slice(b"\\)"),
            _ => re.push(c),
        }
    }
    if re.last() != Some(&b'$') {
        re.push(b'$');
    }
    let re = String::from_utf8_lossy(&re).into_owned();
    if with_brackets {
        format!("/{}/", re)
    } else {
        re
    }
}

fn int_lit(v: i64) -> Expression {
    Expression::new(
        ExprKind::Number(NumberLiteral::Integer {
            size: Some(32),
            signed: true,
            base: NumberBase::Decimal,
            value: v.to_string(),
            cached_val: crate::ast::cache::LiteralCache::new(Some((v as u32 as u64, 0u64, 32u32))),
        }),
        crate::ast::Span::dummy(),
    )
}

fn str_lit(s: &str) -> Expression {
    Expression::new(
        ExprKind::StringLiteral(s.to_string()),
        crate::ast::Span::dummy(),
    )
}

fn int_val(v: i32) -> Value {
    let mut r = Value::from_u64(v as u32 as u64, 32);
    r.is_signed = true;
    r
}

fn chandle_val(h: u64) -> Value {
    Value::from_u64(h, 64)
}

impl Simulator {
    /// Serve UVM's DPI-C helper `c_name` (imported as `sv_name`). `None`
    /// when `c_name` is not one of them.
    pub(super) fn exec_uvm_dpi_builtin(
        &mut self,
        c_name: &str,
        sv_name: &str,
        args: &[Expression],
    ) -> Option<Value> {
        let v = match c_name {
            "uvm_re_match" => {
                let re = self.uvm_dpi_str(args, 0);
                let s = self.uvm_dpi_str(args, 1);
                int_val(self.uvm_re_match(&re, &s))
            }
            "uvm_glob_to_re" => {
                let glob = self.uvm_dpi_str(args, 0);
                Value::from_string(&self.uvm_glob_to_re(&glob))
            }
            "uvm_re_deglobbed" => {
                let glob = self.uvm_dpi_str(args, 0);
                let with_brackets = self.uvm_dpi_int(args, 1) != 0;
                Value::from_string(
                    &self
                        .uvm_re_deglobbed(&glob, with_brackets)
                        .unwrap_or_default(),
                )
            }
            "uvm_dump_re_cache" => {
                self.uvm_dpi_report(
                    M_UVM_INFO,
                    "UVM/DPI/REGEX_MAX",
                    "uvm_dump_re_cache: cache not implemented",
                    M_UVM_LOW,
                );
                Value::zero(32)
            }
            "uvm_dpi_regcomp" => {
                let pat = self.uvm_dpi_str(args, 0);
                match PosixRe::compile(&pat, libc::REG_NOSUB | libc::REG_EXTENDED) {
                    Ok(re) => chandle_val(self.uvm_dpi_new_handle(re)),
                    Err(_) => {
                        let first = pat.chars().next().map(String::from).unwrap_or_default();
                        let msg = format!(
                            "uvm_dpi_regcomp : Unable to compile regex: |{}|, Element 0 is: {}",
                            pat, first
                        );
                        self.uvm_dpi_report(M_UVM_ERROR, "UVM/DPI/REGCOMP", &msg, M_UVM_NONE);
                        chandle_val(0)
                    }
                }
            }
            "uvm_re_comp" => {
                let re = self.uvm_dpi_str(args, 0);
                let deglobbed = self.uvm_dpi_int(args, 1) != 0;
                chandle_val(self.uvm_re_comp(&re, deglobbed))
            }
            "uvm_dpi_regexec" | "uvm_re_exec" => {
                let h = self.uvm_dpi_u64(args, 0);
                let s = self.uvm_dpi_str(args, 1);
                if h == 0 && c_name == "uvm_re_exec" {
                    self.uvm_dpi.re_buffer = "uvm_re_exec: NULL rexp".to_string();
                }
                int_val(self.uvm_dpi_handle_exec(c_name, h, &s))
            }
            "uvm_dpi_regfree" | "uvm_re_free" => {
                let h = self.uvm_dpi_u64(args, 0);
                self.uvm_dpi.handles.remove(&h);
                Value::zero(32)
            }
            "uvm_re_buffer" => Value::from_string(&self.uvm_dpi.re_buffer.clone()),
            "uvm_re_compexec" | "uvm_re_compexecfree" => {
                let re = self.uvm_dpi_str(args, 0);
                let s = self.uvm_dpi_str(args, 1);
                let deglobbed = self.uvm_dpi_int(args, 2) != 0;
                if c_name == "uvm_re_compexecfree" {
                    // The compiled chandle never escapes to SV (the SV
                    // wrapper frees it immediately), so a per-(pattern,
                    // deglob) regex cache is unobservable from SV: identical
                    // match results without a regcomp per call.
                    let key = (re.clone(), deglobbed);
                    let (h, keep) = match self.uvm_dpi.re_exec_handles.get(&key) {
                        Some(&h) => (h, true),
                        None => {
                            let h = self.uvm_re_comp(&re, deglobbed);
                            let keep = h != 0 && self.uvm_dpi.re_exec_handles.len() < RE_CACHE_MAX;
                            if keep {
                                self.uvm_dpi.re_exec_handles.insert(key, h);
                            }
                            (h, keep)
                        }
                    };
                    let r = if h == 0 {
                        libc::REG_NOMATCH
                    } else {
                        self.uvm_dpi_handle_exec(c_name, h, &s)
                    };
                    if let Some(out) = args.get(3) {
                        self.assign_value(out, &int_val(r));
                    }
                    if !keep {
                        self.uvm_dpi.handles.remove(&h);
                    }
                    return Some(Value::from_u64((h != 0) as u64, 1));
                }
                let h = self.uvm_re_comp(&re, deglobbed);
                // A pattern that fails to compile reports REG_NOMATCH.
                let r = if h == 0 {
                    libc::REG_NOMATCH
                } else {
                    self.uvm_dpi_handle_exec(c_name, h, &s)
                };
                if let Some(out) = args.get(3) {
                    self.assign_value(out, &int_val(r));
                }
                chandle_val(h)
            }
            "uvm_dpi_get_next_arg_c" => Value::from_string(&self.uvm_dpi_next_arg(args)),
            "uvm_dpi_get_tool_name_c" => Value::from_string("xezim"),
            "uvm_dpi_get_tool_version_c" => Value::from_string(env!("CARGO_PKG_VERSION")),
            "uvm_hdl_check_path" => {
                let path = self.uvm_dpi_str(args, 0);
                let found = !matches!(self.uvm_hdl_lookup(&path), HdlLookup::Missing);
                int_val(found as i32)
            }
            "uvm_hdl_read" => {
                let path = self.uvm_dpi_str(args, 0);
                let width = self.uvm_hdl_formal_width(sv_name);
                let got = self.uvm_hdl_get(c_name, &path, width);
                let ok = got.is_some();
                if let Some(out) = args.get(1) {
                    // The C code clears the value buffer before looking up.
                    self.assign_value(out, &got.unwrap_or_else(|| Value::zero(width)));
                }
                int_val(ok as i32)
            }
            "uvm_hdl_deposit" | "uvm_hdl_force" => {
                let path = self.uvm_dpi_str(args, 0);
                let width = self.uvm_hdl_formal_width(sv_name);
                let mut v = match args.get(1) {
                    Some(e) => self.eval_expr(e),
                    None => Value::zero(width),
                };
                if v.is_real {
                    v = Value::from_u64(v.to_f64().round() as i64 as u64, 64);
                    v.is_signed = true;
                }
                // Argument passing converts the actual to `uvm_hdl_data_t`.
                let mut v = v.resize_for_assign(width);
                v.is_signed = false;
                let ok = self.uvm_hdl_put(c_name, &path, v, c_name == "uvm_hdl_force");
                int_val(ok as i32)
            }
            "uvm_hdl_release" | "uvm_hdl_release_and_read" => {
                let path = self.uvm_dpi_str(args, 0);
                let mut ok = self.uvm_hdl_release(c_name, &path);
                if ok && c_name == "uvm_hdl_release_and_read" {
                    let width = self.uvm_hdl_formal_width(sv_name);
                    let got = self.uvm_hdl_get(c_name, &path, width);
                    ok = got.is_some();
                    if let Some(out) = args.get(1) {
                        self.assign_value(out, &got.unwrap_or_else(|| Value::zero(width)));
                    }
                }
                int_val(ok as i32)
            }
            "uvm_hdl_signal_size" => {
                let path = self.uvm_dpi_str(args, 0);
                int_val(self.uvm_hdl_signal_size(&path) as i32)
            }
            "uvm_polling_create" => {
                let name = self.uvm_dpi_str(args, 0);
                let key = self.uvm_dpi_int(args, 1);
                chandle_val(self.uvm_polling_create(&name, key))
            }
            "uvm_polling_set_enable_callback" => {
                let h = self.uvm_dpi_u64(args, 0);
                let enable = self.uvm_dpi_int(args, 1) != 0;
                self.uvm_polling_set_enable(h, enable);
                Value::zero(32)
            }
            "uvm_polling_get_callback_enable" => {
                let h = self.uvm_dpi_u64(args, 0);
                let on = match self.uvm_dpi.polls.get(&h) {
                    Some(p) => p.enabled,
                    None => {
                        self.uvm_polling_bad_handle();
                        false
                    }
                };
                int_val(on as i32)
            }
            "uvm_polling_setup_notifier" => {
                let name = self.uvm_dpi_str(args, 0);
                int_val(self.uvm_polling_setup_notifier(&name) as i32)
            }
            "uvm_polling_process_changelist" => {
                self.uvm_polling_process_changelist();
                Value::zero(32)
            }
            _ => return None,
        };
        Some(v)
    }

    /// uvm_hdl_polling.c `uvm_hdl_signal_size`: the width at `path`, 0 with
    /// an error when nothing is there.
    fn uvm_hdl_signal_size(&mut self, path: &str) -> u32 {
        match self.uvm_hdl_lookup(path) {
            HdlLookup::Found(t) => match &t.bits {
                Some(bits) => bits.len() as u32,
                None => self.uvm_hdl_width(&t.obj),
            },
            HdlLookup::Unsupported(why) => {
                self.uvm_dpi_note(format!("uvm_hdl_signal_size(\"{}\"): {}", path, why));
                0
            }
            HdlLookup::Scope | HdlLookup::Missing => {
                let msg = format!("uvm_hdl_signal_size : Cannot find name '{}'", path);
                self.uvm_dpi_report(M_UVM_ERROR, "UVM/DPI/HDL_POLLING", &msg, M_UVM_NONE);
                0
            }
        }
    }

    /// The probed bits of `t` as they are now.
    fn uvm_polling_sample(&self, t: &HdlTarget) -> Value {
        let cur = self.uvm_hdl_value(&t.obj);
        match &t.bits {
            Some(bits) => {
                let mut v = Value::zero(bits.len().max(1) as u32);
                for (i, &b) in bits.iter().enumerate() {
                    v.set_bit_code(i, cur.get_bit_code(b as usize));
                }
                v
            }
            None => cur,
        }
    }

    /// uvm_hdl_polling.c `uvm_polling_create`: a probe on the signal at
    /// `name` reporting as `sv_key`, or null (with the C code's notes).
    fn uvm_polling_create(&mut self, name: &str, sv_key: i64) -> u64 {
        let t = match self.uvm_hdl_lookup(name) {
            HdlLookup::Found(t) => t,
            HdlLookup::Unsupported(why) => {
                self.uvm_dpi_note(format!("uvm_polling_create(\"{}\"): {}", name, why));
                return 0;
            }
            HdlLookup::Scope => {
                let msg = format!(
                    "uvm_hdl_polling_create(\"{}\"): object is not a variable or net of integral type\n",
                    name
                );
                self.uvm_dpi_report(M_UVM_INFO, "UVM/DPI/HDL_POLLING", &msg, M_UVM_MEDIUM);
                return 0;
            }
            HdlLookup::Missing => {
                let msg = format!(
                    "uvm_polling_create: create(\"{}\") could not locate requested signal\n",
                    name
                );
                self.uvm_dpi_report(M_UVM_INFO, "UVM/DPI/HDL_POLLING", &msg, M_UVM_MEDIUM);
                return 0;
            }
        };
        // Value changes are observed on signal-table writes only.
        let slot = match t.obj {
            HdlObj::Slot(id) if !is_packed_id(id) => id,
            _ => {
                self.uvm_dpi_note(format!(
                    "uvm_polling_create(\"{}\"): value changes of this storage cannot be observed",
                    name
                ));
                return 0;
            }
        };
        let last = self.uvm_polling_sample(&t);
        self.uvm_dpi.next_handle += 1;
        let h = self.uvm_dpi.next_handle;
        self.uvm_dpi.polls.insert(
            h,
            PollProbe {
                target: t,
                slot,
                sv_key,
                enabled: false,
                last,
            },
        );
        h
    }

    /// chandle_to_hook's complaint about a handle no probe owns.
    fn uvm_polling_bad_handle(&mut self) {
        self.uvm_dpi_report(
            M_UVM_FATAL,
            "UVM/DPI/HDL_POLLING",
            "Bad chandle argument is not a valid created hook",
            M_UVM_NONE,
        );
    }

    /// uvm_polling_set_enable_callback: add or remove the probe's
    /// value-change callback.
    fn uvm_polling_set_enable(&mut self, h: u64, enable: bool) {
        let Some(p) = self.uvm_dpi.polls.get_mut(&h) else {
            self.uvm_polling_bad_handle();
            return;
        };
        if p.enabled == enable {
            return;
        }
        p.enabled = enable;
        let slot = p.slot;
        let routine = uvm_polling_value_change as *const () as usize;
        if enable {
            let t = p.target.clone();
            let now = self.uvm_polling_sample(&t);
            if let Some(p) = self.uvm_dpi.polls.get_mut(&h) {
                p.last = now;
            }
            self.vpi_cb_add_internal_watch(slot, routine, h as usize);
        } else {
            self.vpi_cb_remove_internal_watch(slot, routine, h as usize);
        }
    }

    /// A write to an enabled probe's slot: queue the probe if its bits
    /// changed, toggling the notifier for the first queued change.
    fn uvm_polling_changed(&mut self, h: u64) {
        let Some(p) = self.uvm_dpi.polls.get(&h) else {
            return;
        };
        let t = p.target.clone();
        let now = self.uvm_polling_sample(&t);
        let Some(p) = self.uvm_dpi.polls.get_mut(&h) else {
            return;
        };
        if now == p.last {
            return;
        }
        p.last = now;
        // Nothing listens before the notifier is set up (UVM does so at
        // time 0, in a process forked when the first probe is created).
        if self.uvm_dpi.poll_notifier.is_none() || self.uvm_dpi.poll_changes.contains(&h) {
            return;
        }
        let first = self.uvm_dpi.poll_changes.is_empty();
        self.uvm_dpi.poll_changes.push(h);
        if first {
            self.uvm_polling_toggle_notifier();
        }
    }

    /// Flip the notifier bit so `@(notifier)` wakes the changelist
    /// service.
    fn uvm_polling_toggle_notifier(&mut self) {
        let Some(id) = self.uvm_dpi.poll_notifier else {
            return;
        };
        let one = self.signal_table[id].get_bit(0) == LogicBit::One;
        let v = Value::from_u64(!one as u64, 1);
        // The VPI deposit path: an external write that must wake `@(notifier)`.
        write_sig!(self, id, v);
        self.after_signal_write(id);
        if id < self.dirty_signals.len() && !self.dirty_signals[id] {
            self.dirty_signals[id] = true;
            self.dirty_list.push(id);
        }
        self.dirty_any = true;
    }

    /// uvm_polling_setup_notifier: `name` must be a `bit` variable (UVM
    /// passes `uvm_polling_pkg.notifier`).
    fn uvm_polling_setup_notifier(&mut self, name: &str) -> bool {
        let slot = match self.uvm_hdl_lookup(name) {
            HdlLookup::Found(HdlTarget {
                obj: HdlObj::Slot(id),
                bits: None,
                key,
            }) if !is_packed_id(id) => Some((id, key)),
            _ => self.uvm_polling_package_var(name),
        };
        let Some((id, key)) = slot else {
            self.uvm_dpi_report(
                M_UVM_ERROR,
                "UVM/DPI/HDL_POLLING",
                "uvm_polling_setup_notifier() could not locate requested signal",
                M_UVM_NONE,
            );
            return false;
        };
        // vpiBitVar: one bit of a 2-state type.
        let two_state = self.signal_two_state.get(id).copied().unwrap_or(false)
            || self.module.var_decl_types.get(&key).is_some_and(|dt| {
                crate::compiler::elaborate::is_type_two_state_resolved(
                    dt,
                    &self.module.typedef_types,
                )
            });
        if self.signal_widths[id] != 1 || !two_state {
            self.uvm_dpi_report(
                M_UVM_ERROR,
                "UVM/DPI/HDL_POLLING",
                "uvm_polling_setup_notifier: object is not a bit variable",
                M_UVM_NONE,
            );
            return false;
        }
        self.uvm_dpi.poll_notifier = Some(id);
        true
    }

    /// A package variable named `pkg.var` or `pkg::var`. Package variables
    /// are stored under their bare name unless another package declares
    /// the same one.
    fn uvm_polling_package_var(&self, name: &str) -> Option<(usize, String)> {
        let (pkg, var) = name.split_once("::").or_else(|| name.split_once('.'))?;
        if !self.module.packages.contains(pkg) || var.contains('.') {
            return None;
        }
        let qualified = format!("{}::{}", pkg, var);
        if let Some(&id) = self.signal_name_to_id.get(qualified.as_str()) {
            return Some((id, qualified));
        }
        if self.pkg_ambiguous_names.contains(var) {
            return None;
        }
        let id = self.signal_name_to_id.get(var).copied()?;
        Some((id, var.to_string()))
    }

    /// uvm_polling_process_changelist: notify SV of each queued probe,
    /// newest first, until the list is empty.
    fn uvm_polling_process_changelist(&mut self) {
        while let Some(h) = self.uvm_dpi.poll_changes.pop() {
            let Some(key) = self.uvm_dpi.polls.get(&h).map(|p| p.sv_key) else {
                continue;
            };
            let Some(fd) = self
                .fn_decl_rc("uvm_polling_value_change_notify")
                .or_else(|| self.fn_decl_rc("uvm_pkg::uvm_polling_value_change_notify"))
            else {
                self.uvm_dpi_note(
                    "uvm_polling_process_changelist: the design exports no \
                     uvm_polling_value_change_notify"
                        .to_string(),
                );
                self.uvm_dpi.poll_changes.clear();
                return;
            };
            self.exec_function_call(&fd, &[int_lit(key)]);
        }
    }

    fn uvm_dpi_str(&mut self, args: &[Expression], i: usize) -> String {
        match args.get(i) {
            Some(e) => self.eval_expr(e).to_sv_string(),
            None => String::new(),
        }
    }

    fn uvm_dpi_int(&mut self, args: &[Expression], i: usize) -> i64 {
        match args.get(i) {
            Some(e) => self.eval_expr(e).to_i64().unwrap_or(0),
            None => 0,
        }
    }

    fn uvm_dpi_u64(&mut self, args: &[Expression], i: usize) -> u64 {
        match args.get(i) {
            Some(e) => self.eval_expr(e).to_u64().unwrap_or(0),
            None => 0,
        }
    }

    /// uvm_common.c `m_uvm_report_dpi`: report through UVM's exported
    /// `m__uvm_report_dpi`. UVM 1.1d exports none; its C code printed.
    fn uvm_dpi_report(&mut self, severity: i64, id: &str, msg: &str, verbosity: i64) {
        let fd = self
            .fn_decl_rc("m__uvm_report_dpi")
            .or_else(|| self.fn_decl_rc("uvm_pkg::m__uvm_report_dpi"));
        if let Some(fd) = fd {
            let args = [
                int_lit(severity),
                str_lit(id),
                str_lit(msg),
                int_lit(verbosity),
                str_lit(""),
                int_lit(0),
            ];
            self.exec_function_call(&fd, &args);
            return;
        }
        let line = if severity == M_UVM_INFO {
            msg.to_string()
        } else {
            let sev = ["UVM_INFO", "UVM_WARNING", "UVM_ERROR", "UVM_FATAL"];
            format!("{}: {}", sev[severity.clamp(0, 3) as usize], msg)
        };
        self.stdout_writeln(&line);
        self.record_output(line);
    }

    /// Print a `[DPI]` error once per distinct text.
    fn uvm_dpi_note(&mut self, msg: String) {
        if self.uvm_dpi.noted.insert(msg.clone()) {
            eprintln!("[DPI] error: {}", msg);
        }
    }

    fn uvm_dpi_new_handle(&mut self, re: PosixRe) -> u64 {
        self.uvm_dpi.next_handle += 1;
        let h = self.uvm_dpi.next_handle;
        self.uvm_dpi.handles.insert(h, re);
        h
    }

    fn uvm_dpi_handle_exec(&mut self, c_name: &str, h: u64, s: &str) -> i32 {
        if h == 0 {
            return 1;
        }
        match self.uvm_dpi.handles.get(&h) {
            Some(re) => re.exec(s),
            None => {
                self.uvm_dpi_note(format!(
                    "{}: chandle {:#x} is not a live compiled regular expression",
                    c_name, h
                ));
                1
            }
        }
    }

    /// uvm_regex.cc `uvm_re_match`: 0 when `s` matches, else regexec's or
    /// regcomp's nonzero code.
    fn uvm_re_match(&mut self, re: &str, s: &str) -> i32 {
        if re.len() > UVM_REGEX_MAX_LENGTH {
            let msg = format!(
                "uvm_re_match : regular expression greater than max {}: |{}|",
                UVM_REGEX_MAX_LENGTH, re
            );
            self.uvm_dpi_report(M_UVM_ERROR, "UVM/DPI/REGEX_MAX", &msg, M_UVM_NONE);
            return 1;
        }
        let rex = strip_re_brackets(re);
        if !self.uvm_dpi.re_cache.contains_key(rex) {
            if self.uvm_dpi.re_cache.len() >= RE_CACHE_MAX {
                self.uvm_dpi.re_cache.clear();
            }
            let compiled = PosixRe::compile(rex, libc::REG_EXTENDED);
            self.uvm_dpi.re_cache.insert(rex.to_string(), compiled);
        }
        let (code, text) = match self.uvm_dpi.re_cache.get(rex) {
            Some(Ok(compiled)) => return compiled.exec(s),
            Some(Err((code, text))) => (*code, text.clone()),
            None => return 1,
        };
        let msg = format!(
            "uvm_re_match : invalid glob or regular expression: |{}||{}|",
            re, text
        );
        self.uvm_dpi.re_buffer = text;
        self.uvm_dpi_report(M_UVM_ERROR, "UVM/DPI/REGEX_INV", &msg, M_UVM_NONE);
        code
    }

    /// uvm_regex.cc (1800.2-2020.3) `uvm_re_deglobbed`: the glob as a
    /// regex, with or without the `/.../` brackets; `None` (the C code's
    /// NULL) when the expansion exceeds the buffer, with the reason in
    /// `uvm_re_buffer`. Like the C code's static buffer, `uvm_re_buffer`
    /// then holds the result.
    fn uvm_re_deglobbed(&mut self, glob: &str, with_brackets: bool) -> Option<String> {
        if glob.is_empty() || glob == "/" {
            return Some(if with_brackets { "/^$/" } else { "^$" }.to_string());
        }
        let b = glob.as_bytes();
        let bracketed = b.len() > 1 && b[0] == b'/' && b[b.len() - 1] == b'/';
        let expanded = if bracketed {
            glob.len()
        } else {
            // The C code's running count: brackets, `^`, two per `*`, `+`
            // and escaped character, `$`, and the terminating NUL.
            glob.len()
                + usize::from(with_brackets) * 2
                + usize::from(b[0] != b'^')
                + b.iter()
                    .filter(|c| matches!(c, b'*' | b'+' | b'.' | b'[' | b']' | b'(' | b')'))
                    .count()
                    * 2
                + usize::from(b[b.len() - 1] != b'$')
                + 1
        };
        if expanded > UVM_REGEX_MAX_LENGTH {
            self.uvm_dpi.re_buffer = format!(
                "uvm_glob_to_re() expansion exceeds max length({})",
                UVM_REGEX_MAX_LENGTH
            );
            return None;
        }
        let re = deglob(glob, with_brackets);
        self.uvm_dpi.re_buffer = re.clone();
        Some(re)
    }

    /// uvm_regex.cc (1800.2-2020.3) `uvm_re_comp`: a chandle to the
    /// compiled pattern (brackets stripped, or the glob converted when
    /// `deglobbed`), or null with the reason in `uvm_re_buffer`.
    fn uvm_re_comp(&mut self, re: &str, deglobbed: bool) -> u64 {
        let rex = if deglobbed {
            match self.uvm_re_deglobbed(re, false) {
                Some(r) => r,
                None => return 0,
            }
        } else if re.len() > UVM_REGEX_MAX_LENGTH {
            self.uvm_dpi.re_buffer = format!(
                "uvm_re_comp() re exceeds max length ({})",
                UVM_REGEX_MAX_LENGTH
            );
            return 0;
        } else {
            let stripped = strip_re_brackets(re);
            if stripped.len() != re.len() {
                self.uvm_dpi.re_buffer = stripped.to_string();
            }
            stripped.to_string()
        };
        match PosixRe::compile(&rex, libc::REG_EXTENDED) {
            Ok(compiled) => self.uvm_dpi_new_handle(compiled),
            Err((_, text)) => {
                self.uvm_dpi.re_buffer = text;
                0
            }
        }
    }

    /// uvm_regex.cc `uvm_glob_to_re`.
    fn uvm_glob_to_re(&mut self, glob: &str) -> String {
        if glob.len() > 2040 {
            let msg = format!(
                "uvm_re_match : glob expression greater than max 2040: |{}|",
                glob
            );
            self.uvm_dpi_report(M_UVM_ERROR, "UVM/DPI/REGEX_MAX", &msg, M_UVM_NONE);
            return glob.to_string();
        }
        if glob.is_empty() || glob == "/" {
            // UVM 1.2's C returns an empty string (a regex that matches
            // anything); 1800.2 returns one that matches only "".
            let legacy = match self.uvm_dpi.legacy_regex {
                Some(l) => l,
                None => {
                    let l = self
                        .module
                        .dpi_imports
                        .values()
                        .any(|s| s.c_name == "uvm_dump_re_cache");
                    self.uvm_dpi.legacy_regex = Some(l);
                    l
                }
            };
            return if legacy {
                String::new()
            } else {
                "/^$/".to_string()
            };
        }
        deglob(glob, true)
    }

    /// uvm_svcmd_dpi.c `uvm_dpi_get_next_arg_c`: the next argv entry, "" at
    /// the end. UVM 1.2 and later pass `init` (1 restarts the walk); UVM
    /// 1.1d passes nothing and the walk restarts after its end. The C walk
    /// splices `-f` file contents in place; the argv here already carries
    /// them, so a `-f`/`-F` flag and its operand are skipped.
    fn uvm_dpi_next_arg(&mut self, args: &[Expression]) -> String {
        let restart_at_end = match args.first() {
            Some(e) => {
                if self.eval_expr(e).to_i64() == Some(1) {
                    self.uvm_dpi.argv_idx = 0;
                }
                false
            }
            None => true,
        };
        loop {
            let idx = self.uvm_dpi.argv_idx;
            let Some(c) = self.vpi_arg_cstrings.get(idx) else {
                if restart_at_end {
                    self.uvm_dpi.argv_idx = 0;
                }
                return String::new();
            };
            let s = c.to_string_lossy().into_owned();
            self.uvm_dpi.argv_idx += 1;
            if s == "-f" || s == "-F" {
                self.uvm_dpi.argv_idx += 1;
                continue;
            }
            return s;
        }
    }

    /// Width of import `sv_name`'s `uvm_hdl_data_t` formal
    /// (`UVM_HDL_MAX_WIDTH`, 1024 unless redefined).
    fn uvm_hdl_formal_width(&self, sv_name: &str) -> u32 {
        let from_proto = self.module.dpi_imports.get(sv_name).and_then(|spec| {
            let ports = match &spec.proto {
                crate::ast::decl::DPIProto::Function(fd) => &fd.ports,
                crate::ast::decl::DPIProto::Task(td) => &td.ports,
            };
            let w = resolve_type_width(
                &ports.get(1)?.data_type,
                Some(&self.module.parameters),
                Some(&self.module.typedefs),
            );
            (w > 1).then_some(w)
        });
        from_proto
            .or_else(|| {
                ["UVM_HDL_MAX_WIDTH", "uvm_pkg::UVM_HDL_MAX_WIDTH"]
                    .iter()
                    .find_map(|p| self.module.parameters.get(*p))
                    .and_then(|v| v.to_u64())
                    .map(|w| w as u32)
                    .filter(|&w| w > 0)
            })
            .unwrap_or(1024)
    }

    /// Resolve a `uvm_hdl_*` path. Paths are absolute, as for
    /// `vpi_handle_by_name` with no scope: `top.u.sig`, optionally prefixed
    /// with `$root.`; with several top modules each one roots its own tree.
    fn uvm_hdl_lookup(&self, path: &str) -> HdlLookup {
        let p = path.trim();
        let p = p.strip_prefix("$root.").unwrap_or(p);
        let top = self.module.name.as_str();
        if top == "__xezim_multi_top" {
            let head = p.split(['.', '[']).next().unwrap_or("");
            let is_top = self
                .module
                .instances
                .iter()
                .any(|i| i.path == head && !i.path.contains('.'));
            if !is_top {
                return HdlLookup::Missing;
            }
            return self.uvm_hdl_lookup_rel(p);
        }
        if p == top {
            return HdlLookup::Scope;
        }
        match p.strip_prefix(top).and_then(|r| r.strip_prefix('.')) {
            Some(rel) => self.uvm_hdl_lookup_rel(rel),
            None => HdlLookup::Missing,
        }
    }

    /// Resolve a top-relative path (the signal table's naming).
    fn uvm_hdl_lookup_rel(&self, name: &str) -> HdlLookup {
        if name.is_empty() {
            return HdlLookup::Missing;
        }
        if self.module.instances.iter().any(|i| i.path == name) {
            return HdlLookup::Scope;
        }
        if let Some(obj) = self.uvm_hdl_whole(name) {
            return self.uvm_hdl_check_storage(HdlTarget {
                obj,
                key: name.to_string(),
                bits: None,
            });
        }
        if let Some((base, off, w)) = self.packed_leaf_of_hier(name) {
            if let Some(obj) = self.uvm_hdl_whole(&base) {
                return self.uvm_hdl_check_storage(HdlTarget {
                    obj,
                    key: base,
                    bits: Some((off..off + w).collect()),
                });
            }
        }
        // A trailing bit-select or part-select: `sig[3]`, `sig[7:4]`,
        // `mem[2][7:0]`.
        let Some(inner) = name.strip_suffix(']') else {
            return HdlLookup::Missing;
        };
        let Some(open) = inner.rfind('[') else {
            return HdlLookup::Missing;
        };
        let (base, sel) = (&inner[..open], &inner[open + 1..]);
        let labels: Vec<i64> = match sel.split_once(':') {
            Some((l, r)) => {
                let (Ok(l), Ok(r)) = (l.trim().parse::<i64>(), r.trim().parse::<i64>()) else {
                    return HdlLookup::Missing;
                };
                // The C code walks from the right bound toward the left one:
                // value bit i is label `r + i` (`r - i` when l < r).
                let step = if l > r { 1 } else { -1 };
                (0..=(l - r).abs()).map(|i| r + i * step).collect()
            }
            None => match sel.trim().parse::<i64>() {
                Ok(i) => vec![i],
                Err(_) => return HdlLookup::Missing,
            },
        };
        let parent = match self.uvm_hdl_lookup_rel(base) {
            HdlLookup::Found(t) => t,
            HdlLookup::Scope => return HdlLookup::Missing,
            other => return other,
        };
        // A part-select must run in the declared direction (§11.5.1); the
        // reference rejects `v[4:7]` of a `[11:4]` vector.
        let ascending_sel = labels.len() > 1 && labels[0] > labels[1];
        let physical: Option<Vec<u32>> = match &parent.bits {
            // A select inside a struct member numbers the member's bits.
            Some(_) if ascending_sel => None,
            Some(bits) => labels
                .iter()
                .map(|&l| usize::try_from(l).ok().and_then(|l| bits.get(l).copied()))
                .collect(),
            None => {
                let width = self.uvm_hdl_width(&parent.obj);
                let decl_base = parent.key.split('[').next().unwrap_or(&parent.key);
                if self
                    .module
                    .packed_full_dims
                    .get(decl_base)
                    .is_some_and(|d| d.len() > 1)
                {
                    return HdlLookup::Unsupported(format!(
                        "'{}' selects into a multi-dimensional packed vector",
                        name
                    ));
                }
                let (l, r) = self
                    .dump_var_range(&parent.key, width)
                    .unwrap_or((width as i64 - 1, 0));
                if ascending_sel != (l < r) && labels.len() > 1 {
                    return HdlLookup::Missing;
                }
                labels
                    .iter()
                    .map(|&label| {
                        let pos = if l >= r {
                            (r..=l).contains(&label).then(|| label - r)
                        } else {
                            (l..=r).contains(&label).then(|| r - label)
                        };
                        pos.map(|p| p as u32)
                    })
                    .collect()
            }
        };
        match physical {
            Some(bits) => HdlLookup::Found(HdlTarget {
                bits: Some(bits),
                ..parent
            }),
            None => HdlLookup::Missing,
        }
    }

    /// Whole-signal storage for a top-relative name.
    fn uvm_hdl_whole(&self, name: &str) -> Option<HdlObj> {
        if let Some(&id) = self.signal_name_to_id.get(name) {
            return Some(HdlObj::Slot(id));
        }
        if let Some(id) = resolve_array_elem_id(name, &self.array_first_id) {
            return Some(HdlObj::Slot(id));
        }
        if self.signals.contains_key(name) {
            return Some(HdlObj::Named(name.to_string()));
        }
        None
    }

    fn uvm_hdl_check_storage(&self, t: HdlTarget) -> HdlLookup {
        match &t.obj {
            HdlObj::Slot(id) if is_packed_id(*id) => HdlLookup::Found(t),
            HdlObj::Slot(id)
                if self.signal_real.get(*id).copied().unwrap_or(false)
                    || self.signal_is_string.get(*id).copied().unwrap_or(false)
                    || self.signal_widths.get(*id).copied().unwrap_or(0) == 0 =>
            {
                HdlLookup::Unsupported(format!("'{}' is not an integral signal", t.key))
            }
            HdlObj::Named(n) if self.signals.get(n).is_some_and(|v| v.is_real) => {
                HdlLookup::Unsupported(format!("'{}' is not an integral signal", t.key))
            }
            _ => HdlLookup::Found(t),
        }
    }

    fn uvm_hdl_width(&self, obj: &HdlObj) -> u32 {
        match obj {
            HdlObj::Slot(id) if is_packed_id(*id) => self.packed.width(*id),
            HdlObj::Slot(id) => self.signal_widths[*id],
            HdlObj::Named(n) => self.signals.get(n).map_or(0, |v| v.width),
        }
    }

    fn uvm_hdl_value(&self, obj: &HdlObj) -> Value {
        match obj {
            HdlObj::Slot(id) if is_packed_id(*id) => {
                let (v, x, w) = self.packed.raw(*id);
                Value::from_inline(v, x, w)
            }
            HdlObj::Slot(id) => self.signal_table[*id].clone(),
            HdlObj::Named(n) => self
                .signals
                .get(n)
                .cloned()
                .unwrap_or_else(|| Value::zero(1)),
        }
    }

    /// Blocking write of a whole signal (a 2-state one drops x/z; a forced
    /// one keeps its forced value).
    fn uvm_hdl_store(&mut self, obj: &HdlObj, v: &Value) {
        match obj {
            HdlObj::Slot(id) => {
                self.fast_signal_write_id(*id, v);
            }
            HdlObj::Named(n) => {
                let w = self.uvm_hdl_width(obj);
                let n = n.clone();
                self.set_signal_value_by_name(&n, v.resize(w));
            }
        }
    }

    fn uvm_hdl_not_found(&mut self, id: &str, verb: &str, path: &str) {
        let msg = format!(
            "{}: unable to locate hdl path ({})\n Either the name is incorrect, or you may not have PLI/ACC visibility to that name",
            verb, path
        );
        self.uvm_dpi_report(M_UVM_ERROR, id, &msg, M_UVM_NONE);
    }

    /// uvm_hdl_get_vlog: the value at `path`, zero-extended to `width`.
    fn uvm_hdl_get(&mut self, c_name: &str, path: &str, width: u32) -> Option<Value> {
        let t = match self.uvm_hdl_lookup(path) {
            HdlLookup::Found(t) => t,
            HdlLookup::Unsupported(why) => {
                self.uvm_dpi_note(format!("{}(\"{}\"): {}", c_name, path, why));
                return None;
            }
            HdlLookup::Scope | HdlLookup::Missing => {
                self.uvm_hdl_not_found("UVM/DPI/HDL_GET", "get", path);
                return None;
            }
        };
        let cur = self.uvm_hdl_value(&t.obj);
        match &t.bits {
            Some(bits) => {
                let mut out = Value::zero(width);
                for (i, &b) in bits.iter().enumerate().take(width as usize) {
                    out.set_bit_code(i, cur.get_bit_code(b as usize));
                }
                Some(out)
            }
            None => {
                if cur.width > width {
                    let msg = format!(
                        "uvm_reg : hdl path '{}' is {} bits, but the maximum size is {}.  You can increase the maximum via a compile-time flag: +define+UVM_HDL_MAX_WIDTH=<value>",
                        path, cur.width, width
                    );
                    self.uvm_dpi_report(M_UVM_ERROR, "UVM/DPI/HDL_SET", &msg, M_UVM_NONE);
                    return None;
                }
                let mut cur = cur;
                cur.is_signed = false;
                Some(cur.resize(width))
            }
        }
    }

    /// uvm_hdl_set_vlog for a deposit (`force` false) or a force.
    fn uvm_hdl_put(&mut self, c_name: &str, path: &str, v: Value, force: bool) -> bool {
        let t = match self.uvm_hdl_lookup(path) {
            HdlLookup::Found(t) => t,
            HdlLookup::Unsupported(why) => {
                self.uvm_dpi_note(format!("{}(\"{}\"): {}", c_name, path, why));
                return false;
            }
            HdlLookup::Scope | HdlLookup::Missing => {
                self.uvm_hdl_not_found("UVM/DPI/HDL_SET", "set", path);
                return false;
            }
        };
        if let Some(bits) = &t.bits {
            if force {
                self.uvm_dpi_note(format!(
                    "{}(\"{}\"): forcing part of a signal is not supported; nothing was forced",
                    c_name, path
                ));
                return false;
            }
            let mut next = self.uvm_hdl_value(&t.obj);
            for (i, &b) in bits.iter().enumerate() {
                next.set_bit_code(b as usize, v.get_bit_code(i));
            }
            self.uvm_hdl_store(&t.obj, &next);
            return true;
        }
        if force {
            // A second force replaces the first (§10.6.2): lift it so the
            // guarded write below lands, then re-arm with the new value.
            if let HdlObj::Slot(id) = t.obj {
                self.unforce_cell(id);
            }
            self.forced_names.remove(&t.key);
            let key = t.key.clone();
            self.active_force_exprs.retain(|entry| entry.key != key);
        }
        self.uvm_hdl_store(&t.obj, &v);
        if force {
            match t.obj {
                HdlObj::Slot(id) => {
                    self.force_cell(id, v);
                }
                HdlObj::Named(_) => {
                    self.forced_names.insert(t.key);
                }
            }
        }
        true
    }

    /// uvm_hdl_set_vlog with vpiReleaseFlag.
    fn uvm_hdl_release(&mut self, c_name: &str, path: &str) -> bool {
        let t = match self.uvm_hdl_lookup(path) {
            HdlLookup::Found(t) => t,
            HdlLookup::Unsupported(why) => {
                self.uvm_dpi_note(format!("{}(\"{}\"): {}", c_name, path, why));
                return false;
            }
            HdlLookup::Scope | HdlLookup::Missing => {
                self.uvm_hdl_not_found("UVM/DPI/HDL_SET", "set", path);
                return false;
            }
        };
        if t.bits.is_some() {
            self.uvm_dpi_note(format!(
                "{}(\"{}\"): releasing part of a signal is not supported; nothing was released",
                c_name, path
            ));
            return false;
        }
        let id = match t.obj {
            HdlObj::Slot(id) => Some(id),
            HdlObj::Named(_) => None,
        };
        self.release_override(Some((t.key.as_str(), id)));
        true
    }
}
