//! IEEE 1800-2017 Clause 39 — the assertion API: `vpi_register_assertion_cb`
//! and the assertion operations of `vpi_control`. The assertion objects
//! themselves are part of the object model (`vpi_model.rs`), which makes them
//! from the sources: an assertion has a handle from time 0, before its
//! statement first runs and registers the engine's site.
//!
//! An object names its assertion by an [`AssertRef`]: the scope it is written
//! in, its label and its source position. The engine's site (`sva_sites`) is
//! looked up from that when a routine needs it, never stored in a handle.
//!
//! Callbacks. Every registration is one entry of [`VpiAssertState`], under
//! the object it was made on. The engine reports an attempt's start and end
//! (`assertions.rs`) and a control its effect through `vpi_assert_fire`,
//! all gated on `vpi_assert_on`, so a run without assertion callbacks pays
//! one load and branch per hook. The handle `vpi_register_assertion_cb`
//! returns is a `Callback` handle like `vpi_register_cb`'s, with an id of
//! its own range; `vpi_remove_cb` removes it.
//!
//! Control (Clause 39.5). Disable and Enable, and the system On and Off, are
//! entries of the §20.12 control log (`assert_ctl.rs`), the state `$asserton`
//! and `$assertoff` write: the two front ends cannot disagree. Kill and Reset
//! act on the site: unlike `$assertkill`, the API's Kill leaves the assertion
//! enabled, and the system tasks have no Reset.
//!
//! Limits: the objects are the labelled concurrent assertion items of a
//! scope and its property declarations (`vpiPropertyInst` is reached by
//! iteration only; `vpi_handle_by_name` on the shared name returns the
//! declaration). Control operations apply only to directives, not property
//! declarations or instances, and `vpi_register_assertion_cb` refuses the
//! Phase 2 action and lock reasons. `vpiAssertionEnableStep` / `DisableStep`
//! select the attempt by its start time,
//! `cbAssertionStepSuccess` fires for each tick an attempt remains in flight
//! (including its start tick), and `cbAssertionStepFailure` accompanies each
//! failure. `info` is always non-NULL (`detail` is NULL, and Disable/Enable/
//! Reset report the callback time as `attemptStartTime`). Coverage counts are
//! per source location across instances, include vacuous successes in
//! `vpiAssertSuccessCovered` on asserts/assumes, leave `vpiAssertFailureCovered`
//! 0 on covers, and approximate `vpiAssertAttemptCovered` as successes +
//! failures. Attempt metadata is not checkpointed: after `$restart`, attempts
//! in flight report the restore time as their start.
use super::*;
use libc::{c_char, c_int, c_void};

// Callback reasons (IEEE 1800-2017 Annex M, sv_vpi_user.h).
pub(super) const CB_START: c_int = 606;
pub(super) const CB_SUCCESS: c_int = 607;
pub(super) const CB_FAILURE: c_int = 608;
pub(super) const CB_STEP_SUCCESS: c_int = 609;
pub(super) const CB_STEP_FAILURE: c_int = 610;
pub(super) const CB_DISABLE: c_int = 611;
pub(super) const CB_ENABLE: c_int = 612;
pub(super) const CB_RESET: c_int = 613;
pub(super) const CB_KILL: c_int = 614;
pub(super) const CB_VACUOUS_SUCCESS: c_int = 657;
pub(super) const CB_DISABLED_EVALUATION: c_int = 658;

// `vpi_control` operations (Annex M).
const OP_DISABLE: c_int = 620;
const OP_ENABLE: c_int = 621;
const OP_RESET: c_int = 622;
const OP_KILL: c_int = 623;
const OP_ENABLE_STEP: c_int = 624;
const OP_DISABLE_STEP: c_int = 625;
const CLOCK_STEPS: c_int = 626;
const OP_SYS_ON: c_int = 627;
const OP_SYS_OFF: c_int = 628;
const OP_SYS_END: c_int = 629;
const OP_SYS_RESET: c_int = 630;
const OP_SYS_KILL: c_int = 632;

// Assertion coverage properties (Annex M).
pub(super) const ATTEMPT_COVERED: c_int = 770;
pub(super) const SUCCESS_COVERED: c_int = 771;
pub(super) const FAILURE_COVERED: c_int = 772;

/// Ids of assertion callback handles: apart from `vpi_register_cb`'s.
pub(super) const CB_ID_BASE: u64 = 1 << 62;

/// Nesting limit for routines whose controls fire callbacks.
const MAX_CB_DEPTH: u32 = 1000;

/// What an assertion object is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum AssertRole {
    /// An `assert`, `assume` or `cover` statement.
    Directive,
    /// The property a directive instantiates: it shares the directive's
    /// attempts.
    PropertyInst,
    /// A property declaration, which has no attempts of its own.
    PropertyDecl,
}

/// The assertion an object of the model stands for.
#[derive(Clone, Debug)]
pub(super) struct AssertRef {
    pub role: AssertRole,
    /// VPI full name of the scope the assertion is written in.
    pub scope: String,
    /// The directive's label.
    pub label: Option<String>,
    /// Start of the directive's source span (`SvaClockedSite::span_key`).
    pub span_key: usize,
    /// 0 assert, 1 assume, 2 cover (`SvaClockedSite::kind`).
    pub kind: u8,
}

/// `s_vpi_attempt_info`: `detail` is the union of `failExpr` (a handle) and
/// `step` (a pointer), both pointers.
#[repr(C)]
pub struct s_vpi_attempt_info {
    detail: *mut c_void,
    attempt_start_time: s_vpi_time,
}

type AssertCbFn = extern "C" fn(
    c_int,
    *mut s_vpi_time,
    *mut c_void,
    *mut s_vpi_attempt_info,
    *mut c_char,
) -> c_int;

struct AssertCb {
    id: u64,
    reason: c_int,
    /// The routine and its user data, as addresses.
    rtn: usize,
    user_data: usize,
}

/// One assertion object with callbacks.
struct Target {
    /// The object's index in the model.
    obj: usize,
    aref: AssertRef,
    /// The handle the routines are called with. Owned here: a routine must
    /// not free it.
    handle: Box<VpiHandle>,
    /// The site, once its statement has run.
    site: Option<usize>,
    cbs: Vec<AssertCb>,
}

#[derive(Default)]
pub(super) struct VpiAssertState {
    targets: Vec<Target>,
    /// The targets of each site, by site index.
    by_site: Vec<Vec<usize>>,
    /// `sva_sites.len()` when the targets were last matched with sites.
    sites_seen: usize,
    next_id: u64,
    pub(super) ended: bool,
    depth: u32,
    /// Attempts with step callbacks enabled via `vpiAssertionEnableStep`,
    /// independent of whether any callback is registered yet.
    step_enabled: HashSet<(String, usize, u8, u64)>,
    step_any: bool,
}

fn step_key(r: &AssertRef, start: u64) -> (String, usize, u8, u64) {
    (r.scope.clone(), r.span_key, r.kind, start)
}

fn known_reason(reason: c_int) -> bool {
    matches!(reason, 606..=614 | 657 | 658)
}

fn sim_time(t: u64) -> s_vpi_time {
    s_vpi_time {
        type_: vpi::SIM_TIME,
        high: (t >> 32) as u32,
        low: (t & 0xFFFF_FFFF) as u32,
        real: t as f64,
    }
}

impl Simulator {
    /// The instance-relative path the control log names `r`'s scope by.
    fn vpi_assert_rel_scope(&self, r: &AssertRef) -> String {
        if self.root_is_multi_top() {
            r.scope.clone()
        } else {
            self.instance_relative_scope(&r.scope)
        }
    }

    /// `r`'s identity for assertion control (§20.12).
    fn vpi_assert_ident(&self, r: &AssertRef) -> CtlIdent {
        CtlIdent {
            scope: self.vpi_assert_rel_scope(r),
            label: r.label.clone(),
            atype: assert_ctl::AT_CONCURRENT,
            dtype: [
                assert_ctl::DT_ASSERT,
                assert_ctl::DT_ASSUME,
                assert_ctl::DT_COVER,
            ][r.kind.min(2) as usize],
        }
    }

    /// The engine's site of the assertion `r` names, if its statement has
    /// run. Two instances of a module share the span, so the scope decides;
    /// a label, where both have one, must agree too.
    pub(super) fn vpi_assert_site_of(&self, r: &AssertRef) -> Option<usize> {
        if r.role == AssertRole::PropertyDecl {
            return None;
        }
        let rel = self.vpi_assert_rel_scope(r);
        self.sva_sites.iter().position(|s| {
            s.kind == r.kind
                && s.ctl.ident.scope == rel
                && match (&r.label, &s.ctl.ident.label) {
                    (Some(a), Some(b)) => a == b,
                    (None, None) => s.span_key == r.span_key,
                    _ => false,
                }
        })
    }

    /// Match the targets that have no site yet with the sites registered
    /// since the last call.
    fn vpi_assert_bind(&mut self) {
        let n = self.sva_sites.len();
        if self.vpi_assert.sites_seen == n {
            return;
        }
        for ti in 0..self.vpi_assert.targets.len() {
            if self.vpi_assert.targets[ti].site.is_some() {
                continue;
            }
            let site = self.vpi_assert_site_of(&self.vpi_assert.targets[ti].aref);
            if let Some(i) = site {
                let st = &mut *self.vpi_assert;
                st.targets[ti].site = Some(i);
                if st.by_site.len() <= i {
                    st.by_site.resize_with(i + 1, Vec::new);
                }
                st.by_site[i].push(ti);
            }
        }
        self.vpi_assert.sites_seen = n;
    }

    /// Report `reason` for an attempt of site `i` that started at `start`
    /// (call only while `vpi_assert_on`).
    pub(super) fn vpi_assert_fire(&mut self, i: usize, reason: c_int, start: u64) {
        self.vpi_assert_bind();
        let mut k = 0;
        // By position: a routine may register on another object of the site.
        while let Some(&ti) = self.vpi_assert.by_site.get(i).and_then(|v| v.get(k)) {
            self.vpi_assert_fire_target(ti, reason, start);
            k += 1;
        }
    }

    /// With stepping enabled at site `i`, report each attempt still in
    /// flight after a tick as a successful step (Clause 39.4.2).
    pub(super) fn vpi_assert_fire_steps(&mut self, i: usize) {
        if !self.vpi_assert.step_any {
            return;
        }
        let starts: Vec<u64> = self.sva_sites[i]
            .attempt_meta
            .iter()
            .map(|m| m.start_time)
            .collect();
        for start in starts {
            self.vpi_assert_fire(i, CB_STEP_SUCCESS, start);
        }
    }

    /// Call the routines registered for `reason` on target `ti`.
    fn vpi_assert_fire_target(&mut self, ti: usize, reason: c_int, start: u64) {
        let Some(t) = self.vpi_assert.targets.get(ti) else {
            return;
        };
        match t.aref.role {
            AssertRole::Directive => {}
            // Clause 39.4.2: a property instance reports only these.
            AssertRole::PropertyInst => {
                if !matches!(reason, CB_START | CB_SUCCESS | CB_FAILURE) {
                    return;
                }
            }
            AssertRole::PropertyDecl => return,
        }
        // Step callbacks are off until vpiAssertionEnableStep.
        if matches!(reason, CB_STEP_SUCCESS | CB_STEP_FAILURE)
            && !self.vpi_assert.step_enabled.contains(&step_key(&t.aref, start))
        {
            return;
        }
        // A snapshot: a routine may register or remove callbacks.
        let calls: Vec<(u64, usize, usize)> = t
            .cbs
            .iter()
            .filter(|c| c.reason == reason)
            .map(|c| (c.id, c.rtn, c.user_data))
            .collect();
        if calls.is_empty() {
            return;
        }
        if self.vpi_assert.depth >= MAX_CB_DEPTH {
            return;
        }
        let handle = &*t.handle as *const VpiHandle as *mut c_void;
        // `$display` output made so far comes first.
        self.vpi_sync_output();
        let now = self.time;
        let sp = self as *mut Simulator;
        for (id, rtn, user_data) in calls {
            let sim = unsafe { &mut *sp };
            // Removed by an earlier routine of this report.
            let live = sim
                .vpi_assert
                .targets
                .get(ti)
                .is_some_and(|t| t.cbs.iter().any(|c| c.id == id));
            if !live {
                continue;
            }
            let mut cb_time = sim_time(now);
            let mut info = s_vpi_attempt_info {
                detail: std::ptr::null_mut(),
                attempt_start_time: sim_time(start),
            };
            sim.vpi_assert.depth += 1;
            let prev = ACTIVE_SIMULATOR.with(|c| c.replace(sp));
            let routine: AssertCbFn = unsafe { std::mem::transmute(rtn as *const ()) };
            routine(
                reason,
                &mut cb_time,
                handle,
                &mut info,
                user_data as *mut c_char,
            );
            ACTIVE_SIMULATOR.with(|c| c.set(prev));
            unsafe { &mut *sp }.vpi_assert.depth -= 1;
        }
    }

    fn vpi_assert_target_enabled(&self, t: &Target) -> bool {
        t.aref.role != AssertRole::Directive
            || self
                .assert_ctl_state(&self.vpi_assert_ident(&t.aref))
                .enabled()
    }

    /// Whether each target's assertion is enabled, to find the ones a
    /// control turns on or off (`vpi_assert_ctl_changed`).
    pub(super) fn vpi_assert_ctl_snapshot(&self) -> Vec<bool> {
        self.vpi_assert
            .targets
            .iter()
            .map(|t| self.vpi_assert_target_enabled(t))
            .collect()
    }

    /// cbAssertionDisable / cbAssertionEnable for the assertions whose
    /// enabled state differs from `before`, whichever front end wrote the
    /// control.
    pub(super) fn vpi_assert_ctl_changed(&mut self, before: Vec<bool>) {
        let after = self.vpi_assert_ctl_snapshot();
        let now = self.time;
        for (ti, (was, is)) in before.into_iter().zip(after).enumerate() {
            if was != is {
                // A nested control fired from an earlier target's routine has
                // already reported the newer transition if the current state
                // no longer matches `is`.
                let cur = self
                    .vpi_assert
                    .targets
                    .get(ti)
                    .is_some_and(|t| self.vpi_assert_target_enabled(t));
                if cur != is {
                    continue;
                }
                let reason = if is { CB_ENABLE } else { CB_DISABLE };
                self.vpi_assert_fire_target(ti, reason, now);
            }
        }
    }

    /// The targets that stand for the directive `r` names.
    fn vpi_assert_targets_of(&self, r: &AssertRef) -> Vec<usize> {
        self.vpi_assert
            .targets
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                t.aref.role != AssertRole::PropertyDecl
                    && t.aref.scope == r.scope
                    && t.aref.label == r.label
                    && t.aref.span_key == r.span_key
                    && t.aref.kind == r.kind
            })
            .map(|(ti, _)| ti)
            .collect()
    }

    /// A locked assertion ignores every control but Unlock (§20.12).
    fn vpi_assert_locked(&self, r: &AssertRef) -> bool {
        self.assert_ctl_state(&self.vpi_assert_ident(r)).locked()
    }

    /// Clause 39.5 vpiAssertionKill: discard the attempts of site `i` in
    /// flight (those started at `at`, when given), each a cbAssertionKill.
    /// The assertion stays enabled and keeps its sampled-value history.
    pub(super) fn vpi_assert_kill_site(&mut self, i: usize, at: Option<u64>) {
        let attempts = std::mem::take(&mut self.sva_sites[i].attempts);
        let metas = self.sva_take_meta(i, attempts.len());
        let mut killed: Vec<u64> = Vec::new();
        let mut live = Vec::with_capacity(attempts.len());
        let mut live_meta = Vec::with_capacity(attempts.len());
        for (st, meta) in attempts.into_iter().zip(metas) {
            if at.is_none_or(|t| t == meta.start_time) {
                killed.push(meta.start_time);
            } else {
                live.push(st);
                live_meta.push(meta);
            }
        }
        self.sva_sites[i].attempts = live;
        self.sva_sites[i].attempt_meta = live_meta;
        if at.is_none() {
            // The instances waiting for their first tick go too.
            self.sva_sites[i].matured.clear();
            self.sva_proc_pending.retain(|p| p.0 != i);
        }
        if self.vpi_assert_on {
            for start in killed {
                self.vpi_assert_fire(i, CB_KILL, start);
            }
        }
    }

    /// Clause 39.5 vpiAssertionReset: discard the attempts in flight and
    /// the sampled-value history; the enabled state and the tallies stay.
    fn vpi_assert_reset_site(&mut self, i: usize) {
        let s = &mut self.sva_sites[i];
        s.attempts.clear();
        s.attempt_meta.clear();
        s.settled.clear();
        s.settled_meta.clear();
        s.matured.clear();
        s.past_snapshots.clear();
        s.fired = false;
        self.sva_proc_pending.retain(|p| p.0 != i);
    }

    /// vpiAssertSuccessCovered / vpiAssertFailureCovered /
    /// vpiAssertAttemptCovered of the assertion `r` names: the tallies of
    /// its site (0 before its statement has run).
    pub(super) fn vpi_assert_covered(&self, r: &AssertRef, prop: c_int) -> c_int {
        if r.role == AssertRole::PropertyDecl {
            return vpi::UNDEFINED;
        }
        let stat = self
            .vpi_assert_site_of(r)
            .and_then(|i| self.assertion_stats.get(&self.sva_sites[i].stat_key));
        let (pass, fail) = stat.map_or((0, 0), |s| (s.pass_count, s.fail_count));
        let n = match prop {
            SUCCESS_COVERED => pass,
            FAILURE_COVERED => fail,
            _ => pass + fail,
        };
        n.min(c_int::MAX as _) as c_int
    }

    /// Remove the assertion callback `id`; false when it is not registered.
    pub(super) fn vpi_assert_remove_cb(&mut self, id: u64) -> bool {
        let st = &mut *self.vpi_assert;
        let mut found = false;
        for t in &mut st.targets {
            let n = t.cbs.len();
            t.cbs.retain(|c| c.id != id);
            found |= t.cbs.len() != n;
        }
        self.vpi_assert_on = st.targets.iter().any(|t| !t.cbs.is_empty());
        found
    }

    fn vpi_assert_control(
        &mut self,
        op: c_int,
        aref: Option<AssertRef>,
        at: Option<u64>,
    ) -> Result<(), String> {
        if self.vpi_assert.ended {
            return Err("the assertion system has ended".into());
        }
        let now = self.time;
        match op {
            OP_SYS_ON | OP_SYS_OFF => {
                let ctl = if op == OP_SYS_ON {
                    assert_ctl::CTL_ON
                } else {
                    assert_ctl::CTL_OFF
                };
                // The assertion types `$asserton` / `$assertoff` select. The
                // log push reports the Enable / Disable transitions.
                self.assert_ctl_vpi(ctl, None, 15, 7);
                return Ok(());
            }
            OP_SYS_RESET => {
                for i in 0..self.sva_sites.len() {
                    if self.assert_ctl_on && self.sva_site_ctl(i).locked() {
                        continue;
                    }
                    self.vpi_assert_reset_site(i);
                }
                if self.vpi_assert_on {
                    for ti in 0..self.vpi_assert.targets.len() {
                        let fire = self.vpi_assert.targets.get(ti).is_some_and(|t| {
                            t.aref.role == AssertRole::Directive
                                && (!self.assert_ctl_on || !self.vpi_assert_locked(&t.aref))
                        });
                        if fire {
                            self.vpi_assert_fire_target(ti, CB_RESET, now);
                        }
                    }
                }
                // Clause 39.5: the step callbacks are removed.
                self.vpi_assert.step_enabled.clear();
                self.vpi_assert.step_any = false;
                return Ok(());
            }
            // §39.5.1: kill pending attempts and disable new attempts,
            // retaining callbacks so SysOn can resume their reports.
            OP_SYS_KILL => {
                self.assert_ctl_vpi(assert_ctl::CTL_KILL, None, 15, 7);
                return Ok(());
            }
            OP_SYS_END => {
                // §39.5.1: ending the system discards every pending attempt.
                self.assert_ctl_vpi(assert_ctl::CTL_UNLOCK, None, 15, 7);
                for i in 0..self.sva_sites.len() {
                    if self.assert_ctl_on && self.sva_site_ctl(i).locked() {
                        continue;
                    }
                    self.vpi_assert_kill_site(i, None);
                }
                self.assert_ctl_vpi(assert_ctl::CTL_OFF, None, 15, 7);
                // Clause 39.5: all the callbacks are removed.
                for t in &mut self.vpi_assert.targets {
                    t.cbs.clear();
                }
                self.vpi_assert.step_enabled.clear();
                self.vpi_assert.step_any = false;
                self.vpi_assert_on = false;
                self.vpi_assert.ended = true;
                return Ok(());
            }
            _ => {}
        }
        let Some(r) = aref else {
            return Err("the argument is not an assertion handle".into());
        };
        match r.role {
            AssertRole::Directive => {}
            AssertRole::PropertyInst => {
                return Err("a property instance cannot be controlled".into());
            }
            AssertRole::PropertyDecl => {
                return Err("a property declaration cannot be controlled".into());
            }
        }
        match op {
            OP_DISABLE | OP_ENABLE => {
                let Some(label) = &r.label else {
                    return Err("the assertion has no label".into());
                };
                let scope = self.vpi_assert_rel_scope(&r);
                let target = if scope.is_empty() {
                    label.clone()
                } else {
                    format!("{scope}.{label}")
                };
                let ident = self.vpi_assert_ident(&r);
                let ctl = if op == OP_ENABLE {
                    assert_ctl::CTL_ON
                } else {
                    assert_ctl::CTL_OFF
                };
                // The log push reports the Enable / Disable transition.
                self.assert_ctl_vpi(ctl, Some(target), ident.atype, ident.dtype);
            }
            OP_KILL => {
                if self.vpi_assert_locked(&r) {
                    return Ok(());
                }
                // No site: the statement has not run, nothing is in flight.
                if let Some(i) = self.vpi_assert_site_of(&r) {
                    self.vpi_assert_kill_site(i, at);
                }
            }
            OP_RESET => {
                if self.vpi_assert_locked(&r) {
                    return Ok(());
                }
                if let Some(i) = self.vpi_assert_site_of(&r) {
                    self.vpi_assert_reset_site(i);
                }
                if self.vpi_assert_on {
                    for ti in self.vpi_assert_targets_of(&r) {
                        self.vpi_assert_fire_target(ti, CB_RESET, now);
                    }
                }
            }
            OP_ENABLE_STEP | OP_DISABLE_STEP => {
                let on = op == OP_ENABLE_STEP;
                let start = at.ok_or("step control requires an attempt start time")?;
                let key = step_key(&r, start);
                if on {
                    self.vpi_assert.step_enabled.insert(key);
                } else {
                    self.vpi_assert.step_enabled.remove(&key);
                }
                self.vpi_assert.step_any = !self.vpi_assert.step_enabled.is_empty();
            }
            other => return Err(format!("operation {other} not supported")),
        }
        Ok(())
    }
}

/// Register an assertion callback (Clause 39.4.2). Returns a callback
/// handle `vpi_remove_cb` accepts, or NULL (with a `vpi_chk_error`
/// diagnostic) when the request cannot be honoured.
///
/// A property instance reports the start, success and failure of its
/// directive's attempts; a property declaration accepts the registration and
/// reports nothing, having no attempts of its own.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_register_assertion_cb(
    assertion: *mut c_void,
    reason: c_int,
    cb_rtn: *mut c_void,
    user_data: *mut c_char,
) -> *mut c_void {
    let fail = |msg: &str| -> *mut c_void {
        vpi_error(
            vpi::ERROR,
            format!("vpi_register_assertion_cb: reason {reason}: {msg} (not registered)"),
        );
        std::ptr::null_mut()
    };
    if cb_rtn.is_null() {
        return fail("cb_rtn is NULL");
    }
    if !known_reason(reason) {
        return fail("not an assertion callback reason");
    }
    let Some(h) = (unsafe { vpi_deref(assertion) }) else {
        return fail("NULL assertion handle");
    };
    let id = try_active_sim("vpi_register_assertion_cb", |sim| {
        let (obj, aref) = vpi_model::assert_ref(sim, h)?;
        let st = &mut *sim.vpi_assert;
        if st.ended {
            return None;
        }
        let ti = match st.targets.iter().position(|t| t.obj == obj) {
            Some(ti) => ti,
            None => {
                st.targets.push(Target {
                    obj,
                    aref,
                    handle: Box::new(h.clone()),
                    site: None,
                    cbs: Vec::new(),
                });
                // Match it with its site at the next report.
                st.sites_seen = usize::MAX;
                st.targets.len() - 1
            }
        };
        let id = CB_ID_BASE | st.next_id;
        st.next_id += 1;
        st.targets[ti].cbs.push(AssertCb {
            id,
            reason,
            rtn: cb_rtn as usize,
            user_data: user_data as usize,
        });
        sim.vpi_assert_on = true;
        Some(id)
    });
    match id {
        Some(Some(id)) => {
            let mut h = VpiHandle::signal(id as usize, vpi_cb::VPI_CALLBACK, "", "");
            h.kind = VpiKind::Callback;
            h.into_raw()
        }
        Some(None) => fail("the handle is not an assertion"),
        None => std::ptr::null_mut(),
    }
}

/// Backend for the assertion operations of `vpi_control` (Clause 39.5),
/// which is C-variadic and so lives in `src/vpi_printf_shim.c`. `handle` is
/// the assertion (NULL for the system operations), `time_p` the attempt
/// start time of Kill and the step operations, `step` the step constant.
///
/// A client may pass a time it never set up, so `time_p` is trusted only
/// as far as its `type` field: anything but vpiSimTime or vpiScaledRealTime
/// there means "no time" (every attempt, for Kill).
#[unsafe(no_mangle)]
pub extern "C" fn xezim_vpi_control_assertion(
    op: c_int,
    handle: *mut c_void,
    time_p: *mut s_vpi_time,
    step: c_int,
) -> c_int {
    let fail = |msg: String| -> c_int {
        vpi_error(vpi::ERROR, format!("vpi_control({op}): {msg}"));
        0
    };
    if op == OP_ENABLE_STEP && step != CLOCK_STEPS {
        return fail(format!(
            "step control {step} not supported (only vpiAssertionClockSteps)"
        ));
    }
    let time = if matches!(op, OP_KILL | OP_ENABLE_STEP | OP_DISABLE_STEP) && !time_p.is_null() {
        let ty = unsafe { std::ptr::read_unaligned(time_p as *const c_int) };
        if ty == vpi::SIM_TIME || ty == vpi::SCALED_REAL_TIME {
            Some(unsafe { std::ptr::read_unaligned(time_p) })
        } else {
            None
        }
    } else {
        None
    };
    let sys = matches!(op, OP_SYS_ON | OP_SYS_OFF | OP_SYS_END | OP_SYS_RESET | OP_SYS_KILL);
    let r = try_active_sim("vpi_control", |sim| {
        let (aref, def_name) = if sys {
            (None, None)
        } else {
            match unsafe { vpi_deref(handle) }.and_then(|h| vpi_model::assert_ref(sim, h)) {
                Some((obj, r)) => {
                    let def = if time
                        .as_ref()
                        .is_some_and(|t| t.type_ == vpi::SCALED_REAL_TIME)
                    {
                        vpi_model::obj_instance_def(sim, obj)
                    } else {
                        None
                    };
                    (Some(r), def)
                }
                None => (None, None),
            }
        };
        let ticks_per_unit = match def_name {
            Some(d) if sim.module.tick_s > 0.0 => {
                let (unit, _) = sim.reported_timescale_exp(&d);
                (10f64.powi(unit) / sim.module.tick_s).max(1.0)
            }
            _ => 1.0,
        };
        let at = time.map(|t| vpi_cb::time_to_ticks(&t, ticks_per_unit));
        sim.vpi_assert_control(op, aref, at)
    });
    match r {
        Some(Ok(())) => 1,
        Some(Err(msg)) => fail(msg),
        None => 0,
    }
}
