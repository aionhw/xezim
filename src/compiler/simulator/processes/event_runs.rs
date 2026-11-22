//! Running the postponed region and nested event runs.

use super::*;

impl Simulator {
    /// Drain events in the scheduler whose fire time is at or before `target`.
    /// Used when a task-internal `#delay` needs to yield the simulator so
    /// concurrent processes can advance while this task sleeps.
    /// §4.4.2.4 postponed region for ONE time slot: `$monitor`, `$strobe`,
    /// then the waveform dump. Factored out of the main tick so the nested
    /// `run_events_until` loop can service the slots it consumes. Every part
    /// is a no-op when nothing is armed or nothing changed, so calling it once
    /// per slot costs nothing on designs that dump or monitor nothing.
    pub(in crate::compiler::simulator) fn run_postponed_region(&mut self) {
        self.fire_vpi_synch_cbs();
        self.check_monitor();
        self.drain_pending_strobes();
        self.dump_write_changes();
    }

    /// A fresh wake marker for a wait made from the nested path: its id, a pid
    /// for the waiter to resume, and the statement that fires it.
    pub(in crate::compiler::simulator) fn new_sync_wake(&mut self) -> (u64, usize, Statement) {
        let id = self.next_sync_wake;
        self.next_sync_wake += 1;
        let pid = self.next_pid;
        self.next_pid += 1;
        let span = crate::ast::Span::dummy();
        let arg = Expression::new(
            ExprKind::Number(NumberLiteral::Integer {
                size: Some(64),
                signed: false,
                base: NumberBase::Decimal,
                value: id.to_string(),
                cached_val: crate::ast::cache::LiteralCache::new(Some((id, 0u64, 64u32))),
            }),
            span,
        );
        let stmt = Statement::new(
            StatementKind::Expr(Expression::new(
                ExprKind::SystemCall {
                    name: SYNC_WAKE_MARKER.to_string(),
                    args: vec![arg],
                },
                span,
            )),
            span,
        );
        (id, pid, stmt)
    }

    /// Run the scheduler, nested, until `done` holds, the run finishes, or
    /// nothing is left to run. A task entered from C through a DPI export
    /// waits this way: its caller is a native stack frame, so it cannot park
    /// and resume like a process (#204). The synchronous `#delay` arm waits
    /// the same way, through `run_events_until`.
    pub(in crate::compiler::simulator) fn run_nested_until(
        &mut self,
        mut done: impl FnMut(&mut Self) -> bool,
    ) {
        self.requeue_slot_batches();
        let saved_hint = self.name_resolve_hint.borrow().clone();
        if !self.in_edge_block {
            self.apply_nba();
        }
        self.settle_combinatorial();
        self.check_edges();
        let _ = self.drain_edge_cascade(self.cascade_limit);
        self.snapshot_edge_signals();
        let mut stalled = 0u32;
        let mut idle_flushed = false;
        loop {
            if self.finished {
                break;
            }
            // `done` may evaluate the waiting task's own expressions: give it
            // the task's scope back from the processes the last slot ran.
            *self.name_resolve_hint.borrow_mut() = saved_hint.clone();
            if done(self) {
                break;
            }
            let next = [
                self.event_queue.next_time(),
                self.clock_generators
                    .iter()
                    .map(|c| c.next_toggle_time)
                    .min(),
                self.next_delayed_time(),
                self.next_timer_time(),
            ]
            .into_iter()
            .flatten()
            .min();
            let t = match next {
                Some(t) => t,
                None if !self.inactive_queue.is_empty() => self.time,
                // Nothing scheduled: finish this time slot's own work (an
                // NBA, a waiter it frees) once, then look again.
                None if !idle_flushed => {
                    idle_flushed = true;
                    if !self.in_edge_block {
                        self.apply_nba();
                    }
                    self.settle_combinatorial();
                    self.check_edges();
                    let _ = self.drain_edge_cascade(self.cascade_limit);
                    if self.dpi_export_depth > 0 && !self.condition_waiters.is_empty() {
                        self.nested_drain_condition_waiters();
                    }
                    self.snapshot_edge_signals();
                    continue;
                }
                None => {
                    self.end_run_waiting_in_c(false);
                    break;
                }
            };
            idle_flushed = false;
            if t > self.max_time {
                self.end_run_waiting_in_c(true);
                break;
            }
            let before = self.time;
            self.run_events_until(t);
            if self.time == before {
                stalled += 1;
                if stalled > 100_000 {
                    break;
                }
            } else {
                stalled = 0;
            }
        }
        *self.name_resolve_hint.borrow_mut() = saved_hint;
    }

    /// A scheduler is starting nested inside a process that an enclosing
    /// `run_events_until` took from its time slot's batch: hand the batch's
    /// unrun processes back to the queue, so they run in their own time slot
    /// in the nested scheduler. Left in the batch they ran only once the
    /// nested scheduler returned — at a later time, or never when it waited
    /// on one of them. Several processes calling imported tasks in one slot
    /// on the nested-scheduler fallback lost all but the first (#290).
    pub(in crate::compiler::simulator) fn requeue_slot_batches(&mut self) {
        if self.slot_batches.iter().all(|(_, b)| b.is_empty()) {
            return;
        }
        // Innermost first, so the outermost batch's processes end up first.
        for (t, b) in self.slot_batches.iter_mut().rev() {
            let rest = std::mem::take(b);
            if !rest.is_empty() {
                self.event_queue.requeue_front(*t, rest);
            }
        }
    }

    /// `run_nested_until` gave up: the run ends with the task entered from C
    /// still waiting. §35.8: the call returns only when the task has
    /// finished, so neither the rest of the task nor its caller may run —
    /// they used to, at whatever time the run had reached, as though the
    /// wait had been satisfied (#279). Stop the run as the main loop would
    /// have stopped it, without a `$finish`.
    pub(in crate::compiler::simulator) fn end_run_waiting_in_c(&mut self, max_time: bool) {
        if max_time && !RUN_LENGTH_REQUESTED.load(std::sync::atomic::Ordering::Relaxed) {
            eprintln!(
                "[xezim][hang-report] simulation reached --max-time ({} ticks) without $finish",
                self.max_time
            );
            self.report_parked_waiters(8);
        }
        self.ended_waiting_in_c = true;
        self.finished = true;
    }

    /// The end-of-tick condition-waiter pass, for a time slot the nested loop
    /// (`run_events_until`) runs while a task entered from C waits: resume
    /// the waiters whose condition holds, then deliver the edges their
    /// continuations made, as `run_one_tick`'s late-region re-pass does.
    pub(in crate::compiler::simulator) fn nested_drain_condition_waiters(&mut self) {
        let epoch = self.late_resume_epoch;
        self.drain_condition_waiters(false, false);
        if self.late_resume_epoch != epoch && !self.finished {
            if self.dirty_any {
                self.settle_combinatorial();
            }
            self.check_edges();
            let _ = self.drain_edge_cascade(self.cascade_limit);
            self.snapshot_edge_signals();
        }
    }

    pub(in crate::compiler::simulator) fn run_events_until(&mut self, target: u64) {
        self.requeue_slot_batches();
        self.nested_run_epoch += 1;
        let saved_pid = self.current_pid;
        let saved_break = self.break_flag;
        let saved_return = self.return_flag;
        // §4.4.2.3: `#0` continuations parked by earlier same-batch
        // processes must resume in THIS time slot, not drift to a later nt.
        self.promote_inactive_to_active();
        // §4.4.2.4: this is a nested event loop — it advances `self.time`
        // across whole time slots on its own. It used to run NONE of the
        // postponed region for them, so every slot consumed inside a `#delay`
        // was invisible to `$monitor` and to the waveform dump: a signal that
        // changed there surfaced at whatever LATER slot each of those two
        // happened to be serviced at (different mechanisms, so they disagreed
        // with each other AND with the change itself). A dump is worse off
        // than a late monitor — the VCD had no record for the timestamp at
        // all, and those changes were re-attributed to a later one.
        // Starts TRUE, not false: the caller reaches here from a `#delay`,
        // and the statements it ran at the CURRENT time — including the write
        // immediately after the previous delay — have not been through a
        // postponed region yet. Starting false skipped exactly that slot, so a
        // `#50 sig = 1;` inside an edge block landed in the dump and the
        // monitor at the NEXT serviced slot instead of at the write's own time.
        let mut slot_pending = true;
        loop {
            // §20.2: `$finish` ends the SIMULATION, not just the process that
            // called it. This nested loop drives whole time slots on its own
            // (a `#delay` inside a process body), so without this check a
            // finish raised by any process it services — or by the caller's
            // own earlier statements — kept the loop running for as long as
            // other processes kept scheduling. The visible form: an always
            // block calls $finish while a fork..join_none child still loops
            // on `#1` — the run hung forever, and because Ctrl-C/SIGTERM are
            // polled at the OUTER loop's boundary only, even `timeout(1)`
            // could not kill it (SIGKILL was needed). Poll both here.
            if self.finished {
                break;
            }
            if interrupt_requested() {
                self.take_interrupt();
                break;
            }
            // Advance to the EARLIEST of: event_queue, clock_generators,
            // and delayed-update queue (so a `#10` from inside an initial
            // block doesn't silently jump over clock toggles OR inertial /
            // SDF / `assign #N` delayed commits — LRM §4.4.2 + §10.4.2).
            let next_eq = self.event_queue.next_time();
            let next_clk = self
                .clock_generators
                .iter()
                .map(|c| c.next_toggle_time)
                .min();
            let next_dly = self.next_delayed_time();
            let next_vpi = self.next_timer_time();
            let nt_opt = [next_eq, next_clk, next_dly, next_vpi]
                .into_iter()
                .flatten()
                .min();
            let nt = match nt_opt {
                Some(t) if t <= target => t,
                _ => break,
            };
            let mut vpi_fired = false;
            if nt > self.time {
                // #t0-comb-defer: this loop pops time-0 entries without
                // counting them as starts; close the batch before leaving
                // the slot (see the main loop's release).
                if self.t0_starts_remaining > 0 {
                    self.t0_starts_remaining = 0;
                    self.release_time0_held();
                    self.drain_deferred_proc_settle();
                    continue;
                }
                // §38.36.2: the slot's cbAtEndOfSimTime / cbReadOnlySynch run
                // before time moves; look again if any did.
                if self.vpi_cb_mask != 0 && self.vpi_slot_end() {
                    continue;
                }
                // Leaving this timestamp — close out its postponed region
                // before the clock moves, so $monitor and the dump report it
                // at the time it actually happened.
                if slot_pending {
                    slot_pending = false;
                    self.run_postponed_region();
                } else {
                    self.dump_slot_end();
                }
                self.time = nt;
                publish_sim_time(nt);
                if let Some(sink) = self.stdout_sink.as_mut() {
                    sink.flush_if_stale();
                }
                if self.vpi_slot_started != self.time {
                    if self.vpi_cb_mask & vpi_cb::MASK_SLOT_START != 0 && self.vpi_slot_start() {
                        self.dirty_any = true;
                        vpi_fired = true;
                    }
                    self.vpi_slot_started = self.time;
                }
            }
            // Commit delayed updates whose target time matches first —
            // their values must be visible to clock-fire / process eval
            // at this delta cycle.
            let dly_applied = self.apply_delayed_updates();
            if dly_applied {
                self.dirty_any = true;
            }
            // Fire any clock generators whose next_toggle_time matches.
            let fired_clock = self
                .clock_generators
                .iter()
                .any(|c| c.next_toggle_time == self.time);
            if fired_clock {
                self.fire_clock_generators();
                self.dirty_any = true;
            }
            let processes = self.event_queue.remove(self.time);
            let ran_process = !processes.is_empty();
            if ran_process {
                // The batch stays where a nested scheduler started by one of
                // its processes finds it (`requeue_slot_batches`).
                self.slot_batches.push((self.time, processes));
                let depth = self.slot_batches.len();
                while !self.finished {
                    let Some((pid, stmts)) = self
                        .slot_batches
                        .get_mut(depth - 1)
                        .and_then(|(_, b)| b.pop_front())
                    else {
                        break;
                    };
                    self.run_scheduled_process(pid, &stmts);
                    if !self.is_pid_suspended(pid) {
                        self.child_finished(pid);
                    }
                }
                self.slot_batches.truncate(depth - 1);
            }
            // §4.5: `#0` continuations parked by the processes above activate
            // and run BEFORE this slot's NBA region — loop again at the same
            // time without applying NBAs yet. Settle first so they observe
            // completed active-region (cont-assign) state.
            self.drain_active_driver_updates();
            if self.promote_inactive_to_active() && self.event_queue.next_time() == Some(self.time)
            {
                if self.dirty_any {
                    self.settle_combinatorial();
                }
                continue;
            }
            // §38.36.2 cbNBASynch: the Pre-NBA region.
            if self.vpi_cb_mask & vpi_cb::m(vpi_cb::CB_NBA_SYNCH) != 0 {
                self.vpi_nba_synch();
            }
            if !self.nba_fast.is_empty()
            || !self.nba_queue.is_empty()
            // §15.5.2: a bare `->>` (deferred trigger) must flush with the NBA
            // region even when no NBA DATA is queued.
            || !self.pending_nba_triggers.is_empty()
                || !self.packed_nba.is_empty()
            || !self.pending_nba_instance_triggers.is_empty()
            || self.delayed_nba_due()
            {
                self.apply_nba();
            }
            if self.dirty_any {
                self.settle_combinatorial();
            }
            // Zero-delay updates due now, before this pass's edge check
            // (see `apply_due_delayed_now`).
            let mut dly_applied = dly_applied;
            if !self.delayed_updates.is_empty() && self.apply_due_delayed_now() {
                dly_applied = true;
            }
            // After advancing time and any clock/delay fires, check for
            // edge triggers so always_ff blocks fire within this delay
            // window, then re-snapshot for the next iter.
            self.fire_timing_timers();
            vpi_fired |= self.fire_due_after_delay_cbs();
            if vpi_fired {
                self.dirty_any = true;
                self.settle_combinatorial();
            }
            if fired_clock || dly_applied || ran_process || vpi_fired {
                self.check_edges();
                let _ = self.drain_edge_cascade(self.cascade_limit);
                self.snapshot_edge_signals();
            }
            // §9.7.4: `wait (expr)` is level-sensitive and resumes whenever the
            // expression is true. The main loop re-checks parked waiters at the
            // end of every tick (`drain_condition_waiters`); this nested loop
            // did not, so while a task entered from C (§35.8) waited, other
            // processes' waits on an RTL variable or an event's triggered
            // state stayed parked until the C call returned (#279).
            if self.dpi_export_depth > 0 && !self.condition_waiters.is_empty() {
                self.nested_drain_condition_waiters();
            }
            if self.finished {
                break;
            }
            // §4.4.2.3: a process resumed above may have parked a `#0`
            // continuation in the Inactive queue. apply_nba for this pass
            // has already run, so promote now — the next loop iteration
            // (next_eq == self.time <= target) resumes it with post-NBA
            // values, same ordering as the run_one_tick path.
            slot_pending = true;
            self.promote_inactive_to_active();
        }
        // Final slot inside the window: its NBA/settle/edge work has already
        // run above, so service it here rather than letting the caller's tick
        // attribute it to the delay's target time.
        if slot_pending && !self.finished {
            self.run_postponed_region();
        }
        self.current_pid = saved_pid;
        self.break_flag = saved_break;
        self.return_flag = saved_return;
    }

    /// Reset `name_resolve_hint` to the CURRENT process's own instance
    /// scope (None for a top-scope process). Loop-condition re-evaluation
    /// must not run under sticky hint residue left by nested work (other
    /// processes, settle entries) — a leaked sibling hint once made a
    /// testbench loop var `c` shadow-resolve to the DUT instance's `c`,
    /// and the per-node cache froze it: the loop never terminated.
    pub(in crate::compiler::simulator) fn reset_hint_to_process_scope(&self) {
        let h = self.process_scope_hint.get(&self.current_pid);
        let mut cur = self.name_resolve_hint.borrow_mut();
        if cur.as_deref() != h.map(|s| s.as_str()) {
            *cur = h.cloned();
        }
    }
}
