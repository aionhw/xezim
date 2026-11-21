//! The VPI routines of IEEE 1800-2017 clause 38 beyond single-object reads,
//! writes and design traversal:
//!
//! * per-call user data, `vpi_put_userdata` / `vpi_get_userdata`
//!   (§38.33 / §38.14), and `vpi_get_systf_info` (§38.12) with the
//!   `vpiUserSystf` object it reads;
//! * `vpi_handle_by_multi_index` (§38.20) over unpacked arrays of any
//!   dimension, and `vpi_handle_multi` (§38.22, `vpiInterModPath`);
//! * `vpi_get_value_array` / `vpi_put_value_array` (§38.16 / §38.35);
//! * `vpi_get_delays` / `vpi_put_delays` (§38.10 / §38.32) on nets, module
//!   paths, timing checks and intermodule paths;
//! * `vpi_get_data` / `vpi_put_data` (§38.9 / §38.31), defined outside a
//!   save or restart — the only state xezim, which has neither, is ever in;
//! * the value formats `vpi_get_value` / `vpi_put_value` hand off here:
//!   vpiStrengthVal, vpiShortIntVal, vpiLongIntVal, vpiShortRealVal,
//!   vpiRawTwoStateVal and vpiRawFourStateVal.
//!
//! Nothing here is on a simulation path: every function runs only when a
//! VPI or DPI library calls in (or, for the call-site bookkeeping, when a
//! registered `$systf` is invoked).
use super::*;
use libc::{c_int, c_void};
use std::cell::{Cell, RefCell};

/// VPI constants used only by this module, with the values IEEE 1800-2017
/// Annex K assigns them (see `include/vpi_user.h`).
pub(super) mod vc {
    use libc::c_int;

    // Object types.
    pub const INTER_MOD_PATH: c_int = 26;
    pub const MOD_PATH: c_int = 31;
    pub const NET_BIT: c_int = 37;
    pub const REG_BIT: c_int = 49;
    pub const TCHK: c_int = 61;
    pub const USER_SYSTF: c_int = 67;
    pub const NET_ARRAY: c_int = 114;
    pub const REG_ARRAY: c_int = 116;

    // Properties.
    pub const TCHK_TYPE: c_int = 38;
    pub const USER_DEFN: c_int = 45;

    // Value formats beyond `vpi`'s.
    pub const STRENGTH_VAL: c_int = 10;
    pub const SHORT_INT_VAL: c_int = 14;
    pub const LONG_INT_VAL: c_int = 15;
    pub const SHORT_REAL_VAL: c_int = 16;
    pub const RAW_TWO_STATE_VAL: c_int = 17;
    pub const RAW_FOUR_STATE_VAL: c_int = 18;

    // vpiScalarVal codes beyond `vpi`'s.
    pub const SCALAR_H: c_int = 4;
    pub const SCALAR_L: c_int = 5;

    // s_vpi_arrayvalue.flags.
    pub const USER_ALLOC_FLAG: u32 = 0x2000;
    pub const ONE_VALUE: u32 = 0x4000;
    pub const PROPAGATE_OFF: u32 = 0x8000;
    pub const NO_DELAY: u32 = 1;

    // Strength codes (s_vpi_strengthval.s0 / .s1).
    pub const SUPPLY_DRIVE: c_int = 0x80;
    pub const STRONG_DRIVE: c_int = 0x40;
    pub const PULL_DRIVE: c_int = 0x20;
    pub const LARGE_CHARGE: c_int = 0x10;
    pub const WEAK_DRIVE: c_int = 0x08;
    pub const MEDIUM_CHARGE: c_int = 0x04;
    pub const SMALL_CHARGE: c_int = 0x02;
    pub const HI_Z: c_int = 0x01;
}

/// Mirror of `s_vpi_strengthval` (Figure 38-9).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct s_vpi_strengthval {
    logic: c_int,
    s0: c_int,
    s1: c_int,
}

/// Mirror of `s_vpi_delay` (Figure 38-3).
#[repr(C)]
pub struct s_vpi_delay {
    da: *mut s_vpi_time,
    no_of_delays: c_int,
    time_type: c_int,
    mtm_flag: c_int,
    append_flag: c_int,
    pulsere_flag: c_int,
}

/// Mirror of `s_vpi_arrayvalue` (§38.16). Every member of its value union
/// is a pointer, so one pointer field stands for the union.
#[repr(C)]
pub struct s_vpi_arrayvalue {
    format: u32,
    flags: u32,
    value: *mut c_void,
}

// ===========================================================================
// System task / function call instances, user data and vpiUserSystf
// ===========================================================================

/// One `$systf` call instance: a call site in one instance scope.
struct SystfSite {
    /// `vpi_put_userdata`'s value.
    userdata: usize,
    /// compiletf already ran for this instance.
    called: bool,
}

#[derive(Default)]
struct SiteTable {
    /// (`$name`, span start, span end, instance scope) -> site index.
    index: HashMap<(String, usize, usize, String), usize>,
    sites: Vec<SystfSite>,
}

thread_local! {
    static SITES: RefCell<SiteTable> = RefCell::new(SiteTable::default());
    /// Registered `$name`s in registration order, each with a C copy of the
    /// name that stays put for `vpi_get_systf_info`'s `tfname`.
    static REGISTERED: RefCell<Vec<(String, std::ffi::CString)>> =
        const { RefCell::new(Vec::new()) };
    /// Source span of the system task call statement being executed; read
    /// back when that task turns out to be a registered `$systf`.
    static TASK_CALL_SPAN: Cell<(usize, usize)> = const { Cell::new((0, 0)) };
}

/// Remember the span of the system-task statement about to execute.
#[inline(always)]
pub(super) fn set_task_call_span(span: crate::ast::Span) {
    TASK_CALL_SPAN.with(|c| c.set((span.start, span.end)));
}

/// The span recorded by `set_task_call_span`, cleared so a later call that
/// bypasses the statement path cannot inherit it.
pub(super) fn take_task_call_span() -> crate::ast::Span {
    let (start, end) = TASK_CALL_SPAN.with(|c| c.replace((0, 0)));
    crate::ast::Span::new(start, end)
}

/// The call instance of a `$systf` invocation (§38.33: user data belongs to a
/// system task or function call INSTANCE): the call's source span in the
/// instance scope executing it, so one `$name` called from two places, or
/// from one place in two module instances, has two instances, while a call
/// in a loop is the same instance on every iteration. Returns the site and
/// whether this is its first invocation.
pub(super) fn systf_site(sim: &Simulator, name: &str, span: crate::ast::Span) -> (usize, bool) {
    let scope = if sim.m_block_active {
        sim.m_block_scope.clone()
    } else {
        sim.current_scope.clone()
    };
    let key = (name.to_string(), span.start, span.end, scope);
    SITES.with(|t| {
        let mut t = t.borrow_mut();
        let next = t.sites.len();
        let idx = *t.index.entry(key).or_insert(next);
        if idx == next {
            t.sites.push(SystfSite {
                userdata: 0,
                called: false,
            });
        }
        let first = !t.sites[idx].called;
        t.sites[idx].called = true;
        (idx, first)
    })
}

/// Record a `vpi_register_systf` registration (first registration order).
pub(super) fn note_registered_systf(name: &str) {
    REGISTERED.with(|r| {
        let mut r = r.borrow_mut();
        if !r.iter().any(|(n, _)| n == name) {
            let c = std::ffi::CString::new(name).unwrap_or_default();
            r.push((name.to_string(), c));
        }
    });
}

/// The `vpiUserSystf` object for a registered `$name`.
pub(super) fn user_systf_handle(name: &str) -> VpiHandle {
    let mut h = VpiHandle::signal(0, vc::USER_SYSTF, name, name);
    h.kind = VpiKind::UserSystf;
    h
}

/// `vpi_handle(vpiUserSystf, call)`: the registration behind a call handle.
pub(super) fn user_systf_of_call(refh: *mut c_void) -> *mut c_void {
    let Some(h) = (unsafe { vpi_deref(refh) }) else {
        vpi_error(
            vpi::ERROR,
            "vpi_handle(vpiUserSystf): the reference must be a system task or function call handle"
                .into(),
        );
        return std::ptr::null_mut();
    };
    if h.kind != VpiKind::SysTfCall || !vpi_systf_registered(&h.name) {
        vpi_error(
            vpi::ERROR,
            "vpi_handle(vpiUserSystf): the reference is not a call of a registered $systf".into(),
        );
        return std::ptr::null_mut();
    }
    user_systf_handle(&h.name).into_raw()
}

fn systf_call_site(obj: *mut c_void, who: &str) -> Option<usize> {
    match unsafe { vpi_deref(obj) } {
        Some(h) if h.kind == VpiKind::SysTfCall => Some(h.signal_id),
        Some(_) => {
            vpi_error(
                vpi::ERROR,
                format!("{}: not a system task or function call handle", who),
            );
            None
        }
        None => {
            vpi_error(vpi::ERROR, format!("{}: null handle", who));
            None
        }
    }
}

/// §38.33: attach `userdata` to a system task/function call instance.
/// Returns 1 on success, 0 on failure.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_put_userdata(obj: *mut c_void, userdata: *mut c_void) -> c_int {
    let Some(site) = systf_call_site(obj, "vpi_put_userdata") else {
        return 0;
    };
    SITES.with(|t| match t.borrow_mut().sites.get_mut(site) {
        Some(s) => {
            s.userdata = userdata as usize;
            1
        }
        None => 0,
    })
}

/// §38.14: the user data of a call instance; NULL when none was put, or on
/// failure.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_get_userdata(obj: *mut c_void) -> *mut c_void {
    let Some(site) = systf_call_site(obj, "vpi_get_userdata") else {
        return std::ptr::null_mut();
    };
    SITES.with(|t| {
        t.borrow()
            .sites
            .get(site)
            .map_or(std::ptr::null_mut(), |s| s.userdata as *mut c_void)
    })
}

/// §38.12: fill `data` from a `vpiUserSystf` handle.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_get_systf_info(obj: *mut c_void, data: *mut s_vpi_systf_data) {
    if data.is_null() {
        vpi_error(
            vpi::ERROR,
            "vpi_get_systf_info: null s_vpi_systf_data pointer".into(),
        );
        return;
    }
    let name = match unsafe { vpi_deref(obj) } {
        Some(h) if h.kind == VpiKind::UserSystf => h.name.clone(),
        _ => {
            vpi_error(
                vpi::ERROR,
                "vpi_get_systf_info: not a vpiUserSystf handle".into(),
            );
            return;
        }
    };
    let Some(entry) = VPI_SYSTFS.with(|m| m.borrow().get(&name).copied()) else {
        vpi_error(
            vpi::ERROR,
            format!("vpi_get_systf_info: '{}' is not registered", name),
        );
        return;
    };
    let tfname = REGISTERED.with(|r| {
        r.borrow()
            .iter()
            .find(|(n, _)| *n == name)
            .map_or(std::ptr::null_mut(), |(_, c)| {
                c.as_ptr() as *mut libc::c_char
            })
    });
    let d = unsafe { &mut *data };
    d.type_ = entry.tf_type;
    d.sysfunctype = entry.func_type;
    d.tfname = tfname;
    d.calltf = entry.calltf as *mut c_void;
    d.compiletf = entry.compiletf as *mut c_void;
    d.sizetf = entry.sizetf as *mut c_void;
    d.user_data = entry.user_data as *mut libc::c_char;
}

/// `vpi_iterate` for the relations this module adds: `vpiUserSystf` (from
/// NULL), `vpiModPath` and `vpiTchk` (from a module). `None` for any other
/// type, which `vpi_iterate` then handles itself.
pub(super) fn iterate_extra(type_: c_int, refh: *mut c_void) -> Option<*mut c_void> {
    match type_ {
        vc::USER_SYSTF => {
            if !refh.is_null() {
                vpi_error(
                    vpi::ERROR,
                    "vpi_iterate(vpiUserSystf): the reference must be NULL".into(),
                );
                return Some(std::ptr::null_mut());
            }
            let names: Vec<String> = REGISTERED.with(|r| {
                r.borrow()
                    .iter()
                    .map(|(n, _)| n.clone())
                    .filter(|n| vpi_systf_registered(n))
                    .collect()
            });
            Some(vpi_make_iterator(
                names.iter().map(|n| user_systf_handle(n)).collect(),
            ))
        }
        vc::MOD_PATH | vc::TCHK => Some(iterate_timing_objects(type_, refh)),
        _ => None,
    }
}

/// The module paths / timing checks of one module instance.
fn iterate_timing_objects(type_: c_int, refh: *mut c_void) -> *mut c_void {
    let Some(h) = (unsafe { vpi_deref(refh) }) else {
        return std::ptr::null_mut();
    };
    if h.kind != VpiKind::Module {
        return std::ptr::null_mut();
    }
    try_active_sim("vpi_iterate", |sim| {
        let scope = vpi_scope_path(sim, h);
        let mut items: Vec<VpiHandle> = Vec::new();
        if type_ == vc::MOD_PATH {
            let mut nets: Vec<(String, usize, usize)> = sim
                .module_paths
                .as_ref()
                .map(|mp| mp.vpi_nets())
                .unwrap_or_default()
                .into_iter()
                .map(|(id, n)| (sim.name_for_id(id).to_string(), id, n))
                .filter(|(name, _, _)| scope_of(name) == scope)
                .collect();
            nets.sort();
            for (name, id, n) in nets {
                for i in 0..n {
                    items.push(mod_path_handle(sim, &name, id, i));
                }
            }
        } else {
            for i in 0..sim.vpi_tchk_count() {
                let Some((name, tscope, _)) = sim.vpi_tchk_ident(i) else {
                    continue;
                };
                if tscope == scope {
                    let full = format!("{}.{}", sim.hier_path(tscope), name);
                    let mut t = VpiHandle::signal(i, vc::TCHK, name, &full);
                    t.kind = VpiKind::Tchk;
                    items.push(t);
                }
            }
        }
        vpi_make_iterator(items)
    })
    .unwrap_or(std::ptr::null_mut())
}

/// A module path or timing check handed out by the object model
/// (`vpi_model.rs`) names the same simulator object as the handles made
/// here: the k-th path into a given output net of an instance, or the k-th
/// timing check of an instance, both lists in source order. Swap such a
/// handle for this module's own before the simulator is borrowed.
fn native_timing_handle(obj: *mut c_void) -> Option<*mut c_void> {
    const MODULE: c_int = 32;
    const EXPR: c_int = 102;
    const MOD_PATH_OUT: c_int = 96;
    let h = unsafe { vpi_deref(obj) }?;
    if h.kind != VpiKind::Obj || (h.type_code != vc::MOD_PATH && h.type_code != vc::TCHK) {
        return None;
    }
    let (type_, model_idx) = (h.type_code, h.signal_id);
    let scan = |it: *mut c_void| -> Vec<*mut c_void> {
        let mut v = Vec::new();
        if !it.is_null() {
            loop {
                let x = super::vpi_scan(it);
                if x.is_null() {
                    break;
                }
                v.push(x);
            }
        }
        v
    };
    let module = super::vpi_handle(MODULE, obj);
    if module.is_null() {
        return None;
    }
    let model_list = scan(super::vpi_iterate(type_, module));
    let k = model_list
        .iter()
        .position(|&x| unsafe { vpi_deref(x) }.is_some_and(|y| y.signal_id == model_idx))?;
    let native_list = scan(iterate_timing_objects(type_, module));
    if type_ == vc::TCHK {
        return native_list.get(k).copied();
    }
    let out_net = |p: *mut c_void| -> Option<usize> {
        let terms = scan(super::vpi_iterate(MOD_PATH_OUT, p));
        let e = super::vpi_handle(EXPR, *terms.first()?);
        unsafe { vpi_deref(e) }.map(|eh| eh.signal_id)
    };
    let net = out_net(model_list[k])?;
    let ordinal = model_list[..k]
        .iter()
        .filter(|&&p| out_net(p) == Some(net))
        .count();
    native_list.into_iter().find(|&p| {
        unsafe { vpi_deref(p) }.is_some_and(|n| n.signal_id == net && n.lsb as usize == ordinal)
    })
}

/// The instance scope a design-relative object name is declared in.
fn scope_of(name: &str) -> &str {
    name.rsplit_once('.').map_or("", |(s, _)| s)
}

fn mod_path_handle(sim: &Simulator, net: &str, id: usize, i: usize) -> VpiHandle {
    // A module path has no name; its full name is its output net's, which
    // is what identifies it (and what `vpi_handle(vpiScope, ..)` walks).
    let mut h = VpiHandle::signal(id, vc::MOD_PATH, "", &vpi_full_name(sim, net));
    h.kind = VpiKind::ModPath;
    h.lsb = i as u32;
    h
}

/// §31 check name -> vpiTchkType.
fn tchk_type_code(name: &str) -> c_int {
    match name {
        "$setup" => 1,
        "$hold" => 2,
        "$period" => 3,
        "$width" => 4,
        "$skew" => 5,
        "$recovery" => 6,
        "$nochange" => 7,
        "$setuphold" => 8,
        "$fullskew" => 9,
        "$recrem" => 10,
        "$removal" => 11,
        "$timeskew" => 12,
        _ => vpi::UNDEFINED,
    }
}

/// `vpi_get` for the objects this module adds, and for words of a
/// packed-arena memory (which have no side-table entries). `None` leaves
/// the property to `vpi_get`.
pub(super) fn get_property(property: c_int, h: &VpiHandle) -> Option<c_int> {
    match h.kind {
        VpiKind::UserSystf | VpiKind::ModPath | VpiKind::InterModPath => Some(vpi::UNDEFINED),
        VpiKind::Tchk => Some(if property == vc::TCHK_TYPE {
            try_active_sim("vpi_get", |sim| {
                sim.vpi_tchk_ident(h.signal_id)
                    .map_or(vpi::UNDEFINED, |(n, _, _)| tchk_type_code(n))
            })
            .unwrap_or(vpi::UNDEFINED)
        } else {
            vpi::UNDEFINED
        }),
        // Every call handle xezim hands out is of a user-defined $systf.
        VpiKind::SysTfCall if property == vc::USER_DEFN => Some(1),
        VpiKind::Signal | VpiKind::Port if is_packed_id(h.signal_id) => {
            try_active_sim("vpi_get", |sim| {
                let w = sim.packed.width(h.signal_id) as c_int;
                match property {
                    vpi::SIZE => w,
                    vpi::SIGNED => c_int::from(sim.packed.is_signed(h.signal_id)),
                    vpi::SCALAR => c_int::from(w == 1),
                    vpi::VECTOR => c_int::from(w > 1),
                    _ => vpi::UNDEFINED,
                }
            })
        }
        _ => None,
    }
}

// ===========================================================================
// Cells: signal-table slots and packed-arena memory words alike
// ===========================================================================

/// The value of a signal or a packed-arena memory word; `None` for an id
/// that is neither.
pub(super) fn cell_value(sim: &Simulator, id: usize) -> Option<Value> {
    if is_packed_id(id) {
        sim.packed.locate(id).is_some().then(|| sim.packed.read(id))
    } else {
        sim.signal_table.get(id).cloned()
    }
}

pub(super) fn cell_width(sim: &Simulator, id: usize) -> Option<u32> {
    if is_packed_id(id) {
        sim.packed
            .locate(id)
            .is_some()
            .then(|| sim.packed.width(id))
    } else {
        sim.signal_widths.get(id).copied()
    }
}

pub(super) fn cell_signed(sim: &Simulator, id: usize) -> bool {
    if is_packed_id(id) {
        sim.packed.is_signed(id)
    } else {
        sim.signal_signed.get(id).copied().unwrap_or(false)
    }
}

fn cell_real(sim: &Simulator, id: usize) -> bool {
    !is_packed_id(id) && sim.signal_real.get(id).copied().unwrap_or(false)
}

fn cell_two_state(sim: &Simulator, id: usize) -> bool {
    if is_packed_id(id) {
        // A two-state element stores no x/z plane.
        sim.packed.locate(id).is_some_and(|(seg, _)| seg.xmask == 0)
    } else {
        sim.signal_two_state.get(id).copied().unwrap_or(false)
    }
}

/// A VPI deposit (`vpi_put_value` with vpiNoDelay): the value lands and the
/// signal's readers are woken, exactly as `vpi_put_value` does it.
fn deposit(sim: &mut Simulator, id: usize, value: Value) {
    if is_packed_id(id) {
        sim.cell_write(id, &value);
        return;
    }
    write_sig!(sim, id, value);
    sim.after_signal_write(id);
    if id < sim.dirty_signals.len() && !sim.dirty_signals[id] {
        sim.dirty_signals[id] = true;
        sim.dirty_list.push(id);
    }
    sim.dirty_any = true;
}

/// `vpiPropagateOff`: the value lands, but nothing that reads the signal is
/// told (§38.35) — no wake-ups, no value-change callbacks.
fn deposit_quiet(sim: &mut Simulator, id: usize, value: Value) {
    if is_packed_id(id) {
        sim.packed.write(id, &value);
        return;
    }
    write_sig!(sim, id, value);
}

// ===========================================================================
// Value formats
// ===========================================================================

thread_local! {
    /// (s0, s1) strength codes of the object whose value is being filled,
    /// set by `vpi_get_value` and the value-change dispatcher around
    /// `fill_vpi_value`; `None` means strong (a variable, or a net with no
    /// recorded drive strength).
    static STRENGTH_HINT: Cell<Option<(c_int, c_int)>> = const { Cell::new(None) };
    static STRENGTH_SCRATCH: RefCell<Vec<s_vpi_strengthval>> = const { RefCell::new(Vec::new()) };
    static LONG_SCRATCH: RefCell<i64> = const { RefCell::new(0) };
    static RAW_SCRATCH: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    /// `vpi_get_value_array`'s simulator-owned buffer (u64 for alignment).
    static ARRAY_SCRATCH: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
}

/// A §10.3.1 / §28.4 strength keyword as a vpiStrengthVal code.
fn strength_code(tok: &str) -> c_int {
    if tok.starts_with("supply") {
        vc::SUPPLY_DRIVE
    } else if tok.starts_with("pull") {
        vc::PULL_DRIVE
    } else if tok.starts_with("weak") {
        vc::WEAK_DRIVE
    } else if tok.starts_with("highz") {
        vc::HI_Z
    } else if tok.starts_with("large") {
        vc::LARGE_CHARGE
    } else if tok.starts_with("medium") {
        vc::MEDIUM_CHARGE
    } else if tok.starts_with("small") {
        vc::SMALL_CHARGE
    } else {
        vc::STRONG_DRIVE
    }
}

/// The (s0, s1) drive strengths of signal `id`: a net's continuous-assign
/// drive strength — the same record `%v` displays — and strong otherwise.
/// §38.15: a variable's strength is always strong.
fn signal_strengths(sim: &Simulator, id: usize, type_code: c_int) -> Option<(c_int, c_int)> {
    if type_code != vpi::NET || is_packed_id(id) {
        return None;
    }
    let name = sim.name_for_id(id);
    let (s1, s0) = sim.module.net_strengths.get(name).or_else(|| {
        name.rsplit('.')
            .next()
            .and_then(|leaf| sim.module.net_strengths.get(leaf))
    })?;
    Some((strength_code(s0), strength_code(s1)))
}

pub(super) fn set_strength_hint(sim: &Simulator, id: usize, type_code: c_int, format: c_int) {
    if format == vc::STRENGTH_VAL {
        STRENGTH_HINT.with(|c| c.set(signal_strengths(sim, id, type_code)));
    }
}

/// `set_strength_hint` for a callback's object handle.
pub(super) fn set_strength_hint_for_obj(
    sim: &Simulator,
    obj: *mut libc::c_void,
    type_code: Option<c_int>,
    format: c_int,
) {
    if format != vc::STRENGTH_VAL {
        return;
    }
    let id = unsafe { super::vpi_deref(obj) }.map(|h| h.signal_id);
    STRENGTH_HINT.with(|c| {
        c.set(match (id, type_code) {
            (Some(id), Some(t)) => signal_strengths(sim, id, t),
            _ => None,
        })
    });
}

/// Copy the payload a `fill_extra_format` value points at into `keep` and
/// repoint the value there, so a callback's data survives the routine's own
/// `vpi_get_value` calls, which reuse the scratch buffers.
pub(super) fn own_extra_payload(width: u32, vp: &mut s_vpi_value, keep: &mut Vec<u64>) {
    let w = width.max(1) as usize;
    let bytes = match vp.format {
        vc::STRENGTH_VAL => w * std::mem::size_of::<s_vpi_strengthval>(),
        vc::LONG_INT_VAL => 8,
        vc::RAW_TWO_STATE_VAL => w.div_ceil(8),
        vc::RAW_FOUR_STATE_VAL => 2 * w.div_ceil(8),
        _ => return,
    };
    let src = unsafe { vp.value.misc } as *const u8;
    if src.is_null() {
        return;
    }
    keep.clear();
    keep.resize(bytes.div_ceil(8), 0);
    unsafe { std::ptr::copy_nonoverlapping(src, keep.as_mut_ptr() as *mut u8, bytes) };
    vp.value.misc = keep.as_mut_ptr() as *mut libc::c_char;
}

pub(super) fn clear_strength_hint() {
    STRENGTH_HINT.with(|c| c.set(None));
}

/// §38.15 vpiObjTypeVal: the format that suits the object.
pub(super) fn obj_type_format(val: &Value, obj_type_code: Option<c_int>) -> c_int {
    let w = val.width;
    match obj_type_code {
        _ if val.is_real => vpi::REAL_VAL,
        Some(vpi::REAL_VAR) | Some(vpi::SHORT_REAL_VAR) => vpi::REAL_VAL,
        Some(vpi::TIME_VAR) => vpi::TIME_VAL,
        Some(vpi::STRING_VAR) => vpi::STRING_VAL,
        // "For an integer, vpiIntVal": the integer types that fit one.
        Some(vpi::INTEGER_VAR)
        | Some(vpi::INT_VAR)
        | Some(vpi::SHORT_INT_VAR)
        | Some(vpi::BYTE_VAR)
        | Some(vpi::ENUM_VAR)
            if w <= 32 =>
        {
            vpi::INT_VAL
        }
        // An unsized integer parameter or constant is a 32-bit signed integer.
        Some(vpi::PARAMETER) | Some(vpi::CONSTANT) if w == 32 && val.is_signed => vpi::INT_VAL,
        _ if w <= 1 => vpi::SCALAR_VAL,
        _ => vpi::VECTOR_VAL,
    }
}

/// Formats that read a real object as a real rather than an integer.
pub(super) fn format_keeps_real(format: c_int) -> bool {
    matches!(
        format,
        vpi::REAL_VAL | vpi::STRING_VAL | vc::SHORT_REAL_VAL | vpi::SUPPRESS_VAL
    )
}

/// §6.12.2: a real converts to an integer by rounding to the nearest
/// integer, ties away from zero. The result is a signed 64-bit value.
pub(super) fn real_to_integer(val: &Value) -> Value {
    let f = val.to_f64();
    let n = if f.is_finite() { f.round() as i64 } else { 0 };
    let mut v = Value::from_u64(n as u64, 64);
    v.is_signed = true;
    v
}

/// §38.15 vpiStringVal of a real: decimal notation, at most 16 significant
/// digits.
pub(super) fn real_string(f: f64) -> String {
    if !f.is_finite() {
        return format!("{}", f);
    }
    // `{:.15e}` has 16 significant digits; reformat without the exponent
    // when it is modest, as `%.16g` would.
    let e = format!("{:.15e}", f);
    let (mant, exp) = e.split_once('e').unwrap_or((&e, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    if (-5..16).contains(&exp) {
        let decimals = (15 - exp).max(0) as usize;
        let s = format!("{:.*}", decimals, f);
        let s = if s.contains('.') {
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            s
        };
        if s == "-0" { "0".to_string() } else { s }
    } else {
        let m = if mant.contains('.') {
            mant.trim_end_matches('0').trim_end_matches('.')
        } else {
            mant
        };
        format!("{}e{}{:02}", m, if exp < 0 { '-' } else { '+' }, exp.abs())
    }
}

/// Octal / hex string with the §38.15 Table 38-3 digits for unknown bits:
/// `x` when every bit of the digit is x, `X` when only some are, `z` / `Z`
/// likewise for z (a digit with any x bit is an x digit).
pub(super) fn radix_string_xz(v: &Value, bits_per_digit: usize) -> String {
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
        if xs == n {
            s.push('x');
        } else if xs > 0 {
            s.push('X');
        } else if zs == n {
            s.push('z');
        } else if zs > 0 {
            s.push('Z');
        } else {
            s.push(char::from_digit(digit, 1u32 << bits_per_digit).unwrap_or('0'));
        }
    }
    if s.is_empty() {
        s.push('0');
    }
    s
}

/// The value as a signed or unsigned integer, x/z bits read as 0.
fn value_as_i64(v: &Value) -> i64 {
    if v.is_signed {
        v.to_i64().unwrap_or(0)
    } else {
        v.to_u64().unwrap_or(0) as i64
    }
}

/// `ngroups` bytes of aval then (for four-state) `ngroups` bytes of bval,
/// bit `i` of the value at bit `i % 8` of byte `i / 8` (§38.16).
fn value_to_raw(v: &Value, four_state: bool, out: &mut [u8]) {
    let w = v.width.max(1) as usize;
    let ngroups = w.div_ceil(8);
    for b in out.iter_mut() {
        *b = 0;
    }
    for i in 0..w {
        let (a, bv) = bit_code_to_ab(v.get_bit_code(i));
        if four_state {
            if a != 0 {
                out[i / 8] |= 1 << (i % 8);
            }
            if bv != 0 {
                out[ngroups + i / 8] |= 1 << (i % 8);
            }
        } else if a != 0 && bv == 0 {
            // Two-state: x and z read as 0.
            out[i / 8] |= 1 << (i % 8);
        }
    }
}

/// Inverse of `value_to_raw`. Two-state data (or four-state data put to a
/// two-state object) has no bval: every bit is 0 or 1.
fn raw_to_value(raw: &[u8], w: u32, four_state: bool, is_signed: bool) -> Value {
    let wu = w.max(1) as usize;
    let ngroups = wu.div_ceil(8);
    let mut v = Value::zero(w.max(1));
    for i in 0..wu {
        let a = ((raw[i / 8] >> (i % 8)) & 1) as u32;
        let b = if four_state {
            ((raw[ngroups + i / 8] >> (i % 8)) & 1) as u32
        } else {
            0
        };
        v.set_bit_code(i, ab_to_bit_code(a, b));
    }
    v.is_signed = is_signed;
    v
}

/// The formats `fill_vpi_value` does not handle itself. False when `format`
/// is not a format xezim can supply.
pub(super) fn fill_extra_format(
    val: &Value,
    obj_type_code: Option<c_int>,
    format: c_int,
    vp: &mut s_vpi_value,
) -> bool {
    match format {
        vc::STRENGTH_VAL => {
            // §38.15: one s_vpi_strengthval per bit, LSB first.
            let (s0, s1) = STRENGTH_HINT
                .with(|c| c.get())
                .unwrap_or((vc::STRONG_DRIVE, vc::STRONG_DRIVE));
            let _ = obj_type_code;
            let ptr = STRENGTH_SCRATCH.with(|cell| {
                let mut buf = cell.borrow_mut();
                buf.clear();
                for i in 0..val.width.max(1) as usize {
                    buf.push(match val.get_bit_code(i) {
                        0 => s_vpi_strengthval {
                            logic: vpi::SCALAR_0,
                            s0,
                            s1: s0,
                        },
                        1 => s_vpi_strengthval {
                            logic: vpi::SCALAR_1,
                            s0: s1,
                            s1,
                        },
                        2 => s_vpi_strengthval {
                            logic: vpi::SCALAR_X,
                            s0,
                            s1,
                        },
                        _ => s_vpi_strengthval {
                            logic: vpi::SCALAR_Z,
                            s0: vc::HI_Z,
                            s1: vc::HI_Z,
                        },
                    });
                }
                buf.as_mut_ptr()
            });
            vp.value.misc = ptr as *mut libc::c_char;
        }
        // The value in `integer`, sign-extended from 16 bits.
        vc::SHORT_INT_VAL => vp.value.integer = value_as_i64(val) as i16 as c_int,
        // `misc` points at a PLI_INT64 (simulator-owned, like vpiVectorVal).
        vc::LONG_INT_VAL => {
            let ptr = LONG_SCRATCH.with(|c| {
                *c.borrow_mut() = value_as_i64(val);
                c.as_ptr()
            });
            vp.value.misc = ptr as *mut libc::c_char;
        }
        // The value in `real`, rounded to single precision.
        vc::SHORT_REAL_VAL => vp.value.real = val.to_f64() as f32 as f64,
        // `misc` points at the §38.16 raw layout of one element.
        vc::RAW_TWO_STATE_VAL | vc::RAW_FOUR_STATE_VAL => {
            let four = format == vc::RAW_FOUR_STATE_VAL;
            let ngroups = (val.width.max(1) as usize).div_ceil(8);
            let ptr = RAW_SCRATCH.with(|c| {
                let mut buf = c.borrow_mut();
                buf.clear();
                buf.resize(ngroups * if four { 2 } else { 1 }, 0);
                value_to_raw(val, four, &mut buf);
                buf.as_mut_ptr()
            });
            vp.value.misc = ptr as *mut libc::c_char;
        }
        _ => return false,
    }
    true
}

/// Decode the `vpi_put_value` formats `vpi_put_value` does not handle
/// itself, at the object's width.
pub(super) fn decode_extra_put(
    vp: &s_vpi_value,
    format: c_int,
    w: u32,
    is_signed: bool,
) -> Result<Value, String> {
    let int_value = |n: i64| {
        let mut v = Value::from_u64(n as u64, w);
        v.is_signed = is_signed;
        v
    };
    let ptr_or = |p: *mut libc::c_char, what: &str| -> Result<*mut libc::c_char, String> {
        if p.is_null() {
            Err(format!("{} with a null pointer", what))
        } else {
            Ok(p)
        }
    };
    Ok(match format {
        vpi::TIME_VAL => {
            let t = unsafe { vp.value.time };
            if t.is_null() {
                return Err("vpiTimeVal with a null time pointer".into());
            }
            let t = unsafe { &*t };
            int_value((((t.high as u64) << 32) | t.low as u64) as i64)
        }
        vc::STRENGTH_VAL => {
            // §38.34: illegal for a vector object. The logic value is what
            // lands; xezim keeps no strength per value, so a later read
            // reports the net's driving strength again.
            if w != 1 {
                return Err("vpiStrengthVal can only be put to a scalar object".into());
            }
            let p = ptr_or(unsafe { vp.value.misc }, "vpiStrengthVal")?;
            let sv = unsafe { *(p as *const s_vpi_strengthval) };
            let code = match sv.logic {
                vpi::SCALAR_0 | vc::SCALAR_L => 0,
                vpi::SCALAR_1 | vc::SCALAR_H => 1,
                vpi::SCALAR_X => 2,
                vpi::SCALAR_Z => 3,
                other => return Err(format!("vpiStrengthVal with logic value {}", other)),
            };
            let valid = |s: c_int| s != 0 && s & !0xff == 0;
            if !valid(sv.s0) || !valid(sv.s1) {
                return Err(format!(
                    "vpiStrengthVal with strength codes s0={:#x} s1={:#x}",
                    sv.s0, sv.s1
                ));
            }
            let mut v = Value::zero(1);
            v.set_bit_code(0, code);
            v
        }
        vc::SHORT_INT_VAL => int_value(unsafe { vp.value.integer } as i16 as i64),
        vc::LONG_INT_VAL => {
            let p = ptr_or(unsafe { vp.value.misc }, "vpiLongIntVal")?;
            int_value(unsafe { std::ptr::read_unaligned(p as *const i64) })
        }
        vc::SHORT_REAL_VAL => Value::from_f64(unsafe { vp.value.real } as f32 as f64),
        vc::RAW_TWO_STATE_VAL | vc::RAW_FOUR_STATE_VAL => {
            let p = ptr_or(unsafe { vp.value.misc }, "a raw format")?;
            let four = format == vc::RAW_FOUR_STATE_VAL;
            let ngroups = (w.max(1) as usize).div_ceil(8);
            let raw = unsafe {
                std::slice::from_raw_parts(p as *const u8, ngroups * if four { 2 } else { 1 })
            };
            raw_to_value(raw, w, four, is_signed)
        }
        other => return Err(format!("unsupported format {}", other)),
    })
}

/// §6.12.2 for `vpi_put_value`: an integral value put to a real object
/// converts to real; a real put to an integral object rounds to an integer
/// (without this the real's IEEE bits landed in the integral signal).
pub(super) fn fit_put_value(
    sim: &Simulator,
    h: &VpiHandle,
    value: Value,
    w: u32,
    is_signed: bool,
) -> Value {
    let dest_real = h.kind != VpiKind::Slice && cell_real(sim, h.signal_id);
    if dest_real && !value.is_real {
        return Value::from_f64(if value.is_signed {
            value.to_i64().unwrap_or(0) as f64
        } else {
            value.to_u64().unwrap_or(0) as f64
        });
    }
    if !dest_real && value.is_real {
        let mut v = real_to_integer(&value).resize(w);
        v.is_signed = is_signed;
        return v;
    }
    value
}

// ===========================================================================
// Unpacked arrays: vpi_handle_by_multi_index and the array value routines
// ===========================================================================

/// An unpacked array, or a sub-array of one, as a VPI handle addresses it.
struct ArrayView {
    /// Design-relative declaration name.
    base: String,
    /// Indices already applied, leftmost dimension first.
    fixed: Vec<i64>,
    /// DECLARED (left, right) bounds of every unpacked dimension.
    dims: Vec<(i64, i64)>,
}

fn in_range((l, r): (i64, i64), i: i64) -> bool {
    i >= l.min(r) && i <= l.max(r)
}

fn dim_len((l, r): (i64, i64)) -> usize {
    ((l - r).unsigned_abs() + 1) as usize
}

impl ArrayView {
    fn remaining(&self) -> &[(i64, i64)] {
        &self.dims[self.fixed.len()..]
    }

    fn count(&self) -> usize {
        self.remaining().iter().map(|&d| dim_len(d)).product()
    }

    /// Design-relative name of the (sub)array, or of the element when every
    /// dimension is fixed.
    fn name_with(&self, extra: &[i64]) -> String {
        let mut s = self.base.clone();
        for i in self.fixed.iter().chain(extra) {
            s.push('[');
            s.push_str(&i.to_string());
            s.push(']');
        }
        s
    }
}

/// A static unpacked array's declared dimensions (leftmost first).
fn array_dims(sim: &Simulator, base: &str) -> Option<Vec<(i64, i64)>> {
    let m = &sim.module;
    if m.dynamic_arrays.contains(base)
        || m.associative_arrays.contains_key(base)
        || m.queue_vars.contains(base)
    {
        return None;
    }
    let declared = m.unpacked_decl_dims.get(base);
    let pick = |norm: Vec<(i64, i64)>| -> Vec<(i64, i64)> {
        match declared {
            Some(d)
                if d.len() == norm.len()
                    && d.iter().zip(&norm).all(|(&a, &b)| dim_len(a) == dim_len(b)) =>
            {
                d.clone()
            }
            _ => norm,
        }
    };
    if let Some(&(lo, hi, _)) = m.arrays.get(base) {
        let norm = if m.descending_arrays.contains(base) {
            (hi, lo)
        } else {
            (lo, hi)
        };
        return Some(pick(vec![norm]));
    }
    if let Some(&(d1, d2, _)) = m.arrays_2d.get(base) {
        return Some(pick(vec![d1, d2]));
    }
    if let Some((shape, _)) = m.arrays_nd.get(base) {
        return Some(pick(shape.clone()));
    }
    None
}

/// Parse a design-relative name `base[i][j]..` into the array it indexes.
fn array_view(sim: &Simulator, rel: &str) -> Option<ArrayView> {
    let mut base = rel;
    let mut fixed: Vec<i64> = Vec::new();
    loop {
        if let Some(dims) = array_dims(sim, base) {
            fixed.reverse();
            if fixed.len() > dims.len() || fixed.iter().zip(&dims).any(|(&i, &d)| !in_range(d, i)) {
                return None;
            }
            return Some(ArrayView {
                base: base.to_string(),
                fixed,
                dims,
            });
        }
        let inner = base.strip_suffix(']')?;
        let open = inner.rfind('[')?;
        fixed.push(inner[open + 1..].trim().parse().ok()?);
        base = &base[..open];
    }
}

/// The cell id of the element at full index `idx` (one per dimension).
fn element_id(sim: &Simulator, view: &ArrayView, idx: &[i64]) -> Option<usize> {
    if view.dims.len() == 1 {
        if let Some(&(first, lo, hi)) = sim.array_first_id.get(view.base.as_str()) {
            let i = idx[0];
            return (i >= lo && i <= hi).then(|| first + (i - lo) as usize);
        }
    }
    let mut name = view.base.clone();
    for i in idx {
        name.push('[');
        name.push_str(&i.to_string());
        name.push(']');
    }
    sim.signal_name_to_id.get(name.as_str()).copied()
}

/// A handle to a (sub)array: vpiMemory for a whole one-dimensional array
/// (as `vpi_handle_by_name` has always answered), vpiRegArray / vpiNetArray
/// for a multi-dimensional array or a sub-array of one.
fn array_handle(sim: &Simulator, view: &ArrayView) -> Option<VpiHandle> {
    let mut first: Vec<i64> = view.fixed.clone();
    first.extend(view.remaining().iter().map(|&(l, _)| l));
    let id = element_id(sim, view, &first)?;
    let is_net = sim.module.nets.contains(&view.base);
    let type_code = if view.dims.len() == 1 {
        vpi::MEMORY
    } else if is_net {
        vc::NET_ARRAY
    } else {
        vc::REG_ARRAY
    };
    let rel = view.name_with(&[]);
    let leaf = rel.rsplit('.').next().unwrap_or(&rel).to_string();
    let mut h = VpiHandle::signal(id, type_code, &leaf, &vpi_full_name(sim, &rel));
    h.kind = VpiKind::Memory;
    h.width = view.count() as u32;
    Some(h)
}

/// A handle to one element.
fn element_handle(sim: &Simulator, view: &ArrayView, idx: &[i64]) -> Option<VpiHandle> {
    let id = element_id(sim, view, idx)?;
    let ty = vpi_type_of(sim, &view.base, id);
    let mut full_idx: Vec<i64> = Vec::new();
    full_idx.extend_from_slice(idx);
    let rel = ArrayView {
        base: view.base.clone(),
        fixed: Vec::new(),
        dims: view.dims.clone(),
    }
    .name_with(&full_idx);
    let leaf = rel.rsplit('.').next().unwrap_or(&rel).to_string();
    Some(VpiHandle::signal(id, ty, &leaf, &vpi_full_name(sim, &rel)))
}

/// The declared (left, right) packed dimensions, outermost first, of an
/// object named `decl` of width `w` (a plain `[w-1:0]` when nothing more is
/// recorded).
fn packed_dims(sim: &Simulator, decl: &str, w: u32) -> Vec<(i64, i64)> {
    use xezim_core::ast::types::{DataType, PackedDimension};
    if let Some(d) = sim.module.packed_full_dims.get(decl) {
        if d.iter().map(|&x| dim_len(x)).product::<usize>() == w as usize {
            return d.clone();
        }
    }
    if let Some(&(lo, hi)) = sim.module.ascending_packed.get(decl) {
        return vec![(lo, hi)];
    }
    let dims = match sim.module.var_decl_types.get(decl) {
        Some(DataType::IntegerVector { dimensions, .. }) => Some(dimensions),
        Some(DataType::Implicit { dimensions, .. }) => Some(dimensions),
        _ => None,
    };
    if let Some([PackedDimension::Range { left, right, .. }]) = dims.map(|d| d.as_slice()) {
        let eval =
            |e| xezim_core::elaborate::const_eval_i64_with_params(e, Some(&sim.module.parameters));
        if let (Some(l), Some(r)) = (eval(left), eval(right)) {
            if dim_len((l, r)) == w as usize {
                return vec![(l, r)];
            }
        }
    }
    vec![(w as i64 - 1, 0)]
}

/// A select through the packed dimensions of cell `id` (declared as
/// `decl`): one index per dimension from the outermost, a bit when every
/// dimension is indexed (vpiRegBit / vpiNetBit), a part-select otherwise.
fn packed_select(
    sim: &Simulator,
    id: usize,
    decl: &str,
    rel: &str,
    labels: &[i64],
    is_net: bool,
) -> Option<VpiHandle> {
    let w = cell_width(sim, id)?;
    let dims = packed_dims(sim, decl, w);
    if labels.is_empty() || labels.len() > dims.len() {
        return None;
    }
    // Width of one element of dimension k: the product of the dimensions
    // inside it.
    let inner = |k: usize| -> usize { dims[k + 1..].iter().map(|&d| dim_len(d)).product() };
    let mut lsb = 0usize;
    for (k, &label) in labels.iter().enumerate() {
        let (l, r) = dims[k];
        if !in_range((l, r), label) {
            return None;
        }
        let pos = (if l >= r { label - r } else { r - label }) as usize;
        lsb += pos * inner(k);
    }
    let width = inner(labels.len() - 1);
    let mut name = rel.to_string();
    for label in labels {
        name.push_str(&format!("[{}]", label));
    }
    let leaf = name.rsplit('.').next().unwrap_or(&name).to_string();
    let ty = match (width, is_net) {
        (1, true) => vc::NET_BIT,
        (1, false) => vc::REG_BIT,
        _ => vpi::PART_SELECT,
    };
    let mut h = VpiHandle::signal(id, ty, &leaf, &vpi_full_name(sim, &name));
    h.kind = VpiKind::Slice;
    h.lsb = lsb as u32;
    h.width = width as u32;
    Some(h)
}

/// Apply `idx` to an array view: a sub-array, an element, or — with
/// indices past the last unpacked dimension — a select through the
/// element's packed dimensions, down to a bit (§38.20).
fn index_view(sim: &Simulator, view: ArrayView, idx: &[i64]) -> Option<VpiHandle> {
    let rem = view.remaining().len();
    let n_arr = idx.len().min(rem);
    for (k, &i) in idx[..n_arr].iter().enumerate() {
        if !in_range(view.remaining()[k], i) {
            return None;
        }
    }
    let mut fixed = view.fixed.clone();
    fixed.extend_from_slice(&idx[..n_arr]);
    let sub = ArrayView {
        base: view.base.clone(),
        fixed,
        dims: view.dims.clone(),
    };
    match idx.len().cmp(&rem) {
        std::cmp::Ordering::Less => array_handle(sim, &sub),
        std::cmp::Ordering::Equal => element_handle(sim, &sub, &sub.fixed),
        std::cmp::Ordering::Greater => {
            let id = element_id(sim, &sub, &sub.fixed)?;
            let is_net = sim.module.nets.contains(&sub.base);
            packed_select(sim, id, &sub.base, &sub.name_with(&[]), &idx[rem..], is_net)
        }
    }
}

/// `vpi_handle_by_name` fallback: a multi-dimensional array, a sub-array,
/// or an element of an array too large to have named elements.
pub(super) fn array_object_of(sim: &Simulator, rel: &str) -> Option<VpiHandle> {
    let view = array_view(sim, rel)?;
    if view.fixed.len() == view.dims.len() {
        let idx = view.fixed.clone();
        let whole = ArrayView {
            base: view.base,
            fixed: Vec::new(),
            dims: view.dims,
        };
        element_handle(sim, &whole, &idx)
    } else {
        array_handle(sim, &view)
    }
}

/// An array handle, whichever part of the VPI made it: its type says so,
/// and its full name locates it.
fn is_array_handle(h: &VpiHandle) -> bool {
    h.kind == VpiKind::Memory || matches!(h.type_code, vpi::MEMORY | vc::REG_ARRAY | vc::NET_ARRAY)
}

fn view_of_handle(sim: &Simulator, h: &VpiHandle) -> Option<ArrayView> {
    if !is_array_handle(h) {
        return None;
    }
    array_view(sim, vpi_strip_top(sim, &h.full_name))
}

/// `vpi_handle_by_index` on an array handle: `None` when the handle is not
/// an array `vpi_api` understands (the caller then answers itself).
pub(super) fn index_array_handle(h: &VpiHandle, index: c_int) -> Option<*mut c_void> {
    try_active_sim("vpi_handle_by_index", |sim| {
        let view = view_of_handle(sim, h)?;
        Some(index_view(sim, view, &[index as i64]).map_or(std::ptr::null_mut(), |r| r.into_raw()))
    })
    .flatten()
}

/// §38.20: the subobject selected by `num_index` indices, leftmost first —
/// one per unpacked dimension still open on `obj`, then optionally indices
/// into the element's packed dimensions (the last of which selects a bit).
/// A plain vector takes only packed indices. NULL when the indices do not
/// form a legal select.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_handle_by_multi_index(
    obj: *mut c_void,
    num_index: c_int,
    index_array: *mut c_int,
) -> *mut c_void {
    let Some(h) = (unsafe { vpi_deref(obj) }) else {
        vpi_error(vpi::ERROR, "vpi_handle_by_multi_index: null handle".into());
        return std::ptr::null_mut();
    };
    if num_index <= 0 || index_array.is_null() {
        vpi_error(
            vpi::ERROR,
            "vpi_handle_by_multi_index: needs at least one index".into(),
        );
        return std::ptr::null_mut();
    }
    let idx: Vec<i64> = unsafe { std::slice::from_raw_parts(index_array, num_index as usize) }
        .iter()
        .map(|&i| i as i64)
        .collect();
    try_active_sim("vpi_handle_by_multi_index", |sim| match h.kind {
        _ if is_array_handle(h) => view_of_handle(sim, h).and_then(|v| index_view(sim, v, &idx)),
        VpiKind::Signal | VpiKind::Port => {
            let rel = vpi_strip_top(sim, &h.full_name).to_string();
            // An array element keeps its declaration's packed dimensions.
            let decl = array_view(sim, &rel).map_or(rel.clone(), |v| v.base);
            let is_net = h.type_code == vpi::NET || sim.module.nets.contains(&decl);
            packed_select(sim, h.signal_id, &decl, &rel, &idx, is_net)
        }
        _ => None,
    })
    .flatten()
    .map_or(std::ptr::null_mut(), |r| r.into_raw())
}

/// Element data type for the array value formats (§38.16 / §38.35).
#[derive(Clone, Copy, PartialEq)]
enum ElemKind {
    Real,
    ShortReal,
    ShortInt,
    Byte,
    Int,
    LongInt,
    /// Any other integral element: logic/bit vectors, integer, time, enums,
    /// packed structs and unions.
    Integral,
}

fn elem_kind(sim: &Simulator, base: &str, id: usize) -> Option<ElemKind> {
    Some(match vpi_type_of(sim, base, id) {
        vpi::SHORT_REAL_VAR => ElemKind::ShortReal,
        vpi::REAL_VAR => ElemKind::Real,
        vpi::SHORT_INT_VAR => ElemKind::ShortInt,
        vpi::BYTE_VAR => ElemKind::Byte,
        vpi::INT_VAR => ElemKind::Int,
        vpi::LONG_INT_VAR => ElemKind::LongInt,
        vpi::STRING_VAR => return None,
        _ if cell_real(sim, id) => ElemKind::Real,
        _ => ElemKind::Integral,
    })
}

/// Is `format` allowed for an array of `kind` elements? The integral
/// formats (vpiIntVal, vpiTimeVal, vpiVectorVal, both raw formats) suit
/// every integral array; the rest only the types §38.16 (get) and §38.35
/// (put) name for them.
fn array_format_ok(format: c_int, kind: ElemKind, put: bool) -> bool {
    use ElemKind::*;
    let real = matches!(kind, Real | ShortReal);
    match format {
        vpi::INT_VAL
        | vpi::TIME_VAL
        | vpi::VECTOR_VAL
        | vc::RAW_TWO_STATE_VAL
        | vc::RAW_FOUR_STATE_VAL => !real,
        vpi::REAL_VAL => real,
        vc::SHORT_REAL_VAL => kind == ShortReal,
        vc::SHORT_INT_VAL if put => matches!(kind, ShortInt | Int | LongInt),
        vc::SHORT_INT_VAL => matches!(kind, ShortInt | Byte),
        vc::LONG_INT_VAL if put => kind == LongInt,
        vc::LONG_INT_VAL => matches!(kind, LongInt | ShortInt | Byte),
        _ => false,
    }
}

/// Bytes one element occupies in the s_vpi_arrayvalue buffer.
fn array_elem_bytes(format: c_int, w: u32) -> usize {
    let w = w.max(1) as usize;
    match format {
        vpi::INT_VAL => 4,
        vc::SHORT_INT_VAL => 2,
        vc::LONG_INT_VAL | vpi::REAL_VAL => 8,
        vc::SHORT_REAL_VAL => 4,
        vpi::TIME_VAL => std::mem::size_of::<s_vpi_time>(),
        vpi::VECTOR_VAL => w.div_ceil(32) * std::mem::size_of::<s_vpi_vecval>(),
        vc::RAW_FOUR_STATE_VAL => 2 * w.div_ceil(8),
        _ => w.div_ceil(8), // vpiRawTwoStateVal
    }
}

/// The cell ids of `num` consecutive elements starting at `start` (one
/// index per open dimension): the rightmost dimension varies fastest, and
/// each dimension runs from its declared left bound to its right (§38.16).
fn section_ids(
    sim: &Simulator,
    view: &ArrayView,
    start: &[i64],
    num: usize,
) -> Result<Vec<usize>, String> {
    let dims = view.remaining().to_vec();
    for (k, (&i, &d)) in start.iter().zip(&dims).enumerate() {
        if !in_range(d, i) {
            return Err(format!(
                "start index {} is outside dimension {} [{}:{}]",
                i, k, d.0, d.1
            ));
        }
    }
    let mut cur = start.to_vec();
    let mut ids = Vec::with_capacity(num);
    let mut full: Vec<i64> = Vec::with_capacity(view.dims.len());
    for n in 0..num {
        full.clear();
        full.extend_from_slice(&view.fixed);
        full.extend_from_slice(&cur);
        let id = element_id(sim, view, &full)
            .ok_or_else(|| format!("element {} has no storage", view.name_with(&cur)))?;
        ids.push(id);
        if n + 1 == num {
            break;
        }
        // Advance: the rightmost dimension first, carrying leftward.
        let mut k = dims.len();
        loop {
            if k == 0 {
                return Err(format!(
                    "{} elements from the start index run past the end of the array",
                    num
                ));
            }
            k -= 1;
            let (l, r) = dims[k];
            if cur[k] == r {
                cur[k] = l;
                continue;
            }
            cur[k] += if l <= r { 1 } else { -1 };
            break;
        }
    }
    Ok(ids)
}

/// Resolve the section an array value call addresses: the view, the cell
/// ids, and the element kind and width.
fn array_section(
    sim: &Simulator,
    h: &VpiHandle,
    index_p: *mut c_int,
    num: u32,
) -> Result<(ArrayView, Vec<usize>, ElemKind, u32), String> {
    if !is_array_handle(h) {
        return Err("the object is not an unpacked array".into());
    }
    let view = view_of_handle(sim, h)
        .ok_or_else(|| "the object is not a static unpacked array".to_string())?;
    if num == 0 {
        return Err("num must be at least 1".into());
    }
    if index_p.is_null() {
        return Err("null index_p".into());
    }
    let rank = view.remaining().len();
    let start: Vec<i64> = unsafe { std::slice::from_raw_parts(index_p, rank) }
        .iter()
        .map(|&i| i as i64)
        .collect();
    let ids = section_ids(sim, &view, &start, num as usize)?;
    let kind = elem_kind(sim, &view.base, ids[0])
        .ok_or_else(|| "string arrays have dynamic elements".to_string())?;
    let w = cell_width(sim, ids[0]).unwrap_or(1);
    Ok((view, ids, kind, w))
}

/// §38.16: read `num` consecutive elements of a static unpacked array.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_get_value_array(
    obj: *mut c_void,
    arrayvalue_p: *mut s_vpi_arrayvalue,
    index_p: *mut c_int,
    num: u32,
) {
    if arrayvalue_p.is_null() {
        vpi_error(
            vpi::ERROR,
            "vpi_get_value_array: null s_vpi_arrayvalue pointer".into(),
        );
        return;
    }
    let av = unsafe { &mut *arrayvalue_p };
    let result = (|| -> Result<(), String> {
        let h = unsafe { vpi_deref(obj) }.ok_or_else(|| "null handle".to_string())?;
        let format = av.format as c_int;
        let user_alloc = av.flags & vc::USER_ALLOC_FLAG != 0;
        if user_alloc && av.value.is_null() {
            return Err("vpiUserAllocFlag with a null value buffer".into());
        }
        let fill = try_active_sim("vpi_get_value_array", |sim| -> Result<Vec<u8>, String> {
            let (_, ids, kind, w) = array_section(sim, h, index_p, num)?;
            if !array_format_ok(format, kind, false) {
                return Err(format!(
                    "format {} is not available for this array's elements",
                    format
                ));
            }
            let per = array_elem_bytes(format, w);
            let mut out = vec![0u8; per * ids.len()];
            for (k, &id) in ids.iter().enumerate() {
                let v = cell_value(sim, id).unwrap_or_else(|| Value::new(w));
                let dst = &mut out[k * per..(k + 1) * per];
                encode_array_elem(&v, format, dst);
            }
            Ok(out)
        })
        .ok_or_else(|| "no active simulator".to_string())??;
        let dst = if user_alloc {
            av.value as *mut u8
        } else {
            ARRAY_SCRATCH.with(|c| {
                let mut buf = c.borrow_mut();
                buf.clear();
                buf.resize(fill.len().div_ceil(8).max(1), 0);
                buf.as_mut_ptr() as *mut u8
            })
        };
        unsafe { std::ptr::copy_nonoverlapping(fill.as_ptr(), dst, fill.len()) };
        av.value = dst as *mut c_void;
        Ok(())
    })();
    if let Err(msg) = result {
        vpi_error(vpi::ERROR, format!("vpi_get_value_array: {}", msg));
        // §38.16: a NULL value pointer is how the caller learns of the error.
        av.value = std::ptr::null_mut();
    }
}

/// One element into its slot of the s_vpi_arrayvalue buffer.
fn encode_array_elem(v: &Value, format: c_int, dst: &mut [u8]) {
    let rounded;
    let v = if v.is_real && !format_keeps_real(format) {
        rounded = real_to_integer(v);
        &rounded
    } else {
        v
    };
    match format {
        vpi::INT_VAL => dst.copy_from_slice(&(value_as_i64(v) as i32).to_ne_bytes()),
        vc::SHORT_INT_VAL => dst.copy_from_slice(&(value_as_i64(v) as i16).to_ne_bytes()),
        vc::LONG_INT_VAL => dst.copy_from_slice(&value_as_i64(v).to_ne_bytes()),
        vpi::REAL_VAL => dst.copy_from_slice(&v.to_f64().to_ne_bytes()),
        vc::SHORT_REAL_VAL => dst.copy_from_slice(&(v.to_f64() as f32).to_ne_bytes()),
        vpi::TIME_VAL => {
            let t = v.to_u64().unwrap_or(0);
            let tv = s_vpi_time {
                type_: vpi::SIM_TIME,
                high: (t >> 32) as u32,
                low: t as u32,
                real: t as f64,
            };
            unsafe { std::ptr::write_unaligned(dst.as_mut_ptr() as *mut s_vpi_time, tv) };
        }
        vpi::VECTOR_VAL => {
            let mut words: Vec<s_vpi_vecval> = Vec::new();
            value_to_vecval(v, &mut words);
            for (k, wv) in words.iter().enumerate() {
                let o = k * 8;
                if o + 8 > dst.len() {
                    break;
                }
                dst[o..o + 4].copy_from_slice(&wv.aval.to_ne_bytes());
                dst[o + 4..o + 8].copy_from_slice(&wv.bval.to_ne_bytes());
            }
        }
        vc::RAW_FOUR_STATE_VAL => value_to_raw(v, true, dst),
        _ => value_to_raw(v, false, dst),
    }
}

/// One element out of its slot of the s_vpi_arrayvalue buffer, at the
/// element's width.
fn decode_array_elem(src: &[u8], format: c_int, w: u32, is_signed: bool, two_state: bool) -> Value {
    let int_value = |n: i64| {
        let mut v = Value::from_u64(n as u64, w);
        v.is_signed = is_signed;
        v
    };
    let rd4 = |o: usize| i32::from_ne_bytes([src[o], src[o + 1], src[o + 2], src[o + 3]]);
    match format {
        vpi::INT_VAL => int_value(rd4(0) as i64),
        vc::SHORT_INT_VAL => int_value(i16::from_ne_bytes([src[0], src[1]]) as i64),
        vc::LONG_INT_VAL => int_value(i64::from_ne_bytes(src[..8].try_into().unwrap_or([0; 8]))),
        vpi::REAL_VAL => Value::from_f64(f64::from_ne_bytes(src[..8].try_into().unwrap_or([0; 8]))),
        vc::SHORT_REAL_VAL => {
            Value::from_f64(f32::from_ne_bytes([src[0], src[1], src[2], src[3]]) as f64)
        }
        vpi::TIME_VAL => {
            let t = unsafe { std::ptr::read_unaligned(src.as_ptr() as *const s_vpi_time) };
            int_value((((t.high as u64) << 32) | t.low as u64) as i64)
        }
        vpi::VECTOR_VAL => {
            let words: Vec<s_vpi_vecval> = (0..src.len() / 8)
                .map(|k| s_vpi_vecval {
                    aval: rd4(k * 8),
                    bval: if two_state { 0 } else { rd4(k * 8 + 4) },
                })
                .collect();
            vecval_to_value(&words, w, is_signed)
        }
        // §38.35: four-state data put to a two-state array drops bval;
        // two-state data put to a four-state array has bval 0.
        vc::RAW_FOUR_STATE_VAL if !two_state => raw_to_value(src, w, true, is_signed),
        _ => raw_to_value(src, w, false, is_signed),
    }
}

/// §38.35: write `num` consecutive elements of a static unpacked array.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_put_value_array(
    obj: *mut c_void,
    arrayvalue_p: *mut s_vpi_arrayvalue,
    index_p: *mut c_int,
    num: u32,
) {
    let result = (|| -> Result<(), String> {
        if arrayvalue_p.is_null() {
            return Err("null s_vpi_arrayvalue pointer".into());
        }
        let av = unsafe { &*arrayvalue_p };
        let h = unsafe { vpi_deref(obj) }.ok_or_else(|| "null handle".to_string())?;
        let allowed = vc::PROPAGATE_OFF | vc::ONE_VALUE | vc::NO_DELAY;
        if av.flags & !allowed != 0 {
            return Err(format!(
                "flags {:#x}: only vpiPropagateOff, vpiOneValue and vpiNoDelay are allowed",
                av.flags
            ));
        }
        if av.value.is_null() {
            return Err("null value buffer".into());
        }
        let format = av.format as c_int;
        let one_value = av.flags & vc::ONE_VALUE != 0;
        let quiet = av.flags & vc::PROPAGATE_OFF != 0;
        try_active_sim("vpi_put_value_array", |sim| -> Result<(), String> {
            let (_, ids, kind, w) = array_section(sim, h, index_p, num)?;
            if !array_format_ok(format, kind, true) {
                return Err(format!(
                    "format {} is not available for this array's elements",
                    format
                ));
            }
            let per = array_elem_bytes(format, w);
            let n_src = if one_value { 1 } else { ids.len() };
            let src = unsafe { std::slice::from_raw_parts(av.value as *const u8, per * n_src) };
            let real = matches!(kind, ElemKind::Real | ElemKind::ShortReal);
            for (k, &id) in ids.iter().enumerate() {
                let slot = if one_value { 0 } else { k };
                let mut v = decode_array_elem(
                    &src[slot * per..(slot + 1) * per],
                    format,
                    w,
                    cell_signed(sim, id),
                    cell_two_state(sim, id),
                );
                if kind == ElemKind::ShortReal {
                    v = Value::from_f64(v.to_f64() as f32 as f64);
                }
                if !real {
                    v = v.resize(w);
                }
                if quiet {
                    deposit_quiet(sim, id, v);
                } else {
                    deposit(sim, id, v);
                }
            }
            Ok(())
        })
        .ok_or_else(|| "no active simulator".to_string())?
    })();
    if let Err(msg) = result {
        vpi_error(
            vpi::ERROR,
            format!("vpi_put_value_array: {} (nothing written)", msg),
        );
    }
}

// ===========================================================================
// vpi_handle_multi: intermodule paths
// ===========================================================================

/// A port reference for `vpiInterModPath`: its signal, direction and width.
/// A port object without a signal id of its own is found by its full name.
fn port_ref(sim: &Simulator, h: &VpiHandle) -> Option<(usize, c_int, u32, String)> {
    if h.direction == vpi::NO_DIRECTION {
        return None;
    }
    let id = match h.kind {
        VpiKind::Port | VpiKind::Signal => h.signal_id,
        VpiKind::Iterator | VpiKind::Module | VpiKind::Constant | VpiKind::SysTfCall => {
            return None;
        }
        _ => *sim
            .signal_name_to_id
            .get(vpi_strip_top(sim, &h.full_name))?,
    };
    Some((id, h.direction, cell_width(sim, id)?, h.full_name.clone()))
}

/// Backend of the C-variadic `vpi_handle_multi` (§38.22). The standard
/// defines one relation through it: `vpiInterModPath`, the interconnect
/// between an output port and an input port of the same size.
#[unsafe(no_mangle)]
pub extern "C" fn xezim_vpi_handle_multi(
    type_: c_int,
    ref1: *mut c_void,
    ref2: *mut c_void,
) -> *mut c_void {
    if type_ != vc::INTER_MOD_PATH {
        vpi_error(
            vpi::ERROR,
            format!(
                "vpi_handle_multi: type {} has no many-to-one relation (only vpiInterModPath does)",
                type_
            ),
        );
        return std::ptr::null_mut();
    }
    let (Some(a), Some(b)) = (unsafe { vpi_deref(ref1) }, unsafe { vpi_deref(ref2) }) else {
        vpi_error(
            vpi::ERROR,
            "vpi_handle_multi(vpiInterModPath): needs two port handles".into(),
        );
        return std::ptr::null_mut();
    };
    let made = try_active_sim("vpi_handle_multi", |sim| -> Result<VpiHandle, String> {
        let pa = port_ref(sim, a).ok_or("the first reference is not a port")?;
        let pb = port_ref(sim, b).ok_or("the second reference is not a port")?;
        let drives = |d: c_int| d == vpi::OUTPUT || d == vpi::INOUT;
        let loads = |d: c_int| d == vpi::INPUT || d == vpi::INOUT;
        // Output port first, as §38.22 lists them; accept the pair either way.
        let (out, inp) = if drives(pa.1) && loads(pb.1) {
            (pa, pb)
        } else if drives(pb.1) && loads(pa.1) {
            (pb, pa)
        } else {
            return Err("needs an output port and an input port".into());
        };
        if out.2 != inp.2 {
            return Err(format!(
                "the ports differ in size ({} and {} bits)",
                out.2, inp.2
            ));
        }
        if out.0 == inp.0 {
            return Err(format!(
                "{} and {} are one net in xezim (the connection was collapsed), so there is \
                 no interconnect between them",
                out.3, inp.3
            ));
        }
        let mut h = VpiHandle::signal(inp.0, vc::INTER_MOD_PATH, "", "");
        h.kind = VpiKind::InterModPath;
        h.inst_idx = out.0 as isize;
        h.width = inp.2;
        Ok(h)
    });
    match made {
        Some(Ok(h)) => h.into_raw(),
        Some(Err(msg)) => {
            vpi_error(
                vpi::ERROR,
                format!("vpi_handle_multi(vpiInterModPath): {}", msg),
            );
            std::ptr::null_mut()
        }
        None => std::ptr::null_mut(),
    }
}

// ===========================================================================
// Delays: vpi_get_delays / vpi_put_delays
// ===========================================================================

/// What a delay handle addresses.
enum DelayObj {
    /// A net (or a port's net): the delay xezim applies to updates of it —
    /// its driving gate's or continuous assignment's, or an SDF / VPI
    /// annotation of the net.
    Net(usize),
    /// Path `i` into net `id`.
    ModPath(usize, usize),
    /// Timing check `i`.
    Tchk(usize),
    /// The interconnect into input-port net `id` (where SDF INTERCONNECT
    /// delays land).
    InterMod(usize),
}

fn delay_obj(h: &VpiHandle) -> Result<DelayObj, String> {
    Ok(match h.kind {
        VpiKind::Signal | VpiKind::Port if !is_packed_id(h.signal_id) => DelayObj::Net(h.signal_id),
        VpiKind::ModPath => DelayObj::ModPath(h.signal_id, h.lsb as usize),
        VpiKind::Tchk => DelayObj::Tchk(h.signal_id),
        VpiKind::InterModPath => DelayObj::InterMod(h.signal_id),
        _ => return Err("this object has no delays".into()),
    })
}

/// The module definition an object is declared in, for vpiScaledRealTime.
fn delay_obj_def(sim: &Simulator, obj: &DelayObj) -> String {
    let scope_def = |scope: &str| -> String {
        sim.module
            .instances
            .iter()
            .find(|i| i.path == scope)
            .map_or_else(|| sim.module.name.clone(), |i| i.def_name.clone())
    };
    match obj {
        DelayObj::Net(id) | DelayObj::ModPath(id, _) | DelayObj::InterMod(id) => {
            scope_def(scope_of(sim.name_for_id(*id)))
        }
        DelayObj::Tchk(i) => sim
            .vpi_tchk_ident(*i)
            .map_or_else(|| sim.module.name.clone(), |(_, _, d)| d.to_string()),
    }
}

/// The interpreted continuous assignments with a delay that drive `id` —
/// every delayed gate and `assign #d` lowers to one.
fn delayed_drivers(sim: &Simulator, id: usize) -> Vec<usize> {
    sim.comb_entries
        .iter()
        .enumerate()
        .filter(|(_, e)| {
            matches!(e.item, CombItem::ContAssign { delay, .. } if delay > 0)
                && e.cold.write_signal_ids.contains(&id)
        })
        .map(|(i, _)| i)
        .collect()
}

fn is_path_net(sim: &Simulator, id: usize) -> bool {
    sim.module_paths
        .as_ref()
        .is_some_and(|mp| mp.is_path_net(id))
}

/// (rise, fall, turn-off) of net `id` as `schedule_delayed_with_delay`
/// applies them. `sdf_only` ignores a delayed continuous assignment (an
/// interconnect's delay is the net's annotation alone).
fn net_delays(sim: &Simulator, id: usize, sdf_only: bool) -> [u64; 3] {
    if is_path_net(sim, id) {
        // The net's own delay is none: its module paths carry the delays.
        return [0; 3];
    }
    let explicit = if sdf_only {
        None
    } else {
        delayed_drivers(sim, id)
            .first()
            .and_then(|&e| match sim.comb_entries[e].item {
                CombItem::ContAssign { delay, .. } => Some(delay),
                _ => None,
            })
    };
    let rise = explicit.unwrap_or_else(|| sim.sdf_delays.get(id).copied().unwrap_or(0));
    let fall = sim.gate_fall_delay_by_id.get(&id).copied();
    let off = sim.gate_off_delay_by_id.get(&id).copied();
    let fall_v = fall.unwrap_or(rise);
    let off_v = off.unwrap_or_else(|| fall.map_or(rise, |f| f.min(rise)));
    [rise, fall_v, off_v]
}

/// Store (rise, fall, turn-off) for net `id`: into its delayed continuous
/// assignments when it has them, otherwise as a net annotation — which only
/// the driver forms that consult one (a plain copy, a one-bit gate, an
/// interpreted assignment) honour, so any other driver is refused rather
/// than silently left undelayed.
fn set_net_delays(
    sim: &mut Simulator,
    id: usize,
    rise: u64,
    fall: u64,
    off: Option<u64>,
    sdf_only: bool,
) -> Result<(), String> {
    if is_path_net(sim, id) {
        return Err(
            "the net is driven through module paths; put the delays on its vpiModPath objects"
                .into(),
        );
    }
    if rise == 0 && (fall > 0 || off.is_some_and(|o| o > 0)) {
        return Err(
            "a zero rise delay with a non-zero fall or turn-off delay is not supported: xezim \
             delays a driver's updates only while its rise delay is non-zero"
                .into(),
        );
    }
    let explicit = if sdf_only {
        Vec::new()
    } else {
        delayed_drivers(sim, id)
    };
    if explicit.is_empty() {
        let writers: Vec<usize> = sim
            .comb_entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.cold.write_signal_ids.contains(&id))
            .map(|(i, _)| i)
            .collect();
        if writers.is_empty() {
            return Err("the net has no continuous driver to delay".into());
        }
        for &e in &writers {
            match &sim.comb_entries[e].item {
                CombItem::ContAssign { .. }
                | CombItem::DirectCopy { .. }
                | CombItem::FastDirectCopy { .. }
                | CombItem::FusedGate { .. } => {}
                _ => {
                    return Err(
                        "its driver was compiled without a delay, and xezim cannot add one to \
                         that form at run time"
                            .into(),
                    );
                }
            }
        }
        for &e in &writers {
            if let CombItem::FastDirectCopy { dst_id, src_id } = sim.comb_entries[e].item {
                let width = sim.signal_widths[dst_id];
                sim.comb_entries[e].item = CombItem::DirectCopy {
                    dst_id,
                    src_id,
                    width,
                };
            }
        }
        if sim.sdf_delays.len() != sim.signal_table.len() {
            let n = sim.signal_table.len();
            sim.sdf_delays.resize(n, 0);
        }
        sim.sdf_delays[id] = rise;
        // Gate edges assume no delays (`comb_entry_read_masks`, `CtlGate`):
        // rebuild them at the next settle.
        sim.comb_dep_edges.clear();
    } else {
        for e in explicit {
            if let CombItem::ContAssign { delay, delay_exprs, .. } = &mut sim.comb_entries[e].item {
                *delay_exprs = None;
                *delay = rise;
            }
        }
    }
    if fall != rise {
        sim.gate_fall_delay_by_id.insert(id, fall);
    } else {
        sim.gate_fall_delay_by_id.remove(&id);
    }
    match off {
        Some(o) => {
            sim.gate_off_delay_by_id.insert(id, o);
        }
        None => {
            sim.gate_off_delay_by_id.remove(&id);
        }
    }
    Ok(())
}

/// The object's delays in ticks, in the §38.10 order, for `n` delays.
fn read_delays(sim: &Simulator, obj: &DelayObj, n: usize) -> Result<Vec<i64>, String> {
    match *obj {
        DelayObj::Net(id) | DelayObj::InterMod(id) => {
            let inter = matches!(obj, DelayObj::InterMod(_));
            let ok = if inter {
                (2..=3).contains(&n)
            } else {
                (1..=3).contains(&n)
            };
            if !ok {
                return Err(format!(
                    "no_of_delays {} (a {} takes {})",
                    n,
                    if inter { "vpiInterModPath" } else { "net" },
                    if inter { "2 or 3" } else { "1, 2 or 3" }
                ));
            }
            Ok(net_delays(sim, id, inter)[..n]
                .iter()
                .map(|&d| d as i64)
                .collect())
        }
        DelayObj::ModPath(id, i) => {
            if ![1, 2, 3, 6, 12].contains(&n) {
                return Err(format!(
                    "no_of_delays {} (a module path takes 1, 2, 3, 6 or 12)",
                    n
                ));
            }
            let d = sim
                .module_paths
                .as_ref()
                .and_then(|mp| mp.vpi_delays(id, i))
                .ok_or("the module path no longer exists (SDF replaced it)")?;
            Ok(d[..n].iter().map(|&d| d as i64).collect())
        }
        DelayObj::Tchk(i) => {
            let l = sim.vpi_tchk_limits(i).ok_or("no such timing check")?;
            if l.len() != n {
                return Err(format!(
                    "no_of_delays {}: this timing check has {} limit(s)",
                    n,
                    l.len()
                ));
            }
            Ok(l)
        }
    }
}

fn write_delays(sim: &mut Simulator, obj: &DelayObj, v: &[i64]) -> Result<(), String> {
    let tchk = matches!(obj, DelayObj::Tchk(_));
    if !tchk && v.iter().any(|&d| d < 0) {
        return Err("a delay cannot be negative".into());
    }
    let u = |k: usize| v[k] as u64;
    match *obj {
        DelayObj::Net(id) | DelayObj::InterMod(id) => {
            let inter = matches!(obj, DelayObj::InterMod(_));
            match v.len() {
                1 => set_net_delays(sim, id, u(0), u(0), None, inter),
                2 => set_net_delays(sim, id, u(0), u(1), None, inter),
                _ => set_net_delays(sim, id, u(0), u(1), Some(u(2)), inter),
            }
        }
        DelayObj::ModPath(id, i) => {
            let d: Vec<u64> = v.iter().map(|&x| x as u64).collect();
            let twelve = xezim_core::elaborate::path_transition_delays(&d);
            let done = sim
                .module_paths
                .as_mut()
                .is_some_and(|mp| mp.vpi_set_delays(id, i, twelve));
            if done {
                Ok(())
            } else {
                Err("the module path no longer exists (SDF replaced it)".into())
            }
        }
        DelayObj::Tchk(i) => sim.vpi_set_tchk_limits(i, v),
    }
}

/// Ticks <-> s_vpi_time in `time_type`, scaled to the unit exponent `unit`
/// for vpiScaledRealTime.
fn ticks_to_time(ticks: i64, time_type: c_int, unit: i32, tick_s: f64) -> s_vpi_time {
    let t = ticks as u64;
    s_vpi_time {
        type_: time_type,
        high: (t >> 32) as u32,
        low: t as u32,
        real: if time_type == vpi::SCALED_REAL_TIME {
            ticks as f64 * tick_s / 10f64.powi(unit)
        } else {
            ticks as f64
        },
    }
}

fn time_to_ticks(t: &s_vpi_time, time_type: c_int, unit: i32, tick_s: f64) -> i64 {
    if time_type == vpi::SCALED_REAL_TIME {
        (t.real * 10f64.powi(unit) / tick_s).round() as i64
    } else {
        (((t.high as u64) << 32) | t.low as u64) as i64
    }
}

/// Number of `da` entries one delay takes (§38.10 Table 38-2).
fn slots_per_delay(d: &s_vpi_delay) -> usize {
    (if d.mtm_flag != 0 { 3 } else { 1 }) * (if d.pulsere_flag != 0 { 3 } else { 1 })
}

/// Checks shared by both routines; returns the delay object and the unit
/// exponent and tick for time conversion.
fn delay_call(
    sim: &Simulator,
    obj: *mut c_void,
    d: &s_vpi_delay,
) -> Result<(DelayObj, i32, f64), String> {
    let h = unsafe { vpi_deref(obj) }.ok_or("null handle")?;
    let o = delay_obj(h)?;
    if d.da.is_null() {
        return Err("null da array".into());
    }
    if d.no_of_delays <= 0 {
        return Err(format!("no_of_delays {}", d.no_of_delays));
    }
    if d.time_type != vpi::SIM_TIME && d.time_type != vpi::SCALED_REAL_TIME {
        return Err("time_type must be vpiSimTime or vpiScaledRealTime".into());
    }
    if d.pulsere_flag != 0 && matches!(o, DelayObj::Tchk(_)) {
        return Err("a timing check has limits, not pulse limits (pulsere_flag)".into());
    }
    let unit = sim.reported_timescale_exp(&delay_obj_def(sim, &o)).0;
    Ok((o, unit, sim.module.tick_s))
}

/// §38.10: the delays or timing limits of an object. Supported: nets and
/// ports (the delay of their driving gate, continuous assignment or
/// annotation), `vpiModPath`, `vpiTchk` and `vpiInterModPath` objects.
///
/// xezim keeps one value per delay, the min:typ:max selection it simulates
/// with, so with `mtm_flag` the min, typ and max slots all read that value;
/// and its delays are inertial with no separate pulse limits, so with
/// `pulsere_flag` the reject and error limits read the delay itself.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_get_delays(obj: *mut c_void, delay_p: *mut s_vpi_delay) {
    let obj = native_timing_handle(obj).unwrap_or(obj);
    if delay_p.is_null() {
        vpi_error(
            vpi::ERROR,
            "vpi_get_delays: null s_vpi_delay pointer".into(),
        );
        return;
    }
    let d = unsafe { &*delay_p };
    let r = try_active_sim("vpi_get_delays", |sim| -> Result<(), String> {
        let (o, unit, tick_s) = delay_call(sim, obj, d)?;
        let n = d.no_of_delays as usize;
        let vals = read_delays(sim, &o, n)?;
        let per = slots_per_delay(d);
        let da = unsafe { std::slice::from_raw_parts_mut(d.da, n * per) };
        for (k, &v) in vals.iter().enumerate() {
            let t = ticks_to_time(v, d.time_type, unit, tick_s);
            for s in &mut da[k * per..(k + 1) * per] {
                *s = t;
            }
        }
        Ok(())
    });
    if let Some(Err(msg)) = r {
        vpi_error(vpi::ERROR, format!("vpi_get_delays: {}", msg));
    }
}

/// §38.32: set the delays or timing limits of an object — the values the
/// simulator uses from then on. With `mtm_flag` the value of the active
/// min:typ:max selection is taken; with `append_flag` the values are added
/// to the current ones; with `pulsere_flag` the reject and error limits
/// must equal the delay (xezim's delays are inertial with no separate pulse
/// limits), otherwise nothing is written.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_put_delays(obj: *mut c_void, delay_p: *mut s_vpi_delay) {
    let obj = native_timing_handle(obj).unwrap_or(obj);
    if delay_p.is_null() {
        vpi_error(
            vpi::ERROR,
            "vpi_put_delays: null s_vpi_delay pointer".into(),
        );
        return;
    }
    let d = unsafe { &*delay_p };
    let r = try_active_sim("vpi_put_delays", |sim| -> Result<(), String> {
        let (o, unit, tick_s) = delay_call(sim, obj, d)?;
        let n = d.no_of_delays as usize;
        let per = slots_per_delay(d);
        let da = unsafe { std::slice::from_raw_parts(d.da, n * per) };
        // Which of min/typ/max the simulation runs with.
        let pick = match sim.sdf_select {
            Some(crate::compiler::sdf::DelaySelect::Min) => 0,
            Some(crate::compiler::sdf::DelaySelect::Max) => 2,
            _ => 1,
        };
        let mtm = d.mtm_flag != 0;
        let mut vals = Vec::with_capacity(n);
        for k in 0..n {
            let slot = &da[k * per..(k + 1) * per];
            let at = |i: usize| time_to_ticks(&slot[i], d.time_type, unit, tick_s);
            let delay = if mtm { at(pick) } else { at(0) };
            if d.pulsere_flag != 0 {
                let (reject, error) = if mtm {
                    (at(3 + pick), at(6 + pick))
                } else {
                    (at(1), at(2))
                };
                if reject != delay || error != delay {
                    return Err(
                        "xezim's delays are inertial with no separate pulse limits: the reject \
                         and error limits must equal the delay"
                            .into(),
                    );
                }
            }
            vals.push(delay);
        }
        if d.append_flag != 0 {
            let cur = read_delays(sim, &o, n)?;
            for (v, c) in vals.iter_mut().zip(cur) {
                *v += c;
            }
        } else {
            // Validate the count against the object before writing.
            read_delays(sim, &o, n)?;
        }
        write_delays(sim, &o, &vals)
    });
    if let Some(Err(msg)) = r {
        vpi_error(
            vpi::ERROR,
            format!("vpi_put_delays: {} (nothing written)", msg),
        );
    }
}

// ===========================================================================
// Save / restart data
// ===========================================================================

/// §38.9: data from a save/restart location, callable only from a
/// cbStartOfRestart / cbEndOfRestart callback. xezim has no `$save` /
/// `$restart`, so no restart is ever in progress: the call fails, returning
/// 0 (no bytes) and reporting the error through `vpi_chk_error`.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_get_data(id: c_int, data_loc: *mut libc::c_char, num_bytes: c_int) -> c_int {
    let _ = (data_loc, num_bytes);
    vpi_error(
        vpi::ERROR,
        format!(
            "vpi_get_data(id {}): only callable while a restart is in progress, and xezim has \
             no $save/$restart",
            id
        ),
    );
    0
}

/// §38.31: data into a save/restart location, callable only from a
/// cbStartOfSave / cbEndOfSave callback. Fails outside one — always, in
/// xezim — returning 0 and reporting through `vpi_chk_error`.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_put_data(id: c_int, data_loc: *mut libc::c_char, num_bytes: c_int) -> c_int {
    let _ = (data_loc, num_bytes);
    vpi_error(
        vpi::ERROR,
        format!(
            "vpi_put_data(id {}): only callable while a save is in progress, and xezim has no \
             $save/$restart",
            id
        ),
    );
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_string_has_at_most_16_significant_digits() {
        assert_eq!(real_string(0.1), "0.1");
        assert_eq!(real_string(2.5), "2.5");
        assert_eq!(real_string(-2.5), "-2.5");
        assert_eq!(real_string(1e10), "10000000000");
        assert_eq!(real_string(1e20), "1e+20");
        assert_eq!(real_string(1.5e-7), "1.5e-07");
        assert_eq!(real_string(0.0), "0");
        assert_eq!(real_string(1.0 / 3.0), "0.3333333333333333");
    }

    #[test]
    fn radix_digits_mark_partly_unknown_digits_in_capitals() {
        let mut v = Value::zero(8);
        // 1x10_zzzz
        for (i, c) in [3u8, 3, 3, 3, 0, 1, 2, 1].iter().enumerate() {
            v.set_bit_code(i, *c);
        }
        assert_eq!(radix_string_xz(&v, 4), "Xz");
        let mut w = Value::zero(8);
        for i in 0..8 {
            w.set_bit_code(i, 2);
        }
        assert_eq!(radix_string_xz(&w, 4), "xx");
        let mut z = Value::zero(6);
        z.set_bit_code(5, 3);
        assert_eq!(radix_string_xz(&z, 3), "Z0");
    }

    #[test]
    fn raw_layout_round_trips() {
        let mut v = Value::zero(12);
        for (i, c) in [0u8, 1, 3, 2, 1, 1, 0, 0, 1, 0, 2, 3].iter().enumerate() {
            v.set_bit_code(i, *c);
        }
        let mut raw = [0u8; 4];
        value_to_raw(&v, true, &mut raw);
        // aval: bits 1,3,4,5,8,10 ; bval: bits 2,3,10,11
        assert_eq!(raw, [0x3a, 0x05, 0x0c, 0x0c]);
        let back = raw_to_value(&raw, 12, true, false);
        for i in 0..12 {
            assert_eq!(back.get_bit_code(i), v.get_bit_code(i), "bit {i}");
        }
        let mut two = [0u8; 2];
        value_to_raw(&v, false, &mut two);
        assert_eq!(two, [0x32, 0x01]);
    }

    #[test]
    fn array_formats_follow_the_element_types() {
        use ElemKind::*;
        assert!(array_format_ok(vpi::INT_VAL, Integral, false));
        assert!(!array_format_ok(vpi::INT_VAL, Real, false));
        assert!(array_format_ok(vc::SHORT_INT_VAL, Byte, false));
        assert!(!array_format_ok(vc::SHORT_INT_VAL, Byte, true));
        assert!(array_format_ok(vc::SHORT_INT_VAL, LongInt, true));
        assert!(array_format_ok(vc::LONG_INT_VAL, ShortInt, false));
        assert!(!array_format_ok(vc::LONG_INT_VAL, ShortInt, true));
        assert!(!array_format_ok(vc::SHORT_REAL_VAL, Real, false));
        assert!(!array_format_ok(vpi::STRING_VAL, Integral, false));
        assert_eq!(array_elem_bytes(vc::RAW_FOUR_STATE_VAL, 42), 12);
        assert_eq!(array_elem_bytes(vpi::VECTOR_VAL, 42), 16);
    }
}
