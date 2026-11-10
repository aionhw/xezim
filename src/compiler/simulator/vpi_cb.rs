//! IEEE 1800-2017 §38.36 VPI callbacks (`vpi_register_cb`, `vpi_get_cb_info`,
//! `vpi_remove_cb`) and §38.14 `vpi_control`.
//!
//! Every registration becomes one [`VpiCbEntry`] in [`VpiCbState`], keyed by a
//! never-reused id. The handle `vpi_register_cb` returns is an ordinary
//! `VpiHandle` of kind `Callback` that carries that id, so `vpi_get(vpiType)`,
//! `vpi_free_object` and `vpi_compare_objects` treat it like any other object.
//! Freeing the handle does not remove the callback; `vpi_remove_cb` does both.
//!
//! Hooks in the scheduler and the interpreter are gated on `vpi_cb_mask`, which
//! has bit `r` set while a callback of reason `r` is registered, so a run with
//! no VPI client pays one load and branch per hook site and nothing else.
//!
//! Where each simulation-time reason fires within a time slot (§4.4, §38.36.2):
//!
//! | reason               | region        | xezim                                   |
//! |----------------------|---------------|-----------------------------------------|
//! | cbNextSimTime        | Pre-Active    | first delta of the next time, first     |
//! | cbAtStartOfSimTime   | Pre-Active    | first delta of the given time, second   |
//! | cbAfterDelay         | Active        | first delta of now + delay              |
//! | cbNBASynch           | Pre-NBA       | before the first NBA region of the time |
//! | cbReadWriteSynch     | Post-NBA      | after the NBA region and edge cascade   |
//! | cbAtEndOfSimTime     | Pre-Postponed | once every other region is empty        |
//! | cbReadOnlySynch      | Postponed     | last; writes are refused                |
//!
//! Callbacks that may write (all but cbReadOnlySynch) open a fresh delta when
//! they fire late in a slot, so a value they deposit is seen by edge-sensitive
//! processes in the same time step.
use super::*;
use libc::c_int;
use std::collections::BTreeSet;

// Callback reasons (IEEE 1800-2017 Annex K, vpi_user.h).
pub(super) const CB_VALUE_CHANGE: c_int = 1;
pub(super) const CB_STMT: c_int = 2;
pub(super) const CB_FORCE: c_int = 3;
pub(super) const CB_RELEASE: c_int = 4;
pub(super) const CB_AT_START_OF_SIM_TIME: c_int = 5;
pub(super) const CB_READ_WRITE_SYNCH: c_int = 6;
pub(super) const CB_READ_ONLY_SYNCH: c_int = 7;
pub(super) const CB_NEXT_SIM_TIME: c_int = 8;
pub(super) const CB_AFTER_DELAY: c_int = 9;
pub(super) const CB_END_OF_COMPILE: c_int = 10;
pub(super) const CB_START_OF_SIMULATION: c_int = 11;
pub(super) const CB_END_OF_SIMULATION: c_int = 12;
pub(super) const CB_ERROR: c_int = 13;
pub(super) const CB_TCHK_VIOLATION: c_int = 14;
pub(super) const CB_START_OF_SAVE: c_int = 15;
pub(super) const CB_END_OF_SAVE: c_int = 16;
pub(super) const CB_START_OF_RESTART: c_int = 17;
pub(super) const CB_END_OF_RESTART: c_int = 18;
pub(super) const CB_START_OF_RESET: c_int = 19;
pub(super) const CB_END_OF_RESET: c_int = 20;
pub(super) const CB_ENTER_INTERACTIVE: c_int = 21;
pub(super) const CB_EXIT_INTERACTIVE: c_int = 22;
pub(super) const CB_INTERACTIVE_SCOPE_CHANGE: c_int = 23;
pub(super) const CB_UNRESOLVED_SYSTF: c_int = 24;
pub(super) const CB_ASSIGN: c_int = 25;
pub(super) const CB_DEASSIGN: c_int = 26;
pub(super) const CB_DISABLE: c_int = 27;
pub(super) const CB_PLI_ERROR: c_int = 28;
pub(super) const CB_SIGNAL: c_int = 29;
pub(super) const CB_NBA_SYNCH: c_int = 30;
pub(super) const CB_AT_END_OF_SIM_TIME: c_int = 31;

/// `vpi_get(vpiType)` of a callback handle.
pub(super) const VPI_CALLBACK: c_int = 107;
/// `vpi_control` operation (§38.14) beyond the three in `vpi::`.
const SET_INTERACTIVE_SCOPE: c_int = 69;

/// `vpi_cb_mask` bit for reason `r`.
pub(super) const fn m(r: c_int) -> u64 {
    1u64 << r
}
/// Release / deassign / disable notifications waiting for the next checkpoint.
pub(super) const MASK_PENDING: u64 = 1u64 << 63;
/// Reasons fired at the start of a time slot.
pub(super) const MASK_SLOT_START: u64 = m(CB_NEXT_SIM_TIME) | m(CB_AT_START_OF_SIM_TIME);
/// Reasons that hold the scheduler at a future time.
const MASK_TIMED: u64 = m(CB_AT_START_OF_SIM_TIME)
    | m(CB_AFTER_DELAY)
    | m(CB_NBA_SYNCH)
    | m(CB_READ_WRITE_SYNCH)
    | m(CB_AT_END_OF_SIM_TIME)
    | m(CB_READ_ONLY_SYNCH);
/// Work checked at the end of every delta and at the end of a time slot.
const MASK_TICK_END: u64 = MASK_PENDING | m(CB_VALUE_CHANGE) | m(CB_READ_WRITE_SYNCH);
const MASK_SLOT_END: u64 =
    MASK_TICK_END | m(CB_NBA_SYNCH) | m(CB_AT_END_OF_SIM_TIME) | m(CB_READ_ONLY_SYNCH);

/// Nesting limit for callbacks that trigger callbacks (a value-change routine
/// writing the object it watches recurses without bound otherwise).
const MAX_CB_DEPTH: u32 = 1000;

/// Deltas `sva_post_nba` runs before it evaluates a clock event anyway: a
/// slot that never settles is the event loop's livelock to report.
const SVA_POST_NBA_ROUNDS: u32 = 1 << 24;

/// The one-shot, time-ordered reasons, each kept as a `(due tick, id)` set.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum TimedSet {
    AtStart,
    AfterDelay,
    Nba,
    Rw,
    AtEnd,
    Ro,
}

/// What a callback is attached to.
#[derive(Clone)]
enum CbTarget {
    /// Action and time reasons.
    None,
    /// cbValueChange: the watched signal ids (one per memory word, with its
    /// index), the bit range for a part-select, and the last value reported
    /// for each id (so a change is reported once, whichever write path made
    /// it).
    Watch {
        sigs: Vec<(usize, c_int)>,
        slice: Option<(u32, u32)>,
        last: Vec<Value>,
        type_code: c_int,
    },
    /// cbForce / cbRelease / cbAssign / cbDeassign on one object, or on every
    /// object (`obj == NULL`).
    Override(Option<usize>),
    /// cbStmt: statements executed by processes of this instance
    /// (design-relative path, "" for the top).
    Scope(String),
    /// cbDisable: fires when a `disable` names one of `names` (leaf names of
    /// the enclosing named blocks and subroutines) and, for a `$systf` call,
    /// affects the process `pid` that made the call.
    Disable {
        pid: Option<usize>,
        names: Vec<String>,
    },
}

/// One `vpi_register_cb` registration.
#[derive(Clone)]
pub(super) struct VpiCbEntry {
    reason: c_int,
    rtn: usize,
    user_data: usize,
    /// The `obj` handle as registered (client-owned), passed back unchanged.
    obj: usize,
    /// Registration `time` / `value->format` / `index`, for `vpi_get_cb_info`.
    reg_time: Option<s_vpi_time>,
    reg_value_format: Option<c_int>,
    reg_index: c_int,
    /// Format of `cb_data_p->time` / `->value` when the callback fires.
    time_type: c_int,
    value_format: c_int,
    /// Ticks per time unit of `obj`, for vpiScaledRealTime.
    ticks_per_unit: f64,
    /// Absolute due tick (timed reasons); registration tick (cbNextSimTime).
    due: u64,
    target: CbTarget,
    /// Cleared when the callback is removed, or when a one-shot fires.
    live: bool,
    one_shot: bool,
    /// Registered by xezim itself (the UVM HDL polling service).
    internal: bool,
}

/// Deferred notifications, fired at the next checkpoint so they see the state
/// after the operation completed (§38.36.1: "after a release", "after a named
/// block or task ... has been disabled").
#[derive(Clone)]
enum Pending {
    Release { reason: c_int, sig: Option<usize> },
    Disable { ids: Vec<u64> },
}

/// All VPI callback state. Boxed on the simulator: untouched unless a VPI
/// client registers callbacks.
#[derive(Default)]
pub(super) struct VpiCbState {
    next_id: u64,
    entries: HashMap<u64, VpiCbEntry>,
    /// Live callbacks per reason, which `vpi_cb_mask` mirrors.
    counts: [u32; 32],
    at_start: BTreeSet<(u64, u64)>,
    after_delay: BTreeSet<(u64, u64)>,
    nba: BTreeSet<(u64, u64)>,
    rw: BTreeSet<(u64, u64)>,
    at_end: BTreeSet<(u64, u64)>,
    ro: BTreeSet<(u64, u64)>,
    /// The other reasons, in registration order.
    lists: HashMap<c_int, Vec<u64>>,
    pending: Vec<Pending>,
    /// Inside the cbReadOnlySynch phase: writes and same-time registrations
    /// are refused (§38.36.2).
    in_ro: bool,
    /// `$stop` / `vpi_control(vpiStop)` ran (cbEnterInteractive).
    stop_requested: bool,
    /// cbUnresolvedSystf fires once per unknown name.
    unresolved_seen: HashSet<String>,
    /// The statement the process loop just reported to cbStmt, so the
    /// `exec_statement` it falls through to does not report it again.
    stmt_reported: usize,
}

impl VpiCbState {
    fn set(&mut self, s: TimedSet) -> &mut BTreeSet<(u64, u64)> {
        match s {
            TimedSet::AtStart => &mut self.at_start,
            TimedSet::AfterDelay => &mut self.after_delay,
            TimedSet::Nba => &mut self.nba,
            TimedSet::Rw => &mut self.rw,
            TimedSet::AtEnd => &mut self.at_end,
            TimedSet::Ro => &mut self.ro,
        }
    }

    fn set_ref(&self, s: TimedSet) -> &BTreeSet<(u64, u64)> {
        match s {
            TimedSet::AtStart => &self.at_start,
            TimedSet::AfterDelay => &self.after_delay,
            TimedSet::Nba => &self.nba,
            TimedSet::Rw => &self.rw,
            TimedSet::AtEnd => &self.at_end,
            TimedSet::Ro => &self.ro,
        }
    }

    fn mask(&self) -> u64 {
        let mut m = 0u64;
        for (r, &n) in self.counts.iter().enumerate() {
            if n > 0 {
                m |= 1u64 << r;
            }
        }
        if !self.pending.is_empty() {
            m |= MASK_PENDING;
        }
        m
    }

    /// Ids of the live list callbacks of `reason`, snapshotted so a routine may
    /// register or remove callbacks while the batch runs.
    fn list(&self, reason: c_int) -> Vec<u64> {
        self.lists.get(&reason).cloned().unwrap_or_default()
    }
}

fn timed_set_of(reason: c_int) -> Option<TimedSet> {
    Some(match reason {
        CB_AT_START_OF_SIM_TIME => TimedSet::AtStart,
        CB_AFTER_DELAY => TimedSet::AfterDelay,
        CB_NBA_SYNCH => TimedSet::Nba,
        CB_READ_WRITE_SYNCH => TimedSet::Rw,
        CB_AT_END_OF_SIM_TIME => TimedSet::AtEnd,
        CB_READ_ONLY_SYNCH => TimedSet::Ro,
        _ => return None,
    })
}

/// Reasons that fire at most once and are removed when they do.
fn is_one_shot(reason: c_int) -> bool {
    timed_set_of(reason).is_some()
        || matches!(
            reason,
            CB_NEXT_SIM_TIME | CB_END_OF_COMPILE | CB_START_OF_SIMULATION | CB_END_OF_SIMULATION
        )
}

/// Callback-specific data a firing supplies on top of the registration.
pub(super) struct Fire<'a> {
    /// `cb_data_p->obj`; NULL means "the registered obj".
    obj: *mut c_void,
    /// Value rendered in the registered format.
    value: Option<&'a Value>,
    type_code: Option<c_int>,
    /// A `vpiStringVal` payload (cbUnresolvedSystf, cbTchkViolation).
    text: Option<&'a str>,
    index: c_int,
}

impl Fire<'_> {
    const NONE: Fire<'static> = Fire {
        obj: std::ptr::null_mut(),
        value: None,
        type_code: None,
        text: None,
        index: 0,
    };
}

thread_local! {
    static CB_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    /// Inside a cbPLIError routine: its own VPI errors do not re-enter it.
    static IN_PLI_ERROR_CB: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Storage `vpi_get_cb_info` hands back for `time` and `value`.
    static CB_INFO_TIME: std::cell::RefCell<s_vpi_time> =
        const { std::cell::RefCell::new(s_vpi_time { type_: 2, high: 0, low: 0, real: 0.0 }) };
    static CB_INFO_VALUE: std::cell::RefCell<s_vpi_value> =
        const { std::cell::RefCell::new(s_vpi_value { format: 13, value: s_vpi_value_union { integer: 0 } }) };
}

/// Run callback `id` if it is still registered. Looking the entry up per call
/// (rather than iterating a copied list of entries) is what lets a routine
/// remove itself, or a later callback of the same batch, while the batch runs.
fn vpi_cb_invoke(sp: *mut Simulator, id: u64, f: &Fire) -> bool {
    if sp.is_null() {
        return false;
    }
    let depth = CB_DEPTH.with(|d| d.get());
    if depth >= MAX_CB_DEPTH {
        static WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if !WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            eprintln!(
                "[VPI] callbacks nested {} deep (a routine re-triggering itself?); deeper ones are dropped",
                MAX_CB_DEPTH
            );
        }
        return false;
    }
    let sim = unsafe { &mut *sp };
    let (reason, rtn, user_data, reg_obj, time_type, value_format, tpu, one_shot) =
        match sim.vpi_cb.entries.get_mut(&id) {
            Some(e) if e.live => {
                if e.one_shot {
                    e.live = false;
                }
                (
                    e.reason,
                    e.rtn,
                    e.user_data,
                    e.obj,
                    e.time_type,
                    e.value_format,
                    e.ticks_per_unit,
                    e.one_shot,
                )
            }
            _ => return false,
        };
    if one_shot {
        sim.vpi_cb_uncount(reason);
    }
    let now = sim.time;
    let mut time = match time_type {
        vpi::SUPPRESS_TIME => s_vpi_time {
            type_: vpi::SUPPRESS_TIME,
            high: 0,
            low: 0,
            real: 0.0,
        },
        t => s_vpi_time {
            type_: t,
            high: (now >> 32) as u32,
            low: (now & 0xFFFF_FFFF) as u32,
            real: if t == vpi::SCALED_REAL_TIME {
                now as f64 / tpu
            } else {
                now as f64
            },
        },
    };
    // Payloads that live in thread-local scratch are copied into this frame:
    // a routine that calls vpi_get_value would otherwise overwrite its own
    // callback data.
    let mut own_vec: Vec<s_vpi_vecval>;
    let mut own_str: Vec<u8> = Vec::new();
    let mut own_time: s_vpi_time;
    let mut own_extra: Vec<u64> = Vec::new();
    let mut value = s_vpi_value {
        format: vpi::SUPPRESS_VAL,
        value: s_vpi_value_union { integer: 0 },
    };
    if let Some(t) = f.text {
        own_str.extend_from_slice(t.as_bytes());
        own_str.push(0);
        value.format = vpi::STRING_VAL;
        value.value.str = own_str.as_mut_ptr() as *mut libc::c_char;
    } else if let Some(v) = f.value {
        if value_format != vpi::SUPPRESS_VAL {
            value.format = value_format;
            let obj_ptr = if f.obj.is_null() {
                reg_obj as *mut c_void
            } else {
                f.obj
            };
            super::vpi_api::set_strength_hint_for_obj(sim, obj_ptr, f.type_code, value_format);
            let filled = fill_vpi_value(v, now, f.type_code, &mut value);
            super::vpi_api::clear_strength_hint();
            if filled {
                match value.format {
                    vpi::VECTOR_VAL => {
                        let words = (v.width.max(1) as usize).div_ceil(32);
                        own_vec = unsafe { std::slice::from_raw_parts(value.value.vector, words) }
                            .to_vec();
                        value.value.vector = own_vec.as_mut_ptr();
                    }
                    vpi::BIN_STR_VAL
                    | vpi::OCT_STR_VAL
                    | vpi::HEX_STR_VAL
                    | vpi::DEC_STR_VAL
                    | vpi::STRING_VAL => {
                        own_str = unsafe { std::ffi::CStr::from_ptr(value.value.str) }
                            .to_bytes_with_nul()
                            .to_vec();
                        value.value.str = own_str.as_mut_ptr() as *mut libc::c_char;
                    }
                    vpi::TIME_VAL => {
                        own_time = unsafe { *value.value.time };
                        value.value.time = &mut own_time;
                    }
                    _ => super::vpi_api::own_extra_payload(v.width, &mut value, &mut own_extra),
                }
            } else {
                value.format = vpi::SUPPRESS_VAL;
            }
        }
    }
    let mut cb_data = s_cb_data {
        reason,
        cb_rtn: rtn as *mut c_void,
        obj: if f.obj.is_null() {
            reg_obj as *mut c_void
        } else {
            f.obj
        },
        time: &mut time,
        value: &mut value,
        index: f.index,
        user_data: user_data as *mut c_void,
    };
    // `$display` output and file writes made so far come first.
    sim.vpi_sync_output();
    let prev = ACTIVE_SIMULATOR.with(|c| c.replace(sp));
    CB_DEPTH.with(|d| d.set(depth + 1));
    type CbFn = extern "C" fn(*mut s_cb_data) -> c_int;
    let routine: CbFn = unsafe { std::mem::transmute(rtn as *const ()) };
    routine(&mut cb_data);
    CB_DEPTH.with(|d| d.set(depth));
    ACTIVE_SIMULATOR.with(|c| c.set(prev));
    if one_shot {
        // The handle stays valid (the client frees it); the entry goes.
        unsafe { &mut *sp }.vpi_cb.entries.remove(&id);
    }
    true
}

/// The simulator the calling VPI routine runs under, or NULL.
fn active_sim_ptr() -> *mut Simulator {
    let p = ACTIVE_SIMULATOR.with(|c| c.get());
    if p.is_null() {
        GLOBAL_ACTIVE_SIMULATOR.load(std::sync::atomic::Ordering::Acquire)
    } else {
        p
    }
}

/// A `vpiSimTime` / `vpiScaledRealTime` record as a tick count, scaled by the
/// time unit of the registration obj.
fn time_to_ticks(t: &s_vpi_time, ticks_per_unit: f64) -> u64 {
    if t.type_ == vpi::SCALED_REAL_TIME {
        let v = t.real * ticks_per_unit;
        if v > 0.0 { v.round() as u64 } else { 0 }
    } else {
        ((t.high as u64) << 32) | t.low as u64
    }
}

/// Ticks per time unit of `obj` for vpiScaledRealTime: a module's own
/// timeunit, else (NULL or any other object) the simulation's, which is the
/// tick. Matches `vpi_get(vpiTimeUnit, obj)`.
fn ticks_per_unit_of(sim: &Simulator, obj: *mut c_void) -> f64 {
    let def = unsafe { vpi_deref(obj) }
        .filter(|h| h.kind == VpiKind::Module)
        .map(|h| h.def_name.clone());
    match def {
        Some(d) if sim.module.tick_s > 0.0 => {
            let (unit, _) = sim.reported_timescale_exp(&d);
            (10f64.powi(unit) / sim.module.tick_s).max(1.0)
        }
        _ => 1.0,
    }
}

/// Leaf of a `m_scope_stack` entry or a hierarchical name.
fn scope_leaf(s: &str) -> String {
    let s = s.trim_start_matches([M_ROOT_MARK, M_SUBR_MARK]);
    s.rsplit('.').next().unwrap_or(s).to_string()
}

/// Is `format` one `fill_vpi_value` can render?
fn value_format_supported(format: c_int) -> bool {
    if format == vpi::SUPPRESS_VAL {
        return true;
    }
    let mut probe = s_vpi_value {
        format,
        value: s_vpi_value_union { integer: 0 },
    };
    fill_vpi_value(&Value::zero(1), 0, None, &mut probe)
}

impl Simulator {
    /// Recompute `vpi_cb_mask` from the per-reason counts.
    fn vpi_cb_sync_mask(&mut self) {
        self.vpi_cb_mask = self.vpi_cb.mask();
    }

    fn vpi_cb_uncount(&mut self, reason: c_int) {
        if let Some(n) = self.vpi_cb.counts.get_mut(reason as usize) {
            *n = n.saturating_sub(1);
        }
        self.vpi_cb_sync_mask();
    }

    /// Store a new entry and index it. Returns its id.
    fn vpi_cb_insert(&mut self, e: VpiCbEntry) -> u64 {
        self.vpi_cb.next_id += 1;
        let id = self.vpi_cb.next_id;
        let reason = e.reason;
        if let Some(set) = timed_set_of(reason) {
            self.vpi_cb.set(set).insert((e.due, id));
        } else {
            self.vpi_cb.lists.entry(reason).or_default().push(id);
        }
        if let CbTarget::Watch { sigs, .. } = &e.target {
            for &(sig, _) in sigs {
                let l = self.dpi_value_change_cbs.entry(sig).or_default();
                if !l.contains(&id) {
                    l.push(id);
                }
            }
        }
        self.vpi_cb.entries.insert(id, e);
        if let Some(n) = self.vpi_cb.counts.get_mut(reason as usize) {
            *n += 1;
        }
        self.vpi_cb_sync_mask();
        id
    }

    /// Unindex and drop callback `id`. True when it was still registered
    /// (not yet fired, if one-shot).
    fn vpi_cb_delete(&mut self, id: u64) -> bool {
        let Some(e) = self.vpi_cb.entries.remove(&id) else {
            return false;
        };
        if let Some(set) = timed_set_of(e.reason) {
            self.vpi_cb.set(set).remove(&(e.due, id));
        } else if let Some(l) = self.vpi_cb.lists.get_mut(&e.reason) {
            l.retain(|&x| x != id);
        }
        if let CbTarget::Watch { sigs, .. } = &e.target {
            for &(sig, _) in sigs {
                if let Some(l) = self.dpi_value_change_cbs.get_mut(&sig) {
                    l.retain(|&x| x != id);
                    if l.is_empty() {
                        self.dpi_value_change_cbs.remove(&sig);
                    }
                }
            }
        }
        if e.live {
            if let Some(n) = self.vpi_cb.counts.get_mut(e.reason as usize) {
                *n = n.saturating_sub(1);
            }
        }
        self.vpi_cb_sync_mask();
        e.live
    }

    /// Register an xezim-internal value-change routine on `sig` (the UVM HDL
    /// polling service). Its `cb_data_p->value` is vpiSuppressVal.
    pub(super) fn vpi_cb_add_internal_watch(&mut self, sig: usize, rtn: usize, user_data: usize) {
        let last = self
            .signal_table
            .get(sig)
            .cloned()
            .unwrap_or_else(|| Value::zero(1));
        self.vpi_cb_insert(VpiCbEntry {
            reason: CB_VALUE_CHANGE,
            rtn,
            user_data,
            obj: 0,
            reg_time: None,
            reg_value_format: Some(vpi::SUPPRESS_VAL),
            reg_index: 0,
            time_type: vpi::SUPPRESS_TIME,
            value_format: vpi::SUPPRESS_VAL,
            ticks_per_unit: 1.0,
            due: 0,
            target: CbTarget::Watch {
                sigs: vec![(sig, 0)],
                slice: None,
                last: vec![last],
                type_code: vpi::REG,
            },
            live: true,
            one_shot: false,
            internal: true,
        });
    }

    /// Remove the internal value-change routine `rtn`/`user_data` on `sig`.
    pub(super) fn vpi_cb_remove_internal_watch(
        &mut self,
        sig: usize,
        rtn: usize,
        user_data: usize,
    ) {
        let ids: Vec<u64> = self
            .dpi_value_change_cbs
            .get(&sig)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|id| {
                self.vpi_cb
                    .entries
                    .get(id)
                    .is_some_and(|e| e.internal && e.rtn == rtn && e.user_data == user_data)
            })
            .collect();
        for id in ids {
            self.vpi_cb_delete(id);
        }
    }

    /// Flush `$display` output (only if anything was written since the last
    /// flush) and file writes, so a callback's `vpi_printf` lands after them.
    fn vpi_sync_output(&mut self) {
        if self.stdout_unsynced {
            self.stdout_unsynced = false;
            self.flush_stdout();
        }
        self.flush_file_writes();
    }

    /// Fire the `set` callbacks due at or before now, earliest first. True
    /// when any ran.
    ///
    /// cbNBASynch, cbReadWriteSynch and cbAtEndOfSimTime fire only the batch
    /// pending on entry: one a routine registers for the same time waits for
    /// the next pass through its region, after the deltas the routine's own
    /// writes cause (cocotb applies queued writes from cbReadWriteSynch and
    /// queues the writes they trigger for the next one). The Pre-Active and
    /// read-only reasons, which cannot cause deltas, drain completely.
    fn vpi_fire_due(&mut self, set: TimedSet) -> bool {
        let now = self.time;
        let sp = self as *mut Simulator;
        let one_batch = matches!(set, TimedSet::Nba | TimedSet::Rw | TimedSet::AtEnd);
        let mut fired = false;
        loop {
            let batch: Vec<(u64, u64)> = self
                .vpi_cb
                .set_ref(set)
                .range(..=(now, u64::MAX))
                .copied()
                .collect();
            if batch.is_empty() {
                break;
            }
            for key in batch {
                // A routine earlier in the batch may have removed this one.
                if self.vpi_cb.set(set).remove(&key) {
                    fired |= vpi_cb_invoke(sp, key.1, &Fire::NONE);
                }
            }
            if one_batch {
                break;
            }
        }
        fired
    }

    /// Anything left to run in the current time slot?
    fn vpi_same_time_work(&self) -> bool {
        self.dirty_any
            || self.event_queue.next_time() == Some(self.time)
            || !self.nba_fast.is_empty()
            || !self.nba_queue.is_empty()
            || !self.inactive_queue.is_empty()
            || !self.pending_reactive.is_empty()
            || !self.nba_region_waiters.is_empty()
            || !self.deferred_clocking_conts.is_empty()
            || !self.deferred_clocking_guarded.is_empty()
    }

    /// After callbacks that may have written: a fresh delta, so a deposited
    /// value is evaluated like any active-region write — edge-sensitive
    /// blocks see it before the next snapshot absorbs it as the baseline.
    /// True when same-time work remains for the scheduler.
    fn vpi_open_delta(&mut self) -> bool {
        if self.dirty_any {
            self.settle_combinatorial();
        }
        self.check_edges();
        let _ = self.drain_edge_cascade(self.cascade_limit);
        // §14.13: a clock deposited by a synchronization callback opens a
        // new delta at this time. Complete its sampling and clocking-event
        // delivery after the resulting NBAs, before absorbing the edge into
        // the next snapshot. Otherwise @(cb) remains deferred until another
        // clock toggle, and #0 inputs still expose the preceding edge.
        self.drain_observed_region();
        // §16.5.1: the assertions tick in the slot of an edge a routine makes.
        self.vpi_observed();
        self.tick_clocking_blocks();
        let edge_drove = std::mem::take(&mut self.clocking_edge_drive_dirty);
        if edge_drove && self.dirty_any {
            self.settle_combinatorial();
        }
        if self.clocking_mirror_dirty || edge_drove {
            self.clocking_mirror_dirty = false;
            let saved = self.edge_dispatch_phase;
            self.edge_dispatch_phase = "clocking-mirror";
            self.check_edges();
            self.edge_dispatch_phase = saved;
            let _ = self.drain_edge_cascade(self.cascade_limit);
        }
        // The Reactive work waits while `sva_post_nba` finishes the slot
        // before an assertion clock event (§4.4.2).
        if !self.sva_post_nba_busy {
            self.drain_deferred_clocking_conts();
            self.drain_reactive_region();
            self.drain_nba_region_waiters();
        }
        self.snapshot_edge_signals();
        self.vpi_same_time_work()
    }

    /// A cbNBASynch or cbReadWriteSynch due now: the active region set
    /// (§4.4.2) still has work, which comes before the Observed region.
    #[inline]
    pub(super) fn vpi_synch_due(&self) -> bool {
        let now = self.time;
        let due = |s: &BTreeSet<(u64, u64)>| s.first().is_some_and(|&(t, _)| t <= now);
        self.vpi_cb_mask & (m(CB_NBA_SYNCH) | m(CB_READ_WRITE_SYNCH)) != 0
            && (due(&self.vpi_cb.nba) || due(&self.vpi_cb.rw))
    }

    /// §16.5.1, §4.4.2: the Observed region of a delta a VPI routine opened.
    /// cocotb's Clock writes the clock from cbReadWriteSynch; the concurrent
    /// assertions tick in that slot, with its Preponed samples
    /// (`sva_observed`). Without this they ticked in a later delta or the
    /// next slot and read every flop's new value. A slot the nested
    /// scheduler ran took no samples. Once a routine has finished the run
    /// (`vpi_control(vpiFinish)` runs `$finish`, §20.2) nothing is
    /// evaluated: cocotb ends a test from the value-change callback of an
    /// edge, and that edge's Observed region never comes.
    fn vpi_observed(&mut self) {
        if self.finished || self.sva_preponed_slot != Some(self.time) {
            return;
        }
        self.sva_observed();
    }

    /// §4.4.2: the Observed region of a delta in whose slot a cbNBASynch or
    /// cbReadWriteSynch is still due. The whole active region set comes
    /// before the Observed region (its Active, Inactive and NBA work, the
    /// Pre-NBA cbNBASynch and Post-NBA cbReadWriteSynch routines), and the
    /// Observed region before the Reactive work (clocking-event waiters,
    /// program code). xezim runs cbReadWriteSynch at the end of a delta,
    /// after that Reactive work, so evaluated there, `disable iff` would
    /// read a reset a cocotb test releases after `RisingEdge` too late. With
    /// a clock event recorded, this runs the three steps in order, in one
    /// loop that a routine queuing another cannot turn into recursion: the
    /// nested `sva_observed` of a delta opened here only records clock
    /// events (a routine that moves the clock back does not lose one), and
    /// that delta holds its Reactive work, so a reset a clocking-event waiter
    /// raises at the edge still comes after the attempt.
    #[inline(never)]
    pub(super) fn sva_post_nba(&mut self) {
        self.note_sva_edges();
        if self.sva_post_nba_busy {
            return;
        }
        self.sva_post_nba_busy = true;
        // 1. Finish the slot's active region set, one delta at a time, in
        //    region order: the Active and Inactive work until none is left
        //    (as `run_one_tick` runs it: the processes due now, the `wait`
        //    conditions they satisfy, the `#0` continuations they park),
        //    then the due cbNBASynch batch, then the NBA region
        //    (`vpi_open_delta`), and once all of that is done the due
        //    cbReadWriteSynch batch. A batch that queues another is the
        //    next round.
        let now = self.time;
        let due = |s: &BTreeSet<(u64, u64)>| s.first().is_some_and(|&(t, _)| t <= now);
        let mut rounds = 0u32;
        // A slot that never settles (a zero-delay spinner) is left to the
        // event loop's livelock handling once the loop gives up.
        while self.sva_owed
            && !self.finished
            && self.time == now
            && !self.zero_delay_defer_pending
            && rounds < SVA_POST_NBA_ROUNDS
            && !crate::compiler::interrupt::interrupt_requested()
        {
            rounds += 1;
            self.vpi_checkpoint();
            let mut work = false;
            loop {
                while self.event_queue.next_time() == Some(now)
                    && !self.finished
                    && !self.zero_delay_defer_pending
                {
                    let Some((pid, stmts)) = self.event_queue.pop_front(now) else {
                        break;
                    };
                    self.run_scheduled_process(pid, &stmts);
                    if !self.is_pid_suspended(pid) {
                        self.child_finished(pid);
                    }
                    work = true;
                }
                if self.finished || self.zero_delay_defer_pending {
                    break;
                }
                self.drain_condition_waiters(true, false);
                if self.event_queue.next_time() == Some(now) {
                    continue;
                }
                if self.promote_inactive_to_active() {
                    if self.dirty_any {
                        self.settle_combinatorial();
                    }
                    self.check_edges();
                    work = true;
                    continue;
                }
                break;
            }
            if !self.inactive_queue.is_empty() {
                self.drain_inactive_pre_nba();
                work = true;
            }
            if self.finished {
                break;
            }
            if self.vpi_cb_mask & m(CB_NBA_SYNCH) != 0 && due(&self.vpi_cb.nba) {
                work |= self.vpi_fire_due(TimedSet::Nba);
            }
            work |= self.dirty_any
                || !self.nba_fast.is_empty()
                || !self.nba_queue.is_empty()
                || !self.packed_nba.is_empty();
            if !work && self.vpi_cb_mask & m(CB_READ_WRITE_SYNCH) != 0 && due(&self.vpi_cb.rw) {
                work = self.vpi_fire_due(TimedSet::Rw);
            }
            if !work {
                break;
            }
            self.vpi_open_delta();
        }
        self.sva_post_nba_busy = false;
        // 2. Evaluate the recorded clock events.
        self.tick_sva_sites();
        // 3. The caller then resumes the Reactive work: clocking-event
        //    waiters, program code and Re-NBA waiters, held by the deltas
        //    above.
    }

    /// Earliest tick a pending simulation-time callback holds the scheduler
    /// at. Same-time cbNBASynch/cbReadWriteSynch/cbAtEndOfSimTime/
    /// cbReadOnlySynch are serviced by the slot's own deltas and its end, so
    /// only future ones count.
    pub(super) fn next_vpi_cb_time(&self) -> Option<u64> {
        if self.vpi_cb_mask & MASK_TIMED == 0 {
            return None;
        }
        let now = self.time;
        let st = &self.vpi_cb;
        let later = |s: &BTreeSet<(u64, u64)>| {
            s.range((now.saturating_add(1), 0)..)
                .next()
                .map(|&(t, _)| t)
        };
        [
            st.after_delay.first().map(|&(t, _)| t),
            st.at_start.first().map(|&(t, _)| t),
            later(&st.nba),
            later(&st.rw),
            later(&st.at_end),
            later(&st.ro),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// Does a simulation-time callback still need the scheduler to run?
    pub(super) fn vpi_timed_future(&self) -> bool {
        self.next_vpi_cb_time().is_some()
    }

    /// cbAfterDelay due now (§38.36.2), in the first delta of its time.
    pub(super) fn fire_due_after_delay_cbs(&mut self) -> bool {
        if self.vpi_cb_mask & m(CB_AFTER_DELAY) == 0 {
            return false;
        }
        self.vpi_fire_due(TimedSet::AfterDelay)
    }

    /// Start of a time slot (Pre-Active): cbNextSimTime registered before now,
    /// then cbAtStartOfSimTime for now. True when any fired.
    pub(super) fn vpi_slot_start(&mut self) -> bool {
        let now = self.time;
        let sp = self as *mut Simulator;
        let mut fired = false;
        if self.vpi_cb_mask & m(CB_NEXT_SIM_TIME) != 0 {
            for id in self.vpi_cb.list(CB_NEXT_SIM_TIME) {
                let due = self
                    .vpi_cb
                    .entries
                    .get(&id)
                    .filter(|e| e.live)
                    .map(|e| e.due < now);
                if due == Some(true) {
                    if let Some(l) = self.vpi_cb.lists.get_mut(&CB_NEXT_SIM_TIME) {
                        l.retain(|&x| x != id);
                    }
                    fired |= vpi_cb_invoke(sp, id, &Fire::NONE);
                }
            }
        }
        if self.vpi_cb_mask & m(CB_AT_START_OF_SIM_TIME) != 0 {
            fired |= self.vpi_fire_due(TimedSet::AtStart);
        }
        fired
    }

    /// cbNBASynch (Pre-NBA region), called just before the NBA region runs.
    pub(super) fn vpi_nba_synch(&mut self) {
        if self.vpi_fire_due(TimedSet::Nba) && self.dirty_any {
            self.settle_combinatorial();
        }
    }

    /// End of a delta: deferred notifications, value changes made by paths
    /// that bypass `write_sig!`, then cbReadWriteSynch — the Post-NBA region,
    /// which runs once the active, inactive and NBA regions are empty, so
    /// not while this delta left same-time processes to run.
    pub(super) fn vpi_tick_end(&mut self) {
        if self.vpi_cb_mask & MASK_TICK_END == 0 {
            return;
        }
        self.vpi_checkpoint();
        if self.vpi_cb_mask & m(CB_READ_WRITE_SYNCH) != 0
            && !self.vpi_same_time_work()
            && self.vpi_fire_due(TimedSet::Rw)
        {
            self.vpi_open_delta();
        }
    }

    /// End of the current time slot: leftover cbNBASynch / cbReadWriteSynch,
    /// cbAtEndOfSimTime (Pre-Postponed), then cbReadOnlySynch (Postponed).
    ///
    /// True when a callback ran: the caller must look again before moving
    /// time, since the routine may have created work in this slot or
    /// registered an earlier timer than the one it was about to advance to.
    pub(super) fn vpi_slot_end(&mut self) -> bool {
        if self.vpi_cb_mask & MASK_SLOT_END == 0 {
            return false;
        }
        let mut fired = self.vpi_checkpoint();
        for set in [TimedSet::Nba, TimedSet::Rw, TimedSet::AtEnd] {
            if self.vpi_fire_due(set) {
                fired = true;
                if self.vpi_open_delta() {
                    return true;
                }
            }
        }
        if fired {
            return true;
        }
        let now = self.time;
        if self.vpi_cb.ro.first().is_some_and(|&(t, _)| t <= now) {
            self.vpi_cb.in_ro = true;
            self.vpi_fire_due(TimedSet::Ro);
            self.vpi_cb.in_ro = false;
            return true;
        }
        false
    }

    /// Deferred notifications and the value-change poll. True when any
    /// callback ran.
    pub(super) fn vpi_checkpoint(&mut self) -> bool {
        let mut fired = false;
        if self.vpi_cb_mask & MASK_PENDING != 0 {
            fired |= self.vpi_flush_pending();
        }
        if self.vpi_cb_mask & m(CB_VALUE_CHANGE) != 0 {
            fired |= self.vpi_vc_poll();
        }
        fired
    }

    // ------------------------------------------------------------------
    // cbValueChange
    // ------------------------------------------------------------------

    /// Compare every watched object against the value last reported for it
    /// and report the differences. Catches writes that do not go through
    /// `write_sig!` (continuous assignments, compiled blocks).
    fn vpi_vc_poll(&mut self) -> bool {
        let mut fired = false;
        for id in self.vpi_cb.list(CB_VALUE_CHANGE) {
            let n = match self.vpi_cb.entries.get(&id) {
                Some(VpiCbEntry {
                    target: CbTarget::Watch { sigs, .. },
                    live: true,
                    ..
                }) => sigs.len(),
                _ => continue,
            };
            for k in 0..n {
                fired |= self.vpi_vc_check(id, k);
            }
        }
        fired
    }

    /// Report watched word `k` of callback `id` if it changed.
    fn vpi_vc_check(&mut self, id: u64, k: usize) -> bool {
        let (cur, index, type_code) = {
            let Some(VpiCbEntry {
                target:
                    CbTarget::Watch {
                        sigs,
                        slice,
                        last,
                        type_code,
                    },
                live: true,
                ..
            }) = self.vpi_cb.entries.get_mut(&id)
            else {
                return false;
            };
            let Some(&(sig, index)) = sigs.get(k) else {
                return false;
            };
            let Some(v) = self.signal_table.get(sig) else {
                return false;
            };
            let cur = match slice {
                Some((lsb, w)) => {
                    let mut s = Value::zero(*w);
                    for i in 0..*w as usize {
                        s.set_bit_code(i, v.get_bit_code(*lsb as usize + i));
                    }
                    s
                }
                None => v.clone(),
            };
            if last[k] == cur {
                return false;
            }
            last[k] = cur.clone();
            (cur, index, *type_code)
        };
        vpi_cb_invoke(
            self as *mut Simulator,
            id,
            &Fire {
                obj: std::ptr::null_mut(),
                value: Some(&cur),
                type_code: Some(type_code),
                text: None,
                index,
            },
        )
    }

    // ------------------------------------------------------------------
    // cbForce / cbRelease / cbAssign / cbDeassign / cbDisable
    // ------------------------------------------------------------------

    /// A force or procedural assign of `sig` (None: storage outside the
    /// signal table) has taken effect: cbForce / cbAssign.
    pub(super) fn vpi_notify_override(&mut self, reason: c_int, sig: Option<usize>) {
        if self.vpi_cb_mask & m(reason) == 0 {
            return;
        }
        let sp = self as *mut Simulator;
        let value = sig.and_then(|s| self.signal_table.get(s).cloned());
        let mut temp: *mut c_void = std::ptr::null_mut();
        for id in self.vpi_cb.list(reason) {
            let hit = match self.vpi_cb.entries.get(&id).map(|e| &e.target) {
                Some(CbTarget::Override(None)) => true,
                Some(CbTarget::Override(Some(t))) => Some(*t) == sig,
                _ => false,
            };
            if !hit {
                continue;
            }
            // NULL-registered: the object is the forced one (xezim has no
            // statement objects to hand out instead).
            let registered_null = self.vpi_cb.entries.get(&id).is_some_and(|e| e.obj == 0);
            if registered_null && temp.is_null() {
                if let Some(s) = sig {
                    let name = self.name_for_id(s).to_string();
                    if !name.is_empty() {
                        temp = new_vpi_handle(self, &name, s);
                    }
                }
            }
            let type_code = sig.map(|s| vpi_type_of(self, self.name_for_id(s), s));
            vpi_cb_invoke(
                sp,
                id,
                &Fire {
                    obj: if registered_null {
                        temp
                    } else {
                        std::ptr::null_mut()
                    },
                    value: value.as_ref(),
                    type_code,
                    text: None,
                    index: 0,
                },
            );
        }
        if !temp.is_null() {
            vpi_free_object(temp);
        }
    }

    /// A release or deassign: reported at the next checkpoint, once the
    /// released net has been re-driven.
    pub(super) fn vpi_queue_release(&mut self, reason: c_int, sig: Option<usize>) {
        if self.vpi_cb_mask & m(reason) == 0 {
            return;
        }
        self.vpi_cb.pending.push(Pending::Release { reason, sig });
        self.vpi_cb_sync_mask();
    }

    /// A `disable name` is executing in the current process.
    pub(super) fn vpi_note_disable(&mut self, name: &str) {
        let leaf = scope_leaf(name);
        // Processes the disable terminates, as the Disable arm computes them.
        let mut affected: HashSet<usize> = HashSet::default();
        affected.insert(self.current_pid);
        if let Some(children) = self.fork_block_children.get(name).cloned() {
            for c in children {
                affected.insert(c);
                affected.extend(self.collect_fork_descendants(c));
            }
        }
        if let Some(&pid) = self.disable_labels.get(name) {
            affected.insert(pid);
        }
        if let Some(pids) = self.active_task_pids.get(name) {
            affected.extend(pids.iter().copied());
        }
        let ids: Vec<u64> = self
            .vpi_cb
            .list(CB_DISABLE)
            .into_iter()
            .filter(|id| match self.vpi_cb.entries.get(id).map(|e| &e.target) {
                Some(CbTarget::Disable { pid, names }) => {
                    names.iter().any(|n| *n == leaf) && pid.is_none_or(|p| affected.contains(&p))
                }
                _ => false,
            })
            .collect();
        if !ids.is_empty() {
            self.vpi_cb.pending.push(Pending::Disable { ids });
            self.vpi_cb_sync_mask();
        }
    }

    fn vpi_flush_pending(&mut self) -> bool {
        let pending = std::mem::take(&mut self.vpi_cb.pending);
        self.vpi_cb_sync_mask();
        let sp = self as *mut Simulator;
        let mut fired = false;
        for p in pending {
            match p {
                Pending::Release { reason, sig } => {
                    if self.vpi_cb_mask & m(reason) == 0 {
                        continue;
                    }
                    let value = sig.and_then(|s| self.signal_table.get(s).cloned());
                    let mut temp: *mut c_void = std::ptr::null_mut();
                    for id in self.vpi_cb.list(reason) {
                        let (hit, null_obj) = match self.vpi_cb.entries.get(&id) {
                            Some(e) => match &e.target {
                                CbTarget::Override(None) => (true, e.obj == 0),
                                CbTarget::Override(Some(t)) => (Some(*t) == sig, e.obj == 0),
                                _ => (false, false),
                            },
                            None => (false, false),
                        };
                        if !hit {
                            continue;
                        }
                        if null_obj && temp.is_null() {
                            if let Some(s) = sig {
                                let name = self.name_for_id(s).to_string();
                                if !name.is_empty() {
                                    temp = new_vpi_handle(self, &name, s);
                                }
                            }
                        }
                        let type_code = sig.map(|s| vpi_type_of(self, self.name_for_id(s), s));
                        fired |= vpi_cb_invoke(
                            sp,
                            id,
                            &Fire {
                                obj: if null_obj { temp } else { std::ptr::null_mut() },
                                value: value.as_ref(),
                                type_code,
                                text: None,
                                index: 0,
                            },
                        );
                    }
                    if !temp.is_null() {
                        vpi_free_object(temp);
                    }
                }
                Pending::Disable { ids } => {
                    for id in ids {
                        fired |= vpi_cb_invoke(sp, id, &Fire::NONE);
                    }
                }
            }
        }
        fired
    }

    // ------------------------------------------------------------------
    // cbStmt
    // ------------------------------------------------------------------

    /// Before `stmt` executes (only called while a cbStmt is registered).
    /// `from_stream`: called by the process loop, which runs some statements
    /// itself (timing controls, blocking blocks and branches) and hands the
    /// rest to `exec_statement` — that second sighting is not reported.
    #[cold]
    #[inline(never)]
    pub(super) fn vpi_stmt_hook(&mut self, stmt: &Statement, from_stream: bool) {
        let p = stmt as *const Statement as usize;
        if from_stream {
            self.vpi_cb.stmt_reported = p;
        } else if std::mem::take(&mut self.vpi_cb.stmt_reported) == p {
            return;
        }
        if matches!(
            stmt.kind,
            StatementKind::Null
                | StatementKind::ScopePop
                | StatementKind::LoopStep
                | StatementKind::ForeachTail { .. }
                | StatementKind::ForeverTail { .. }
        ) {
            return;
        }
        let sp = self as *mut Simulator;
        for id in self.vpi_cb.list(CB_STMT) {
            let hit = matches!(
                self.vpi_cb.entries.get(&id).map(|e| &e.target),
                Some(CbTarget::Scope(s)) if *s == self.current_scope
            );
            if hit {
                vpi_cb_invoke(sp, id, &Fire::NONE);
            }
        }
    }

    // ------------------------------------------------------------------
    // Action callbacks
    // ------------------------------------------------------------------

    /// Fire every callback of an action `reason`.
    fn vpi_fire_action(&mut self, reason: c_int, f: &Fire) -> bool {
        if self.vpi_cb_mask & m(reason) == 0 {
            return false;
        }
        let sp = self as *mut Simulator;
        let mut fired = false;
        for id in self.vpi_cb.list(reason) {
            if is_one_shot(reason) {
                if let Some(l) = self.vpi_cb.lists.get_mut(&reason) {
                    l.retain(|&x| x != id);
                }
            }
            fired |= vpi_cb_invoke(sp, id, f);
        }
        fired
    }

    /// cbEndOfCompile then cbStartOfSimulation, at the start of `simulate()`.
    pub(super) fn vpi_start_of_simulation(&mut self) {
        self.vpi_fire_action(CB_END_OF_COMPILE, &Fire::NONE);
        self.vpi_fire_action(CB_START_OF_SIMULATION, &Fire::NONE);
    }

    /// After the event loop: cbEnterInteractive when `$stop` ended the run,
    /// then cbEndOfSimulation.
    pub(super) fn vpi_end_of_simulation(&mut self) {
        if std::mem::take(&mut self.vpi_cb.stop_requested) {
            self.vpi_fire_action(CB_ENTER_INTERACTIVE, &Fire::NONE);
        }
        self.vpi_fire_action(CB_END_OF_SIMULATION, &Fire::NONE);
    }

    /// `$stop` (or `vpi_control(vpiStop)`) executed.
    pub(super) fn vpi_note_stop(&mut self) {
        if self.vpi_cb_mask & m(CB_ENTER_INTERACTIVE) != 0 {
            self.vpi_cb.stop_requested = true;
        }
    }

    /// A run-time error (`$error`, `$fatal`, an illegal bin, a timing
    /// violation): cbError, with `vpi_chk_error` reporting it inside the
    /// routine.
    pub(super) fn vpi_notify_error(&mut self, message: &str) {
        if self.vpi_cb_mask & m(CB_ERROR) == 0 {
            return;
        }
        let saved = VPI_LAST_ERROR.with(|c| c.replace(Some((vpi::ERROR, message.to_string()))));
        self.vpi_fire_action(CB_ERROR, &Fire::NONE);
        VPI_LAST_ERROR.with(|c| *c.borrow_mut() = saved);
    }

    /// A timing-check violation: cbTchkViolation. xezim has no vpiTchk
    /// objects, so `obj` is NULL and `value` carries the violation text.
    pub(super) fn vpi_notify_tchk(&mut self, text: &str) {
        self.vpi_fire_action(
            CB_TCHK_VIOLATION,
            &Fire {
                text: Some(text),
                ..Fire::NONE
            },
        );
    }

    /// SIGINT / SIGTERM taken by the event loop: cbSignal, with the signal
    /// number in `index`.
    pub(super) fn vpi_notify_signal(&mut self) {
        let sig = INTERRUPT_SIGNAL.load(std::sync::atomic::Ordering::Relaxed);
        self.vpi_fire_action(
            CB_SIGNAL,
            &Fire {
                index: sig,
                ..Fire::NONE
            },
        );
    }

    /// An unknown `$name` was reached. The first time for each name,
    /// cbUnresolvedSystf runs with the name in `value->value.str`; it may
    /// register the name with `vpi_register_systf`. True when the name is
    /// registered afterwards, so the caller dispatches it instead of warning.
    pub(super) fn vpi_resolve_unknown_systf(&mut self, name: &str) -> bool {
        if self.vpi_cb_mask & m(CB_UNRESOLVED_SYSTF) == 0
            || name.starts_with("$__")
            || Self::known_system_name(name)
        {
            return false;
        }
        if self.vpi_cb.unresolved_seen.insert(name.to_string()) {
            self.vpi_fire_action(
                CB_UNRESOLVED_SYSTF,
                &Fire {
                    text: Some(name),
                    ..Fire::NONE
                },
            );
        }
        vpi_systf_registered(name)
    }

    /// Writes are refused while cbReadOnlySynch routines run.
    pub(super) fn vpi_in_read_only(&self) -> bool {
        self.vpi_cb.in_ro
    }
}

/// cbPLIError: a VPI routine reported an error (which `vpi_chk_error` returns
/// inside the routine, and still returns to the caller afterwards).
pub(super) fn vpi_notify_pli_error() {
    let sp = active_sim_ptr();
    if sp.is_null() || IN_PLI_ERROR_CB.with(|c| c.get()) {
        return;
    }
    let sim = unsafe { &mut *sp };
    if sim.vpi_cb_mask & m(CB_PLI_ERROR) == 0 {
        return;
    }
    let saved = VPI_LAST_ERROR.with(|c| c.borrow().clone());
    IN_PLI_ERROR_CB.with(|c| c.set(true));
    sim.vpi_fire_action(CB_PLI_ERROR, &Fire::NONE);
    IN_PLI_ERROR_CB.with(|c| c.set(false));
    VPI_LAST_ERROR.with(|c| *c.borrow_mut() = saved);
}

/// `write_sig!` wrote a watched signal: report the change now, so the
/// routine runs in the same region as the write. A write from inside a
/// callback (`vpi_put_value` from a routine) is not reported from within
/// it: the settle after the routine returns compares the watched values
/// and reports the change in the same time slot. Calling a routine while
/// another one runs aborts cocotb 1.x ("calling up again").
#[cold]
#[inline(never)]
pub(super) fn vpi_vc_written(sp: *mut Simulator, sig: usize) {
    if sp.is_null() || CB_DEPTH.with(|d| d.get()) > 0 {
        return;
    }
    let sim = unsafe { &mut *sp };
    let ids = sim
        .dpi_value_change_cbs
        .get(&sig)
        .cloned()
        .unwrap_or_default();
    for id in ids {
        let ks: Vec<usize> = match sim.vpi_cb.entries.get(&id) {
            Some(VpiCbEntry {
                target: CbTarget::Watch { sigs, .. },
                ..
            }) => sigs
                .iter()
                .enumerate()
                .filter(|(_, (s, _))| *s == sig)
                .map(|(k, _)| k)
                .collect(),
            _ => continue,
        };
        for k in ks {
            unsafe { &mut *sp }.vpi_vc_check(id, k);
        }
    }
}

/// The objects a cbValueChange on `h` watches.
fn vpi_watch_target(sim: &Simulator, h: &VpiHandle) -> Result<CbTarget, String> {
    let current = |sig: usize| sim.signal_table.get(sig).cloned();
    match h.kind {
        VpiKind::Signal | VpiKind::Port => {
            let v = current(h.signal_id).ok_or("obj names no signal")?;
            Ok(CbTarget::Watch {
                sigs: vec![(h.signal_id, 0)],
                slice: None,
                last: vec![v],
                type_code: h.type_code,
            })
        }
        VpiKind::Slice => {
            let v = vpi_slice_read(sim, h).ok_or("obj names no signal")?;
            Ok(CbTarget::Watch {
                sigs: vec![(h.signal_id, 0)],
                slice: Some((h.lsb, h.width)),
                last: vec![v],
                type_code: h.type_code,
            })
        }
        VpiKind::Memory => {
            // §38.36.1: a change of any word; `index` names the word.
            let rel = vpi_strip_top(sim, &h.full_name).to_string();
            let (lo, hi, _) = sim
                .module
                .arrays
                .get(rel.as_str())
                .copied()
                .ok_or("memory not found")?;
            let mut sigs = Vec::new();
            let mut last = Vec::new();
            for i in lo.min(hi)..=lo.max(hi) {
                if let Some(&id) = sim
                    .signal_name_to_id
                    .get(format!("{}[{}]", rel, i).as_str())
                {
                    sigs.push((id, i as c_int));
                    last.push(current(id).unwrap_or_else(|| Value::zero(1)));
                }
            }
            if sigs.is_empty() {
                return Err("memory has no words".into());
            }
            Ok(CbTarget::Watch {
                sigs,
                slice: None,
                last,
                type_code: vpi::REG,
            })
        }
        _ => Err("cbValueChange needs a net, variable, part-select, port or memory".into()),
    }
}

/// Register a callback (§38.36). Returns a callback handle, or NULL (with a
/// `vpi_chk_error` diagnostic) when the request cannot be honoured.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_register_cb(cb_p: *mut s_cb_data) -> *mut c_void {
    let fail = |msg: String| -> *mut c_void {
        vpi_error(
            vpi::ERROR,
            format!("vpi_register_cb: {} (not registered)", msg),
        );
        std::ptr::null_mut()
    };
    if cb_p.is_null() {
        return fail("NULL cb_data_p".into());
    }
    let d = unsafe { &*cb_p };
    let reason = d.reason;
    if d.cb_rtn.is_null() {
        return fail(format!("reason {}: cb_rtn is NULL", reason));
    }
    let reg_time = if d.time.is_null() {
        None
    } else {
        Some(unsafe { *d.time })
    };
    let reg_value_format = if d.value.is_null() {
        None
    } else {
        Some(unsafe { (*d.value).format })
    };
    let obj_h = unsafe { vpi_deref(d.obj) };

    let r = try_active_sim("vpi_register_cb", |sim| -> Result<u64, String> {
        let now = sim.time;
        let tpu = ticks_per_unit_of(sim, d.obj);
        let mut e = VpiCbEntry {
            reason,
            rtn: d.cb_rtn as usize,
            user_data: d.user_data as usize,
            obj: d.obj as usize,
            reg_time,
            reg_value_format,
            reg_index: d.index,
            time_type: vpi::SIM_TIME,
            value_format: vpi::SUPPRESS_VAL,
            ticks_per_unit: tpu,
            due: 0,
            target: CbTarget::None,
            live: true,
            one_shot: is_one_shot(reason),
            internal: false,
        };
        // The time format a value / statement callback reports with.
        let event_time_type = || -> Result<c_int, String> {
            match reg_time.map(|t| t.type_) {
                None => Ok(vpi::SIM_TIME),
                Some(t @ (vpi::SIM_TIME | vpi::SCALED_REAL_TIME | vpi::SUPPRESS_TIME)) => Ok(t),
                Some(t) => Err(format!("time->type {} is not a time format", t)),
            }
        };
        match reason {
            CB_VALUE_CHANGE => {
                let h = obj_h.ok_or("cbValueChange needs an obj")?;
                e.target = vpi_watch_target(sim, h)?;
                e.time_type = event_time_type()?;
                // A NULL value struct reads as vpiIntVal (compatibility).
                let fmt = reg_value_format.unwrap_or(vpi::INT_VAL);
                if !value_format_supported(fmt) {
                    return Err(format!("value->format {} is not supported", fmt));
                }
                e.value_format = fmt;
            }
            CB_STMT => {
                let h = obj_h.ok_or("cbStmt needs a scope obj")?;
                if h.kind != VpiKind::Module {
                    return Err(
                        "cbStmt needs a module (scope) obj; xezim has no statement objects".into(),
                    );
                }
                let scope = if h.inst_idx < 0 {
                    String::new()
                } else {
                    sim.module
                        .instances
                        .get(h.inst_idx as usize)
                        .map(|i| i.path.clone())
                        .unwrap_or_default()
                };
                e.target = CbTarget::Scope(scope);
                e.time_type = event_time_type()?;
            }
            CB_FORCE | CB_RELEASE | CB_ASSIGN | CB_DEASSIGN => {
                e.target = match obj_h {
                    None => CbTarget::Override(None),
                    Some(h) if matches!(h.kind, VpiKind::Signal | VpiKind::Port) => {
                        CbTarget::Override(Some(h.signal_id))
                    }
                    Some(_) => return Err("obj must be a net or variable, or NULL".into()),
                };
                e.time_type = event_time_type()?;
                let fmt = reg_value_format.unwrap_or(vpi::SUPPRESS_VAL);
                if !value_format_supported(fmt) {
                    return Err(format!("value->format {} is not supported", fmt));
                }
                e.value_format = fmt;
            }
            CB_DISABLE => {
                let h = obj_h.ok_or("cbDisable needs an obj")?;
                e.target = match h.kind {
                    // The call's enclosing named blocks and subroutines, in
                    // the process that made it.
                    VpiKind::SysTfCall => {
                        // The process's own (flattened) outermost label is
                        // kept apart from `m_scope_stack`.
                        let pid = sim.current_pid;
                        let mut names: Vec<String> =
                            sim.m_scope_stack.iter().map(|s| scope_leaf(s)).collect();
                        if let Some(l) = sim.process_m_label.get(&pid) {
                            names.push(scope_leaf(l));
                        }
                        CbTarget::Disable {
                            pid: Some(pid),
                            names,
                        }
                    }
                    VpiKind::Signal
                    | VpiKind::Port
                    | VpiKind::Slice
                    | VpiKind::Memory
                    | VpiKind::Module
                    | VpiKind::Iterator
                    | VpiKind::Constant
                    | VpiKind::Callback => {
                        return Err(
                            "cbDisable needs a $systf call, named block or subroutine obj".into(),
                        );
                    }
                    #[allow(unreachable_patterns)]
                    _ => CbTarget::Disable {
                        pid: None,
                        names: vec![scope_leaf(&h.full_name)],
                    },
                };
                e.time_type = event_time_type()?;
            }
            CB_AT_START_OF_SIM_TIME | CB_AT_END_OF_SIM_TIME => {
                let t = reg_time.ok_or("a simulation-time callback needs a time")?;
                if t.type_ != vpi::SIM_TIME && t.type_ != vpi::SCALED_REAL_TIME {
                    return Err(format!(
                        "time->type {} is not vpiSimTime/vpiScaledRealTime",
                        t.type_
                    ));
                }
                let due = time_to_ticks(&t, tpu);
                let started = sim.vpi_slot_started == now;
                let too_late = due < now
                    || (due == now
                        && (sim.vpi_cb.in_ro || (reason == CB_AT_START_OF_SIM_TIME && started)));
                if too_late {
                    return Err(format!("time {} has already started (now {})", due, now));
                }
                e.due = due;
                e.time_type = t.type_;
            }
            CB_AFTER_DELAY | CB_READ_WRITE_SYNCH | CB_READ_ONLY_SYNCH | CB_NBA_SYNCH => {
                // Relative to now. A NULL time is a zero delay (lenient).
                let (delay, tt) = match reg_time {
                    None => (0, vpi::SIM_TIME),
                    Some(t) if t.type_ == vpi::SIM_TIME || t.type_ == vpi::SCALED_REAL_TIME => {
                        (time_to_ticks(&t, tpu), t.type_)
                    }
                    Some(t) => {
                        return Err(format!(
                            "time->type {} is not vpiSimTime/vpiScaledRealTime",
                            t.type_
                        ));
                    }
                };
                if delay == 0 && sim.vpi_cb.in_ro && reason != CB_READ_ONLY_SYNCH {
                    return Err("the current time is in its read-only region".into());
                }
                e.due = now.saturating_add(delay);
                e.time_type = tt;
            }
            CB_NEXT_SIM_TIME => {
                // The time value is ignored; its type picks the report format.
                e.due = now;
                e.time_type = match reg_time.map(|t| t.type_) {
                    Some(vpi::SCALED_REAL_TIME) => vpi::SCALED_REAL_TIME,
                    _ => vpi::SIM_TIME,
                };
            }
            CB_END_OF_COMPILE
            | CB_START_OF_SIMULATION
            | CB_END_OF_SIMULATION
            | CB_ERROR
            | CB_PLI_ERROR
            | CB_TCHK_VIOLATION
            | CB_SIGNAL
            | CB_UNRESOLVED_SYSTF
            | CB_ENTER_INTERACTIVE
            | CB_EXIT_INTERACTIVE
            | CB_INTERACTIVE_SCOPE_CHANGE
            | CB_START_OF_RESET
            | CB_END_OF_RESET
            | CB_START_OF_SAVE
            | CB_END_OF_SAVE
            | CB_START_OF_RESTART
            | CB_END_OF_RESTART => {
                e.time_type = match reg_time.map(|t| t.type_) {
                    Some(vpi::SCALED_REAL_TIME) => vpi::SCALED_REAL_TIME,
                    _ => vpi::SIM_TIME,
                };
            }
            other => {
                return Err(format!("reason {} is not supported", other));
            }
        }
        Ok(sim.vpi_cb_insert(e))
    });
    match r {
        Some(Ok(id)) => {
            let mut h = VpiHandle::signal(id as usize, VPI_CALLBACK, "", "");
            h.kind = VpiKind::Callback;
            h.into_raw()
        }
        Some(Err(msg)) => fail(format!("reason {}: {}", reason, msg)),
        None => std::ptr::null_mut(),
    }
}

/// The callback id a `vpi_register_cb` handle carries.
fn cb_id_of(handle: *mut c_void) -> Option<u64> {
    unsafe { vpi_deref(handle) }
        .filter(|h| h.kind == VpiKind::Callback)
        .map(|h| h.signal_id as u64)
}

/// Fill `cb_data_p` with the data `cb_obj` was registered with (§38.5).
/// `time` and `value` point at simulator-owned copies (NULL when none was
/// registered), valid until the next call. Returns 1, or 0 for a handle
/// that is not a registered callback — including a one-shot that has fired.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_get_cb_info(cb_obj: *mut c_void, cb_data_p: *mut s_cb_data) -> c_int {
    if cb_data_p.is_null() {
        vpi_error(vpi::ERROR, "vpi_get_cb_info: NULL cb_data_p".into());
        return 0;
    }
    let Some(id) = cb_id_of(cb_obj) else {
        vpi_error(vpi::ERROR, "vpi_get_cb_info: not a callback handle".into());
        return 0;
    };
    let e = try_active_sim("vpi_get_cb_info", |sim| {
        sim.vpi_cb.entries.get(&id).cloned()
    })
    .flatten();
    let Some(e) = e else {
        vpi_error(
            vpi::ERROR,
            "vpi_get_cb_info: the callback is no longer registered".into(),
        );
        return 0;
    };
    let out = unsafe { &mut *cb_data_p };
    out.reason = e.reason;
    out.cb_rtn = e.rtn as *mut c_void;
    out.obj = e.obj as *mut c_void;
    out.index = e.reg_index;
    out.user_data = e.user_data as *mut c_void;
    out.time = match e.reg_time {
        Some(t) => CB_INFO_TIME.with(|c| {
            *c.borrow_mut() = t;
            c.as_ptr()
        }),
        None => std::ptr::null_mut(),
    };
    out.value = match e.reg_value_format {
        Some(f) => CB_INFO_VALUE.with(|c| {
            let mut v = c.borrow_mut();
            v.format = f;
            v.value = s_vpi_value_union { integer: 0 };
            drop(v);
            c.as_ptr()
        }),
        None => std::ptr::null_mut(),
    };
    1
}

/// Remove a callback and free its handle (§38.35). Works from inside any
/// callback routine, including the callback's own. Removing a one-shot that
/// has already fired just frees the handle. Returns 1, or 0 when `cb_obj` is
/// not a callback handle.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_remove_cb(cb_obj: *mut c_void) -> c_int {
    let Some(id) = cb_id_of(cb_obj) else {
        vpi_error(vpi::ERROR, "vpi_remove_cb: not a callback handle".into());
        return 0;
    };
    let sp = active_sim_ptr();
    if !sp.is_null() {
        let sim = unsafe { &mut *sp };
        if sim.vpi_cb.entries.get(&id).is_some_and(|e| !e.internal) {
            sim.vpi_cb_delete(id);
        }
    }
    vpi_free_object(cb_obj);
    1
}

/// Backend for `vpi_control` (§38.14), which is C-variadic and so lives in
/// `src/vpi_printf_shim.c`. `arg` is the `$stop`/`$finish` diagnostic level,
/// `handle` the vpiSetInteractiveScope scope.
///
/// vpiStop and vpiFinish end the run once the calling routine returns;
/// vpiStop also counts as entering interactive mode (cbEnterInteractive).
/// vpiSetInteractiveScope accepts a module and reports the change through
/// cbInteractiveScopeChange. vpiReset is refused: xezim cannot rewind a
/// simulation.
#[unsafe(no_mangle)]
pub extern "C" fn xezim_vpi_control(operation: c_int, _arg: c_int, handle: *mut c_void) -> c_int {
    match operation {
        vpi::STOP | vpi::FINISH => try_active_sim("vpi_control", |sim| {
            sim.flush_file_writes();
            sim.finished = true;
            if operation == vpi::STOP {
                sim.vpi_note_stop();
            }
            1
        })
        .unwrap_or(0),
        vpi::RESET => {
            vpi_error(
                vpi::ERROR,
                "vpi_control(vpiReset): xezim cannot reset a running simulation".into(),
            );
            0
        }
        SET_INTERACTIVE_SCOPE => {
            let ok = unsafe { vpi_deref(handle) }.is_some_and(|h| h.kind == VpiKind::Module);
            if !ok {
                vpi_error(
                    vpi::ERROR,
                    "vpi_control(vpiSetInteractiveScope): the argument is not a scope handle"
                        .into(),
                );
                return 0;
            }
            try_active_sim("vpi_control", |sim| {
                sim.vpi_fire_action(
                    CB_INTERACTIVE_SCOPE_CHANGE,
                    &Fire {
                        obj: handle,
                        ..Fire::NONE
                    },
                );
                1
            })
            .unwrap_or(0)
        }
        other => {
            vpi_error(
                vpi::ERROR,
                format!("vpi_control: operation {} not supported", other),
            );
            0
        }
    }
}
