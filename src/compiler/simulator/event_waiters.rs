//! Parked `@(...)` waiters and the per-signal index that tells the drain
//! which of them to examine.
//!
//! §9.4.2: a process parked on an event control resumes when one of its
//! terms sees its event. Examining a waiter compares each term's signal
//! with the baseline the waiter holds (`EventWaiter::captured_prev`) and
//! then takes the current value as the new baseline, so examining a waiter
//! none of whose signals was written since its last examination changes
//! nothing. The index keeps, per signal, the waiters whose terms read it;
//! a write to such a signal queues it (`note_write`), and a drain examines
//! the waiters of the queued signals only, in parking order. A term on an
//! expression (`@(v[3])`, `@(a ^ b)`, a port connected to a select) lists
//! its operand signals, and only a move of one of them makes the
//! examination evaluate the expression, so its waiter is followed through
//! those. A waiter the index cannot follow by signal id (a term on an
//! array cell picked by a variable index, a memory-arena cell, an
//! expression with a call, a clocking event) is examined at every drain,
//! as every waiter was before.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

/// `EventWaiters::watch` flags of one signal.
///
/// An indexed waiter reads the signal and no write is queued for the drain:
/// the next write queues it.
pub(super) const WATCH_DRAIN: u8 = 1;
/// The signal's edge baseline (`prev_val`/`prev_xz`) equals its value: the
/// next write queues it for the snapshot.
pub(super) const WATCH_SNAP: u8 = 2;
/// Either: a write has something to note.
pub(super) const WATCH_ANY: u8 = WATCH_DRAIN | WATCH_SNAP;
/// Queued in `EventWaiters::dirty`.
const QUEUED_DRAIN: u8 = 4;
/// Queued in `EventWaiters::snap_dirty`.
const QUEUED_SNAP: u8 = 8;

/// The indexed waiters with a term on one signal.
#[derive(Default)]
struct SigWatchers {
    /// Parked waiters among `seqs`.
    live: u32,
    /// Their sequence numbers, with those of waiters that left since
    /// (dropped when the list is next purged).
    seqs: Vec<u64>,
}

/// How a drain finds a parked waiter.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum WaiterClass {
    /// Listed under each of its signals in `EventWaiters::sigs`.
    Indexed,
    /// In `EventWaiters::cons` until its signals can be hooked.
    Unhooked,
    /// In `EventWaiters::cons` for good: the index cannot follow it.
    Fixed,
}

pub(super) struct ParkedWaiter {
    pub(super) w: EventWaiter,
    pub(super) class: WaiterClass,
}

impl ParkedWaiter {
    #[inline]
    fn indexed(&self) -> bool {
        self.class == WaiterClass::Indexed
    }
}

/// The processes parked on an event control, in parking order.
#[derive(Default)]
pub(super) struct EventWaiters {
    /// By sequence number: ascending is parking order, the order a drain
    /// examines them in and so the order it wakes them in.
    pub(super) entries: BTreeMap<u64, ParkedWaiter>,
    /// Parked waiters per process.
    pub(super) by_pid: HashMap<usize, u32>,
    next: u64,
    /// The waiters every drain examines.
    pub(super) cons: std::collections::BTreeSet<u64>,
    sigs: HashMap<usize, SigWatchers>,
    /// Per signal id (empty until a waiter is indexed): `WATCH_*` and
    /// `QUEUED_*` flags.
    pub(super) watch: Vec<u8>,
    /// Signals written since the last drain re-armed them.
    pub(super) dirty: Vec<u32>,
    /// Signals whose edge baseline the next snapshot refreshes.
    pub(super) snap_dirty: Vec<u32>,
    /// Bitmap of the signals made observable for the index: a write to one
    /// reaches `note_edge_write` as a write to an edge signal does.
    hooked: Vec<u64>,
    /// The next drain examines every waiter.
    pub(super) scan_all: bool,
    /// A drain holds `entries`: the lists cannot be purged against it.
    pub(super) draining: bool,
    /// Settles that had to rebuild the write-observer tables for a newly
    /// hooked signal, and the settle count at the last one.
    hook_rebuilds: u64,
    hook_stamp: u64,
    /// Scratch for the drain's candidates.
    pub(super) cand_buf: Vec<u64>,
}

impl EventWaiters {
    #[inline]
    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[inline]
    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    /// The parked waiters, in parking order.
    pub(super) fn iter(&self) -> impl Iterator<Item = &EventWaiter> {
        self.entries.values().map(|p| &p.w)
    }

    /// The parked waiters with their sequence numbers, in parking order.
    pub(super) fn iter_seq(&self) -> impl Iterator<Item = (u64, &EventWaiter)> {
        self.entries.iter().map(|(&seq, p)| (seq, &p.w))
    }

    #[inline]
    pub(super) fn contains_pid(&self, pid: usize) -> bool {
        self.by_pid.contains_key(&pid)
    }

    /// The waiter `pid` parked first.
    pub(super) fn first_of_pid(&self, pid: usize) -> Option<&EventWaiter> {
        if !self.contains_pid(pid) {
            return None;
        }
        self.iter().find(|w| w.pid == pid)
    }

    #[inline]
    pub(super) fn is_hooked(&self, id: usize) -> bool {
        self.hooked
            .get(id >> 6)
            .is_some_and(|w| (w >> (id & 63)) & 1 != 0)
    }

    fn set_hooked(&mut self, id: usize) {
        if self.hooked.len() <= id >> 6 {
            self.hooked.resize((id >> 6) + 1, 0);
        }
        self.hooked[id >> 6] |= 1u64 << (id & 63);
    }

    /// A watched signal was written: queue it for whoever asked.
    #[cold]
    #[inline(never)]
    pub(super) fn note_write(&mut self, id: usize) {
        let f = &mut self.watch[id];
        if *f & WATCH_DRAIN != 0 {
            *f = (*f & !WATCH_DRAIN) | QUEUED_DRAIN;
            self.dirty.push(id as u32);
        }
        if *f & WATCH_SNAP != 0 {
            *f = (*f & !WATCH_SNAP) | QUEUED_SNAP;
            self.snap_dirty.push(id as u32);
        }
    }

    /// Queue `id` for the next drain whether or not a write was noted.
    fn queue_drain(&mut self, id: usize) {
        let f = &mut self.watch[id];
        if *f & QUEUED_DRAIN == 0 {
            *f = (*f & !WATCH_DRAIN) | QUEUED_DRAIN;
            self.dirty.push(id as u32);
        }
    }

    /// The distinct signals of `w`'s terms.
    fn term_ids(w: &EventWaiter) -> impl Iterator<Item = usize> + '_ {
        let t = &w.resolved_sensitivities;
        t.iter().enumerate().filter_map(move |(i, s)| {
            (!t[..i].iter().any(|p| p.signal_id == s.signal_id)).then_some(s.signal_id)
        })
    }

    /// List waiter `seq` under each of its signals. `nsig` is the signal
    /// table's length.
    fn index(&mut self, seq: u64, w: &EventWaiter, nsig: usize) {
        if self.watch.len() < nsig {
            self.watch.resize(nsig, 0);
        }
        for id in Self::term_ids(w) {
            let s = self.sigs.entry(id).or_default();
            // Waiters that left stay listed until their signal is written;
            // on one that never is, drop them once they outnumber the rest.
            if !self.draining && s.seqs.len() >= 16 && s.seqs.len() > 2 * s.live as usize {
                let entries = &self.entries;
                s.seqs.retain(|q| entries.contains_key(q));
            }
            s.live += 1;
            s.seqs.push(seq);
            let f = &mut self.watch[id];
            // Newly watched: the next snapshot takes its edge baseline, as
            // it does for every signal a waiter is parked on.
            if s.live == 1 && *f & (WATCH_SNAP | QUEUED_SNAP) == 0 {
                *f |= QUEUED_SNAP;
                self.snap_dirty.push(id as u32);
            }
            if *f & QUEUED_DRAIN == 0 {
                *f |= WATCH_DRAIN;
            }
        }
    }

    /// Waiter `seq` left the list.
    fn unindex(&mut self, seq: u64, w: &EventWaiter, indexed: bool) {
        if !indexed {
            self.cons.remove(&seq);
            return;
        }
        for id in Self::term_ids(w) {
            let Some(s) = self.sigs.get_mut(&id) else {
                continue;
            };
            s.live = s.live.saturating_sub(1);
            if s.live == 0 {
                s.seqs.clear();
                if let Some(f) = self.watch.get_mut(id) {
                    *f &= !WATCH_ANY;
                }
            }
        }
    }

    fn unlink_pid(by_pid: &mut HashMap<usize, u32>, pid: usize) {
        if let Some(n) = by_pid.get_mut(&pid) {
            *n -= 1;
            if *n == 0 {
                by_pid.remove(&pid);
            }
        }
    }

    /// Park `w`; `class` as `Simulator::waiter_class` judged it.
    fn insert(&mut self, w: EventWaiter, class: WaiterClass, nsig: usize) -> u64 {
        let seq = self.next;
        self.next += 1;
        *self.by_pid.entry(w.pid).or_default() += 1;
        if class == WaiterClass::Indexed {
            self.index(seq, &w, nsig);
        } else {
            self.cons.insert(seq);
        }
        self.entries.insert(seq, ParkedWaiter { w, class });
        seq
    }

    fn remove_seq(&mut self, seq: u64) -> Option<EventWaiter> {
        let p = self.entries.remove(&seq)?;
        Self::unlink_pid(&mut self.by_pid, p.w.pid);
        self.unindex(seq, &p.w, p.indexed());
        Some(p.w)
    }

    /// Drop every waiter of the processes `gone` selects (killed or
    /// disabled). `any` tells whether a candidate pid is parked here at all.
    pub(super) fn remove_where(&mut self, any: bool, mut gone: impl FnMut(usize) -> bool) {
        if !any || self.entries.is_empty() {
            return;
        }
        let seqs: Vec<u64> = self
            .entries
            .iter()
            .filter(|(_, p)| gone(p.w.pid))
            .map(|(&s, _)| s)
            .collect();
        for seq in seqs {
            self.remove_seq(seq);
        }
    }

    /// Drop every waiter of `pid`.
    pub(super) fn remove_pid(&mut self, pid: usize) {
        self.remove_where(self.contains_pid(pid), |p| p == pid);
    }

    /// Drop every waiter of the processes in `pids`.
    pub(super) fn remove_pids(&mut self, pids: &HashSet<usize>) {
        let any = if pids.len() <= self.by_pid.len() {
            pids.iter().any(|p| self.by_pid.contains_key(p))
        } else {
            self.by_pid.keys().any(|p| pids.contains(p))
        };
        self.remove_where(any, |p| pids.contains(&p));
    }

    /// Remove and return the waiter `pid` parked first.
    pub(super) fn take_first_of_pid(&mut self, pid: usize) -> Option<EventWaiter> {
        if !self.contains_pid(pid) {
            return None;
        }
        let seq = self
            .iter_seq()
            .find(|(_, w)| w.pid == pid)
            .map(|(s, _)| s)?;
        self.remove_seq(seq)
    }

    /// Give term `k` of waiter `seq` the baseline `prev`. The waiter no
    /// longer holds its signal's value, so the next drain examines all.
    pub(super) fn set_captured_prev(&mut self, seq: u64, k: usize, prev: (u64, u64)) {
        if let Some(p) = self.entries.get_mut(&seq) {
            p.w.captured_prev[k] = prev;
            self.scan_all = true;
        }
    }

    /// Start a drain: re-arm the signals written since the last one and
    /// push their waiters onto `cand`.
    fn take_dirty(&mut self, cand: &mut Vec<u64>) {
        let mut dirty = std::mem::take(&mut self.dirty);
        for &id in &dirty {
            let id = id as usize;
            let f = &mut self.watch[id];
            *f &= !QUEUED_DRAIN;
            if let Some(s) = self.sigs.get(&id) {
                if s.live > 0 {
                    *f |= WATCH_DRAIN;
                    cand.extend_from_slice(&s.seqs);
                }
            }
        }
        dirty.clear();
        self.dirty = dirty;
    }

    /// Whether signal `id`, queued for the snapshot, still has an indexed
    /// waiter; if so its baseline is about to be refreshed.
    fn snap_take(&mut self, id: usize) -> bool {
        let f = &mut self.watch[id];
        *f &= !QUEUED_SNAP;
        if self.sigs.get(&id).is_some_and(|s| s.live > 0) {
            *f |= WATCH_SNAP;
            true
        } else {
            false
        }
    }
}

// A checkpoint holds the waiters in parking order, as the list it replaced
// did. The index is not saved: a restored waiter is examined at every drain
// until its next examination indexes it again.
impl serde::Serialize for EventWaiters {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(self.iter())
    }
}

impl<'de> serde::Deserialize<'de> for EventWaiters {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let mut waiters = Self::default();
        for w in Vec::<EventWaiter>::deserialize(d)? {
            waiters.insert(w, WaiterClass::Unhooked, 0);
        }
        waiters.scan_all = true;
        Ok(waiters)
    }
}

impl Simulator {
    /// Whether a write to signal `id` reaches `EventWaiters::note_write`:
    /// an edge-sensitive signal's does (the dirty-driven edge scan depends
    /// on it), and so does that of a signal hooked for the index.
    #[inline]
    pub(super) fn waiter_signal_hooked(&self, id: usize) -> bool {
        self.sig_to_edge_pos.get(id).is_some_and(|&p| p >= 0) || self.event_waiters.is_hooked(id)
    }

    /// Make writes to `id` observable: it stops being a quiet signal and a
    /// gate-lane destination notes its writes. Both tables are rebuilt by
    /// the next settle.
    fn hook_waiter_signal(&mut self, id: usize) {
        self.event_waiters.set_hooked(id);
        self.quiet_valid = false;
        if !self.gate_ops.is_empty() {
            self.gate_lane_valid = false;
        }
        if let Some(plan) = self.signal_commit_plan.get_mut(id) {
            *plan |= 1;
        }
        let ew = &mut self.event_waiters;
        if ew.hook_rebuilds == 0 || ew.hook_stamp != self.settle_calls {
            ew.hook_rebuilds += 1;
            ew.hook_stamp = self.settle_calls;
        }
    }

    /// Whether another signal may be hooked now. Not from inside a settle,
    /// which holds the tables the hook invalidates; and each batch of new
    /// signals costs the next settle a rebuild, so past the first batches
    /// at most one settle in 1024 pays for one.
    fn may_hook_waiter_signal(&self) -> bool {
        if self.settling {
            return false;
        }
        let ew = &self.event_waiters;
        (ew.hook_rebuilds != 0 && ew.hook_stamp == self.settle_calls)
            || ew.hook_rebuilds < 64 + self.settle_calls / 1024
    }

    /// An event expression made of selects and operators over names and
    /// numbers: nothing it evaluates can write a signal or read anything
    /// but its operands.
    fn plain_event_expr(e: &Expression) -> bool {
        match &e.kind {
            ExprKind::Number(_) => true,
            ExprKind::Ident(h) => h
                .path
                .iter()
                .all(|seg| seg.selects.iter().all(Self::plain_event_expr)),
            ExprKind::Paren(x) | ExprKind::Unary { operand: x, .. } => Self::plain_event_expr(x),
            ExprKind::Index { expr, index } => {
                Self::plain_event_expr(expr) && Self::plain_event_expr(index)
            }
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => {
                Self::plain_event_expr(expr)
                    && Self::plain_event_expr(left)
                    && Self::plain_event_expr(right)
            }
            ExprKind::Binary { left, right, .. } => {
                Self::plain_event_expr(left) && Self::plain_event_expr(right)
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                Self::plain_event_expr(condition)
                    && Self::plain_event_expr(then_expr)
                    && Self::plain_event_expr(else_expr)
            }
            ExprKind::Concatenation(parts) => parts.iter().all(Self::plain_event_expr),
            _ => false,
        }
    }

    /// Can the index follow `w` by the signals of its terms? Every term
    /// must name a whole stored signal (not an array cell looked up again
    /// when its index moves, not a memory-arena cell) whose writes are
    /// observable, and an expression term a plain expression; signals not
    /// yet observable are hooked here when that is allowed.
    pub(super) fn waiter_class(&mut self, w: &EventWaiter) -> WaiterClass {
        if !self.waiter_index_on
            || w.is_clocking
            || w.resolved_sensitivities.is_empty()
            || w.value_ctx.as_ref().is_some_and(|c| c.cells)
        {
            return WaiterClass::Fixed;
        }
        let nsig = self.signal_table.len();
        let mut unhooked = false;
        for t in &w.resolved_sensitivities {
            let id = t.signal_id;
            if t.cell
                || is_packed_id(id)
                || id >= nsig
                || t.value_of
                    .as_ref()
                    .is_some_and(|e| !Self::plain_event_expr(e))
            {
                return WaiterClass::Fixed;
            }
            unhooked |= !self.waiter_signal_hooked(id);
        }
        if !unhooked {
            return WaiterClass::Indexed;
        }
        if !self.may_hook_waiter_signal() {
            return WaiterClass::Unhooked;
        }
        for t in &w.resolved_sensitivities {
            if !self.waiter_signal_hooked(t.signal_id) {
                self.hook_waiter_signal(t.signal_id);
            }
        }
        WaiterClass::Indexed
    }

    /// Does term `i` of `w` hold its signal's current value as baseline?
    fn waiter_term_current(&self, w: &EventWaiter, i: usize) -> bool {
        let id = w.resolved_sensitivities[i].signal_id;
        if w.captured_prev[i] != self.term_raw_bits(id) {
            return false;
        }
        if !is_packed_id(id) && self.signal_widths[id] > 64 {
            return w.captured_prev_wide[i].as_ref() == Some(&self.signal_table[id]);
        }
        true
    }

    /// Park `w` on its event control.
    pub(super) fn park_event_waiter(&mut self, w: EventWaiter) {
        let class = self.waiter_class(&w);
        let nsig = self.signal_table.len();
        let seq = self.event_waiters.insert(w, class, nsig);
        if class != WaiterClass::Indexed {
            return;
        }
        // An indexed waiter is skipped until one of its signals is written,
        // which is only right from a baseline that is the current value.
        // Every parking site captures it on the spot; one that did not gets
        // its signal queued so the next drain examines the waiter.
        let w = &self.event_waiters.entries[&seq].w;
        let mut stale: Vec<usize> = Vec::new();
        for i in 0..w.resolved_sensitivities.len() {
            if !self.waiter_term_current(w, i) {
                stale.push(w.resolved_sensitivities[i].signal_id);
            }
        }
        for id in stale {
            self.event_waiters.queue_drain(id);
        }
    }

    /// Examine one parked waiter for a drain and account for its leaving
    /// the list. A waiter kept that the index can follow by now moves into
    /// it. Returns whether the waiter stays parked.
    fn examine_parked(
        &mut self,
        seq: u64,
        p: &mut ParkedWaiter,
        by_pid: &mut HashMap<usize, u32>,
        conts: &mut Vec<(usize, ProcCont)>,
        ranks: &mut Vec<u8>,
    ) -> bool {
        self.prof_waiter_iters += 1;
        if self.examine_event_waiter(&mut p.w, conts, ranks) {
            if p.class == WaiterClass::Unhooked {
                p.class = self.waiter_class(&p.w);
                if p.class == WaiterClass::Indexed {
                    // Kept, so its baseline was just refreshed to the
                    // current values.
                    self.event_waiters.cons.remove(&seq);
                    let nsig = self.signal_table.len();
                    self.event_waiters.index(seq, &p.w, nsig);
                }
            }
            true
        } else {
            EventWaiters::unlink_pid(by_pid, p.w.pid);
            self.event_waiters.unindex(seq, &p.w, p.indexed());
            false
        }
    }

    /// Detect which parked event waiters' sensitivities fired (against
    /// their own captured_prev baselines) and remove them from the parked
    /// list, refreshing the baseline of everyone examined and left. Returns
    /// the (pid, continuation) pairs to run. Shared by the classic
    /// post-edge-block drain and the pre-edge-block drain (see
    /// XEZIM_WAITERS_FIRST in check_edges_inner).
    pub(super) fn drain_triggered_event_waiters(&mut self) -> Vec<(usize, ProcCont)> {
        if self.event_waiters.is_empty() {
            return Vec::new();
        }
        let mut triggered_conts = std::mem::take(&mut self.triggered_conts_buf);
        triggered_conts.clear();
        // Wake rank of each entry of `triggered_conts`, kept only from the
        // first nonzero rank on: empty means every wakeup is depth 0.
        let mut ranks: Vec<u8> = Vec::new();
        // The drain holds the list: code a guard reaches sees no parked
        // waiter, and a waiter that does not fire stays where it is.
        let mut entries = std::mem::take(&mut self.event_waiters.entries);
        let mut by_pid = std::mem::take(&mut self.event_waiters.by_pid);
        self.event_waiters.draining = true;
        let mut cand = std::mem::take(&mut self.event_waiters.cand_buf);
        cand.clear();
        if self.waiter_index_on {
            self.event_waiters.take_dirty(&mut cand);
        }
        // Time 0 delivers port and alias updates to the waiters themselves
        // (`deliver_time0_port_edges`), outside any signal write. And when
        // a good part of the waiters is due anyway, one pass over all of
        // them is cheaper than looking each one up.
        let all = !self.waiter_index_on
            || self.time == 0
            || self.event_waiters.scan_all
            || self.waiter_index_shadow
            || 4 * (cand.len() + self.event_waiters.cons.len()) >= entries.len();
        if all {
            if self.waiter_index_shadow
                && self.waiter_index_on
                && self.time != 0
                && !self.event_waiters.scan_all
            {
                self.verify_skipped_waiters(&entries, &mut cand);
            }
            self.event_waiters.scan_all = false;
            entries.retain(|&seq, p| {
                self.examine_parked(seq, p, &mut by_pid, &mut triggered_conts, &mut ranks)
            });
        } else {
            cand.extend(self.event_waiters.cons.iter().copied());
            // Runs of ascending sequence numbers, one per signal.
            cand.sort();
            cand.dedup();
            let mut i = 0;
            let mut rest_taken = false;
            while i < cand.len() {
                let seq = cand[i];
                i += 1;
                // A waiter that left since its signal listed it.
                let Some(p) = entries.get_mut(&seq) else {
                    continue;
                };
                if !self.examine_parked(seq, p, &mut by_pid, &mut triggered_conts, &mut ranks) {
                    entries.remove(&seq);
                }
                // A guard wrote a watched signal: the waiters after this
                // one see that write in this pass, as they would when every
                // waiter is examined. (The earlier ones see it in the next
                // drain: the signal stays queued.)
                if !rest_taken && !self.event_waiters.dirty.is_empty() {
                    rest_taken = true;
                    cand.truncate(i);
                    cand.extend(
                        entries
                            .range((std::ops::Bound::Excluded(seq), std::ops::Bound::Unbounded))
                            .map(|(&s, _)| s),
                    );
                }
            }
        }
        cand.clear();
        self.event_waiters.cand_buf = cand;
        self.event_waiters.draining = false;
        // A waiter parked from inside the examination was dropped when the
        // drain put its list back; it still is.
        let nested = std::mem::replace(&mut self.event_waiters.entries, entries);
        for (seq, p) in nested {
            self.event_waiters.unindex(seq, &p.w, p.indexed());
        }
        self.event_waiters.by_pid = by_pid;
        // Within-region process resumption order is LRM-indeterminate
        // (§4.7), but the reference simulator wakes the LAST-armed waiter
        // first — and this campaign matches the reference's observable
        // ordering so differential runs stay comparable. Registration order
        // is FIFO; reverse to LIFO at the single hand-off point.
        triggered_conts.reverse();
        // §4.4/§10.3: a continuous assignment's update is its own active
        // event, so a net it drives changes — and wakes its waiters — only
        // after every waiter of its source has been scheduled. Order the
        // wakeups by scheduling depth (`build_sig_wake_rank`), keeping LIFO
        // within a depth.
        if !ranks.is_empty() {
            self.order_wakeups_by_rank(&mut triggered_conts, ranks);
        }
        triggered_conts
    }

    /// `XEZIM_WAITER_INDEX_SHADOW=1`: every waiter is examined, and an
    /// indexed waiter the index would have skipped must hold its signals'
    /// current values, or a write went unnoted.
    #[cold]
    #[inline(never)]
    fn verify_skipped_waiters(&self, entries: &BTreeMap<u64, ParkedWaiter>, cand: &mut Vec<u64>) {
        cand.sort();
        cand.dedup();
        for (seq, p) in entries {
            if !p.indexed() || cand.binary_search(seq).is_ok() {
                continue;
            }
            for i in 0..p.w.resolved_sensitivities.len() {
                if !self.waiter_term_current(&p.w, i) {
                    let id = p.w.resolved_sensitivities[i].signal_id;
                    panic!(
                        "XEZIM_WAITER_INDEX_SHADOW: at time {} the waiter of process {} \
                         (parked at {}) was not queued for a write to '{}' (id {}, watch \
                         flags {:#x}): baseline {:?}, now {:?}",
                        self.time,
                        p.w.pid,
                        p.w.parked_time,
                        self.name_for_id(id),
                        id,
                        self.event_waiters.watch.get(id).copied().unwrap_or(0),
                        p.w.captured_prev[i],
                        self.term_raw_bits(id),
                    );
                }
            }
        }
    }

    /// Refresh the edge baseline (`prev_val`/`prev_xz`/`prev_wide`) of the
    /// signals event waiters are parked on, for a snapshot. Every such
    /// signal takes its current value. For one the index follows that is a
    /// change only after a write, or when it first gets a waiter, and both
    /// queue it; the others are walked.
    pub(super) fn snapshot_waiter_signals(&mut self) {
        #[inline]
        fn snap_one(
            id: usize,
            signal_table: &[Value],
            signal_widths: &[u32],
            prev_val: &mut [u64],
            prev_xz: &mut [u64],
            prev_wide: &mut HashMap<usize, Value>,
        ) {
            let (v, x) = signal_table[id].raw_bits();
            prev_val[id] = v;
            prev_xz[id] = x;
            if signal_widths[id] > 64 {
                Simulator::snapshot_wide(prev_wide, id, &signal_table[id]);
            }
        }
        let by_index = self.waiter_index_on && self.time != 0;
        if by_index {
            let mut queued = std::mem::take(&mut self.event_waiters.snap_dirty);
            for &id in &queued {
                let id = id as usize;
                if self.event_waiters.snap_take(id) && id < self.prev_val.len() {
                    snap_one(
                        id,
                        &self.signal_table,
                        &self.signal_widths,
                        &mut self.prev_val,
                        &mut self.prev_xz,
                        &mut self.prev_wide,
                    );
                }
            }
            queued.clear();
            self.event_waiters.snap_dirty = queued;
            if self.waiter_index_shadow {
                self.verify_waiter_baselines();
            }
        }
        let ew = &self.event_waiters;
        let mut walk = |w: &EventWaiter| {
            for t in &w.resolved_sensitivities {
                let sid = t.signal_id;
                // A cell of a memory stored without element names (arena or
                // bulk) has no edge snapshot; its waiter compares against
                // its own `captured_prev`.
                if sid >= self.prev_val.len() {
                    continue;
                }
                snap_one(
                    sid,
                    &self.signal_table,
                    &self.signal_widths,
                    &mut self.prev_val,
                    &mut self.prev_xz,
                    &mut self.prev_wide,
                );
            }
        };
        if by_index && 4 * ew.cons.len() < ew.entries.len() {
            for seq in &ew.cons {
                if let Some(p) = ew.entries.get(seq) {
                    walk(&p.w);
                }
            }
        } else if by_index {
            for p in ew.entries.values().filter(|p| !p.indexed()) {
                walk(&p.w);
            }
        } else {
            for w in ew.iter() {
                walk(w);
            }
        }
    }

    /// `XEZIM_WAITER_INDEX_SHADOW=1`: a signal the snapshot skipped must
    /// already hold its value as edge baseline.
    #[cold]
    #[inline(never)]
    fn verify_waiter_baselines(&self) {
        for p in self.event_waiters.entries.values() {
            if !p.indexed() {
                continue;
            }
            for t in &p.w.resolved_sensitivities {
                let id = t.signal_id;
                if id >= self.prev_val.len() {
                    continue;
                }
                let cur = self.signal_table[id].raw_bits();
                let wide_ok = self.signal_widths[id] <= 64
                    || *Self::wide_edge_baseline(&self.prev_wide, id, self.signal_widths[id])
                        == self.signal_table[id];
                if (self.prev_val[id], self.prev_xz[id]) != cur || !wide_ok {
                    panic!(
                        "XEZIM_WAITER_INDEX_SHADOW: at time {} the snapshot skipped '{}' \
                         (id {}, watch flags {:#x}) with baseline {:?} but value {:?}",
                        self.time,
                        self.name_for_id(id),
                        id,
                        self.event_waiters.watch.get(id).copied().unwrap_or(0),
                        (self.prev_val[id], self.prev_xz[id]),
                        cur,
                    );
                }
            }
        }
    }
}
