//! IEEE 1800-2023 §20.11/§20.12 assertion control: `$asserton`,
//! `$assertoff`, `$assertkill`, the action-control tasks and
//! `$assertcontrol`.
//!
//! Every control call appends one entry to a log; the control state of an
//! assertion is the log replayed over it (an assertion that has not run yet
//! still has a state, because it existed from time 0). Concurrent sites
//! cache their replayed state against the log's generation, and immediate
//! and deferred assertions replay the (short, compacted) log when they run.
//! Until the first control call `assert_ctl_on` is false and every
//! assertion path pays one flag test.
//!
//! What a state controls:
//! * On/Off/Kill: whether a new attempt starts. Off leaves attempts in
//!   flight and queued deferred reports alone; Kill also aborts the attempts
//!   in flight, the matured procedural instances and (as §20.12 specifies)
//!   the deferred reports still queued.
//! * PassOn/PassOff/NonvacuousOn/VacuousOff and FailOn/FailOff: whether the
//!   pass or fail action block (and the default `$error`) runs, decided when
//!   the attempt finishes or the deferred report matures.
//! * Lock/Unlock: a locked assertion ignores every control except Unlock.
//!
//! Pinned against the reference simulator: the pass action of a vacuous
//! success is off until PassOn (§20.12 lists the controls, the default is
//! the reference's); levels count module instances only, so a generate or
//! named block is part of its instance's level; levels with no list count
//! from the top modules; `expect` statements ignore the controls, as in the
//! reference. Unique/priority violation reports follow On/Off/Kill.
use super::*;

/// `control_type` values (Table 20-5).
pub(super) const CTL_LOCK: u8 = 1;
pub(super) const CTL_UNLOCK: u8 = 2;
pub(super) const CTL_ON: u8 = 3;
pub(super) const CTL_OFF: u8 = 4;
pub(super) const CTL_KILL: u8 = 5;
pub(super) const CTL_PASS_ON: u8 = 6;
pub(super) const CTL_PASS_OFF: u8 = 7;
pub(super) const CTL_FAIL_ON: u8 = 8;
pub(super) const CTL_FAIL_OFF: u8 = 9;
pub(super) const CTL_NONVACUOUS_ON: u8 = 10;
pub(super) const CTL_VACUOUS_OFF: u8 = 11;

/// `assertion_type` bits (Table 20-6).
pub(super) const AT_CONCURRENT: u8 = 1;
pub(super) const AT_SIMPLE_IMMEDIATE: u8 = 2;
pub(super) const AT_OBSERVED_DEFERRED: u8 = 4;
pub(super) const AT_FINAL_DEFERRED: u8 = 8;
pub(super) const AT_UNIQUE: u8 = 32;
pub(super) const AT_UNIQUE0: u8 = 64;
pub(super) const AT_PRIORITY: u8 = 128;

/// `directive_type` bits (Table 20-7).
pub(super) const DT_ASSERT: u8 = 1;
pub(super) const DT_COVER: u8 = 2;
pub(super) const DT_ASSUME: u8 = 4;

/// One assertion's control state; 0 is the default (enabled, unlocked,
/// pass action on nonvacuous success only, fail action on).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub(super) struct CtlState(u8);

impl CtlState {
    const LOCKED: u8 = 1;
    const OFF: u8 = 2;
    const PASS_NONVAC_OFF: u8 = 4;
    const PASS_VAC_ON: u8 = 8;
    const FAIL_OFF: u8 = 16;

    #[inline]
    pub(super) fn enabled(self) -> bool {
        self.0 & Self::OFF == 0
    }
    #[inline]
    pub(super) fn pass_action(self, vacuous: bool) -> bool {
        if vacuous {
            self.0 & Self::PASS_VAC_ON != 0
        } else {
            self.0 & Self::PASS_NONVAC_OFF == 0
        }
    }
    #[inline]
    pub(super) fn fail_action(self) -> bool {
        self.0 & Self::FAIL_OFF == 0
    }
    fn locked(self) -> bool {
        self.0 & Self::LOCKED != 0
    }
    fn set(&mut self, bit: u8, on: bool) {
        if on {
            self.0 |= bit;
        } else {
            self.0 &= !bit;
        }
    }
    /// Apply control `ctl` to an assertion it selects.
    fn apply(&mut self, ctl: u8) {
        if ctl == CTL_UNLOCK {
            self.set(Self::LOCKED, false);
            return;
        }
        if self.locked() {
            return;
        }
        match ctl {
            CTL_LOCK => self.set(Self::LOCKED, true),
            CTL_ON => self.set(Self::OFF, false),
            CTL_OFF | CTL_KILL => self.set(Self::OFF, true),
            CTL_PASS_ON => {
                self.set(Self::PASS_NONVAC_OFF, false);
                self.set(Self::PASS_VAC_ON, true);
            }
            CTL_PASS_OFF => {
                self.set(Self::PASS_NONVAC_OFF, true);
                self.set(Self::PASS_VAC_ON, false);
            }
            CTL_FAIL_ON => self.set(Self::FAIL_OFF, false),
            CTL_FAIL_OFF => self.set(Self::FAIL_OFF, true),
            CTL_NONVACUOUS_ON => self.set(Self::PASS_NONVAC_OFF, false),
            CTL_VACUOUS_OFF => self.set(Self::PASS_VAC_ON, false),
            _ => {}
        }
    }
}

/// What a control call selects an assertion by.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub(super) struct CtlIdent {
    /// Instance-relative path of the scope the assertion sits in: instance,
    /// generate and named blocks (a labelled procedural assertion's label is
    /// a block of its own).
    pub(super) scope: String,
    /// A module-item assertion's label (`ap: assert property ...`).
    pub(super) label: Option<String>,
    pub(super) atype: u8,
    pub(super) dtype: u8,
}

/// A concurrent site's identity plus its cached state.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct CtlSite {
    pub(super) ident: CtlIdent,
    epoch: u64,
    state: CtlState,
}

impl CtlSite {
    pub(super) fn new(ident: CtlIdent) -> Self {
        CtlSite {
            ident,
            epoch: u64::MAX,
            state: CtlState::default(),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct CtlEntry {
    ctl: u8,
    atype: u8,
    dtype: u8,
    levels: u32,
    /// Instance-relative scope or assertion paths; `None` = the design.
    targets: Option<Vec<String>>,
}

/// The state bits a control writes.
fn ctl_dims(ctl: u8) -> u8 {
    match ctl {
        CTL_ON | CTL_OFF | CTL_KILL => CtlState::OFF,
        CTL_PASS_ON | CTL_PASS_OFF => CtlState::PASS_NONVAC_OFF | CtlState::PASS_VAC_ON,
        CTL_NONVACUOUS_ON => CtlState::PASS_NONVAC_OFF,
        CTL_VACUOUS_OFF => CtlState::PASS_VAC_ON,
        CTL_FAIL_ON | CTL_FAIL_OFF => CtlState::FAIL_OFF,
        _ => CtlState::LOCKED,
    }
}

/// A control that writes the default value of every bit it touches.
fn ctl_writes_default(ctl: u8) -> bool {
    matches!(
        ctl,
        CTL_ON | CTL_FAIL_ON | CTL_NONVACUOUS_ON | CTL_VACUOUS_OFF | CTL_UNLOCK
    )
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub(super) struct AssertCtl {
    log: Vec<CtlEntry>,
    /// Bumped on every change of `log`; concurrent sites cache against it.
    epoch: u64,
    /// Instance-relative paths of every module and interface instance.
    insts: HashSet<String>,
    multi_top: bool,
}

/// Is `t` the path of the assertion labelled `label` in `scope`?
pub(super) fn names(t: &str, scope: &str, label: &str) -> bool {
    if scope.is_empty() {
        return t == label;
    }
    t.len() == scope.len() + 1 + label.len()
        && t.starts_with(scope)
        && t.as_bytes()[scope.len()] == b'.'
        && t.ends_with(label)
}

fn under(scope: &str, target: &str) -> Option<usize> {
    if target.is_empty() {
        return Some(0);
    }
    if scope == target {
        return Some(target.len());
    }
    (scope.len() > target.len()
        && scope.as_bytes()[target.len()] == b'.'
        && scope.starts_with(target))
    .then_some(target.len() + 1)
}

impl AssertCtl {
    /// The instance level of `scope` counted from `base` (`base` itself is
    /// level 1; generate and named blocks do not add a level).
    fn depth(&self, base: &str, scope: &str, from: usize) -> u32 {
        let mut d = if base.is_empty() && self.multi_top {
            0
        } else {
            1
        };
        let rest = &scope[from.min(scope.len())..];
        if rest.is_empty() {
            return d;
        }
        let mut end = from;
        for seg in rest.split('.') {
            end += seg.len();
            if self.insts.contains(&scope[..end]) {
                d += 1;
            }
            end += 1;
        }
        d
    }

    fn selects(&self, e: &CtlEntry, id: &CtlIdent) -> bool {
        if e.atype & id.atype == 0 || e.dtype & id.dtype == 0 {
            return false;
        }
        let in_scope = |t: &str| -> bool {
            if id.label.as_deref().is_some_and(|l| names(t, &id.scope, l)) {
                return true;
            }
            match under(&id.scope, t) {
                Some(from) => e.levels == 0 || self.depth(t, &id.scope, from) <= e.levels,
                None => false,
            }
        };
        match &e.targets {
            None => in_scope(""),
            Some(ts) => ts.iter().any(|t| in_scope(t)),
        }
    }

    fn state_of(&self, id: &CtlIdent, upto: usize) -> CtlState {
        let mut st = CtlState::default();
        for e in &self.log[..upto] {
            if self.selects(e, id) {
                st.apply(e.ctl);
            }
        }
        st
    }

    /// Append `e`, dropping the entries it makes dead. Without a Lock in the
    /// log, each state bit is the last write that selects the assertion, so
    /// an earlier entry whose selection and bits a later one covers is dead,
    /// and a default-writing entry with no live earlier writer of its bits
    /// changes nothing.
    fn push(&mut self, e: CtlEntry) {
        self.epoch += 1;
        let has_lock = self.log.iter().any(|x| x.ctl == CTL_LOCK) || e.ctl == CTL_LOCK;
        self.log.push(e);
        if has_lock {
            return;
        }
        let new = self.log.last().unwrap().clone();
        let covers = |old: &CtlEntry| -> bool {
            if old.atype & !new.atype != 0 || old.dtype & !new.dtype != 0 {
                return false;
            }
            if ctl_dims(old.ctl) & !ctl_dims(new.ctl) != 0 {
                return false;
            }
            match (&new.targets, &old.targets) {
                (None, None) => new.levels == 0 || (old.levels != 0 && old.levels <= new.levels),
                (None, Some(_)) => new.levels == 0,
                (Some(a), Some(b)) => {
                    a == b && (new.levels == 0 || (old.levels != 0 && old.levels <= new.levels))
                }
                (Some(_), None) => false,
            }
        };
        let n = self.log.len() - 1;
        let mut keep: Vec<CtlEntry> = Vec::with_capacity(self.log.len());
        for old in self.log.drain(..n) {
            if old.ctl != CTL_UNLOCK && !covers(&old) {
                keep.push(old);
            }
        }
        keep.push(new);
        // Default writes with no earlier writer of their bits.
        let mut written = 0u8;
        keep.retain(|x| {
            let dims = ctl_dims(x.ctl);
            if ctl_writes_default(x.ctl) && written & dims == 0 {
                return false;
            }
            written |= dims;
            true
        });
        self.log = keep;
    }
}

impl Simulator {
    /// §20.11/§20.12: one assertion control system task call.
    pub(super) fn exec_assert_control(&mut self, name: &str, args: &[Expression]) {
        let is_empty = |a: &Expression| matches!(a.kind, ExprKind::Empty);
        let int_arg = |sim: &mut Self, a: Option<&Expression>| -> Option<i64> {
            let a = a.filter(|a| !is_empty(a))?;
            sim.eval_expr(a).to_i64()
        };
        let (ctl, atype, dtype, levels, list_from): (i64, i64, i64, Option<i64>, usize) = match name
        {
            "$assertcontrol" => {
                let Some(ctl) = int_arg(self, args.first()) else {
                    self.warn_system_task_once(
                        "$assertcontrol#ctl",
                        "Warning: $assertcontrol ignored — control_type is missing or unknown",
                    );
                    return;
                };
                (
                    ctl,
                    int_arg(self, args.get(1)).unwrap_or(255),
                    int_arg(self, args.get(2)).unwrap_or(7),
                    int_arg(self, args.get(3)),
                    4,
                )
            }
            _ => {
                let (ctl, atype) = match name {
                    "$asserton" => (CTL_ON, 15),
                    "$assertoff" => (CTL_OFF, 15),
                    "$assertkill" => (CTL_KILL, 15),
                    "$assertpasson" => (CTL_PASS_ON, 31),
                    "$assertpassoff" => (CTL_PASS_OFF, 31),
                    "$assertfailon" => (CTL_FAIL_ON, 31),
                    "$assertfailoff" => (CTL_FAIL_OFF, 31),
                    "$assertnonvacuouson" => (CTL_NONVACUOUS_ON, 31),
                    _ => (CTL_VACUOUS_OFF, 31),
                };
                (ctl as i64, atype, 7, int_arg(self, args.first()), 1)
            }
        };
        let bad = |what: &str, v: i64, range: &str| {
            format!(
                "Warning: {} ignored — {} {} is out of range (expected {})",
                name, what, v, range
            )
        };
        if !(1..=11).contains(&ctl) {
            let msg = bad("control_type", ctl, "1 to 11");
            self.warn_system_task_once(&format!("{name}#ctl{ctl}"), &msg);
            return;
        }
        if !(1..=255).contains(&atype) {
            let msg = bad("assertion_type", atype, "1 to 255");
            self.warn_system_task_once(&format!("{name}#at{atype}"), &msg);
            return;
        }
        if !(1..=7).contains(&dtype) {
            let msg = bad("directive_type", dtype, "1 to 7");
            self.warn_system_task_once(&format!("{name}#dt{dtype}"), &msg);
            return;
        }
        let levels = levels.unwrap_or(0).max(0) as u32;
        let mut targets: Vec<String> = Vec::new();
        for a in args.iter().skip(list_from) {
            if is_empty(a) {
                continue;
            }
            match self.assert_ctl_target(a) {
                Some(t) => targets.push(t),
                None => self.warn_system_task_once(
                    &format!("{name}#list"),
                    &format!(
                        "Warning: {} — a list argument that is not a scope or assertion name is ignored",
                        name
                    ),
                ),
            }
        }
        // A list whose items were all unusable selects nothing.
        if targets.is_empty() && args.iter().skip(list_from).any(|a| !is_empty(a)) {
            return;
        }
        let entry = CtlEntry {
            ctl: ctl as u8,
            atype: atype as u8,
            dtype: dtype as u8,
            levels,
            targets: (!targets.is_empty()).then_some(targets),
        };
        if self.assert_ctl.is_none() {
            let mut c = AssertCtl {
                multi_top: self.root_is_multi_top(),
                ..Default::default()
            };
            for inst in &self.module.instances {
                c.insts.insert(inst.path.clone());
            }
            self.assert_ctl = Some(Box::new(c));
        }
        if entry.ctl == CTL_KILL {
            self.assert_ctl_kill(&entry);
        }
        let c = self.assert_ctl.as_mut().unwrap();
        c.push(entry);
        self.assert_ctl_on = !c.log.is_empty();
    }

    /// The instance-relative path a list argument names: a scope (instance,
    /// generate or named block) or an assertion, resolved upward from the
    /// calling scope (§23.8) or from the top.
    fn assert_ctl_target(&mut self, a: &Expression) -> Option<String> {
        let ExprKind::Ident(h) = &a.kind else {
            return None;
        };
        let mut segs: Vec<String> = Vec::with_capacity(h.path.len());
        for s in &h.path {
            let mut seg = s.name.name.clone();
            for sel in &s.selects {
                let v = self.eval_expr(sel).to_i64()?;
                seg.push_str(&format!("[{v}]"));
            }
            segs.push(seg);
        }
        let path = segs.join(".");
        if path.is_empty() {
            return None;
        }
        let known = |p: &str| -> bool {
            self.module.instances.iter().any(|i| i.path == p)
                || self.sva_sites.iter().any(|s| {
                    let id = &s.ctl.ident;
                    id.scope == p
                        || id.scope.starts_with(&format!("{p}."))
                        || id.label.as_deref().is_some_and(|l| names(p, &id.scope, l))
                })
                || self.signal_name_to_id.contains_key(p)
        };
        // Rooted at the top module.
        if !self.root_is_multi_top() {
            let top = self.module.name.as_str();
            if path == top {
                return Some(String::new());
            }
            if let Some(rest) = path.strip_prefix(top).and_then(|r| r.strip_prefix('.')) {
                if known(rest) || !known(&path) {
                    return Some(rest.to_string());
                }
            }
        } else if self.module.instances.iter().any(|i| i.path == segs[0]) {
            return Some(path);
        }
        // Upward from the calling scope.
        let here = self.instance_relative_scope(&self.m_path());
        let mut base = here.as_str();
        let mut first: Option<String> = None;
        loop {
            let cand = if base.is_empty() {
                path.clone()
            } else {
                format!("{base}.{path}")
            };
            if known(&cand) {
                return Some(cand);
            }
            first.get_or_insert(cand);
            if base.is_empty() {
                break;
            }
            base = base.rfind('.').map(|i| &base[..i]).unwrap_or("");
        }
        first
    }

    /// §20.12 Kill: abort the attempts in flight of every selected, unlocked
    /// assertion, with its matured procedural instances and queued deferred
    /// reports.
    fn assert_ctl_kill(&mut self, e: &CtlEntry) {
        let c = self.assert_ctl.as_ref().unwrap();
        let upto = c.log.len();
        let mut killed: Vec<usize> = Vec::new();
        for (i, s) in self.sva_sites.iter().enumerate() {
            if c.selects(e, &s.ctl.ident) && !c.state_of(&s.ctl.ident, upto).locked() {
                killed.push(i);
            }
        }
        let pending = std::mem::take(&mut self.deferred_asserts);
        let mut kept = Vec::with_capacity(pending.len());
        for d in pending {
            let dead = c.selects(e, &d.ctl) && !c.state_of(&d.ctl, upto).locked();
            if !dead {
                kept.push(d);
            }
        }
        self.deferred_asserts = kept;
        for i in killed {
            self.sva_sites[i].attempts.clear();
            self.sva_sites[i].matured.clear();
            self.sva_proc_pending.retain(|p| p.0 != i);
        }
    }

    /// The control state of concurrent site `i` (call only while
    /// `assert_ctl_on`).
    pub(super) fn sva_site_ctl(&mut self, i: usize) -> CtlState {
        let c = self.assert_ctl.as_ref().unwrap();
        let s = &mut self.sva_sites[i].ctl;
        if s.epoch != c.epoch {
            s.state = c.state_of(&s.ident, c.log.len());
            s.epoch = c.epoch;
        }
        s.state
    }

    /// The control state of an assertion identified by `id`.
    pub(super) fn assert_ctl_state(&self, id: &CtlIdent) -> CtlState {
        match &self.assert_ctl {
            Some(c) if self.assert_ctl_on => c.state_of(id, c.log.len()),
            _ => CtlState::default(),
        }
    }

    /// The identity of the immediate or deferred assertion `a` executing now.
    pub(super) fn imm_assert_ident(&self, a: &crate::ast::stmt::AssertionStatement) -> CtlIdent {
        use crate::ast::stmt::{AssertionKind, DeferredAssertion};
        CtlIdent {
            scope: self.instance_relative_scope(&self.m_path()),
            label: a.label.as_ref().map(|l| l.name.clone()),
            atype: match a.deferred {
                None => AT_SIMPLE_IMMEDIATE,
                Some(DeferredAssertion::Observed) => AT_OBSERVED_DEFERRED,
                Some(DeferredAssertion::Final) => AT_FINAL_DEFERRED,
            },
            dtype: match a.kind {
                AssertionKind::Assert => DT_ASSERT,
                AssertionKind::Cover => DT_COVER,
                AssertionKind::Assume => DT_ASSUME,
            },
        }
    }

    /// Whether a unique/unique0/priority violation check runs here.
    pub(super) fn unique_check_enabled(
        &self,
        up: Option<&crate::ast::stmt::UniquePriority>,
    ) -> bool {
        use crate::ast::stmt::UniquePriority;
        if !self.assert_ctl_on {
            return true;
        }
        let atype = match up {
            Some(UniquePriority::Unique) => AT_UNIQUE,
            Some(UniquePriority::Unique0) => AT_UNIQUE0,
            Some(UniquePriority::Priority) => AT_PRIORITY,
            None => return true,
        };
        let id = CtlIdent {
            scope: self.instance_relative_scope(&self.m_path()),
            label: None,
            atype,
            dtype: DT_ASSERT,
        };
        self.assert_ctl_state(&id).enabled()
    }
}
