//! IPC & synchronization subsystem — mailboxes, semaphores, and named
//! (instance) events.
//!
//! Extracted from `mod.rs` (Step 17 of the codebase rework). The storage for
//! all of this lives in `Simulator::ipc` ([crate::compiler::simulator::IpcState]);
//! the structs below model individual waiters and the methods drive the
//! Waker/rescheduler logic.

use super::*;

/// A process blocked in `semaphore.get(n)` because the count was below `n`
/// (IEEE 1800-2017 §15.3.3). Woken by a `put` that raises the count enough.
pub(crate) struct SemGetWaiter {
    pub(crate) pid: usize,
    /// Keys still required.
    pub(crate) n: i64,
    pub(crate) cont: ProcCont,
}

pub(crate) struct MailboxGetWaiter {
    pub(crate) pid: usize,
    pub(crate) lvalue: Expression,
    pub(crate) cont: ProcCont,
    /// `peek` (not `get`): the waiter reads the front WITHOUT removing it, so
    /// a `put` that wakes it must also leave the item in the mailbox for the
    /// subsequent `get`/`try_get`.
    pub(crate) is_peek: bool,
}

/// §15.4.1 — a process blocked on `mailbox.put(v)` because a BOUNDED mailbox is
/// full. The value is captured at the (blocking) call; when a `get`/`try_get`
/// frees a slot the value is stored and `cont` is rescheduled.
pub(crate) struct MailboxPutWaiter {
    pub(crate) pid: usize,
    pub(crate) value: Value,
    pub(crate) cont: ProcCont,
}

#[derive(Debug, Clone)]
pub(crate) struct EventWaiter {
    pub(crate) pid: usize,
    /// Simulation time when this waiter parked. Drives the hang report:
    /// waiters sorted oldest-first expose "who has been stuck longest",
    /// and `arm_bits` tells whether the awaited signals ever moved since.
    pub(crate) parked_time: u64,
    /// Pre-resolved signal IDs for O(1) edge checking.
    pub(crate) resolved_sensitivities: Vec<SensitivityId>,
    /// Value (raw v,x low-64) of each `resolved_sensitivities` signal at the
    /// moment this waiter armed.
    pub(crate) arm_bits: Vec<(u64, u64)>,
    pub(crate) continuation: ProcCont,
    /// Each sensitivity signal's value captured AT registration time
    /// (`raw_bits()` for the ≤64-bit fast path).
    pub(crate) captured_prev: Vec<(u64, u64)>,
    /// Full captured value for >64-bit sensitivity signals (parallel to
    /// `captured_prev`); `None` for ≤64-bit signals.
    pub(crate) captured_prev_wide: Vec<Option<Value>>,
    /// §9.4.2 value guard for non-trivial event expressions — parallel to
    /// `resolved_sensitivities`.
    pub(crate) guard_prev: Vec<Option<Value>>,
    pub(crate) remaining_events: u64,
    /// LRM §14.13: clocking-event waiter (`@(cb)` / `##N`).
    pub(crate) is_clocking: bool,
}

/// A process parked on a CLASS-FIELD named event (`event m_event` inside a
/// class, awaited/triggered as `@m_event` / `->m_event` from a method on
/// `this`). Each (owning instance handle, field name) pair is a distinct
/// synchronization object (IEEE 1800-2023 §15.5).
#[derive(Clone)]
pub(crate) struct InstanceEventWaiter {
    pub(crate) key: (usize, String),
    pub(crate) pid: usize,
    pub(crate) continuation: ProcCont,
}

impl super::Simulator {
    pub(crate) fn make_event_waiter(
        &mut self,
        pid: usize,
        sens: Vec<Sensitivity>,
        continuation: ProcCont,
    ) -> EventWaiter {
        self.make_event_waiter_kind(pid, sens, continuation, false)
    }

    pub(crate) fn make_event_waiter_kind(
        &mut self,
        pid: usize,
        sens: Vec<Sensitivity>,
        continuation: ProcCont,
        is_clocking: bool,
    ) -> EventWaiter {
        let resolved: Vec<SensitivityId> = sens
            .iter()
            .filter_map(|s| {
                self.signal_name_to_id
                    .get(s.signal_name.as_str())
                    .map(|&id| SensitivityId {
                        signal_id: id,
                        edge: s.edge,
                        iff: s.iff.clone(),
                        value_of: s.value_of.clone(),
                    })
            })
            .collect();
        // §9.4.2 value guard: capture each non-trivial term's value at arm.
        let guard_prev: Vec<Option<Value>> = resolved
            .iter()
            .map(|sid| sid.value_of.clone().map(|e| self.eval_expr(&e)))
            .collect();
        // Capture each sensitivity signal's value AT ARM TIME, parallel to
        // `resolved`. A waiter registered within the current snapshot
        // generation is checked against these (not the tick-start snapshot),
        // so only a change made AFTER it armed counts as an edge — see the
        // firing loop in `check_edges_inner`.
        let arm_bits: Vec<(u64, u64)> = resolved
            .iter()
            .map(|sid| self.signal_table[sid.signal_id].raw_bits())
            .collect();
        // `sens` (Vec<Sensitivity>) is consumed for resolution and dropped;
        // EventWaiter only carries the resolved IDs from here on.
        //
        // Capture each sensitivity signal's current value at registration
        // so the waiter fires on a change relative to THIS point (see the
        // `captured_prev` field doc for the NBA reasoning).
        let captured_prev: Vec<(u64, u64)> = resolved
            .iter()
            .map(|s| self.signal_table[s.signal_id].raw_bits())
            .collect();
        let captured_prev_wide: Vec<Option<Value>> = resolved
            .iter()
            .map(|s| {
                if self.signal_widths[s.signal_id] > 64 {
                    Some(self.signal_table[s.signal_id].clone())
                } else {
                    None
                }
            })
            .collect();
        EventWaiter {
            pid,
            parked_time: self.time,
            resolved_sensitivities: resolved,
            arm_bits,
            continuation,
            captured_prev,
            captured_prev_wide,
            guard_prev,
            remaining_events: 1,
            is_clocking,
        }
    }

    pub(crate) fn drain_triggered_event_waiters(&mut self) -> Vec<(usize, ProcCont)> {
        let waiters = std::mem::take(&mut self.ipc.event_waiters);
        self.prof_waiter_iters += waiters.len() as u64;
        self.ipc.event_waiters_swap.clear();
        let mut triggered_conts: Vec<(usize, ProcCont)> = Vec::new();
        for mut waiter in waiters {
            let mut triggered = false;
            for (i, sid) in waiter.resolved_sensitivities.iter().enumerate() {
                let (pv, px) = waiter.captured_prev[i];
                let pw = waiter.captured_prev_wide[i].as_ref();
                if !self.edge_fires_prev(sid.signal_id, sid.edge, pv, px, pw) {
                    continue;
                }
                // LRM §9.4.2.3: `@(posedge clk iff g)` only fires when the
                // guard `g` holds at edge time. A false guard re-arms the
                // waiter (it stays in event_waiters for the next edge)
                // rather than resuming the process.
                let guard_ok = match &sid.iff {
                    Some(g) => self.eval_expr(g).is_true(),
                    None => true,
                };
                // §9.4.2: a non-trivial event expression fires only when its
                // VALUE changed since arm.
                let value_ok = match (&sid.value_of, waiter.guard_prev.get(i)) {
                    (Some(e), Some(Some(prev))) => {
                        let e = e.clone();
                        let prev = prev.clone();
                        self.eval_expr(&e) != prev
                    }
                    _ => true,
                };
                if guard_ok && value_ok {
                    triggered = true;
                    break;
                }
            }
            if triggered && waiter.remaining_events > 1 {
                waiter.remaining_events -= 1;
                triggered = false;
            }
            if triggered {
                if sim_debug_enabled() {
                    eprintln!(
                        "[DEBUG] waiter for process {} triggered at time {}",
                        waiter.pid,
                        self.time
                    );
                }
                if waiter.is_clocking {
                    // §14.13: resume in the Reactive region, not here in the
                    // Active region — defer the continuation past apply_nba +
                    // tick_clocking_blocks so it reads post-edge state and this
                    // cycle's clocking samples.
                    self.deferred_clocking_conts
                        .push((waiter.pid, waiter.continuation));
                } else {
                    triggered_conts.push((waiter.pid, waiter.continuation));
                }
            } else {
                // Refresh this waiter's `captured_prev` baseline to each
                // sensitivity signal's CURRENT value so that a qualifying
                // edge occurring over multiple steps is detected.
                //
                // Without this, a waiter armed while its signal sits at the
                // edge-target level can never see the NEXT edge: e.g. a
                // process resumes on the first `@(posedge clk)` at t=5
                // (clk=1) and immediately re-arms on the next line; its
                // captured_prev is frozen at 1, so when clk later goes
                // 1→0→1 the Posedge test `!pb_one && cb_one` stays false
                // (pb_one clings to the stale 1) and the second posedge is
                // lost — the process strands (see
                // tests/bound_module / sequential_event_waits). Tracking the
                // running level here preserves the same-tick NBA-region
                // semantics `captured_prev` was added for (a waiter still
                // won't fire on an edge that completed BEFORE it armed, since
                // at arm time `captured_prev` already equals current), while
                // catching cross-tick transitions through the target level.
                for (i, sid) in waiter.resolved_sensitivities.iter().enumerate() {
                    let (cv, cx) = self.signal_table[sid.signal_id].raw_bits();
                    waiter.captured_prev[i] = (cv, cx);
                    if self.signal_widths[sid.signal_id] > 64 {
                        waiter.captured_prev_wide[i] =
                            Some(self.signal_table[sid.signal_id].clone());
                    }
                }
                self.ipc.event_waiters_swap.push(waiter);
            }
        }
        std::mem::swap(&mut self.ipc.event_waiters, &mut self.ipc.event_waiters_swap);
        // Within-region process resumption order is LRM-indeterminate
        // (§4.7), but the reference simulator wakes the LAST-armed waiter
        // first — and this campaign matches the reference's observable
        // ordering so differential runs stay comparable. Registration order
        // is FIFO; reverse to LIFO at the single hand-off point.
        triggered_conts.reverse();
        triggered_conts
    }

    pub(crate) fn fire_instance_event(&mut self, key: (usize, String)) {
        let now = self.time;
        // §15.5.3: `<h>.<ev>.triggered` must read 1 for the rest of this slot.
        // Instance events have no backing signal, so stamp the same table the
        // name-keyed path uses, under a synthetic per-instance key.
        let stamp = Self::instance_event_stamp_key(&key);
        self.ipc.event_triggered_time.insert(stamp, now);
        let mut woken = Vec::new();
        self.ipc.instance_event_waiters.retain(|w| {
            if w.key == key {
                woken.push((w.pid, w.continuation.clone()));
                false
            } else {
                true
            }
        });
        for (pid, cont) in woken {
            self.event_queue.schedule(now, pid, cont);
        }
    }

    pub(crate) fn wake_semaphore_waiters(&mut self, handle: usize) {
        loop {
            let count = self.ipc.semaphores.get(&handle).copied().unwrap_or(0);
            let next_n = self
                .ipc.semaphore_get_waiters
                .get(&handle)
                .and_then(|q| q.front())
                .map(|w| w.n);
            match next_n {
                Some(n) if count >= n => {
                    let w = self
                        .ipc.semaphore_get_waiters
                        .get_mut(&handle)
                        .unwrap()
                        .pop_front()
                        .unwrap();
                    *self.ipc.semaphores.get_mut(&handle).unwrap() = count - n;
                    if w.cont.is_empty() {
                        self.child_finished(w.pid);
                    } else {
                        self.event_queue.schedule(self.time, w.pid, w.cont);
                    }
                }
                _ => break,
            }
        }
    }

    /// §15.4.1 — a slot just freed on a bounded mailbox: admit one parked `put`
    /// (store its value, resume the producer). No-op for an unbounded mailbox or
    /// when no producer is waiting.
    pub(crate) fn admit_mailbox_put_waiter(&mut self, handle: usize) {
        let bound = self.ipc.mailbox_bound.get(&handle).copied().unwrap_or(0);
        if bound == 0 {
            return;
        }
        let len = self.ipc.mailboxes.get(&handle).map(|q| q.len()).unwrap_or(0);
        if len >= bound {
            return;
        }
        if let Some(w) = self
            .ipc.mailbox_put_waiters
            .get_mut(&handle)
            .and_then(|q| q.pop_front())
        {
            self.ipc.mailboxes.get_mut(&handle).unwrap().push_back(w.value);
            if w.cont.is_empty() {
                self.child_finished(w.pid);
            } else {
                self.event_queue.schedule(self.time, w.pid, w.cont);
            }
        }
    }
}