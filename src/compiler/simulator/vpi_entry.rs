//! VPI and DPI C entry points: constants, handles, iteration, values, scopes, time and
//! multichannel descriptors.

use super::*;

// ============================================================================
// VPI (Verification Procedural Interface) Support
// ============================================================================

// VPI constants. Every value is the one IEEE 1800-2017 Annex K assigns
// it, and matches `include/vpi_user.h` symbol for symbol. These are not
// free parameters: a C file compiled against a vendor `vpi_user.h` and
// linked to xezim agrees with the numbers below or it silently takes the
// wrong branch.
pub(super) mod vpi {
    use libc::c_int;

    // Value formats (Table 38-44).
    pub const BIN_STR_VAL: c_int = 1;
    pub const OCT_STR_VAL: c_int = 2;
    pub const DEC_STR_VAL: c_int = 3;
    pub const HEX_STR_VAL: c_int = 4;
    pub const SCALAR_VAL: c_int = 5;
    pub const INT_VAL: c_int = 6;
    pub const REAL_VAL: c_int = 7;
    pub const STRING_VAL: c_int = 8;
    pub const VECTOR_VAL: c_int = 9;
    pub const TIME_VAL: c_int = 11;
    pub const OBJ_TYPE_VAL: c_int = 12;
    /// Set into `value_p->format` when a value cannot be supplied
    /// (§38.16). This is the only failure channel `vpi_get_value` has.
    pub const SUPPRESS_VAL: c_int = 13;

    // vpiScalarVal codes.
    pub const SCALAR_0: c_int = 0;
    pub const SCALAR_1: c_int = 1;
    pub const SCALAR_Z: c_int = 2;
    pub const SCALAR_X: c_int = 3;

    // vpi_put_value flags.
    pub const FORCE_FLAG: c_int = 5;
    pub const RELEASE_FLAG: c_int = 6;

    // vpi_get properties.
    pub const UNDEFINED: c_int = -1;
    pub const TYPE: c_int = 1;
    pub const NAME: c_int = 2;
    pub const FULL_NAME: c_int = 3;
    pub const SIZE: c_int = 4;
    /// §38.35 Table 38-4: simulation time unit / precision as a base-10
    /// exponent of seconds. Both are GLOBAL properties — the standard permits
    /// (and cocotb uses) a NULL object handle to ask for the simulation-wide
    /// value.
    pub const TIME_UNIT: c_int = 11;
    pub const TIME_PRECISION: c_int = 12;
    pub const DEF_NAME: c_int = 9;
    pub const SCALAR: c_int = 17;
    pub const VECTOR: c_int = 18;
    pub const SIGNED: c_int = 65;
    pub const DIRECTION: c_int = 20;
    pub const INPUT: c_int = 1;
    pub const OUTPUT: c_int = 2;
    pub const INOUT: c_int = 3;
    #[allow(dead_code)] // Reserved VPI ABI value; retained alongside supported values.
    pub const MIXED_IO: c_int = 4;
    pub const NO_DIRECTION: c_int = 5;

    // vpi_control operations.
    pub const STOP: c_int = 66;
    pub const FINISH: c_int = 67;
    pub const RESET: c_int = 68;

    // vpi_chk_error levels and states.
    #[allow(dead_code)] // Reserved VPI ABI value; retained alongside supported values.
    pub const NOTICE: c_int = 1;
    pub const WARNING: c_int = 2;
    pub const ERROR: c_int = 3;
    pub const RUN_STATE: c_int = 3;

    // System-function return kinds (`s_vpi_systf_data.sysfunctype`).
    #[allow(dead_code)] // Reserved VPI ABI value; retained alongside supported values.
    pub const INT_FUNC: c_int = 1;
    pub const REAL_FUNC: c_int = 2;
    pub const TIME_FUNC: c_int = 3;
    pub const SIZED_FUNC: c_int = 4;
    pub const SIZED_SIGNED_FUNC: c_int = 5;

    // Object types returned by vpi_get(vpiType), and traversal relations.
    pub const CONSTANT: c_int = 7;
    pub const ITERATOR: c_int = 27;
    pub const FUNC_TYPE: c_int = 44;
    pub const SYS_FUNC_CALL: c_int = 56;
    pub const SYS_TASK_CALL: c_int = 57;
    pub const SYS_TF_CALL: c_int = 85;
    pub const ARGUMENT: c_int = 89;
    pub const MEMORY: c_int = 29;
    pub const MODULE: c_int = 32;
    /// The generic "instance" class (module/program/interface/package). It is
    /// a SystemVerilog extension, so it lives in `sv_vpi_user.h` at 745 — NOT
    /// in `vpi_user.h`. cocotb asks for this, not vpiModule, when discovering
    /// the design root.
    pub const INSTANCE: c_int = 745;
    pub const PARAMETER: c_int = 41;
    pub const PART_SELECT: c_int = 42;
    pub const SCOPE: c_int = 84;
    pub const INTERNAL_SCOPE: c_int = 92;
    pub const VARIABLES: c_int = 100;
    pub const INTEGER_VAR: c_int = 25;
    pub const NET: c_int = 36;
    pub const PORT: c_int = 44;
    pub const PORT_BIT: c_int = 45;
    pub const REAL_VAR: c_int = 47;
    /// Also `vpiLogicVar` — the standard aliases them.
    pub const REG: c_int = 48;
    pub const TIME_VAR: c_int = 63;
    pub const LONG_INT_VAR: c_int = 610;
    pub const SHORT_INT_VAR: c_int = 611;
    pub const INT_VAR: c_int = 612;
    pub const SHORT_REAL_VAR: c_int = 613;
    pub const BYTE_VAR: c_int = 614;
    pub const STRING_VAR: c_int = 616;
    pub const ENUM_VAR: c_int = 617;
    pub const STRUCT_VAR: c_int = 618;
    pub const UNION_VAR: c_int = 619;
    pub const BIT_VAR: c_int = 620;

    // Time types.
    pub const SCALED_REAL_TIME: c_int = 1;
    pub const SIM_TIME: c_int = 2;
    pub const SUPPRESS_TIME: c_int = 3;

    // s_vpi_systf_data.type
    pub const SYS_TASK: c_int = 1;
    pub const SYS_FUNC: c_int = 2;

    // Callback reasons (Table 38-49).
    #[allow(dead_code)] // Reserved VPI ABI value; retained alongside supported values.
    pub const CB_VALUE_CHANGE: c_int = 1;
    #[allow(dead_code)] // Reserved VPI ABI value; retained alongside supported values.
    pub const CB_NEXT_SIM_TIME: c_int = 8;
    #[allow(dead_code)] // Reserved VPI ABI value; retained alongside supported values.
    pub const CB_START_OF_SIMULATION: c_int = 11;
    #[allow(dead_code)] // Reserved VPI ABI value; retained alongside supported values.
    pub const CB_END_OF_SIMULATION: c_int = 12;
    #[allow(dead_code)] // Reserved VPI ABI value; retained alongside supported values.
    pub const CB_READ_WRITE_SYNCH: c_int = 6;
    #[allow(dead_code)] // Reserved VPI ABI value; retained alongside supported values.
    pub const CB_READ_ONLY_SYNCH: c_int = 7;
    #[allow(dead_code)] // Reserved VPI ABI value; retained alongside supported values.
    pub const CB_AFTER_DELAY: c_int = 9;
    #[allow(dead_code)] // Reserved VPI ABI value; retained alongside supported values.
    pub const CB_START_OF_RESET: c_int = 19;
}

// VPI types matching vpi_user.h. `aval`/`bval` are PLI_INT32 (signed) in
// the standard header, so mirror that exactly — the bit patterns are what
// matter, but the sizes must agree.
//
// Per-bit encoding (§38.10.1):
//     aval bval   value
//       0    0      0
//       1    0      1
//       0    1      Z
//       1    1      X
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct s_vpi_vecval {
    pub(super) aval: i32,
    pub(super) bval: i32,
}

#[repr(C)]
pub(super) union s_vpi_value_union {
    pub(super) str: *mut libc::c_char,
    pub(super) scalar: libc::c_int,
    pub(super) integer: libc::c_int,
    pub(super) real: libc::c_double,
    pub(super) time: *mut s_vpi_time,
    pub(super) vector: *mut s_vpi_vecval,
    pub(super) misc: *mut libc::c_char,
}

#[repr(C)]
pub struct s_vpi_value {
    pub(super) format: libc::c_int,
    pub(super) value: s_vpi_value_union,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct s_vpi_time {
    pub(super) type_: libc::c_int,
    pub(super) high: u32,
    pub(super) low: u32,
    pub(super) real: libc::c_double,
}

// Scratch storage handed back to C by `vpi_get_value` for the formats
// whose value is a pointer (vectors, strings, time). IEEE 1800-2017
// §38.16 lets the simulator own this memory and only guarantees it
// until the next `vpi_get_value` call, which is exactly what these
// give: each call overwrites the previous contents.
thread_local! {
    pub(super) static VPI_VEC_SCRATCH: std::cell::RefCell<Vec<s_vpi_vecval>> =
        const { std::cell::RefCell::new(Vec::new()) };
    static VPI_STR_SCRATCH: std::cell::RefCell<Vec<u8>> =
        const { std::cell::RefCell::new(Vec::new()) };
    // vpi_get_str() rotates through a pool rather than reusing one buffer.
    // `vpi_printf("%s %s", vpi_get_str(vpiName, h), vpi_get_str(vpiDefName, h))`
    // is idiomatic VPI: both pointers are live at once, and C leaves the
    // argument evaluation order unspecified. A single shared buffer makes
    // both strings read as whichever call happened to run last.
    pub(super) static VPI_STR2_SCRATCH: std::cell::RefCell<(usize, Vec<Vec<u8>>)> =
        std::cell::RefCell::new((0, vec![Vec::new(); 8]));
    pub(super) static VPI_TIME_SCRATCH: std::cell::RefCell<s_vpi_time> =
        const { std::cell::RefCell::new(s_vpi_time { type_: 2, high: 0, low: 0, real: 0.0 }) };
}

/// `Value` bit code (0=0, 1=1, 2=X, 3=Z) -> one `(aval, bval)` bit pair.
#[inline]
pub(super) fn bit_code_to_ab(code: u8) -> (u32, u32) {
    match code {
        0 => (0, 0),
        1 => (1, 0),
        2 => (1, 1), // X
        _ => (0, 1), // Z
    }
}

/// One `(aval, bval)` bit pair -> `Value` bit code.
#[inline]
pub(super) fn ab_to_bit_code(a: u32, b: u32) -> u8 {
    match (a, b) {
        (0, 0) => 0,
        (1, 0) => 1,
        (1, 1) => 2, // X
        _ => 3,      // Z
    }
}

/// Render a `Value` into `s_vpi_vecval` words, preserving X and Z.
pub(super) fn value_to_vecval(v: &Value, out: &mut Vec<s_vpi_vecval>) {
    let w = v.width.max(1) as usize;
    let words = w.div_ceil(32);
    out.clear();
    out.resize(words, s_vpi_vecval { aval: 0, bval: 0 });
    let mut aval = vec![0u32; words];
    let mut bval = vec![0u32; words];
    for i in 0..w {
        let (a, b) = bit_code_to_ab(v.get_bit_code(i));
        if a != 0 {
            aval[i / 32] |= 1u32 << (i % 32);
        }
        if b != 0 {
            bval[i / 32] |= 1u32 << (i % 32);
        }
    }
    for i in 0..words {
        out[i] = s_vpi_vecval {
            aval: aval[i] as i32,
            bval: bval[i] as i32,
        };
    }
}

/// Build a `Value` of `width` bits from `s_vpi_vecval` words, preserving
/// X and Z. Words beyond the slice read as zero.
pub(super) fn vecval_to_value(vec: &[s_vpi_vecval], width: u32, is_signed: bool) -> Value {
    let mut v = Value::zero(width);
    for i in 0..width as usize {
        let w = i / 32;
        if w >= vec.len() {
            break;
        }
        let a = (vec[w].aval as u32 >> (i % 32)) & 1;
        let b = (vec[w].bval as u32 >> (i % 32)) & 1;
        v.set_bit_code(i, ab_to_bit_code(a, b));
    }
    v.is_signed = is_signed;
    v
}

/// Radix string with 4-state digits, `bits_per_digit` of 1, 3 or 4.
/// A digit is `z` when every bit in it is Z, otherwise `x` when any bit
/// is unknown — the Verilog display rule.
/// Inverse of [`vpi_radix_string`]: decode a `vpi*StrVal` payload into a
/// `w`-bit `Value`.
///
/// `x`/`z` (and `?`, which the LRM accepts for z) cover a WHOLE digit, so one
/// `x` in a hex string is four x bits. Digits are read LSB-first from the right;
/// a string shorter than the signal left-extends with 0, longer is truncated.
/// Underscores and surrounding whitespace are ignored (§38.16).
pub(super) fn vpi_value_from_radix_str(s: &str, bits_per_digit: u32, w: u32) -> Value {
    let mut v = Value::zero(w);
    let mut bit: u32 = 0;
    for ch in s.chars().rev() {
        if bit >= w {
            break;
        }
        if ch == '_' || ch.is_whitespace() {
            continue;
        }
        let lc = ch.to_ascii_lowercase();
        let per_bit_code = match lc {
            'x' => Some(2u8),
            'z' | '?' => Some(3u8),
            _ => None,
        };
        match per_bit_code {
            Some(code) => {
                for k in 0..bits_per_digit {
                    if bit + k < w {
                        v.set_bit_code((bit + k) as usize, code);
                    }
                }
            }
            None => {
                // An unparsable digit is not silently a zero — leave the run
                // as x so a malformed payload is visible rather than wrong.
                let d = lc.to_digit(1u32 << bits_per_digit);
                for k in 0..bits_per_digit {
                    if bit + k >= w {
                        break;
                    }
                    let code = match d {
                        Some(dv) => ((dv >> k) & 1) as u8,
                        None => 2u8,
                    };
                    v.set_bit_code((bit + k) as usize, code);
                }
            }
        }
        bit += bits_per_digit;
    }
    v
}

pub(super) fn vpi_radix_string(v: &Value, bits_per_digit: usize) -> String {
    let w = v.width.max(1) as usize;
    let ndigits = w.div_ceil(bits_per_digit);
    let mut s = String::with_capacity(ndigits);
    for d in (0..ndigits).rev() {
        let mut digit = 0u32;
        let (mut xs, mut zs, mut n) = (0usize, 0usize, 0usize);
        for b in 0..bits_per_digit {
            let idx = d * bits_per_digit + b;
            if idx >= w {
                continue;
            }
            n += 1;
            match v.get_bit_code(idx) {
                1 => digit |= 1 << b,
                2 => xs += 1,
                3 => zs += 1,
                _ => {}
            }
        }
        if n > 0 && zs == n {
            s.push('z');
        } else if xs + zs > 0 {
            s.push('x');
        } else {
            s.push(char::from_digit(digit, 1u32 << bits_per_digit).unwrap_or('0'));
        }
    }
    if s.is_empty() {
        s.push('0');
    }
    s
}

// Thread-local pointer to the current simulator instance.
// Set during DPI import calls so VPI C callbacks can access it.
thread_local! {
    pub(super) static ACTIVE_SIMULATOR: std::cell::Cell<*mut Simulator> = const { std::cell::Cell::new(std::ptr::null_mut()) };
    /// Current DPI scope set by `svSetScope` and read by `svGetScope`.
    /// Cached as a raw pointer into the simulator's `dpi_scopes` map.
    /// Points to a `DpiScope` struct, or null if no scope is active.
    pub(super) static ACTIVE_SCOPE: std::cell::Cell<*mut libc::c_void> = const { std::cell::Cell::new(std::ptr::null_mut()) };
}

/// Process-wide fallback for VPI callers that execute on a different thread
/// than the one that installed `ACTIVE_SIMULATOR` in TLS.
pub(super) static GLOBAL_ACTIVE_SIMULATOR: std::sync::atomic::AtomicPtr<Simulator> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

/// What a `vpiHandle` points at. `vpiHandle` is `void *` to C, so this is
/// entirely private; every entry point checks `kind` before using the
/// fields that only some kinds populate.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum VpiKind {
    /// A module port (§37.16): its own object, with `vpiDirection`; the
    /// connected signal keeps its `vpiNet`/`vpiReg` type. `signal_id` is the
    /// port's signal so value reads work through the port handle.
    Port,
    /// A whole signal in the flat signal table.
    Signal,
    /// A bit range of a signal — a packed-struct member or part-select.
    Slice,
    /// A module instance. `inst_idx == -1` is the top module.
    Module,
    /// An unpacked array. `vpi_handle_by_index` selects a word.
    Memory,
    /// The result of `vpi_iterate`, consumed by `vpi_scan`.
    Iterator,
    /// A read-only value: a `$systf` argument that is not signal-backed
    /// (a literal, or any computed expression).
    Constant,
    /// The `$systf` call currently executing. `vpi_iterate(vpiArgument, ..)`
    /// walks its arguments; `vpi_put_value` on it sets a system function's
    /// return value.
    SysTfCall,
    /// A system task or function registered with `vpi_register_systf`
    /// (`vpiUserSystf`). `name` is its `$name`.
    UserSystf,
    /// A module path (`vpiModPath`): `signal_id` is its output net, `lsb`
    /// the path's position among that net's paths.
    ModPath,
    /// A timing check (`vpiTchk`): `signal_id` indexes the simulator's
    /// timing checks.
    Tchk,
    /// An intermodule path (`vpiInterModPath`): `signal_id` is the input
    /// port's signal, `inst_idx` the output port's.
    InterModPath,
    /// A `vpi_register_cb` handle; `signal_id` holds the callback's id in
    /// `Simulator::vpi_cb` (see `vpi_cb.rs`).
    Callback,
    /// A scope of the object model that is not an instance: a package, a
    /// generate scope, a named block, a task or a function. `signal_id` is
    /// its index in the model (see `vpi_model.rs`).
    Scope,
    /// Any other object of the object model: processes, continuous
    /// assignments, primitives, ports without storage, expressions, ranges,
    /// typespecs, ... `signal_id` is its index in the model; `usize::MAX`
    /// marks a constant made on the fly (value in `value`, `vpiConstType`
    /// in `lsb`).
    Obj,
}

/// Wrapper for VPI handles stored as raw pointers.
#[derive(Clone)]
pub(super) struct VpiHandle {
    pub(super) kind: VpiKind,
    /// Signal / Slice / Memory: index into the signal table. For Memory this
    /// is the id of element 0.
    pub(super) signal_id: usize,
    /// `vpi_get(vpiType)` answer, resolved from the DECLARED type when the
    /// handle is created. It must not be derived from the current value:
    /// a `logic` signal is a `vpiReg` whether or not it happens to hold X.
    pub(super) type_code: libc::c_int,
    pub(super) direction: libc::c_int,
    /// Slice only: bit range within `signal_id`.
    pub(super) lsb: u32,
    pub(super) width: u32,
    /// Module only: index into `module.instances`, or -1 for the top module.
    pub(super) inst_idx: isize,
    /// Local name (`data_bus`) and hierarchical name (`tb_top.u_sub.data_bus`).
    pub(super) name: String,
    pub(super) full_name: String,
    /// Module only: the definition this instantiates (`vpiDefName`).
    pub(super) def_name: String,
    /// Iterator only: the objects still to be handed out by `vpi_scan`.
    items: Vec<VpiHandle>,
    pos: usize,
    /// Constant only: the argument's value, captured at call time.
    pub(super) value: Option<Value>,
    /// SysTfCall only: depth of the frame in `VPI_SYSTF_STACK`, so a handle
    /// held across a nested `$systf` call still names its own frame.
    pub(super) frame: usize,
}

impl VpiHandle {
    pub(super) fn signal(id: usize, type_code: libc::c_int, name: &str, full: &str) -> Self {
        VpiHandle {
            kind: VpiKind::Signal,
            signal_id: id,
            type_code,
            direction: vpi::NO_DIRECTION,
            lsb: 0,
            width: 0,
            inst_idx: -1,
            name: name.to_string(),
            full_name: full.to_string(),
            def_name: String::new(),
            items: Vec::new(),
            pos: 0,
            value: None,
            frame: 0,
        }
    }

    pub(super) fn port(
        id: usize,
        name: &str,
        full: &str,
        direction: libc::c_int,
        width: u32,
    ) -> Self {
        let mut h = VpiHandle::signal(id, vpi::PORT, name, full);
        h.kind = VpiKind::Port;
        h.direction = direction;
        h.width = width;
        h
    }

    pub(super) fn into_raw(self) -> *mut libc::c_void {
        Box::into_raw(Box::new(self)) as *mut libc::c_void
    }

    /// A read-only `$systf` argument that is not signal-backed.
    pub(super) fn constant(v: Value) -> Self {
        VpiHandle {
            kind: VpiKind::Constant,
            signal_id: 0,
            type_code: vpi::CONSTANT,
            direction: vpi::NO_DIRECTION,
            lsb: 0,
            width: v.width,
            inst_idx: -1,
            name: String::new(),
            full_name: String::new(),
            def_name: String::new(),
            items: Vec::new(),
            pos: 0,
            value: Some(v),
            frame: 0,
        }
    }

    /// The `$systf` call at `frame` (an index into `VPI_SYSTF_STACK`).
    fn systf_call(frame: usize, name: &str, is_func: bool, ret_width: u32) -> Self {
        VpiHandle {
            kind: VpiKind::SysTfCall,
            signal_id: 0,
            type_code: if is_func {
                vpi::SYS_FUNC_CALL
            } else {
                vpi::SYS_TASK_CALL
            },
            direction: vpi::NO_DIRECTION,
            lsb: 0,
            width: ret_width,
            inst_idx: -1,
            name: name.to_string(),
            full_name: name.to_string(),
            def_name: String::new(),
            items: Vec::new(),
            pos: 0,
            value: None,
            frame,
        }
    }
}

/// Borrow a handle, returning None for NULL.
///
/// # Safety
/// `handle` must be NULL or a pointer returned by one of the VPI handle
/// constructors and not yet freed.
pub(super) unsafe fn vpi_deref<'a>(handle: *mut libc::c_void) -> Option<&'a VpiHandle> {
    unsafe {
        if handle.is_null() {
            None
        } else {
            Some(&*(handle as *const VpiHandle))
        }
    }
}

/// The `$systf` invocation currently on the stack. `vpi_handle(vpiSysTfCall,
/// NULL)` names the innermost one; `vpi_iterate(vpiArgument, ..)` walks its
/// arguments; `vpi_put_value` on it sets a system function's return value.
pub(super) struct VpiSystfFrame {
    /// The `$name`, as `vpi_get_str(vpiName, ..)` reports it.
    name: String,
    /// Argument objects, resolved once at call time. A signal-backed argument
    /// is writable (an `output` argument); the rest are `Constant`.
    args: Vec<VpiHandle>,
    /// Return value deposited by `vpi_put_value` on the call handle.
    pub(super) ret: Option<Value>,
    /// Width the return value is resized to.
    ret_width: u32,
    /// vpiSysTask or vpiSysFunc.
    pub(super) tf_type: libc::c_int,
    /// `sysfunctype`, for a function.
    func_type: libc::c_int,
    /// The call site (see `vpi_api::systf_site`): what `vpi_put_userdata`
    /// attaches to, carried by the call handle as its `signal_id`.
    site: usize,
}

// Innermost frame last: a `$systf` may invoke another.
thread_local! {
    pub(super) static VPI_SYSTF_STACK: std::cell::RefCell<Vec<VpiSystfFrame>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// The last diagnostic, for `vpi_chk_error`. Cleared once reported.
    pub(super) static VPI_LAST_ERROR: std::cell::RefCell<Option<(libc::c_int, String)>> =
        const { std::cell::RefCell::new(None) };
}

/// Record a diagnostic so `vpi_chk_error` can report it, and echo it to stderr.
///
/// Every VPI failure path routes through here. Without it `vpi_chk_error`
/// would always answer "no error", and a caller that checks it — which is the
/// only reason to call it — would sail straight past a failed call.
pub(super) fn vpi_error(level: libc::c_int, msg: String) {
    eprintln!("[VPI] {}", msg);
    VPI_LAST_ERROR.with(|c| *c.borrow_mut() = Some((level, msg)));
    // §38.36.3 cbPLIError.
    if level >= vpi::ERROR {
        vpi_cb::vpi_notify_pli_error();
    }
}

/// A `$systf` registered from a VPI module via `vpi_register_systf`.
#[derive(Clone, Copy)]
pub(super) struct VpiSystf {
    /// `PLI_INT32 (*calltf)(PLI_BYTE8 *user_data)`, as a `usize` so the
    /// struct stays `Copy` and can live in a thread-local map.
    pub(super) calltf: usize,
    pub(super) compiletf: usize,
    /// Called once per invocation of a `vpiSizedFunc` to learn its return width.
    pub(super) sizetf: usize,
    pub(super) user_data: usize,
    /// vpiSysTask (1) or vpiSysFunc (2).
    pub(super) tf_type: libc::c_int,
    /// `sysfunctype`: vpiIntFunc / vpiRealFunc / vpiTimeFunc / vpiSizedFunc /
    /// vpiSizedSignedFunc. Meaningless for a task.
    pub(super) func_type: libc::c_int,
}

// System tasks and functions registered by a VPI module's
// `vlog_startup_routines`. Registration happens before the Simulator is
// built, so this cannot hang off it. Single-threaded, like the rest of the
// VPI surface.
thread_local! {
    pub(super) static VPI_SYSTFS: std::cell::RefCell<HashMap<String, VpiSystf>> =
        std::cell::RefCell::new(HashMap::default());
}

/// Mirror of `s_vpi_systf_data` from `vpi_user.h` (IEEE 1800-2017 §38.36).
#[repr(C)]
pub struct s_vpi_systf_data {
    pub(super) type_: libc::c_int,
    pub(super) sysfunctype: libc::c_int,
    pub(super) tfname: *mut libc::c_char,
    pub(super) calltf: *mut libc::c_void,
    pub(super) compiletf: *mut libc::c_void,
    pub(super) sizetf: *mut libc::c_void,
    pub(super) user_data: *mut libc::c_char,
}

/// Mirror of `s_cb_data` from `vpi_user.h` — the value-change dispatcher
/// builds one of these on the stack and hands its address to the C
/// callback. Fields match the IEEE 1800-2017 §38.7 layout exactly.
///
/// `index` sits between `value` and `user_data` in the standard struct.
/// It was missing here, so every field after it was read from the wrong
/// offset by any C code compiled against a conforming `vpi_user.h` —
/// `user_data` in particular.
#[repr(C)]
pub struct s_cb_data {
    pub(super) reason: libc::c_int,
    /// C function pointer (`PLI_INT32 (*cb_rtn)(p_cb_data)`).
    pub(super) cb_rtn: *mut libc::c_void,
    /// Object the callback was registered against (the signal handle for
    /// cbValueChange). Passed back to the callback on each fire.
    pub(super) obj: *mut libc::c_void,
    pub(super) time: *mut s_vpi_time,
    pub(super) value: *mut s_vpi_value,
    pub(super) index: libc::c_int,
    pub(super) user_data: *mut libc::c_void,
}

/// Mirror of `s_vpi_vlog_info` from `vpi_user.h`. Returned by
/// `vpi_get_vlog_info` to provide binary name and version info.
#[repr(C)]
pub struct s_vpi_vlog_info {
    argc: libc::c_int,
    argv: *mut *mut libc::c_char,
    product: *mut libc::c_char,
    version: *mut libc::c_char,
}

/// Opaque DPI scope handle. The `name` field carries the hierarchical
/// scope name; `svGetNameFromScope` recovers it by casting the pointer
/// back to a `DpiScope`.
#[repr(C)]
pub(super) struct DpiScope {
    pub(super) name: [u8; 256],
    pub(super) name_len: usize,
}

/// Execute a closure with access to the active simulator.
///
/// Returns `None` when there is no active simulator — a VPI call from
/// outside a DPI context. This used to `panic!`, which unwinds across an
/// `extern "C"` boundary and therefore aborts the process; every VPI
/// entry point now degrades to its documented failure value instead.
pub(super) fn try_active_sim<F, R>(who: &str, f: F) -> Option<R>
where
    F: FnOnce(&mut Simulator) -> R,
{
    let mut sim_ptr = ACTIVE_SIMULATOR.with(|cell| cell.get());
    if sim_ptr.is_null() {
        sim_ptr = GLOBAL_ACTIVE_SIMULATOR.load(std::sync::atomic::Ordering::Acquire);
    }
    if sim_ptr.is_null() {
        eprintln!(
            "[VPI] {}: no active simulator (call from outside a DPI context)",
            who
        );
        return None;
    }
    Some(f(unsafe { &mut *sim_ptr }))
}

/// Execute a closure with access to the active simulator, for the DPI
/// scope primitives, which have no way to report failure.
fn with_active_sim<F, R>(f: F) -> R
where
    F: FnOnce(&mut Simulator) -> R,
{
    let mut sim_ptr = ACTIVE_SIMULATOR.with(|cell| cell.get());
    if sim_ptr.is_null() {
        sim_ptr = GLOBAL_ACTIVE_SIMULATOR.load(std::sync::atomic::Ordering::Acquire);
    }
    if sim_ptr.is_null() {
        panic!("[VPI] No active simulator - VPI call outside DPI context?");
    }
    f(unsafe { &mut *sim_ptr })
}

/// The `vpi_get(vpiType)` code for a signal, from its DECLARED type.
///
/// Falls back to `vpiReg` — a 4-state vector variable — for anything whose
/// declaration we cannot see, which is the safe answer: a caller that
/// believes a signal may hold X will read it with a 4-state format.
pub(super) fn vpi_type_of(sim: &Simulator, name: &str, id: usize) -> libc::c_int {
    use xezim_core::ast::types::{
        DataType, IntegerAtomType, IntegerVectorType, RealType, SimpleType, StructUnionKind,
    };

    // A parameter is also stored as a signal, so this must come first or
    // every parameter reports vpiReg.
    if sim.module.parameters.contains_key(name) {
        return vpi::PARAMETER;
    }
    if sim.module.nets.contains(name) {
        return vpi::NET;
    }
    // The signal table's `is_real` covers reals reached through a path we
    // have no declaration for.
    let looks_real = sim.signal_table.get(id).map(|v| v.is_real).unwrap_or(false);

    let dt = sim.module.var_decl_types.get(name).or_else(|| {
        name.rsplit('.')
            .next()
            .and_then(|leaf| sim.module.var_decl_types.get(leaf))
    });

    match dt {
        Some(DataType::IntegerVector { kind, .. }) => match kind {
            IntegerVectorType::Bit => vpi::BIT_VAR,
            IntegerVectorType::Logic | IntegerVectorType::Reg => vpi::REG,
        },
        Some(DataType::IntegerAtom { kind, .. }) => match kind {
            IntegerAtomType::Byte => vpi::BYTE_VAR,
            IntegerAtomType::ShortInt => vpi::SHORT_INT_VAR,
            IntegerAtomType::Int => vpi::INT_VAR,
            IntegerAtomType::LongInt => vpi::LONG_INT_VAR,
            IntegerAtomType::Integer => vpi::INTEGER_VAR,
            IntegerAtomType::Time => vpi::TIME_VAR,
        },
        Some(DataType::Real { kind, .. }) => match kind {
            RealType::ShortReal => vpi::SHORT_REAL_VAR,
            _ => vpi::REAL_VAR,
        },
        Some(DataType::Simple {
            kind: SimpleType::String,
            ..
        }) => vpi::STRING_VAR,
        Some(DataType::Enum(_)) => vpi::ENUM_VAR,
        Some(DataType::Struct(su)) => match su.kind {
            StructUnionKind::Union => vpi::UNION_VAR,
            _ => vpi::STRUCT_VAR,
        },
        _ if looks_real => vpi::REAL_VAR,
        _ => vpi::REG,
    }
}

fn vpi_direction_code(dir: PortDirection) -> libc::c_int {
    match dir {
        PortDirection::Input => vpi::INPUT,
        PortDirection::Output => vpi::OUTPUT,
        PortDirection::Inout => vpi::INOUT,
        PortDirection::Ref => vpi::NO_DIRECTION,
    }
}

/// Direction of the port a signal belongs to, `vpiNoDirection` for a plain
/// signal. The standard defines `vpiDirection` on port objects; answering
/// it on the port's signal too is a convenience that costs nothing.
pub(super) fn vpi_direction_of(sim: &Simulator, name: &str) -> libc::c_int {
    sim.vpi_port_directions
        .get(name)
        .map(|d| vpi_direction_code(*d))
        .unwrap_or(vpi::NO_DIRECTION)
}

pub(super) fn new_vpi_handle(sim: &Simulator, name: &str, id: usize) -> *mut libc::c_void {
    let type_code = vpi_type_of(sim, name, id);
    let leaf = name.rsplit('.').next().unwrap_or(name);
    let mut handle = VpiHandle::signal(id, type_code, leaf, &vpi_full_name(sim, name));
    handle.direction = vpi_direction_of(sim, name);
    handle.into_raw()
}

/// Prefix a design-relative name with the top module, the way
/// `vpi_get_str(vpiFullName, ..)` must report it.
pub(super) fn vpi_full_name(sim: &Simulator, name: &str) -> String {
    sim.hier_path(name)
}

/// Strip a leading `<top>.` from a hierarchical name. Signal-table names
/// are stored relative to the top module, but VPI callers write the full
/// path.
pub(super) fn vpi_strip_top<'a>(sim: &Simulator, name: &'a str) -> &'a str {
    let top = sim.module.name.as_str();
    if let Some(rest) = name.strip_prefix(top) {
        if let Some(rest) = rest.strip_prefix('.') {
            return rest;
        }
        if rest.is_empty() {
            return "";
        }
    }
    name
}

/// A module handle for `inst_idx` (-1 = the top module).
fn vpi_module_handle(sim: &Simulator, inst_idx: isize) -> *mut libc::c_void {
    let (name, full, def) = if inst_idx < 0 {
        (
            sim.module.name.clone(),
            sim.module.name.clone(),
            sim.module.name.clone(),
        )
    } else {
        let inst = &sim.module.instances[inst_idx as usize];
        let leaf = inst
            .path
            .rsplit('.')
            .next()
            .unwrap_or(&inst.path)
            .to_string();
        (leaf, vpi_full_name(sim, &inst.path), inst.def_name.clone())
    };
    VpiHandle {
        kind: VpiKind::Module,
        signal_id: 0,
        type_code: vpi::MODULE,
        direction: vpi::NO_DIRECTION,
        lsb: 0,
        width: 0,
        inst_idx,
        name,
        full_name: full,
        def_name: def,
        items: Vec::new(),
        pos: 0,
        value: None,
        frame: 0,
    }
    .into_raw()
}

/// The design-relative path of a module handle ("" for the top module).
pub(super) fn vpi_scope_path(sim: &Simulator, h: &VpiHandle) -> String {
    if h.inst_idx < 0 {
        String::new()
    } else {
        sim.module.instances[h.inst_idx as usize].path.clone()
    }
}

/// True when `name` is a signal the elaborator invented for an instance
/// (a 1-bit placeholder so validation recognises the name). Those must not
/// appear as VPI signal objects — they are module instances.
fn vpi_is_instance_name(sim: &Simulator, name: &str) -> bool {
    sim.instance_by_path(&*name).is_some()
}

/// Resolve a packed-struct member path (`pixel_data.r`) to its bit range.
fn vpi_slice_of(sim: &Simulator, name: &str) -> Option<VpiHandle> {
    let (base, member) = name.rsplit_once('.')?;
    let fields = sim.module.packed_struct_fields.get(base)?;
    let &(_, lsb, width) = fields.iter().find(|(m, _, _)| m == member)?;
    let &id = sim.signal_name_to_id.get(base)?;
    Some(VpiHandle {
        kind: VpiKind::Slice,
        signal_id: id,
        type_code: vpi::PART_SELECT,
        direction: vpi::NO_DIRECTION,
        lsb,
        width,
        inst_idx: -1,
        name: member.to_string(),
        full_name: vpi_full_name(sim, name),
        def_name: String::new(),
        items: Vec::new(),
        pos: 0,
        value: None,
        frame: 0,
    })
}

/// Read a slice out of its parent signal.
pub(super) fn vpi_slice_read(sim: &Simulator, h: &VpiHandle) -> Option<Value> {
    let parent = &vpi_api::cell_value(sim, h.signal_id)?;
    let mut v = Value::zero(h.width);
    for i in 0..h.width as usize {
        v.set_bit_code(i, parent.get_bit_code(h.lsb as usize + i));
    }
    Some(v)
}

/// Write a slice back into its parent signal.
pub(super) fn vpi_slice_write(sim: &Simulator, h: &VpiHandle, val: &Value) -> Option<Value> {
    let mut parent = vpi_api::cell_value(sim, h.signal_id)?;
    for i in 0..h.width as usize {
        parent.set_bit_code(h.lsb as usize + i, val.get_bit_code(i));
    }
    Some(parent)
}

/// Get signal ID by hierarchical name.
///
/// Tries the full name, then each successively shorter suffix, so
/// `top.dut.sig`, `dut.sig` and `sig` all resolve. Member paths like
/// `top.dut.some_struct.val_i` are registered as `some_struct.val_i`, so
/// the suffix walk is what finds them.
///
/// The previous loop never advanced its cursor: it only ever tested the
/// name with its first segment stripped, and when that missed it spun
/// forever. Any unresolvable dotted name hung the simulator when checking
/// paths that may not exist.
/// §35.5.4 DPI export dispatch. The generated trampoline shared object calls
/// this from C to run an exported SystemVerilog subroutine. `id` is the
/// subroutine's declaration-order index; `n` args are 64-bit integer slots.
#[unsafe(no_mangle)]
pub extern "C" fn __xezim_dpi_export_dispatch(
    id: libc::c_longlong,
    n: libc::c_longlong,
    a: *const libc::c_longlong,
) -> libc::c_longlong {
    try_active_sim("__xezim_dpi_export_dispatch", |sim| {
        if n < 0 {
            // An export C cannot call (`load_dpi_export_trampoline`).
            return sim.run_dpi_export_unmodelled(id as usize) as libc::c_longlong;
        }
        sim.run_dpi_export(id as usize, n as usize, a as *const i64) as libc::c_longlong
    })
    .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub extern "C" fn vpi_handle_by_name(
    name: *mut libc::c_char,
    scope: *mut libc::c_void,
) -> *mut libc::c_void {
    if name.is_null() {
        return std::ptr::null_mut();
    }
    let full_name = unsafe { std::ffi::CStr::from_ptr(name) }
        .to_string_lossy()
        .trim()
        .to_string();

    try_active_sim("vpi_handle_by_name", |sim| {
        // VPI object model (vpi_model.rs): every declared object, scope and
        // generate block by its full name, or relative to `scope`.
        if let Some(h) = vpi_model::handle_by_name(sim, &full_name, unsafe { vpi_deref(scope) }) {
            return h;
        }
        // The top module itself, and any instance in the hierarchy.
        let rel = vpi_strip_top(sim, &full_name);
        if rel.is_empty() {
            return vpi_module_handle(sim, -1);
        }
        if let Some(i) = sim.module.instances.iter().position(|i| i.path == rel) {
            return vpi_module_handle(sim, i as isize);
        }

        let mut rest: &str = full_name.as_str();
        loop {
            // An instance's 1-bit placeholder signal must not be handed out
            // as a signal object.
            if !vpi_is_instance_name(sim, rest) {
                // Arrays first: a continuous assignment that reads an element
                // leaves a 1-bit placeholder signal under the array's own
                // name, which must not shadow the array.
                if let Some(h) = vpi_memory_of(sim, rest) {
                    return h.into_raw();
                }
                // Multi-dimensional arrays, their sub-arrays, and elements
                // of arrays too large to have named elements.
                if let Some(h) = vpi_api::array_object_of(sim, rest) {
                    return h.into_raw();
                }
                if let Some(&id) = sim.signal_name_to_id.get(rest) {
                    return new_vpi_handle(sim, rest, id);
                }
                if let Some(h) = vpi_slice_of(sim, rest) {
                    return h.into_raw();
                }
            }
            match rest.find('.') {
                Some(dot) => rest = &rest[dot + 1..],
                None => return std::ptr::null_mut(),
            }
        }
    })
    .unwrap_or(std::ptr::null_mut())
}

/// An unpacked array (`int mem [0:3]`) as a `vpiMemory` object. Its words
/// are separate signals named `mem[0]`..`mem[n-1]`; `vpi_handle_by_index`
/// selects one.
fn vpi_memory_of(sim: &Simulator, name: &str) -> Option<VpiHandle> {
    let &(lo, hi, _w) = sim.module.arrays.get(name)?;
    let count = (hi - lo).unsigned_abs() as u32 + 1;
    let id = *sim
        .signal_name_to_id
        .get(format!("{}[{}]", name, lo).as_str())?;
    Some(VpiHandle {
        kind: VpiKind::Memory,
        signal_id: id,
        type_code: vpi::MEMORY,
        direction: vpi::NO_DIRECTION,
        lsb: lo.min(hi) as u32,
        width: count,
        inst_idx: -1,
        name: name.rsplit('.').next().unwrap_or(name).to_string(),
        full_name: vpi_full_name(sim, name),
        def_name: String::new(),
        items: Vec::new(),
        pos: 0,
        value: None,
        frame: 0,
    })
}

/// Select one word of a `vpiMemory`.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_handle_by_index(
    handle: *mut libc::c_void,
    index: libc::c_int,
) -> *mut libc::c_void {
    let Some(h) = (unsafe { vpi_deref(handle) }) else {
        return std::ptr::null_mut();
    };
    // VPI object model (vpi_model.rs): arrays, sub-arrays and generate
    // scope arrays.
    if let Some(r) = try_active_sim("vpi_handle_by_index", |sim| {
        vpi_model::handle_by_index(sim, h, index as i64)
    })
    .flatten()
    {
        return r;
    }
    if h.kind != VpiKind::Memory {
        return std::ptr::null_mut();
    }
    // A multi-dimensional array (or a sub-array) indexes its leftmost
    // remaining dimension, exactly as one index of vpi_handle_by_multi_index.
    if let Some(r) = vpi_api::index_array_handle(h, index) {
        return r;
    }
    let full = h.full_name.clone();
    try_active_sim("vpi_handle_by_index", |sim| {
        let rel = vpi_strip_top(sim, &full).to_string();
        let elem = format!("{}[{}]", rel, index);
        match sim.signal_name_to_id.get(elem.as_str()) {
            Some(&id) => {
                let ty = vpi_type_of(sim, &elem, id);
                VpiHandle::signal(id, ty, &elem, &vpi_full_name(sim, &elem)).into_raw()
            }
            None => std::ptr::null_mut(),
        }
    })
    .unwrap_or(std::ptr::null_mut())
}

/// Traverse a one-to-one relationship. Only `vpiScope` is modelled.
///
/// `vpi_handle(vpiScope, NULL)` returns the top module. That is an xezim
/// extension — the standard route to the top is
/// `vpi_scan(vpi_iterate(vpiModule, NULL))` — but enough VPI code in the
/// wild spells it this way that supporting it is worth more than the
/// purity of returning NULL.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_handle(type_: libc::c_int, refh: *mut libc::c_void) -> *mut libc::c_void {
    // The currently executing $systf. A `NULL` reference is the idiomatic
    // spelling; the standard also allows passing the call handle itself.
    if type_ == vpi::SYS_TF_CALL {
        return VPI_SYSTF_STACK.with(|st| {
            let st = st.borrow();
            match st.last() {
                Some(f) => {
                    let mut h = VpiHandle::systf_call(
                        st.len() - 1,
                        &f.name,
                        f.tf_type == vpi::SYS_FUNC,
                        f.ret_width,
                    );
                    h.signal_id = f.site;
                    h.into_raw()
                }
                None => {
                    vpi_error(
                        vpi::ERROR,
                        "vpi_handle(vpiSysTfCall): no $systf is executing".to_string(),
                    );
                    std::ptr::null_mut()
                }
            }
        });
    }
    if type_ == vpi_api::vc::USER_SYSTF {
        return vpi_api::user_systf_of_call(refh);
    }
    try_active_sim("vpi_handle", |sim| {
        // VPI object model (vpi_model.rs): the one-to-one relations.
        if let Some(r) = vpi_model::handle(sim, type_, unsafe { vpi_deref(refh) }) {
            return r;
        }
        if type_ != vpi::SCOPE {
            return std::ptr::null_mut();
        }
        let Some(h) = (unsafe { vpi_deref(refh) }) else {
            return vpi_module_handle(sim, -1);
        };
        match h.kind {
            // The scope containing a module is its parent.
            VpiKind::Module => {
                if h.inst_idx < 0 {
                    return std::ptr::null_mut(); // the top has no parent
                }
                let parent = sim.module.instances[h.inst_idx as usize].parent.clone();
                if parent.is_empty() && sim.root_is_multi_top() {
                    std::ptr::null_mut() // a top under the multi-top wrapper
                } else if parent.is_empty() {
                    vpi_module_handle(sim, -1)
                } else {
                    match sim.module.instances.iter().position(|i| i.path == parent) {
                        Some(i) => vpi_module_handle(sim, i as isize),
                        None => std::ptr::null_mut(),
                    }
                }
            }
            // The scope containing an object is the instance it is declared in.
            _ => {
                let rel = vpi_strip_top(sim, &h.full_name).to_string();
                match rel.rsplit_once('.') {
                    Some((scope, _)) => {
                        match sim.module.instances.iter().position(|i| i.path == scope) {
                            Some(i) => vpi_module_handle(sim, i as isize),
                            None => vpi_module_handle(sim, -1),
                        }
                    }
                    None => vpi_module_handle(sim, -1),
                }
            }
        }
    })
    .unwrap_or(std::ptr::null_mut())
}

/// Objects declared DIRECTLY in `scope` (no deeper dotted component), in
/// sorted order so iteration is deterministic.
pub(super) fn vpi_scope_members(sim: &Simulator, scope: &str) -> Vec<(String, usize)> {
    let mut out: Vec<(String, usize)> = Vec::new();
    for (name, &id) in sim.signal_name_to_id.iter() {
        let name: &str = name;
        let leaf = if scope.is_empty() {
            if name.contains('.') {
                continue;
            }
            name
        } else {
            match name.strip_prefix(scope).and_then(|r| r.strip_prefix('.')) {
                Some(r) if !r.contains('.') => r,
                _ => continue,
            }
        };
        // Array words (`mem[0]`) are reached through their vpiMemory parent,
        // and an instance placeholder is not a signal.
        if leaf.contains('[')
            || vpi_is_instance_name(sim, name)
            || xezim_core::elaborate::is_delay_stage_net(name)
            || event_bits::is_edge_alias_net(name)
        {
            continue;
        }
        out.push((name.to_string(), id));
    }
    // Unpacked arrays have no whole-array signal; surface them as vpiMemory.
    for name in sim.module.arrays.keys() {
        let matches_scope = if scope.is_empty() {
            !name.contains('.')
        } else {
            name.strip_prefix(scope)
                .and_then(|r| r.strip_prefix('.'))
                .map(|r| !r.contains('.'))
                .unwrap_or(false)
        };
        if matches_scope && !out.iter().any(|(n, _)| n == name) {
            out.push((name.clone(), usize::MAX));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Iterate a one-to-many relationship. Returns NULL when the relationship
/// yields nothing, as the standard requires (callers test for it).
#[unsafe(no_mangle)]
pub extern "C" fn vpi_iterate(type_: libc::c_int, refh: *mut libc::c_void) -> *mut libc::c_void {
    // Module paths and timing checks come from the object model when it has
    // the sources; `vpi_api`'s view of the simulator's tables is the fallback
    // (a design loaded from a compiled artifact). The delay routines accept
    // either kind of handle.
    const MOD_PATH: libc::c_int = 31;
    const TCHK: libc::c_int = 61;
    if type_ == MOD_PATH || type_ == TCHK {
        if let Some(r) = try_active_sim("vpi_iterate", |sim| {
            vpi_model::iterate(sim, type_, unsafe { vpi_deref(refh) })
        })
        .flatten()
        {
            return r;
        }
    }
    if let Some(it) = vpi_api::iterate_extra(type_, refh) {
        return it;
    }
    // The arguments of a $systf call. Handled before the design traversal
    // below because it needs no simulator, only the call frame.
    if type_ == vpi::ARGUMENT {
        let Some(h) = (unsafe { vpi_deref(refh) }) else {
            return std::ptr::null_mut();
        };
        if h.kind != VpiKind::SysTfCall {
            return std::ptr::null_mut();
        }
        let items = VPI_SYSTF_STACK.with(|st| {
            st.borrow()
                .get(h.frame)
                .map(|f| f.args.clone())
                .unwrap_or_default()
        });
        return vpi_make_iterator(items);
    }
    try_active_sim("vpi_iterate", |sim| {
        let scope_h = unsafe { vpi_deref(refh) };
        // VPI object model (vpi_model.rs): the one-to-many relations.
        if let Some(r) = vpi_model::iterate(sim, type_, scope_h) {
            return r;
        }

        // A NULL reference means "the whole design": the only top-level
        // module is the one we elaborated.
        let scope_path = match scope_h {
            None => {
                if type_ == vpi::MODULE || type_ == vpi::INSTANCE {
                    // Under the multi-top wrapper the tops are its children.
                    let tops: Vec<isize> = if sim.root_is_multi_top() {
                        (0..sim.module.instances.len())
                            .filter(|&i| sim.module.instances[i].parent.is_empty())
                            .map(|i| i as isize)
                            .collect()
                    } else {
                        vec![-1]
                    };
                    let items = tops
                        .into_iter()
                        .map(|i| unsafe {
                            *Box::from_raw(vpi_module_handle(sim, i) as *mut VpiHandle)
                        })
                        .collect();
                    return vpi_make_iterator(items);
                }
                return std::ptr::null_mut();
            }
            Some(h) if h.kind == VpiKind::Module => vpi_scope_path(sim, h),
            Some(_) => return std::ptr::null_mut(),
        };

        // Child scopes / instances.
        if type_ == vpi::MODULE || type_ == vpi::INSTANCE || type_ == vpi::INTERNAL_SCOPE {
            let items: Vec<VpiHandle> = sim
                .module
                .instances
                .iter()
                .enumerate()
                .filter(|(_, i)| i.parent == scope_path)
                .map(|(idx, _)| unsafe {
                    *Box::from_raw(vpi_module_handle(sim, idx as isize) as *mut VpiHandle)
                })
                .collect();
            return vpi_make_iterator(items);
        }

        // Declared objects, filtered by what the caller asked for.
        if type_ == vpi::PORT {
            // §37.16: ports are their own objects, one per direction-bearing
            // member of the scope, in name order.
            let items: Vec<VpiHandle> = vpi_scope_members(sim, &scope_path)
                .into_iter()
                .filter(|(_, id)| *id != usize::MAX)
                .filter_map(|(name, id)| {
                    let dir = sim.vpi_port_directions.get(name.as_str())?;
                    let leaf = name.rsplit('.').next().unwrap_or(&name).to_string();
                    let width = sim.signal_widths.get(id).copied().unwrap_or(1) as u32;
                    Some(VpiHandle::port(
                        id,
                        &leaf,
                        &vpi_full_name(sim, &name),
                        vpi_direction_code(*dir),
                        width,
                    ))
                })
                .collect();
            return vpi_make_iterator(items);
        }

        let want = |ty: libc::c_int| -> bool {
            match type_ {
                vpi::NET => ty == vpi::NET,
                vpi::REG => ty == vpi::REG || ty == vpi::BIT_VAR,
                vpi::PARAMETER => ty == vpi::PARAMETER,
                vpi::MEMORY => ty == vpi::MEMORY,
                vpi::VARIABLES => ty != vpi::NET && ty != vpi::PARAMETER && ty != vpi::MEMORY,
                _ => false,
            }
        };

        let mut items: Vec<VpiHandle> = Vec::new();
        for (name, id) in vpi_scope_members(sim, &scope_path) {
            if id == usize::MAX {
                if let Some(m) = vpi_memory_of(sim, &name) {
                    if want(vpi::MEMORY) {
                        items.push(m);
                    }
                }
                continue;
            }
            let ty = vpi_type_of(sim, &name, id);
            if want(ty) {
                let leaf = name.rsplit('.').next().unwrap_or(&name);
                let mut handle = VpiHandle::signal(id, ty, leaf, &vpi_full_name(sim, &name));
                handle.direction = vpi_direction_of(sim, &name);
                items.push(handle);
            }
        }
        vpi_make_iterator(items)
    })
    .unwrap_or(std::ptr::null_mut())
}

pub(super) fn vpi_make_iterator(items: Vec<VpiHandle>) -> *mut libc::c_void {
    if items.is_empty() {
        return std::ptr::null_mut();
    }
    VpiHandle {
        kind: VpiKind::Iterator,
        signal_id: 0,
        type_code: vpi::ITERATOR,
        direction: vpi::NO_DIRECTION,
        lsb: 0,
        width: 0,
        inst_idx: -1,
        name: String::new(),
        full_name: String::new(),
        def_name: String::new(),
        items,
        pos: 0,
        value: None,
        frame: 0,
    }
    .into_raw()
}

/// Hand out the next object of an iterator. When the iterator is exhausted
/// it is FREED and NULL returned — the caller must not free it itself, per
/// IEEE 1800-2017 §38.32.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_scan(iter: *mut libc::c_void) -> *mut libc::c_void {
    if iter.is_null() {
        return std::ptr::null_mut();
    }
    let h = unsafe { &mut *(iter as *mut VpiHandle) };
    if h.kind != VpiKind::Iterator {
        return std::ptr::null_mut();
    }
    if h.pos >= h.items.len() {
        drop(unsafe { Box::from_raw(iter as *mut VpiHandle) });
        return std::ptr::null_mut();
    }
    let next = h.items[h.pos].clone();
    h.pos += 1;
    next.into_raw()
}

/// String-valued properties. The returned pointer addresses simulator-owned
/// storage valid until the next `vpi_get_str` call on this thread.
/// Spelling of a `vpiType` code, for `vpi_get_str(vpiType, h)`. Kept beside
/// the `vpi` constant block it mirrors so the two cannot drift.
fn vpi_type_name(code: libc::c_int) -> Option<&'static str> {
    Some(match code {
        vpi::CONSTANT => "vpiConstant",
        vpi::ITERATOR => "vpiIterator",
        vpi::INTEGER_VAR => "vpiIntegerVar",
        vpi::MEMORY => "vpiMemory",
        vpi::MODULE => "vpiModule",
        vpi::NET => "vpiNet",
        vpi::PORT => "vpiPort",
        vpi::PORT_BIT => "vpiPortBit",
        vpi::PARAMETER => "vpiParameter",
        vpi::PART_SELECT => "vpiPartSelect",
        vpi::REAL_VAR => "vpiRealVar",
        vpi::REG => "vpiReg",
        vpi::SCOPE => "vpiScope",
        vpi::SYS_FUNC_CALL => "vpiSysFuncCall",
        vpi::SYS_TASK_CALL => "vpiSysTaskCall",
        vpi::TIME_VAR => "vpiTimeVar",
        vpi::INTERNAL_SCOPE => "vpiInternalScope",
        vpi::LONG_INT_VAR => "vpiLongIntVar",
        vpi::SHORT_INT_VAR => "vpiShortIntVar",
        vpi::INT_VAR => "vpiIntVar",
        vpi::SHORT_REAL_VAR => "vpiShortRealVar",
        vpi::BYTE_VAR => "vpiByteVar",
        vpi::STRING_VAR => "vpiStringVar",
        vpi::ENUM_VAR => "vpiEnumVar",
        vpi::STRUCT_VAR => "vpiStructVar",
        vpi::UNION_VAR => "vpiUnionVar",
        vpi::BIT_VAR => "vpiBitVar",
        vpi_cb::VPI_CALLBACK => "vpiCallback",
        _ => return None,
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn vpi_get_str(
    property: libc::c_int,
    handle: *mut libc::c_void,
) -> *mut libc::c_char {
    let Some(h) = (unsafe { vpi_deref(handle) }) else {
        return std::ptr::null_mut();
    };
    // VPI object model (vpi_model.rs): names, files and type names.
    let model = match vpi_model::get_str_static(property, h) {
        Some(r) => Some(r),
        None if !matches!(
            h.kind,
            VpiKind::SysTfCall | VpiKind::Constant | VpiKind::Iterator
        ) =>
        {
            try_active_sim("vpi_get_str", |sim| vpi_model::get_str(sim, property, h)).flatten()
        }
        None => None,
    };
    if let Some(r) = model {
        return match r {
            Some(s) => vpi_model::str_result(&s),
            None => std::ptr::null_mut(),
        };
    }
    let s = match property {
        vpi::NAME => h.name.clone(),
        vpi::FULL_NAME => h.full_name.clone(),
        vpi::DEF_NAME if h.kind == VpiKind::Module => h.def_name.clone(),
        // §38.38: the STRING spelling of the object's type, which must agree
        // with the code `vpi_get(vpiType, h)` reports. Clients print this in
        // diagnostics — cocotb logs a warning for every object whose type it
        // cannot name.
        vpi::TYPE => match vpi_type_name(h.type_code) {
            Some(n) => n.to_string(),
            None => {
                vpi_error(
                    vpi::WARNING,
                    format!("vpi_get_str: no name for vpiType {}", h.type_code),
                );
                return std::ptr::null_mut();
            }
        },
        _ => {
            vpi_error(
                vpi::WARNING,
                format!("vpi_get_str: property {} not modelled", property),
            );
            return std::ptr::null_mut();
        }
    };
    VPI_STR2_SCRATCH.with(|cell| {
        let mut pool = cell.borrow_mut();
        let slot = pool.0;
        pool.0 = (slot + 1) % 8;
        let buf = &mut pool.1[slot];
        buf.clear();
        buf.extend_from_slice(s.as_bytes());
        buf.push(0);
        buf.as_mut_ptr() as *mut libc::c_char
    })
}

/// Register a system task or function implemented in a VPI module.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_register_systf(data: *mut s_vpi_systf_data) -> *mut libc::c_void {
    if data.is_null() {
        return std::ptr::null_mut();
    }
    let d = unsafe { &*data };
    if d.tfname.is_null() || d.calltf.is_null() {
        vpi_error(
            vpi::ERROR,
            "vpi_register_systf: tfname and calltf are required".into(),
        );
        return std::ptr::null_mut();
    }
    let name = unsafe { std::ffi::CStr::from_ptr(d.tfname) }
        .to_string_lossy()
        .trim()
        .to_string();
    if !name.starts_with('$') {
        vpi_error(
            vpi::ERROR,
            format!("vpi_register_systf: tfname '{}' must start with '$'", name),
        );
        return std::ptr::null_mut();
    }
    if d.type_ != vpi::SYS_TASK && d.type_ != vpi::SYS_FUNC {
        vpi_error(
            vpi::ERROR,
            format!(
                "vpi_register_systf: '{}' has type {}, expected vpiSysTask or vpiSysFunc",
                name, d.type_
            ),
        );
        return std::ptr::null_mut();
    }
    let entry = VpiSystf {
        calltf: d.calltf as usize,
        compiletf: d.compiletf as usize,
        sizetf: d.sizetf as usize,
        user_data: d.user_data as usize,
        tf_type: d.type_,
        func_type: d.sysfunctype,
    };
    VPI_SYSTFS.with(|m| m.borrow_mut().insert(name.clone(), entry));
    // The `vpiUserSystf` object: `vpi_get_systf_info` reads it back.
    vpi_api::note_registered_systf(&name);
    vpi_api::user_systf_handle(&name).into_raw()
}

/// Call a registered `$systf`. Returns false when `name` is not registered,
/// so the caller can fall through to its own diagnostic.
/// Width of a system function's return value, per its `sysfunctype`.
/// `vpiSizedFunc` asks the module via `sizetf`.
fn vpi_sysfunc_width(entry: &VpiSystf) -> u32 {
    type TfFn = extern "C" fn(*mut libc::c_char) -> libc::c_int;
    match entry.func_type {
        vpi::REAL_FUNC => 64,
        vpi::TIME_FUNC => 64,
        vpi::SIZED_FUNC | vpi::SIZED_SIGNED_FUNC if entry.sizetf != 0 => {
            let f: TfFn = unsafe { std::mem::transmute(entry.sizetf as *const ()) };
            let w = f(entry.user_data as *mut libc::c_char);
            if w > 0 { w as u32 } else { 32 }
        }
        _ => 32,
    }
}

/// Invoke a registered `$systf` with a frame carrying its arguments, so the
/// C side can reach them through `vpi_handle(vpiSysTfCall, NULL)` and
/// `vpi_iterate(vpiArgument, ..)`, and deposit a return value.
///
/// Returns the function's return value; `None` for a task, or when `name` is
/// not registered (so the caller can fall through to its own diagnostic).
pub(super) fn vpi_call_systf(
    sim: *mut Simulator,
    name: &str,
    args: &[Expression],
    span: crate::ast::Span,
) -> Option<Value> {
    let entry = VPI_SYSTFS.with(|m| m.borrow().get(name).copied())?;
    // The call instance this invocation belongs to (its user data lives
    // there, and its compiletf runs once).
    let (site, first_call) = vpi_api::systf_site(unsafe { &*sim }, name, span);

    let is_func = entry.tf_type == vpi::SYS_FUNC;
    let ret_width = if is_func {
        vpi_sysfunc_width(&entry)
    } else {
        0
    };

    // Resolve the arguments BEFORE the frame is pushed: building a handle
    // evaluates expressions, which can itself invoke a nested $systf.
    let arg_handles: Vec<VpiHandle> = unsafe {
        let s = &mut *sim;
        args.iter().map(|a| s.vpi_arg_handle(a)).collect()
    };

    VPI_SYSTF_STACK.with(|st| {
        st.borrow_mut().push(VpiSystfFrame {
            name: name.to_string(),
            args: arg_handles,
            ret: None,
            ret_width,
            tf_type: entry.tf_type,
            func_type: entry.func_type,
            site,
        })
    });

    ACTIVE_SIMULATOR.with(|cell| cell.set(sim));
    // Foreign code may read what the design wrote to a file.
    unsafe { (*sim).flush_file_writes() };
    type TfFn = extern "C" fn(*mut libc::c_char) -> libc::c_int;
    // compiletf runs once per call instance, just before that instance's
    // first calltf: xezim has no separate compile phase to run it in.
    if entry.compiletf != 0 && first_call {
        let f: TfFn = unsafe { std::mem::transmute(entry.compiletf as *const ()) };
        f(entry.user_data as *mut libc::c_char);
    }
    let f: TfFn = unsafe { std::mem::transmute(entry.calltf as *const ()) };
    f(entry.user_data as *mut libc::c_char);

    let frame = VPI_SYSTF_STACK.with(|st| st.borrow_mut().pop());
    if !is_func {
        return None;
    }
    // A function that never deposited a value returns 0, as the LRM's own
    // system functions do.
    let mut v = frame
        .and_then(|f| f.ret)
        .unwrap_or_else(|| Value::zero(ret_width.max(1)));
    if entry.func_type == vpi::REAL_FUNC {
        v.is_real = true;
    } else if entry.func_type == vpi::SIZED_SIGNED_FUNC {
        v.is_signed = true;
    }
    Some(v)
}

/// True when a VPI module registered `name`. Lets the caller distinguish a
/// user-defined `$task` from a genuinely unknown one.
pub fn vpi_systf_registered(name: &str) -> bool {
    VPI_SYSTFS.with(|m| m.borrow().contains_key(name))
}

/// Return width of a registered system function, without invoking it.
pub fn vpi_sysfunc_ret_width(name: &str) -> u32 {
    VPI_SYSTFS
        .with(|m| m.borrow().get(name).map(|e| vpi_sysfunc_width(e)))
        .unwrap_or(32)
}

/// True when `name` was registered as a system FUNCTION (`vpiSysFunc`).
pub fn vpi_systf_is_func(name: &str) -> bool {
    VPI_SYSTFS.with(|m| {
        m.borrow()
            .get(name)
            .is_some_and(|e| e.tf_type == vpi::SYS_FUNC)
    })
}

/// Mirror of `s_vpi_error_info` (IEEE 1800-2017 §38.4).
#[repr(C)]
pub struct s_vpi_error_info {
    state: libc::c_int,
    level: libc::c_int,
    message: *mut libc::c_char,
    product: *mut libc::c_char,
    code: *mut libc::c_char,
    file: *mut libc::c_char,
    line: libc::c_int,
}

/// Report the last VPI diagnostic, and clear it. Returns the severity level,
/// or 0 when nothing has gone wrong since the previous call.
///
/// `error_info_p` may be NULL — callers often use `vpi_chk_error(NULL)` purely
/// as a "did anything fail?" probe.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_chk_error(error_info_p: *mut s_vpi_error_info) -> libc::c_int {
    let taken = VPI_LAST_ERROR.with(|c| c.borrow_mut().take());
    let Some((level, msg)) = taken else { return 0 };
    if !error_info_p.is_null() {
        let info = unsafe { &mut *error_info_p };
        info.state = vpi::RUN_STATE;
        info.level = level;
        info.message = vpi_err_scratch(&msg);
        info.product = b"xezim\0".as_ptr() as *mut libc::c_char;
        info.code = b"\0".as_ptr() as *mut libc::c_char;
        // §38.3: every string field points at a string. An error has no
        // source file, so `file` is empty rather than NULL: cocotb logs
        // `file` and `product` as C strings, and a NULL `file` crashed it
        // on the first error it checked.
        info.file = b"\0".as_ptr() as *mut libc::c_char;
        info.line = 0;
    }
    level
}

fn vpi_err_scratch(s: &str) -> *mut libc::c_char {
    thread_local! {
        static BUF: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
    }
    BUF.with(|cell| {
        let mut b = cell.borrow_mut();
        b.clear();
        b.extend_from_slice(s.as_bytes());
        b.push(0);
        b.as_mut_ptr() as *mut libc::c_char
    })
}

/// Load each `--vpi-lib` and run its `vlog_startup_routines`, the standard
/// entry point for a VPI module (IEEE 1800-2017 §38.2).
pub fn vpi_run_startup_routines(libs: &mut Vec<Library>, paths: &[String]) {
    for path in paths {
        // SAFETY: loading a dynamic library is inherently unsafe; the handle
        // is kept alive for the simulator's lifetime.
        let lib = match unsafe { Library::new(path) } {
            Ok(l) => l,
            Err(e) => {
                eprintln!("[VPI] failed to load '{}': {}", path, e);
                continue;
            }
        };
        type StartupFn = unsafe extern "C" fn();
        // `vlog_startup_routines` is a NULL-terminated array of function
        // pointers, not a function.
        let routines: *const Option<StartupFn> =
            match unsafe { lib.get::<*const Option<StartupFn>>(b"vlog_startup_routines\0") } {
                Ok(sym) => *sym,
                Err(_) => {
                    eprintln!(
                        "[VPI] '{}' has no vlog_startup_routines; nothing to register",
                        path
                    );
                    libs.push(lib);
                    continue;
                }
            };
        if routines.is_null() {
            libs.push(lib);
            continue;
        }
        let mut i = 0isize;
        loop {
            let slot = unsafe { *routines.offset(i) };
            match slot {
                Some(f) => unsafe { f() },
                None => break,
            }
            i += 1;
        }
        libs.push(lib);
    }
}

/// Free a VPI handle.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_free_object(handle: *mut libc::c_void) -> libc::c_int {
    // §38.24: 1 on SUCCESS, 0 on failure. This returned 0 unconditionally, so
    // every free reported failure even though the handle was released — which
    // is what made cocotb log "Attempting to free root iterator failed!" after
    // a perfectly good root scan.
    if handle.is_null() {
        return 0;
    }
    drop(unsafe { Box::from_raw(handle as *mut VpiHandle) });
    1
}

/// Get a VPI property value. Returns `vpiUndefined` (-1) for a property
/// xezim does not model, rather than a plausible-looking 0.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_get(property: libc::c_int, handle: *mut libc::c_void) -> libc::c_int {
    // vpiTimeUnit / vpiTimePrecision are answered BEFORE the handle is
    // dereferenced: they are simulation-wide, and a NULL `obj` is the standard
    // way to ask for the whole simulation's value. Deref-first returned
    // `vpiUndefined` (-1) for them, and a consumer that reads the result as a
    // base-10 exponent then believes the timebase is 1e-1 s — which is exactly
    // how cocotb reported "simulator precision of 1e-1" and made every `Timer`
    // unrepresentable.
    if property == vpi::TIME_UNIT || property == vpi::TIME_PRECISION {
        // A module handle answers with that module's own timeunit /
        // timeprecision (#206); NULL, or any other object, with the
        // simulation's — the finest precision in the design.
        let module_def = unsafe { vpi_deref(handle) }
            .filter(|h| h.kind == VpiKind::Module)
            .map(|h| h.def_name.clone());
        let exp = try_active_sim("vpi_get", |sim| match &module_def {
            Some(def) => {
                let (unit, prec) = sim.reported_timescale_exp(def);
                if property == vpi::TIME_UNIT {
                    unit
                } else {
                    prec
                }
            }
            None => secs_to_exp(sim.module.tick_s),
        });
        return exp.unwrap_or(vpi::UNDEFINED);
    }
    let Some(h) = (unsafe { vpi_deref(handle) }) else {
        return vpi::UNDEFINED;
    };
    // VPI object model (vpi_model.rs): the properties of every object it
    // knows, including the declared type.
    let model = match vpi_model::get_static(property, h) {
        Some(v) => Some(v),
        None if !matches!(
            h.kind,
            VpiKind::SysTfCall | VpiKind::Constant | VpiKind::Iterator
        ) =>
        {
            try_active_sim("vpi_get", |sim| vpi_model::get(sim, property, h)).flatten()
        }
        None => None,
    };
    if let Some(v) = model {
        return v;
    }
    if property == vpi::TYPE {
        return h.type_code;
    }
    if property == vpi::DIRECTION {
        return h.direction;
    }
    // Properties of the objects `vpi_api` adds (systf, timing objects,
    // packed-arena memory words).
    if let Some(v) = vpi_api::get_property(property, h) {
        return v;
    }
    // vpiFuncType of a system function call is its `sysfunctype`.
    if h.kind == VpiKind::SysTfCall {
        return match property {
            vpi::SIZE => h.width as libc::c_int,
            vpi::FUNC_TYPE => VPI_SYSTF_STACK
                .with(|st| st.borrow().get(h.frame).map(|f| f.func_type))
                .unwrap_or(vpi::UNDEFINED),
            _ => vpi::UNDEFINED,
        };
    }
    if h.kind == VpiKind::Constant {
        let w = h.width as libc::c_int;
        return match property {
            vpi::SIZE => w,
            vpi::SIGNED => i32::from(h.value.as_ref().is_some_and(|v| v.is_signed)),
            vpi::SCALAR => i32::from(w == 1),
            vpi::VECTOR => i32::from(w > 1),
            _ => vpi::UNDEFINED,
        };
    }
    // A module or an iterator has no width, sign or scalar-ness.
    if property == vpi::DIRECTION {
        return match h.kind {
            VpiKind::Port | VpiKind::Signal => h.direction,
            _ => vpi::UNDEFINED,
        };
    }
    match h.kind {
        VpiKind::Module | VpiKind::Iterator | VpiKind::Callback => return vpi::UNDEFINED,
        // vpiSize of a memory is its number of words; of a slice, its bits.
        VpiKind::Memory | VpiKind::Slice if property == vpi::SIZE => return h.width as libc::c_int,
        _ => {}
    }
    let sig_id = h.signal_id;
    try_active_sim("vpi_get", |sim| {
        let width = sim.signal_widths.get(sig_id).copied().unwrap_or(32) as libc::c_int;
        match property {
            vpi::SIZE => width,
            vpi::SIGNED => i32::from(sim.signal_signed.get(sig_id).copied().unwrap_or(false)),
            vpi::SCALAR => i32::from(width == 1),
            vpi::VECTOR => i32::from(width > 1),
            _ => vpi::UNDEFINED,
        }
    })
    .unwrap_or(vpi::UNDEFINED)
}

/// Base-10 exponent of a seconds value, as VPI reports time unit/precision.
/// Mirrors `Simulator::secs_to_exp`, which is a private associated fn.
pub(super) fn secs_to_exp(s: f64) -> libc::c_int {
    if s <= 0.0 {
        -9
    } else {
        s.log10().round() as libc::c_int
    }
}

/// Store `s` into the thread-local string scratch as a NUL-terminated C
/// string and return a pointer to it.
pub(super) fn vpi_str_scratch(s: &str) -> *mut libc::c_char {
    VPI_STR_SCRATCH.with(|cell| {
        let mut buf = cell.borrow_mut();
        buf.clear();
        buf.extend_from_slice(s.as_bytes());
        buf.push(0);
        buf.as_mut_ptr() as *mut libc::c_char
    })
}

/// Read the current simulation time.
///
/// `object` is accepted for IEEE compatibility and currently ignored.
/// `time_p->type` chooses the requested representation:
/// - `vpiSimTime`: fill `high`/`low` and also mirror into `real`
/// - `vpiScaledRealTime`: fill `real` and also mirror into `high`/`low`
#[unsafe(no_mangle)]
pub extern "C" fn vpi_get_time(_object: *mut libc::c_void, time_p: *mut s_vpi_time) {
    if time_p.is_null() {
        return;
    }

    let now = try_active_sim("vpi_get_time", |sim| sim.time).unwrap_or(0);
    let tp = unsafe { &mut *time_p };
    match tp.type_ {
        vpi::SCALED_REAL_TIME | vpi::SIM_TIME | vpi::SUPPRESS_TIME => {}
        _ => tp.type_ = vpi::SIM_TIME,
    }

    tp.high = (now >> 32) as u32;
    tp.low = (now & 0xFFFF_FFFF) as u32;
    tp.real = now as f64;
}

// ============================================================================
// VPI/DPI surface
// ============================================================================
// The functions below are the minimum subset of IEEE 1800 §35 (DPI) and
// §38 (VPI) needed for standard DPI/VPI libraries to compile and link
// against xezim. They are intentionally simple — each does exactly what
// the VPI/DPI interface specifies. Symbols not exercised are stubbed.
//
// Naming matches the C standard exactly; every function is `#[no_mangle]
// pub extern "C"` so it lands in the simulator's symbol table and is
// resolvable by `dlsym` when DPI imports fire.

// --- vpi_get_vlog_info ------------------------------------------------------
//
// VPI callers call this once at startup to discover the tool name + version.
// We hand back the argv passed via Simulator::set_args (the CString pointers
// stay valid for the simulator lifetime) and a static product/version string.
//
// `info_p` must be allocated by the caller; we fill it in place.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_get_vlog_info(info_p: *mut s_vpi_vlog_info) -> libc::c_int {
    if info_p.is_null() {
        return 0;
    }
    let info = unsafe { &mut *info_p };
    info.argc = 0;
    info.argv = std::ptr::null_mut();
    info.product = b"xezim\0".as_ptr() as *mut libc::c_char;
    // The crate version, so this never drifts from the binary again. The
    // NUL is appended by concat! since env! is a &'static str.
    info.version = concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr() as *mut libc::c_char;

    try_active_sim("vpi_get_vlog_info", |sim| {
        info.argc = sim.vpi_argv.len() as libc::c_int;
        if !sim.vpi_argv.is_empty() {
            info.argv = sim.vpi_argv.as_mut_ptr();
        }
        1
    })
    .unwrap_or(0)
}

// --- vpi_release_handle -----------------------------------------------------
//
// Used by VPI code paths; semantically equivalent to `vpi_free_object`.
// We just delegate to the existing free function.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_release_handle(handle: *mut libc::c_void) -> libc::c_int {
    vpi_free_object(handle)
}

// --- svDpiVersion -----------------------------------------------------------
//
// Returns the DPI version string. Used for log banners and to
// decide which DPI features are available.
#[unsafe(no_mangle)]
pub extern "C" fn svDpiVersion() -> *const libc::c_char {
    // Static C string — no lifetime concerns.
    b"1800-2017\0".as_ptr() as *const libc::c_char
}

// --- svGetScopeFromName -----------------------------------------------------
//
// Returns an opaque handle to the named scope (a module, package, or
// interface instance). DPI code calls this with scope names (e.g. `pkg`,
// `top.dut`). We cache the handles in a HashMap so the same scope always
// returns the same pointer (svSetScope+svGetScope rely on this for equality).
//
// If the scope name is not found we still return a non-null handle
// pointing at a new `DpiScope` — synthetic scopes work for round-tripping
// through `svGetNameFromScope`.
#[unsafe(no_mangle)]
pub extern "C" fn svGetScopeFromName(name: *const libc::c_char) -> *mut libc::c_void {
    if name.is_null() {
        return std::ptr::null_mut();
    }
    let name_str = unsafe { std::ffi::CStr::from_ptr(name) }
        .to_string_lossy()
        .trim()
        .to_string();

    with_active_sim(|sim| {
        if let Some(&existing) = sim.dpi_scopes.get(&name_str) {
            return existing;
        }
        let mut scope = DpiScope {
            name: [0u8; 256],
            name_len: name_str.len().min(255),
        };
        let bytes = name_str.as_bytes();
        scope.name[..scope.name_len].copy_from_slice(&bytes[..scope.name_len]);
        let ptr = Box::into_raw(Box::new(scope)) as *mut libc::c_void;
        sim.dpi_scopes.insert(name_str, ptr);
        ptr
    })
}

// --- svGetNameFromScope -----------------------------------------------------
//
// Recovers the scope name from a handle. DPI code round-trips scope handles
// through this when emitting diagnostics.
#[unsafe(no_mangle)]
pub extern "C" fn svGetNameFromScope(scope: *mut libc::c_void) -> *const libc::c_char {
    if scope.is_null() {
        return std::ptr::null();
    }
    unsafe {
        let s = &*(scope as *const DpiScope);
        // The name is stored as fixed-size [u8; 256] with a length prefix, so
        // it is not NUL-terminated in our storage and we must hand back a
        // fresh buffer. The CALLER FREES IT WITH libc `free` (see
        // tests/dpi/uvm_dpi_test.c), so the buffer MUST come from libc
        // `malloc`. Returning a Rust-allocated `CString::into_raw()` only
        // worked while Rust's global allocator happened to BE glibc malloc;
        // installing any other global allocator turned it into an invalid
        // free. UVM calls this sparingly, so the copy is negligible.
        let bytes = &s.name[..s.name_len];
        // Preserve the previous `CString::new(..).unwrap_or_default()`
        // behaviour: an interior NUL yields an empty string.
        let n = if bytes.contains(&0) { 0 } else { bytes.len() };
        let buf = libc::malloc(n + 1) as *mut u8;
        if buf.is_null() {
            return std::ptr::null();
        }
        if n > 0 {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf, n);
        }
        *buf.add(n) = 0;
        buf as *const libc::c_char
    }
}

// --- svGetScope -------------------------------------------------------------
//
// Returns the currently-active scope set by `svSetScope`. Used when an import
// is called and the DPI runtime needs to know "where am I?".
#[unsafe(no_mangle)]
pub extern "C" fn svGetScope() -> *mut libc::c_void {
    ACTIVE_SCOPE.with(|cell| cell.get())
}

// --- svSetScope -------------------------------------------------------------
//
// Updates the currently-active scope. xezim's exec_dpi_import_call
// sets the scope to the import's declaration site before invoking the
// C function, then restores the previous value on the way out.
#[unsafe(no_mangle)]
pub extern "C" fn svSetScope(scope: *mut libc::c_void) -> *mut libc::c_void {
    let prev = ACTIVE_SCOPE.with(|cell| cell.get());
    ACTIVE_SCOPE.with(|cell| cell.set(scope));
    prev
}

// --- svGetTime / svGetTimeUnit / svGetTimePrecision -------------------------
//
// The time queries for DPI code (#206). A scope naming a module definition or
// an instance path answers with that module's timescale; NULL answers with the
// simulation's (its finest precision). Exponents are powers of ten in seconds
// (-9 = 1 ns), as `vpi_get(vpiTimeUnit, ...)` reports them.

fn dpi_scope_timescale(sim: &Simulator, scope: *mut libc::c_void) -> (i32, i32) {
    if scope.is_null() {
        let exp = secs_to_exp(sim.module.tick_s);
        return (exp, exp);
    }
    let s = unsafe { &*(scope as *const DpiScope) };
    let name = String::from_utf8_lossy(&s.name[..s.name_len]).into_owned();
    let rel = vpi_strip_top(sim, &name);
    let def = if rel.is_empty() || name == sim.module.name {
        sim.module.name.clone()
    } else if let Some(inst) = sim.instance_by_path(&*rel) {
        inst.def_name.clone()
    } else if sim.module.module_timescale_exp.contains_key(name.as_str()) {
        name
    } else {
        sim.module.name.clone()
    };
    sim.reported_timescale_exp(&def)
}

#[unsafe(no_mangle)]
pub extern "C" fn svGetTimeUnit(scope: *mut libc::c_void, time_unit: *mut i32) -> libc::c_int {
    if time_unit.is_null() {
        return -1;
    }
    match try_active_sim("svGetTimeUnit", |sim| dpi_scope_timescale(sim, scope).0) {
        Some(u) => {
            unsafe { *time_unit = u };
            0
        }
        None => -1,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn svGetTimePrecision(
    scope: *mut libc::c_void,
    time_precision: *mut i32,
) -> libc::c_int {
    if time_precision.is_null() {
        return -1;
    }
    match try_active_sim("svGetTimePrecision", |sim| {
        dpi_scope_timescale(sim, scope).1
    }) {
        Some(p) => {
            unsafe { *time_precision = p };
            0
        }
        None => -1,
    }
}

/// Current simulation time. `time->type` selects the form: `vpiSimTime`
/// fills `high`/`low` with simulation ticks, `vpiScaledRealTime` fills
/// `real` in the scope's time unit.
#[unsafe(no_mangle)]
pub extern "C" fn svGetTime(scope: *mut libc::c_void, time: *mut s_vpi_time) -> libc::c_int {
    if time.is_null() {
        return -1;
    }
    let Some((ticks, tick_s, unit)) = try_active_sim("svGetTime", |sim| {
        (
            sim.time,
            sim.module.tick_s,
            dpi_scope_timescale(sim, scope).0,
        )
    }) else {
        return -1;
    };
    let t = unsafe { &mut *time };
    if t.type_ == vpi::SCALED_REAL_TIME {
        t.real = ticks as f64 * tick_s / 10f64.powi(unit);
    } else {
        t.type_ = vpi::SIM_TIME;
        t.high = (ticks >> 32) as u32;
        t.low = (ticks & 0xFFFF_FFFF) as u32;
    }
    0
}

// --- Multichannel descriptors and output (#205) -----------------------------
//
// vpi_mcd_open/close/flush/name share the channel table `$fopen` uses, so a
// descriptor opened from C can be written from SystemVerilog and the other way
// round. vpi_mcd_printf is C-variadic, so it lives in vpi_printf_shim.c, which
// formats the text and hands the file channels to xezim_vpi_mcd_write.

#[unsafe(no_mangle)]
pub extern "C" fn vpi_mcd_open(name: *const libc::c_char) -> u32 {
    if name.is_null() {
        return 0;
    }
    let path = unsafe { std::ffi::CStr::from_ptr(name) }
        .to_string_lossy()
        .into_owned();
    try_active_sim("vpi_mcd_open", |sim| {
        sim.open_file_path(&path, true, "w").to_u64().unwrap_or(0) as u32
    })
    .unwrap_or(0)
}

/// Closes the file channels in `mcd`; bit 0 (stdout) stays open. Returns 0.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_mcd_close(mcd: u32) -> u32 {
    let files = mcd & !1;
    if files == 0 {
        return 0;
    }
    try_active_sim("vpi_mcd_close", |sim| {
        sim.close_channels(files as i32);
        0
    })
    .unwrap_or(mcd)
}

#[unsafe(no_mangle)]
pub extern "C" fn vpi_mcd_flush(mcd: u32) -> libc::c_int {
    if mcd == 0 {
        return 0;
    }
    try_active_sim("vpi_mcd_flush", |sim| {
        sim.flush_channels(mcd as i32);
        0
    })
    .unwrap_or(1)
}

thread_local! {
    static VPI_MCD_NAME: RefCell<std::ffi::CString> = RefCell::new(std::ffi::CString::default());
}

/// The file name behind the lowest channel set in `cd` ("stdout" for bit 0),
/// or NULL for a channel that is not open. Valid until the next call.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_mcd_name(cd: u32) -> *mut libc::c_char {
    if cd == 0 {
        return std::ptr::null_mut();
    }
    let bit = cd.trailing_zeros() as i32;
    let name = if bit == 0 {
        Some("stdout".to_string())
    } else {
        try_active_sim("vpi_mcd_name", |sim| sim.mcd_names.get(&bit).cloned()).flatten()
    };
    match name {
        Some(n) => VPI_MCD_NAME.with(|c| {
            *c.borrow_mut() = std::ffi::CString::new(n).unwrap_or_default();
            c.borrow().as_ptr() as *mut libc::c_char
        }),
        None => std::ptr::null_mut(),
    }
}

/// Backend of `vpi_mcd_printf` for the file channels in `mcd` (the shim
/// prints bit 0 itself, next to `vpi_printf`).
#[unsafe(no_mangle)]
pub extern "C" fn xezim_vpi_mcd_write(
    mcd: u32,
    buf: *const libc::c_char,
    len: libc::c_int,
) -> libc::c_int {
    let files = mcd & !1;
    if buf.is_null() || len <= 0 || files == 0 {
        return 0;
    }
    let bytes = unsafe { std::slice::from_raw_parts(buf as *const u8, len as usize) };
    let text = String::from_utf8_lossy(bytes).into_owned();
    try_active_sim("vpi_mcd_printf", |sim| {
        sim.write_channels(files as i32, &text);
        len
    })
    .unwrap_or(0)
}

/// Flushes the simulator's output: stdout and every open file. Returns 0.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_flush() -> libc::c_int {
    try_active_sim("vpi_flush", |sim| {
        sim.flush_channels(0);
        0
    })
    .unwrap_or(0)
}

/// 1 when both handles refer to the same object, else 0.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_compare_objects(a: *mut libc::c_void, b: *mut libc::c_void) -> libc::c_int {
    if a.is_null() || b.is_null() {
        return 0;
    }
    if a == b {
        return 1;
    }
    let (Some(x), Some(y)) = (unsafe { vpi_deref(a) }, unsafe { vpi_deref(b) }) else {
        return 0;
    };
    if x.kind == VpiKind::Iterator {
        return 0;
    }
    i32::from(
        x.kind == y.kind
            && x.signal_id == y.signal_id
            && x.inst_idx == y.inst_idx
            && x.lsb == y.lsb
            && x.width == y.width
            && x.full_name == y.full_name,
    )
}

/// `vpi_get` with a 64-bit result.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_get64(property: libc::c_int, handle: *mut libc::c_void) -> i64 {
    i64::from(vpi_get(property, handle))
}
