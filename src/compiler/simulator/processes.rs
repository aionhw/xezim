//! Simulator processes.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

impl Simulator {
    pub(super) fn snapshot_process_context(&self) -> ProcessContext {
        ProcessContext {
            this_stack: self.this_stack.clone(),
            local_stack: self.local_stack.clone(),
            block_scope_saves: self.block_scope_saves.clone(),
            local_gen_stack: self.local_gen_stack.clone(),
            local_type_stack: self.local_type_stack.clone(),
            class_context_stack: self.class_context_stack.clone(),
            current_spec: self.current_spec.clone(),
            spec_scope_stack: self.spec_scope_stack.clone(),
            cg_this: self.cg_this,
            return_value: self.return_value.clone(),
            break_flag: self.break_flag,
            continue_flag: self.continue_flag,
            return_flag: self.return_flag,
            local_iface_aliases: self.local_iface_aliases.clone(),
            ref_binding_stack: self.ref_binding_stack.clone(),
            ref_alias_stack: self.ref_alias_stack.clone(),
            ref_identity_stack: self.ref_identity_stack.clone(),
            queue_frame_saves: self.queue_frame_saves.clone(),
            task_cleanup: self.task_cleanup.clone(),
            local_dyn: self.local_dyn.clone(),
            formal_dyn: self.formal_dyn.clone(),
            static_local_syncs: self.static_local_syncs.clone(),
            method_local_base: self.method_local_base.clone(),
            current_static_task: self.current_static_task.clone(),
            current_auto_task: self.current_auto_task.clone(),
            task_string_formals: self.task_string_formals.clone(),
        }
    }

    /// Move the active execution context out without cloning it. This is used
    /// by retained simple timed assignments, whose body cannot suspend and has
    /// no process-local state of its own. A behavioral clock can fire while a
    /// task is inside `run_events_until`; moving the task context aside avoids
    /// cloning its local arrays on every clock edge.
    /// A `String` holding `text`, reusing a pooled buffer when one exists.
    #[inline]
    pub(super) fn pooled_string(pool: &mut Vec<String>, text: &str) -> String {
        match pool.pop() {
            Some(mut s) => {
                s.clear();
                s.push_str(text);
                s
            }
            None => text.to_string(),
        }
    }

    /// Return a hint string to the pool (bounded so a burst cannot pin memory).
    #[inline]
    pub(super) fn recycle_string(pool: &mut Vec<String>, s: Option<String>) {
        if let Some(s) = s {
            if pool.len() < 64 {
                pool.push(s);
            }
        }
    }

    /// True when every field `take_process_context` would move out is
    /// already empty/false — the state a top-level wake-up starts from.
    #[inline]
    pub(super) fn process_context_is_default(&self) -> bool {
        self.this_stack.is_empty()
            && self.local_stack.is_empty()
            && self.block_scope_saves.is_empty()
            && self.local_gen_stack.is_empty()
            && self.local_type_stack.is_empty()
            && self.class_context_stack.is_empty()
            && self.current_spec.is_none()
            && self.spec_scope_stack.is_empty()
            && self.cg_this.is_none()
            && self.return_value.is_none()
            && !self.break_flag
            && !self.continue_flag
            && !self.return_flag
            && self.local_iface_aliases.is_empty()
            && self.ref_binding_stack.is_empty()
            && self.ref_alias_stack.is_empty()
            && self.ref_identity_stack.is_empty()
            && self.queue_frame_saves.is_empty()
            && self.task_cleanup.is_empty()
            && self.local_dyn.is_empty()
            && self.formal_dyn.is_empty()
            && self.static_local_syncs.is_empty()
            && self.method_local_base.is_empty()
            && self.current_static_task.is_none()
            && self.current_auto_task.is_none()
            && self.task_string_formals.is_empty()
            && !self.ref_redirect_hot
    }

    pub(super) fn take_process_context(&mut self) -> ProcessContext {
        // The ref stacks leave with the context; nothing can redirect until
        // a context is restored or a new ref frame is pushed.
        self.ref_redirect_hot = false;
        ProcessContext {
            this_stack: std::mem::take(&mut self.this_stack),
            local_stack: std::mem::take(&mut self.local_stack),
            block_scope_saves: std::mem::take(&mut self.block_scope_saves),
            local_gen_stack: std::mem::take(&mut self.local_gen_stack),
            local_type_stack: std::mem::take(&mut self.local_type_stack),
            class_context_stack: std::mem::take(&mut self.class_context_stack),
            current_spec: self.current_spec.take(),
            spec_scope_stack: std::mem::take(&mut self.spec_scope_stack),
            cg_this: self.cg_this.take(),
            return_value: self.return_value.take(),
            break_flag: std::mem::replace(&mut self.break_flag, false),
            continue_flag: std::mem::replace(&mut self.continue_flag, false),
            return_flag: std::mem::replace(&mut self.return_flag, false),
            local_iface_aliases: std::mem::take(&mut self.local_iface_aliases),
            ref_binding_stack: std::mem::take(&mut self.ref_binding_stack),
            ref_alias_stack: std::mem::take(&mut self.ref_alias_stack),
            ref_identity_stack: std::mem::take(&mut self.ref_identity_stack),
            queue_frame_saves: std::mem::take(&mut self.queue_frame_saves),
            task_cleanup: std::mem::take(&mut self.task_cleanup),
            local_dyn: std::mem::take(&mut self.local_dyn),
            formal_dyn: std::mem::take(&mut self.formal_dyn),
            static_local_syncs: std::mem::take(&mut self.static_local_syncs),
            method_local_base: std::mem::take(&mut self.method_local_base),
            current_static_task: self.current_static_task.take(),
            current_auto_task: self.current_auto_task.take(),
            task_string_formals: std::mem::take(&mut self.task_string_formals),
        }
    }

    pub(super) fn restore_process_context(&mut self, ctx: ProcessContext) {
        self.this_stack = ctx.this_stack;
        self.local_stack = ctx.local_stack;
        self.block_scope_saves = ctx.block_scope_saves;
        self.local_gen_stack = ctx.local_gen_stack;
        self.local_type_stack = ctx.local_type_stack;
        self.class_context_stack = ctx.class_context_stack;
        self.current_spec = ctx.current_spec;
        self.spec_scope_stack = ctx.spec_scope_stack;
        self.cg_this = ctx.cg_this;
        self.return_value = ctx.return_value;
        self.break_flag = ctx.break_flag;
        self.continue_flag = ctx.continue_flag;
        self.return_flag = ctx.return_flag;
        self.local_iface_aliases = ctx.local_iface_aliases;
        self.ref_binding_stack = ctx.ref_binding_stack;
        self.ref_alias_stack = ctx.ref_alias_stack;
        self.ref_identity_stack = ctx.ref_identity_stack;
        self.refresh_ref_redirect_hot();
        self.queue_frame_saves = ctx.queue_frame_saves;
        self.task_cleanup = ctx.task_cleanup;
        self.local_dyn = ctx.local_dyn;
        self.formal_dyn = ctx.formal_dyn;
        self.static_local_syncs = ctx.static_local_syncs;
        self.method_local_base = ctx.method_local_base;
        self.current_static_task = ctx.current_static_task;
        self.current_auto_task = ctx.current_auto_task;
        self.task_string_formals = ctx.task_string_formals;
    }

    /// Capture the current process context, including the current
    /// values of automatic `for`-loop variables (`auto_loop_vars`) by value into
    /// the child's context so each fork child sees the loop index at fork time
    /// (LRM §9.3.2). Used when spawning fork children.
    /// §21.2.1.7 / §9.3.5: a fork child runs inside the forking process's
    /// named blocks and inside the fork block itself, named or not, so its
    /// `%m` chain starts with the parent's chain (plus the fork's label).
    pub(super) fn inherit_fork_m_scope(
        &mut self,
        child: usize,
        parent: usize,
        fork_name: Option<&crate::ast::Identifier>,
    ) {
        // §24.3: a fork child of a program process is a program process.
        if !self.program_pids.is_empty() && self.program_pids.contains(&parent) {
            self.program_pids.insert(child);
        }
        if let Some(l) = self.process_m_label.get(&parent) {
            let l = l.clone();
            self.process_m_label.insert(child, l);
        }
        if self.m_scope_stack.is_empty() && fork_name.is_none() {
            return;
        }
        let mut chain = self.m_scope_stack.clone();
        if let Some(f) = fork_name {
            chain.push(f.name.clone());
        }
        self.process_m_scope.insert(child, chain);
    }

    /// `a[i][j]` -> (`a`, 2): the operand under its constant-or-not index
    /// selects (not part selects), and how many there are.
    pub(super) fn peel_index_selects(e: &Expression) -> Option<(Expression, usize)> {
        let mut k = 0usize;
        let mut cur = e;
        while let ExprKind::Index { expr, .. } = &cur.kind {
            k += 1;
            cur = expr;
        }
        if let ExprKind::Ident(h) = &cur.kind {
            let last = h.path.last()?;
            if !last.selects.is_empty() {
                if last
                    .selects
                    .iter()
                    .any(|x| matches!(x.kind, ExprKind::Range(..) | ExprKind::RangeSelect { .. }))
                {
                    return None;
                }
                k += last.selects.len();
                let mut hb = h.clone();
                hb.path.last_mut()?.selects.clear();
                hb.cached_signal_id = std::cell::Cell::new(None);
                hb.cached_resolved_name = std::cell::OnceCell::new();
                return Some((Expression::new(ExprKind::Ident(hb), cur.span), k));
            }
        }
        (k > 0).then(|| (cur.clone(), k))
    }

    pub(super) fn int_lit_expr(n: i64, span: crate::ast::Span) -> Expression {
        Expression::new(
            ExprKind::Number(crate::ast::expr::NumberLiteral::Integer {
                size: None,
                signed: true,
                base: crate::ast::expr::NumberBase::Decimal,
                value: n.to_string(),
                cached_val: Default::default(),
            }),
            span,
        )
    }

    /// §7.4.2 / §20.7: an object's store `scoped` of a fixed-array member
    /// with a descending dimension (`int ds[3:1]`) keeps its declared
    /// direction, as a module array does (`array_prop_decl_dims`).
    pub(super) fn note_member_decl_dims(
        &mut self,
        cdef: &crate::compiler::elaborate::ElaboratedClass,
        prop: &str,
        scoped: &str,
    ) {
        if let Some(dd) = cdef.array_prop_decl_dims.get(prop) {
            if dd.first().is_some_and(|&(l, r)| l > r) {
                self.module.descending_arrays.insert(scoped.to_string());
            }
            self.module
                .unpacked_decl_dims
                .insert(scoped.to_string(), dd.clone());
        }
    }

    pub(super) fn inherit_fork_child_context(&mut self, pid: usize) {
        let mut ctx = self.snapshot_process_context();
        // The parent's open block scopes close in the parent's stream, never
        // in the child's.
        ctx.block_scope_saves.clear();
        // §6.21/§9.3.2: automatic block-locals (no call frame) live in
        // `self.signals` and are listed in `auto_loop_vars`. Copy their
        // current VALUES into the child's `local_stack` so each child gets
        // its own snapshot (the child's reads consult its top frame first).
        // Record WHICH names were captured from `signals` so the propagate
        // path can write the child's changed values back into `self.signals`
        // — the storage the parent actually reads from. Without this
        // write-back, the child's writes are stranded in its private frame
        // and the parent sees the stale pre-fork value (§6.21 violation).
        let mut signal_caps: Vec<String> = Vec::new();
        if !self.auto_loop_vars.is_empty() {
            let mut caps: HashMap<String, Value> = HashMap::default();
            for nm in &self.auto_loop_vars {
                if let Some(v) = self.signals.get(nm) {
                    caps.insert(nm.clone(), v.clone());
                    signal_caps.push(nm.clone());
                }
            }
            if !caps.is_empty() {
                // Merge into the top frame if one exists (don't shadow the
                // forking process's other locals — reads consult only the top
                // frame); otherwise this becomes the child's first frame.
                if let Some(top) = ctx.local_stack.last_mut() {
                    for (k, v) in caps {
                        top.insert(k, v);
                    }
                } else {
                    ctx.local_stack.push(caps);
                    // A capture frame, not a call frame (see
                    // `in_process_block_scope`).
                    let g = self.next_frame_gen();
                    self.loop_frame_gens.insert(g);
                    ctx.local_gen_stack.push(g);
                }
            }
        }
        let trivial = ctx.this_stack.is_empty()
            && ctx.local_stack.is_empty()
            && ctx.class_context_stack.is_empty()
            && ctx.cg_this.is_none()
            && ctx.return_value.is_none()
            && !ctx.break_flag
            && !ctx.continue_flag
            && !ctx.return_flag
            && ctx.local_dyn.is_empty()
            && ctx.static_local_syncs.is_empty()
            && ctx.method_local_base.is_empty();
        if trivial {
            self.process_contexts.remove(&pid);
        } else {
            // §9.3.2 baseline: snapshot the child's local frames as forked so
            // `propagate_fork_locals_to_parent` can merge only the keys the
            // child later WRITES (value changes), not keys it merely inherited
            // (which the parent may have modified in the meantime — e.g. a
            // mailbox-get delivery into `phase` while this child runs).
            self.fork_baselines.insert(pid, ctx.local_stack.clone());
            self.fork_signal_captures.insert(pid, signal_caps);
            self.process_contexts.insert(pid, ctx);
        }
    }

    /// Drain events in the scheduler whose fire time is at or before `target`.
    /// Used when a task-internal `#delay` needs to yield the simulator so
    /// concurrent processes can advance while this task sleeps.
    /// §4.4.2.4 postponed region for ONE time slot: `$monitor`, `$strobe`,
    /// then the waveform dump. Factored out of the main tick so the nested
    /// `run_events_until` loop can service the slots it consumes. Every part
    /// is a no-op when nothing is armed or nothing changed, so calling it once
    /// per slot costs nothing on designs that dump or monitor nothing.
    pub(super) fn run_postponed_region(&mut self) {
        self.fire_vpi_synch_cbs();
        self.check_monitor();
        self.drain_pending_strobes();
        self.dump_write_changes();
    }

    /// A fresh wake marker for a wait made from the nested path: its id, a pid
    /// for the waiter to resume, and the statement that fires it.
    pub(super) fn new_sync_wake(&mut self) -> (u64, usize, Statement) {
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
                cached_val: Cell::new(Some((id, 0u64, 64u32))),
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
    pub(super) fn run_nested_until(&mut self, mut done: impl FnMut(&mut Self) -> bool) {
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
    pub(super) fn requeue_slot_batches(&mut self) {
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
    pub(super) fn end_run_waiting_in_c(&mut self, max_time: bool) {
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
    pub(super) fn nested_drain_condition_waiters(&mut self) {
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

    pub(super) fn run_events_until(&mut self, target: u64) {
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
    pub(super) fn reset_hint_to_process_scope(&self) {
        let h = self.process_scope_hint.get(&self.current_pid);
        let mut cur = self.name_resolve_hint.borrow_mut();
        if cur.as_deref() != h.map(|s| s.as_str()) {
            *cur = h.cloned();
        }
    }

    /// Remember that a procedural comb entry ran while a process was mid-flight,
    /// together with the outputs it produced. §4.5 puts such a block in the
    /// ACTIVE region — it should observe the process's FINAL state, not an
    /// intermediate one. xezim settles inline after every blocking assign (a
    /// deep, load-bearing behaviour: nets and primitives must propagate right
    /// away), so instead of reordering anything we simply detect the one case
    /// that inline evaluation gets wrong — the process later overwrote what the
    /// block computed — and re-run the block once the process suspends.
    /// Signal ids a compiled process body can write (sorted, deduped), or
    /// None when it holds array stores or AST fallbacks whose targets are
    /// not visible here. NBA targets are included: a superset only keeps a
    /// snapshot that was not needed.
    pub(super) fn compiled_block_write_ids(
        cb: &super::bytecode::CompiledBlock,
    ) -> Option<Vec<u32>> {
        use super::bytecode::Insn;
        let mut v: Vec<u32> = Vec::new();
        for i in &cb.instructions {
            match i {
                Insn::BlockingAssign(id, ..)
                | Insn::BlockingAssignString(id, _)
                | Insn::BlockingAssignRange(id, ..)
                | Insn::BlockingAssignRangeDyn(id, ..)
                | Insn::BlockingAssignBitDyn(id, ..)
                | Insn::NbaAssign(id, ..)
                | Insn::NbaAssignConst(id, ..)
                | Insn::NbaAssignRange(id, ..)
                | Insn::NbaAssignRangeDyn(id, ..)
                | Insn::NbaAssignBitDyn(id, ..)
                | Insn::NbaAssignArrayRead(id, ..) => v.push(*id as u32),
                Insn::StmtFallback(payload) => {
                    // Output-only system tasks (the fallbacks the FSM gate
                    // admits) write no signal; anything else is opaque.
                    let out_only = match &payload.0.kind {
                        StatementKind::Expr(e) => match &e.kind {
                            ExprKind::SystemCall { name, .. } => matches!(
                                name.as_str(),
                                "$display"
                                    | "$displayb"
                                    | "$displayh"
                                    | "$displayo"
                                    | "$write"
                                    | "$writeb"
                                    | "$writeh"
                                    | "$writeo"
                                    | "$strobe"
                                    | "$monitor"
                                    | "$fdisplay"
                                    | "$fwrite"
                                    | "$fstrobe"
                                    | "$finish"
                                    | "$stop"
                                    | "$info"
                                    | "$warning"
                                    | "$error"
                                    | "$fatal"
                            ),
                            _ => false,
                        },
                        _ => false,
                    };
                    if !out_only {
                        return None;
                    }
                }
                Insn::NbaAssignArray(..)
                | Insn::BlockingAssignArray(..)
                | Insn::NbaAssignArrayRange(..)
                | Insn::BlockingAssignArrayRange(..) => return None,
                _ => {}
            }
        }
        v.sort_unstable();
        v.dedup();
        Some(v)
    }

    pub(super) fn note_comb_ran_in_process(&mut self, eidx: usize, entry: &CombEntry) {
        if entry.cold.write_signal_ids.is_empty() {
            return;
        }
        // A process can only clobber what it writes: an FSM with a known
        // write set that misses every output of this entry needs no snapshot.
        if let Some(w) = &self.cur_fsm_writes {
            let key = w.as_ptr() as usize;
            if self.fsm_clobber_cache.len() <= eidx {
                self.fsm_clobber_cache.resize(eidx + 1, (0, true));
            }
            let can_clobber = if self.fsm_clobber_cache[eidx].0 == key {
                self.fsm_clobber_cache[eidx].1
            } else {
                let v = entry
                    .cold
                    .write_signal_ids
                    .iter()
                    .any(|id| w.binary_search(&(*id as u32)).is_ok());
                self.fsm_clobber_cache[eidx] = (key, v);
                v
            };
            if !can_clobber {
                return;
            }
        }
        // Last write wins: a block that runs several times in one process only
        // needs its most recent output compared. The snapshot vector is a
        // pooled one — a fresh allocation per comb entry per process run
        // was 2% of a small clocked testbench.
        let slot = match self.deferred_comb.iter().position(|(e, _)| *e == eidx) {
            Some(i) => i,
            None => {
                let v = self.deferred_comb_pool.pop().unwrap_or_default();
                self.deferred_comb.push((eidx, v));
                self.deferred_comb.len() - 1
            }
        };
        let snap = &mut self.deferred_comb[slot].1;
        snap.clear();
        for id in entry.cold.write_signal_ids.iter() {
            if *id < self.signal_table.len() {
                snap.push((*id, self.signal_table[*id].clone()));
            }
        }
        if snap.is_empty() {
            let (_, v) = self.deferred_comb.swap_remove(slot);
            self.deferred_comb_pool.push(v);
        }
    }

    /// Re-run the comb entries whose outputs the just-finished process
    /// clobbered. Nothing is re-run when the values still stand, so blocks with
    /// side effects (`$display`) are not duplicated in the common case.
    pub(super) fn drain_deferred_comb(&mut self) {
        if self.deferred_comb.is_empty() {
            return;
        }
        let mut pending = std::mem::take(&mut self.deferred_comb);
        for (eidx, snap) in pending.iter() {
            let eidx = *eidx;
            let clobbered = snap
                .iter()
                .any(|(id, v)| self.signal_table.get(*id).is_some_and(|cur| cur != v));
            if !clobbered {
                continue;
            }
            // Clone only the uncommon replayed entry. Keeping the canonical
            // table installed lets a blocking write settle recursively.
            let Some(entry) = self.comb_entries.get(eidx).cloned() else {
                continue;
            };
            match &entry.item {
                CombItem::AlwaysBlock { .. } => self.eval_ast_comb_entry(&entry),
                CombItem::CompiledAlwaysBlock { compiled, .. } => {
                    if self.vm_regs.len() < compiled.num_regs as usize {
                        self.vm_regs
                            .resize(compiled.num_regs as usize, Value::zero(1));
                    }
                    let insns = unsafe {
                        std::slice::from_raw_parts(
                            compiled.instructions.as_ptr(),
                            compiled.instructions.len(),
                        )
                    };
                    let saved_ts = self.timescale_scope_override.take();
                    self.timescale_scope_override = entry.cold.scope_hint.clone();
                    let saved_hint = if compiled.has_fallback {
                        let prev = self.name_resolve_hint.borrow().clone();
                        if let Some(sc) = entry.cold.scope_hint.as_ref() {
                            *self.name_resolve_hint.borrow_mut() = Some(sc.clone());
                        }
                        Some(prev)
                    } else {
                        None
                    };
                    self.exec_insns(insns);
                    if let Some(prev) = saved_hint {
                        *self.name_resolve_hint.borrow_mut() = prev;
                    }
                    self.timescale_scope_override = saved_ts;
                }
                _ => {}
            }
        }
        for (_, v) in pending.iter_mut() {
            v.clear();
        }
        self.deferred_comb_pool
            .extend(pending.into_iter().map(|(_, v)| v));
    }

    /// §4.5 boundary: run the process, then let the ACTIVE-region work its
    /// blocking writes scheduled actually execute. Deferring the settle to
    /// here (rather than firing it inline after every blocking assign) is what
    /// keeps a triggered `always @(*)` from running in the middle of another
    /// process's statement sequence.
    pub(super) fn run_scheduled_process(&mut self, pid: usize, stmts: &ProcCont) {
        // §24.3 / §4.4: a program process woken outside the Reactive region
        // resumes there, after this slot's NBA updates and clocking events.
        if !self.program_pids.is_empty()
            && !self.in_reactive_drain
            && self.program_pids.contains(&pid)
        {
            self.reactive_conts.push((pid, stmts.clone()));
            self.late_resume_epoch += 1;
            return;
        }
        // §21.2.1.7: swap in THIS process's lexical `%m` chain for the run and
        // park it again on the way out. The single entry/exit here is why the
        // save/restore lives in the outer wrapper — `_inner` has several early
        // returns.
        let outer_m_scope = std::mem::replace(
            &mut self.m_scope_stack,
            self.process_m_scope.remove(&pid).unwrap_or_default(),
        );

        // A PROCESS is never "inside" an edge block, whatever stack frame it
        // happens to be serviced from. A fork child spawned by an edge block
        // and serviced while that block's frame was still live inherited
        // `in_edge_block=true` for the REST OF THE RUN once its `#delay`
        // trapped the event loop in run_events_until — every subsequent
        // signal write then took the edge-rescan path instead of ordinary
        // edge detection (dirty_edge never set again), so no always block
        // fired anymore: the design froze while the child spun, and $finish
        // (a dead always block) could never come. Clear both flags for the
        // process's execution and restore on the way out.
        // `in_edge_cont` must SURVIVE into the servicing: waiter/inline
        // continuations set it just before resuming so their writes join the
        // §9.2 rescan (see waiter_cont_anyedge_wake). Only the edge-BLOCK
        // flag is cleared here.
        let saved_in_edge_block = std::mem::replace(&mut self.in_edge_block, false);

        self.proc_depth += 1;
        self.run_scheduled_process_inner(pid, stmts);
        self.proc_depth -= 1;
        self.in_edge_block = saved_in_edge_block;
        if self.proc_depth == 0 {
            self.drain_deferred_comb();
            self.drain_timing_pending();
        }

        let mine = std::mem::replace(&mut self.m_scope_stack, outer_m_scope);
        // Non-empty ⇒ suspended mid-task; keep it for the resume. A process
        // that ran to completion unwound its own pushes and parks nothing.
        if !mine.is_empty() {
            self.process_m_scope.insert(pid, mine);
        }
    }

    pub(super) fn run_scheduled_process_inner(&mut self, pid: usize, stmts: &ProcCont) {
        // Flags from a prior process must not leak into this one.
        self.break_flag = false;
        self.return_flag = false;
        self.continue_flag = false;
        self.parked_from_exec = false;
        // Consumed by whichever path runs this entry (see the direct-run
        // branch below).
        let trampolined = !self.trampolined_pids.is_empty() && self.trampolined_pids.remove(&pid);
        // A process terminated by `disable fork` (LRM §9.6.2) must not run any
        // remaining queued continuation (e.g. the resume of a `#delay` it was
        // parked on when killed). Pids are monotonic and never reused, so it is
        // safe to leave the entry in `killed_pids` permanently. This is the one
        // chokepoint every dispatch site funnels through.
        if self.killed_pids.contains(&pid) {
            return;
        }
        // A stale `disable` target would keep later loops from consuming
        // their own `break`; a disable posted by another process (§9.6.2)
        // starts this activation's unwind instead.
        self.disable_target = None;
        self.disable_check_pending = false;
        // A freshly-activated scheduled process must resolve unqualified names
        // from a clean slate. `name_resolve_hint` is a transient sibling-scope
        // hint set while resolving DOTTED names (resolve_hier_name records the
        // parent scope); it is NOT part of the per-process ProcessContext. If a
        // prior process (e.g. a monitor touching `ivif.clk`) left it set to
        // `ivif`, this process's bare top-level names (e.g. the clock gen's
        // `clk`) would mis-resolve to `ivif.clk` — freezing the real `clk` net.
        // Save + clear it for the run, restore on EVERY exit path so the hint
        // never leaks across scheduled processes (or back to the caller).
        let saved_hint = self.name_resolve_hint.borrow_mut().take();
        // Fork-capture names (`auto_loop_vars`) are scoped to one process
        // activation. Structured statements pop their own pushes, but a
        // blocking `begin/end` gets FLATTENED into the process's statement
        // stream (see run_process_stmts), so an `automatic` local declared
        // there is pushed with no enclosing SeqBlock arm to pop it. Truncate
        // on both exits so nothing leaks into other processes — such a var
        // stays capturable for the rest of its (flattened) block, which is
        // exactly its scope.
        let saved_auto_len = self.auto_loop_vars.len();
        // Install the process's instance scope (from its initial block) as the
        // resolution hint so AST-evaluated bare names — e.g. a
        // `std::randomize(sig)` target — resolve to THIS instance's signals in
        // a multiply-instantiated module, not the first instance's.
        if let Some(scope) = self.process_scope_hint.get(&pid) {
            let hint = Self::pooled_string(&mut self.hint_string_pool, scope);
            *self.name_resolve_hint.borrow_mut() = Some(hint);
            self.current_scope.clone_from(scope);
        } else {
            self.current_scope.clear();
        }
        // A suspended-and-resumed process running INSIDE an inlined
        // hierarchical/interface task must keep resolving the body's bare
        // names under the task's instance scope, not the process's own.
        if let Some(hint) = self
            .task_cleanup
            .last()
            .and_then(|c| c.frame_scope_hint.clone())
        {
            *self.name_resolve_hint.borrow_mut() = Some(hint);
        }
        // A process is not a comb/edge block, so the scope the settle loop
        // last recorded must not leak into this process's `%m`.
        self.m_block_scope.clear();
        self.m_block_scope_id = u32::MAX;
        self.m_block_active = false;
        // This retained payload is one non-suspending blocking assignment.
        // Isolate it from a caller task's locals by moving that context aside;
        // the generic path clones the entire context because arbitrary
        // continuations may suspend, which is unnecessary for this shape.
        if stmts.is_empty() && self.fast_delay_always.contains_key(&pid) {
            let saved = self.take_process_context();
            self.run_fast_delay_always(pid);
            self.restore_process_context(saved);
            self.auto_loop_vars.truncate(saved_auto_len);
            let mine = self.name_resolve_hint.replace(saved_hint);
            Self::recycle_string(&mut self.hint_string_pool, mine);
            return;
        }
        // A compiled process FSM carries its whole state in its frame — the
        // AST-context save/restore dance is unnecessary, same as fast-delay.
        if stmts.is_empty() && self.proc_fsm.contains_key(&pid) {
            // A top-level FSM wake-up starts from an empty context; taking
            // and restoring the 18 fields moved ~10% of this function's
            // time for nothing. Reset only if the run left something behind.
            if self.process_context_is_default() {
                self.run_proc_fsm(pid);
                if !self.process_context_is_default() {
                    self.restore_process_context(ProcessContext::default());
                }
            } else {
                let saved = self.take_process_context();
                self.run_proc_fsm(pid);
                self.restore_process_context(saved);
            }
            self.auto_loop_vars.truncate(saved_auto_len);
            let mine = self.name_resolve_hint.replace(saved_hint);
            Self::recycle_string(&mut self.hint_string_pool, mine);
            return;
        }
        // Fast path: if we have no saved process context for this pid AND
        // the caller's execution context is empty, skip the full snapshot /
        // restore dance. Forever-loop bodies like `jclk = ~jclk` that run
        // with no locals don't need context bookkeeping; each call paid
        // several `Vec<HashMap<String, Value>>`-level clones for nothing.
        let saved_ctx_needed = !self.this_stack.is_empty()
            || !self.local_stack.is_empty()
            || !self.class_context_stack.is_empty()
            || !self.block_scope_saves.is_empty()
            || self.current_spec.is_some()
            || !self.spec_scope_stack.is_empty();
        let has_pid_ctx = self.process_contexts.contains_key(&pid);
        if !saved_ctx_needed && !has_pid_ctx {
            // This path skips the context dance, so the per-method locals base
            // is moved aside by hand: a process parked inside an inlined
            // blocking method leaves its own base behind, and running another
            // process under it would bound that process's frame-local lookups
            // by an index into a `local_stack` it does not own.
            let saved_mlb = std::mem::take(&mut self.method_local_base);
            self.run_process_payload(pid, stmts);
            let susp = self.is_pid_suspended(pid);
            if susp {
                // Only snapshot if actually suspended and has state worth saving.
                if !self.this_stack.is_empty()
                    || !self.local_stack.is_empty()
                    || !self.class_context_stack.is_empty()
                    || !self.method_local_base.is_empty()
                    || !self.block_scope_saves.is_empty()
                {
                    self.process_contexts
                        .insert(pid, self.snapshot_process_context());
                }
            }
            self.method_local_base = saved_mlb;
            self.auto_loop_vars.truncate(saved_auto_len);
            let mine = self.name_resolve_hint.replace(saved_hint);
            Self::recycle_string(&mut self.hint_string_pool, mine);
            return;
        }
        // A continuation that `continue_stmts_or_trampoline` rescheduled
        // (its `run_process_stmts` recursion crossed `RPS_TRAMPOLINE_DEPTH`)
        // with its working context still LIVE on the simulator and not parked
        // in `process_contexts`. Its `this`/frames must stay in place for the
        // body (e.g. a `foreach (m_fields[i])` on a UVM register that resolves
        // `this`). The fork-child dance below would `take_process_context()`
        // the live frames, restore an EMPTY table entry (this pid was never
        // parked there), run the body context-less, and only put `this` back
        // at the end: the reads already saw null, a null dereference. Run the
        // payload directly so the live context is preserved, parking a
        // snapshot only if the body suspends. Only for a pid marked when it
        // trampolined, with a method context live (`method_local_base`) and
        // no fork parent: nothing else is known to own the live context.
        if trampolined
            && saved_ctx_needed
            && !has_pid_ctx
            && !self.method_local_base.is_empty()
            && self.process_parents.get(&pid).is_none()
        {
            self.run_process_payload(pid, stmts);
            let susp = self.is_pid_suspended(pid);
            if susp
                && (!self.this_stack.is_empty()
                    || !self.local_stack.is_empty()
                    || !self.class_context_stack.is_empty()
                    || !self.method_local_base.is_empty()
                    || !self.block_scope_saves.is_empty())
            {
                self.process_contexts
                    .insert(pid, self.snapshot_process_context());
            } else {
                self.process_contexts.remove(&pid);
            }
            self.auto_loop_vars.truncate(saved_auto_len);
            let mine = self.name_resolve_hint.replace(saved_hint);
            Self::recycle_string(&mut self.hint_string_pool, mine);
            return;
        }
        // The caller's context is MOVED aside (the restore below overwrites
        // every field, so a clone bought nothing) and moved back at the end.
        let mut saved = self.take_process_context();
        let ctx = self.process_contexts.remove(&pid).unwrap_or_default();
        self.restore_process_context(ctx);
        self.run_process_payload(pid, stmts);
        let suspended = self.is_pid_suspended(pid);
        // The process's own context is moved out once; its frames are read
        // for the fork merge below and the whole context is parked in the
        // table only if the process suspended.
        let child_ctx = self.take_process_context();
        // IEEE 1800-2023 §6.21/§9.3.2: automatic variables declared in the
        // parent scope are SHARED with fork children — a child's write must be
        // visible to the parent. xezim gives each fork child a COPY of the
        // parent's locals (inherit_fork_child_context), so the child's writes
        // live in its private copy and must be propagated back. Two storage
        // models must be bridged:
        //   (a) SUBROUTINE locals — live in a `local_stack` frame; merged into
        //       the parent's frame (in `process_contexts` if suspended, or in
        //       `saved` if the parent is the currently-active process whose
        //       context is on THIS Rust stack — e.g. the child ran inside the
        //       parent's own `#delay` via `run_events_until`).
        //   (b) PROCEDURAL block-locals (`initial`/`always`, no call frame) —
        //       live in the global `self.signals`; `inherit_fork_child_context`
        //       copied them into the child's frame and recorded the names in
        //       `fork_signal_captures`. Write the child's changed values back
        //       into `self.signals`, which is what the parent reads from.
        let child_frames: &[HashMap<String, Value>] = &child_ctx.local_stack;
        let child_frames_gen: &[u64] = &child_ctx.local_gen_stack;
        let baseline = self.fork_baselines.get(&pid);
        let signal_caps = self.fork_signal_captures.get(&pid);
        // Parent-frame keys the child's writes reached, for the wake below.
        let mut written: Vec<String> = Vec::new();
        if !child_frames.is_empty() {
            if let Some(parent_pid) = self.process_parents.get(&pid).copied() {
                // (a) subroutine-frame merge
                written = if let Some(parent_ctx) = self.process_contexts.get_mut(&parent_pid) {
                    Self::merge_fork_writes(
                        &mut parent_ctx.local_stack,
                        &parent_ctx.local_gen_stack,
                        child_frames,
                        child_frames_gen,
                        baseline,
                    )
                } else {
                    // Parent is the active process — its context is `saved`.
                    Self::merge_fork_writes(
                        &mut saved.local_stack,
                        &saved.local_gen_stack,
                        child_frames,
                        child_frames_gen,
                        baseline,
                    )
                };
            }
        }
        // (b) signal-capture write-back: procedural block-locals that live in
        // `self.signals`. Write only keys the child CHANGED (relative to the
        // fork-time baseline) so an inherited-unchanged value doesn't
        // clobber a sibling's or the parent's concurrent write.
        if let Some(caps) = signal_caps {
            let top = child_frames.last();
            for nm in caps {
                let Some(v) = top.and_then(|f| f.get(nm)) else {
                    continue;
                };
                let inherited_unchanged = baseline
                    .and_then(|b| b.last())
                    .and_then(|f| f.get(nm))
                    .is_some_and(|old| old == v);
                if !inherited_unchanged {
                    self.signals.insert(nm.clone(), v.clone());
                }
            }
        }
        if !written.is_empty() {
            // A child wrote a key a parked `wait` of the parent reads —
            // promote it to ready so a long-running edge cascade (which
            // drains only READY waiters) unblocks the handshake instead
            // of starving it.
            self.promote_condition_waiters_for_names(&written);
        }
        if suspended {
            self.process_contexts.insert(pid, child_ctx);
        } else {
            self.process_contexts.remove(&pid);
            self.fork_baselines.remove(&pid);
            self.fork_signal_captures.remove(&pid);
        }
        self.restore_process_context(saved);
        self.auto_loop_vars.truncate(saved_auto_len);
        *self.name_resolve_hint.borrow_mut() = saved_hint;
    }

    /// Dispatch a scheduled payload, recognizing the empty marker used by a
    /// retained dynamic-delay always assignment.
    pub(super) fn run_process_payload(&mut self, pid: usize, stmts: &ProcCont) {
        // §9.6.2: a disable posted by another process (`disable_remote_block`)
        // starts this activation's unwind — after the process context (and
        // its saved flags) is back in place.
        if !self.pending_block_disable.is_empty() {
            if let Some(x) = self.pending_block_disable.remove(&pid) {
                self.disable_target = Some(x);
                self.break_flag = true;
            }
        }
        if stmts.is_empty() && self.fast_delay_always.contains_key(&pid) {
            self.run_fast_delay_always(pid);
        } else if stmts.is_empty() && self.proc_fsm.contains_key(&pid) {
            self.run_proc_fsm(pid);
        } else {
            self.run_process_stmts(pid, stmts);
        }
        // #35: the process just YIELDED (parked or finished) — the
        // cont-assign update events its writes scheduled become runnable
        // now (§10.3), so the deferred entries drain before anything else
        // in the slot observes the nets.
        if !self.deferred_proc_entries.is_empty() && !self.proc_settle_defer {
            // The drained entries must evaluate in a NEUTRAL scope: the
            // process's name-resolve hint (its instance path) would anchor a
            // same-named cont-assign LHS into the instance — the u_h.t
            // self-assign hazard (see tests/hierarchy/same_name_port_hop.rs).
            let saved_hint = self.name_resolve_hint.borrow_mut().take();
            self.settle_combinatorial();
            *self.name_resolve_hint.borrow_mut() = saved_hint;
        }
    }

    /// §3.14.3 delay quantization for a process-FSM WaitDelayReg value —
    /// the register-value half of `delay_value_to_ticks`, without the
    /// pathological-real diagnostics (no source expression at hand).
    pub(super) fn fsm_delay_ticks(&self, v: &Value, precision_diff: i32) -> u64 {
        let raw = if v.is_real {
            let f = v.to_f64();
            if f.is_finite() && f > 0.0 { f } else { 0.0 }
        } else {
            v.to_u64().unwrap_or(0) as f64
        };
        if precision_diff > 0 {
            let q = 10f64.powi(precision_diff);
            let x = (raw / q) * (1.0 + 1e-12);
            (x.round() * q).round() as u64
        } else {
            raw.round() as u64
        }
    }

    /// Roadmap step 13 (XEZIM_AOT=1): compile every process FSM body that
    /// the rustc emitter covers into ONE native dylib and install the fn
    /// pointers. Bodies the emitter rejects (or that touch real-typed /
    /// >64-bit signals) stay on the bytecode executor.
    #[cfg(feature = "jit")]
    pub(super) fn aot_compile_proc_fsms(&mut self) {
        if self.proc_fsm.is_empty()
            || !std::env::var("XEZIM_AOT")
                .map(|v| v == "1")
                .unwrap_or(false)
            || !std::env::var("XEZIM_JIT")
                .map(|v| v != "0" && !v.is_empty())
                .unwrap_or(false)
        {
            return;
        }
        use super::bytecode::Insn;
        let verbose = std::env::var("XEZIM_JIT_VERBOSE").is_ok();
        let mut names: Vec<(usize, String)> = Vec::new();
        let mut fns_src: Vec<String> = Vec::new();
        let pids: Vec<usize> = self.proc_fsm.keys().copied().collect();
        if verbose {
            eprintln!("[AOT] fsm pass over {} process FSMs", pids.len());
        }
        for pid in pids {
            let f = &self.proc_fsm[&pid];
            // Signal gate only: >64-bit or real-typed signals stay on the
            // bytecode executor (the u64 planes would truncate / misread
            // f64 bit patterns). Real CONSTANTS are fine — `gen_fsm_fn`
            // itself proves a Real tick literal feeds only a delay wait and
            // rejects any other real const (unlike the shared
            // `block_signals_fit_u64`, whose real-const arm would throw the
            // elaboration-quantized delay literals out wholesale).
            let bad_signal = f.compiled.instructions.iter().any(|i| {
                let sig = match i {
                    Insn::LoadSignal(_, sig)
                    | Insn::LoadSignalSigned(_, sig)
                    | Insn::LoadSignalRange(_, sig, _, _)
                    | Insn::LoadSignalBit(_, sig, _)
                    | Insn::BranchIfSignalFalse(sig, _, _)
                    | Insn::BlockingAssign(sig, _, _)
                    | Insn::BlockingAssignRange(sig, _, _, _)
                    | Insn::BlockingAssignRangeDyn(sig, _, _, _)
                    | Insn::BlockingAssignBitDyn(sig, _, _)
                    | Insn::NbaAssign(sig, _, _)
                    | Insn::NbaAssignConst(sig, _, _)
                    | Insn::NbaAssignRange(sig, _, _, _)
                    | Insn::NbaAssignRangeDyn(sig, _, _, _)
                    | Insn::NbaAssignBitDyn(sig, _, _)
                    | Insn::NbaAssignArrayRead(sig, _, _, _) => Some(*sig as usize),
                    Insn::LoadArrayElem(_, arr, _)
                    | Insn::BlockingAssignArray(arr, _, _, _)
                    | Insn::NbaAssignArray(arr, _, _, _)
                    | Insn::NbaAssignArrayRange(arr, ..)
                    | Insn::BlockingAssignArrayRange(arr, ..) => match arr.as_ref() {
                        super::bytecode::ArrayOperand::Dense { first_id, .. } => Some(*first_id),
                        _ => None,
                    },
                    _ => None,
                };
                match sig {
                    Some(id) => {
                        // A packed-arena memory has no table slot for the
                        // native code's id arithmetic (the `u32` id would
                        // alias an unrelated signal).
                        is_packed_id(id)
                            || self.signal_widths.get(id).copied().unwrap_or(0) > 64
                            || self.signal_real.get(id).copied().unwrap_or(false)
                            || self
                                .signal_table
                                .get(id)
                                .map(|v| v.is_real)
                                .unwrap_or(false)
                    }
                    None => false,
                }
            });
            let packed_read = f.compiled.instructions.iter().any(|i| {
                matches!(i, Insn::NbaAssignArrayRead(_, arr, _, _)
                    if matches!(arr.as_ref(), super::bytecode::ArrayOperand::Dense { first_id, .. }
                        if is_packed_id(*first_id)))
            });
            if bad_signal || packed_read {
                if verbose {
                    eprintln!("[AOT-BAIL-FSM] pid={} (signal gate)", pid);
                }
                continue;
            }
            let name = format!("xezim_aot_fsm_{pid}");
            if let Some(src) = super::aot::gen_fsm_fn(
                &name,
                &f.compiled.instructions,
                f.compiled.num_regs,
                &self.signal_widths,
                &self.signal_signed,
            ) {
                names.push((pid, name));
                fns_src.push(src);
            } else if verbose {
                eprintln!("[AOT-BAIL-FSM] pid={}", pid);
            }
        }
        if fns_src.is_empty() {
            return;
        }
        let ordered: Vec<String> = names.iter().map(|(_, n)| n.clone()).collect();
        let src = super::aot::module_source_named(&fns_src, &ordered);
        let planes = (
            self.signal_inline_bits.as_ptr() as u64,
            self.signal_inline_bits.len() as u32,
        );
        if let Some(lib) = super::aot::compile_and_load(&src, verbose, planes) {
            let mut n = 0usize;
            if let Some(api) = lib.api() {
                for (k, (pid, _)) in names.iter().enumerate() {
                    let p = api(k as u32);
                    if !p.is_null() {
                        if let Some(f) = self.proc_fsm.get_mut(pid) {
                            f.native = Some(unsafe {
                                std::mem::transmute::<*mut u8, super::aot::AotFsmFn>(p)
                            });
                            f.native_frame = vec![0u64; 2 * f.compiled.num_regs as usize];
                            n += 1;
                        }
                    }
                }
            } else {
                for (pid, name) in &names {
                    if let Some(fp) = lib.sym_fsm(name) {
                        if let Some(f) = self.proc_fsm.get_mut(pid) {
                            f.native = Some(fp);
                            f.native_frame = vec![0u64; 2 * f.compiled.num_regs as usize];
                            n += 1;
                        }
                    }
                }
            }
            std::mem::forget(lib);
            chatter!("[AOT] process FSMs compiled {}/{}", n, self.proc_fsm.len());
        }
    }

    /// Run a compiled process FSM activation: resume the bytecode at the
    /// frame's pc with the frame's registers, park on the next wait point
    /// (delay -> timing wheel, edge -> event waiter, both with the empty
    /// payload marker), then settle — the same scheduling boundary
    /// `run_fast_delay_always` maintains.
    /// Lower an FSM body to a two-state stream with wait opcodes; None keeps
    /// the body on the four-state VM (wide registers, dynamic delays, any
    /// opcode the lowering declines).
    pub(super) fn lower_proc_fsm_ts(
        &self,
        cb: &super::bytecode::CompiledBlock,
    ) -> Option<Box<ProcFsmTs>> {
        if std::env::var("XEZIM_PROC_FSM_TS").as_deref() == Ok("0") {
            return None;
        }
        let block = super::bytecode::lower_two_state(
            cb,
            &self.signal_widths,
            &self.signal_signed,
            &self.signal_real,
            &self.array_first_id,
        )?;
        if block.has_wide || !block.has_wait {
            return None;
        }
        let n = block.num_regs as usize;
        Some(Box::new(ProcFsmTs {
            block,
            regs: vec![0; n],
            valid: false,
            snap: vec![0; n],
            fails: 0,
            resumes: 0,
            skip_until: 0,
        }))
    }

    /// Run one wait-to-wait segment of an FSM on the two-state executor.
    /// Returns false when the segment must run on the VM instead (bail,
    /// x-holding registers at the resume point, or back-off); the VM
    /// registers are then the values the segment started from.
    pub(super) fn run_proc_fsm_ts_segment(&mut self, f: &mut ProcFsm) -> bool {
        let Some(ts) = f.ts.as_mut() else {
            return false;
        };
        ts.resumes = ts.resumes.wrapping_add(1);
        if ts.resumes < ts.skip_until {
            return false;
        }
        let vm_pc = f.pc as usize;
        let start = if vm_pc == 0 {
            0
        } else {
            match ts.block.vm_to_ts.get(vm_pc) {
                Some(&t) => t as usize,
                None => return false,
            }
        };
        if !ts.valid {
            // The VM ran the previous segment: import every register the
            // stream defines at this resume point; an x or wide value keeps
            // the segment on the VM.
            if vm_pc != 0 {
                let Some((_, regs)) = ts
                    .block
                    .wait_regs
                    .iter()
                    .find(|(pc, _)| *pc as usize == vm_pc)
                else {
                    return false;
                };
                for &(r, w, _) in regs.iter() {
                    let v = &self.vm_regs[r as usize];
                    if w > 64 || v.has_xz() || v.is_real {
                        return false;
                    }
                    let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
                    ts.regs[r as usize] = v.to_u64().unwrap_or(0) & m;
                }
            }
            ts.valid = true;
        }
        ts.snap.copy_from_slice(&ts.regs);
        std::mem::swap(&mut self.ts_regs, &mut ts.regs);
        let save_base = self.ts_save_list.len();
        self.ts_cur_eidx = u32::MAX;
        self.fsm_suspend = None;
        let insns: *const [super::bytecode::TsInsn] = &ts.block.insns[..];
        let num_regs = ts.block.num_regs;
        // SAFETY: the stream lives in `f.ts`, which nothing below touches.
        let ok = self.exec_two_state_ctrl_from(unsafe { &*insns }, num_regs, start);
        std::mem::swap(&mut self.ts_regs, &mut ts.regs);
        if ok {
            self.ts_save_list.truncate(save_base);
            ts.fails = 0;
            self.prof_ts_evals += 1;
            // The runner armed the VM's start pc for this segment; nothing
            // consumed it, and `exec_insns` takes it on its next call from
            // ANY caller, so it must not leak into an edge block.
            self.fsm_start_pc = 0;
            return true;
        }
        // Bail: undo the segment's hazard stores, hand the segment-start
        // values to the VM, and back off after repeated failures.
        self.ts_restore_saved(save_base);
        self.ts_xread_bail = false;
        self.ts_exec_aborted = false;
        self.fsm_suspend = None;
        if let Some((_, regs)) = ts
            .block
            .wait_regs
            .iter()
            .find(|(pc, _)| *pc as usize == vm_pc)
        {
            for &(r, w, sg) in regs.iter() {
                let m = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
                let mut v = Value::from_u64(ts.snap[r as usize] & m, w);
                v.is_signed = sg;
                self.vm_regs[r as usize] = v;
            }
        }
        ts.valid = false;
        ts.fails += 1;
        if ts.fails >= 4 {
            let back = 1u32 << (ts.fails.min(14));
            ts.skip_until = ts.resumes.wrapping_add(back);
        }
        false
    }

    pub(super) fn run_proc_fsm(&mut self, pid: usize) {
        // The FSM gate admits only system-task statement fallbacks
        // (`$display` and friends); they are the design, not a compile
        // gap, so they must not show up in the edge path's `fallbacks=`
        // coverage statistic.
        let fallbacks_before = self.prof_fallback_insns;
        self.run_proc_fsm_inner(pid);
        self.prof_fallback_insns = fallbacks_before;
    }

    /// Signal-id terms of FSM wait `ix`, resolved on its first suspension and
    /// reused after: re-resolving hashed every term's name again each time
    /// the process parked on its clock. Left uncached while a term names no
    /// signal yet or the wake ranks are not built, so a later suspension
    /// resolves afresh exactly as before.
    pub(super) fn fsm_wait_terms(&mut self, f: &mut ProcFsm, ix: usize) -> Vec<SensitivityId> {
        if let Some(terms) = &f.waits[ix].2 {
            return terms.clone();
        }
        let slot = &mut f.waits[ix];
        if slot.1.is_none() {
            slot.1 = Some(self.event_to_sens(&slot.0));
        }
        let sens = slot.1.as_deref().unwrap_or_default();
        let terms = self.resolve_sens_ids(sens);
        if terms.len() == sens.len() && self.wake_rank_built {
            slot.2 = Some(terms.clone());
        }
        terms
    }

    pub(super) fn run_proc_fsm_inner(&mut self, pid: usize) {
        self.current_pid = pid;
        self.flush_deferred_asserts(pid);
        // Same per-process zero-delay guard as `run_process_stmts` /
        // `run_fast_delay_always`: a `#0` wait parks the FSM on the inactive
        // queue, which the scheduler drains inside one delta, so the outer
        // loop's stall counter never sees `initial forever begin #0; ... end`
        // spin — the run_proc_fsm path hung the stall-report test.
        if self.stall_limit > 0 {
            let hits = self.stall_pid_hits.entry(pid).or_insert(0);
            if *hits >= self.stall_limit {
                self.event_queue.schedule(self.time, pid, Vec::new().into());
                self.zero_delay_defer_pending = true;
                return;
            }
            *hits += 1;
        }
        let Some(mut f) = self.proc_fsm.remove(&pid) else {
            return;
        };
        if f.regs.len() < f.compiled.num_regs as usize {
            f.regs.resize(f.compiled.num_regs as usize, Value::zero(1));
        }
        std::mem::swap(&mut self.vm_regs, &mut f.regs);
        let saved_hint = self.name_resolve_hint.borrow_mut().take();
        if !f.scope.is_empty() {
            let hint = Self::pooled_string(&mut self.hint_string_pool, &f.scope);
            *self.name_resolve_hint.borrow_mut() = Some(hint);
        } else {
            *self.name_resolve_hint.borrow_mut() = saved_hint.clone();
        }
        let pd = *f.precision_diff.get_or_insert_with(|| {
            let (_, prec_exp) = {
                let def = self.current_module_def();
                self.module
                    .module_timescale_exp
                    .get(def)
                    .copied()
                    .unwrap_or((self.module.timeunit_exp, self.module.timeprecision_exp))
            };
            prec_exp - Self::secs_to_exp(self.tick_s)
        });
        self.fsm_precision_diff = pd;
        // Roadmap step 13: the native form suspends by RETURNING — no VM
        // registers involved (its own live in `native_frame`).
        #[cfg(feature = "jit")]
        if let Some(nf) = f.native {
            let mut out = [0u64; 4];
            let mut passes = 0u32;
            loop {
                let sim_ptr = self as *mut Self as *mut u8;
                let npc =
                    unsafe { nf(sim_ptr, f.pc, f.native_frame.as_mut_ptr(), out.as_mut_ptr()) };
                match out[0] {
                    1 => {
                        let v = Value::from_u64(out[2] & !out[3], 64);
                        let t = self.fsm_delay_ticks(&v, pd);
                        if t == 0 {
                            self.inactive_queue.push((pid, ProcCont::empty()));
                        } else {
                            self.event_queue.schedule(
                                self.time.saturating_add(t),
                                pid,
                                Vec::new().into(),
                            );
                        }
                        f.pc = npc;
                        break;
                    }
                    2 => {
                        let ix = out[1] as usize;
                        let terms = self.fsm_wait_terms(&mut f, ix);
                        let clk = f.wait_is_clocking.get(ix).copied().unwrap_or(false);
                        if clk && self.clocking_event_due_now(&f.waits[ix].0) {
                            self.defer_due_clocking_wait(pid, terms, ProcCont::empty());
                        } else {
                            let w =
                                self.make_event_waiter_resolved(pid, terms, ProcCont::empty(), clk);
                            self.event_waiters.push(w);
                        }
                        f.pc = npc;
                        break;
                    }
                    _ => {
                        if self.finished {
                            break;
                        }
                        if !f.is_always {
                            Self::recycle_string(
                                &mut self.hint_string_pool,
                                self.name_resolve_hint.replace(saved_hint.clone()),
                            );
                            self.settle_combinatorial();
                            return;
                        }
                        f.pc = 0;
                        passes += 1;
                        if passes > 1000 {
                            eprintln!(
                                "[xezim][error] native process FSM (pid {}) looped {}                                  times without a wait — parking it",
                                pid, passes
                            );
                            break;
                        }
                    }
                }
            }
            Self::recycle_string(
                &mut self.hint_string_pool,
                self.name_resolve_hint.replace(saved_hint.clone()),
            );
            self.proc_fsm.insert(pid, f);
            self.settle_combinatorial();
            return;
        }
        // An `always` body that completes without suspending restarts at
        // pc 0; the guard bounds a wait-free pass count so a body whose
        // waits are all branched around cannot livelock the scheduler.
        let mut passes = 0u32;
        loop {
            self.fsm_start_pc = f.pc;
            self.fsm_suspend = None;
            if self.profile_report {
                self.prof_cur.store(
                    crate::compiler::prof_sampler::KIND_PROC | pid as u64,
                    std::sync::atomic::Ordering::Relaxed,
                );
            }
            self.cur_fsm_writes = f.writes.clone();
            if !self.run_proc_fsm_ts_segment(&mut f) {
                self.exec_insns(&f.compiled.instructions);
                if let Some(ts) = f.ts.as_mut() {
                    ts.valid = false;
                }
            }
            if self.profile_report {
                self.prof_cur.store(
                    crate::compiler::prof_sampler::KIND_OTHER,
                    std::sync::atomic::Ordering::Relaxed,
                );
            }
            if let Some((npc, kind)) = self.fsm_suspend.take() {
                f.pc = npc;
                match kind {
                    FsmWait::Delay(t) => {
                        if t == 0 {
                            self.inactive_queue.push((pid, ProcCont::empty()));
                        } else {
                            self.event_queue.schedule(
                                self.time.saturating_add(t),
                                pid,
                                Vec::new().into(),
                            );
                        }
                    }
                    FsmWait::Edge(ix) => {
                        let terms = self.fsm_wait_terms(&mut f, ix as usize);
                        // §14.13: a clocking event resumes after the block samples.
                        let clk = f
                            .wait_is_clocking
                            .get(ix as usize)
                            .copied()
                            .unwrap_or(false);
                        if clk && self.clocking_event_due_now(&f.waits[ix as usize].0) {
                            self.defer_due_clocking_wait(pid, terms, ProcCont::empty());
                        } else {
                            let w =
                                self.make_event_waiter_resolved(pid, terms, ProcCont::empty(), clk);
                            self.event_waiters.push(w);
                        }
                    }
                }
                break;
            }
            if self.finished {
                break;
            }
            if !f.is_always {
                // An `initial` body ran to completion: done for good.
                std::mem::swap(&mut self.vm_regs, &mut f.regs);
                Self::recycle_string(
                    &mut self.hint_string_pool,
                    self.name_resolve_hint.replace(saved_hint.clone()),
                );
                self.settle_combinatorial();
                self.cur_fsm_writes = None;
                return;
            }
            f.pc = 0;
            passes += 1;
            if passes > 1000 {
                eprintln!(
                    "[xezim][error] compiled process (pid {}) looped {} times                      without reaching a wait — parking it (body's waits are                      unreachable this activation)",
                    pid, passes
                );
                break;
            }
        }
        std::mem::swap(&mut self.vm_regs, &mut f.regs);
        Self::recycle_string(
            &mut self.hint_string_pool,
            self.name_resolve_hint.replace(saved_hint),
        );
        self.proc_fsm.insert(pid, f);
        // The comb entries this process's writes trigger run in this settle;
        // the write set stays visible to `note_comb_ran_in_process` until then.
        self.settle_combinatorial();
        self.cur_fsm_writes = None;
    }

    pub(super) fn run_fast_delay_always(&mut self, pid: usize) {
        self.current_pid = pid;
        self.flush_deferred_asserts(pid);

        // Match run_process_stmts' per-process zero-delay protection. The
        // marker was removed from the timing wheel before dispatch, so park it
        // again before asking the outer loop to seek a future event.
        if self.stall_limit > 0 {
            let hits = self.stall_pid_hits.entry(pid).or_insert(0);
            // Check BEFORE counting: the activation that trips the guard is
            // re-parked UNEXECUTED, so it must not inflate the "ran N times"
            // attribution in the stall report.
            if *hits >= self.stall_limit {
                self.event_queue.schedule(self.time, pid, Vec::new().into());
                self.zero_delay_defer_pending = true;
                return;
            }
            *hits += 1;
        }

        // Temporarily own the entry so evaluation and assignment can mutably
        // borrow the rest of Simulator. Reinsert before returning; pids are
        // monotonic and a killed process is filtered by run_scheduled_process.
        let Some(mut fast) = self.fast_delay_always.remove(&pid) else {
            return;
        };

        if fast.execute_body {
            if let Some(compiled) = &fast.compiled_body {
                if self.vm_regs.len() < compiled.num_regs as usize {
                    self.vm_regs
                        .resize(compiled.num_regs as usize, Value::zero(1));
                }
                self.exec_insns(&compiled.instructions);
                // AST blocking assignments settle immediately outside edge
                // blocks. The VM marks the same dirty cone; preserve that
                // scheduling boundary explicitly.
                self.settle_combinatorial();
            } else {
                self.exec_statement(&fast.body);
            }
            if self.finished {
                self.fast_delay_always.insert(pid, fast);
                return;
            }
        }

        let precision_diff = *fast.delay_precision_diff.get_or_insert_with(|| {
            let (_, prec_exp) = self.current_timescale_exp();
            prec_exp - Self::secs_to_exp(self.tick_s)
        });
        let delay = if let Some((compiled, result_reg)) = &fast.compiled_delay {
            if self.vm_regs.len() < compiled.num_regs as usize {
                self.vm_regs
                    .resize(compiled.num_regs as usize, Value::zero(1));
            }
            self.exec_insns(&compiled.instructions);
            let value = self.vm_regs[*result_reg as usize].clone();
            self.delay_value_to_ticks(&value, &fast.delay, precision_diff)
        } else {
            self.eval_delay_ticks_with_precision(&fast.delay, precision_diff)
        };
        if !fast.execute_body
            && delay == 0
            && self.time == 0
            && !matches!(fast.delay.kind, ExprKind::Number(_))
            && self.t0_delay_deferred.insert(pid)
        {
            // The delay may depend on a real continuous assignment that has
            // not settled yet. Re-evaluate once after the time-zero inactive
            // boundary before fixing the first clock phase.
            self.inactive_queue.push((pid, ProcCont::empty()));
        } else {
            fast.execute_body = true;
            if delay == 0 {
                self.inactive_queue.push((pid, ProcCont::empty()));
            } else {
                self.event_queue
                    .schedule(self.time.saturating_add(delay), pid, Vec::new().into());
            }
        }
        self.fast_delay_always.insert(pid, fast);
    }

    /// Merge a fork child's local-frame writes into a parent's `local_stack`.
    ///
    /// §9.3.2 baseline: only the keys the child WROTE (its value now differs
    /// from the fork-time snapshot) are propagated — a key it merely
    /// inherited unchanged must NOT clobber a value the parent modified
    /// after the fork (e.g. a mailbox-get delivery into `phase` while this
    /// child ran). If no baseline survived, fall back to the legacy
    /// whole-frame merge so the §9.3.2 write-visibility guarantee still
    /// holds. Only keys that exist in the parent's frame are propagated (a
    /// key the child declared itself, like a loop-local, is child-private).
    /// Write a fork child's changed locals back into the parent's frames.
    /// `parent_gens` / `child_gens` are the `local_gen_stack`s of the parent
    /// (the frame set it owns *right now*) and the child (the frame set it
    /// inherited at fork time). For slot index `i` we only propagate when the
    /// two generations agree: slot `i` in the parent must still be the very
    /// activation the child forked from. A fork child that outlives its
    /// creating method and keeps writing its *stale* automatic locals must not
    /// clobber the unrelated activation that now occupies the same slot
    /// (regression 3627: an alpha reader orphaned by `join_any` kept overwriting
    /// beta's `count` with its own stale value).
    /// Returns the names of keys actually propagated (child value differs
    /// from both baseline and parent, so a real write landed). The caller
    /// uses this to promote parked condition waiters that read those names.
    pub(super) fn merge_fork_writes(
        parent_frames: &mut [HashMap<String, Value>],
        parent_gens: &[u64],
        child_frames: &[HashMap<String, Value>],
        child_gens: &[u64],
        baseline: Option<&Vec<HashMap<String, Value>>>,
    ) -> Vec<String> {
        let mut written = Vec::new();
        let n = parent_frames.len().min(child_frames.len());
        for i in 0..n {
            let same_activation = parent_gens.get(i).copied() == child_gens.get(i).copied();
            if !same_activation {
                // The parent slot `i` no longer belongs to the activation this
                // child forked from — keep this child's locals out of it.
                continue;
            }
            // The unchanged test first: nearly every inherited key is
            // unchanged, and it then needs no parent lookup.
            let base_frame = baseline.and_then(|b| b.get(i));
            // A frame whose keys were never inserted or removed since the
            // fork still has its baseline clone's layout, so the two iterate
            // in the same key order and pair up without a lookup. Any key
            // mismatch falls back to the keyed loop, which re-derives the
            // same writes (a repeated write is a no-op).
            if let Some(bf) = base_frame.filter(|f| f.len() == child_frames[i].len()) {
                let mut paired = true;
                for ((k, v), (bk, bv)) in child_frames[i].iter().zip(bf.iter()) {
                    if k != bk {
                        paired = false;
                        break;
                    }
                    if v == bv {
                        continue;
                    }
                    if let Some(slot) = parent_frames[i].get_mut(k) {
                        if slot != v {
                            *slot = v.clone();
                            // A parked `wait` of the parent may read it.
                            note_store_write(name_bit(k));
                            written.push(k.clone());
                        }
                    }
                }
                if paired {
                    continue;
                }
            }
            for (k, v) in &child_frames[i] {
                let inherited_unchanged = base_frame
                    .and_then(|f| f.get(k))
                    .is_some_and(|old| old == v);
                if inherited_unchanged {
                    continue;
                }
                match parent_frames[i].get_mut(k) {
                    Some(slot) => {
                        if slot != v {
                            *slot = v.clone();
                            // A parked `wait` of the parent may read it.
                            note_store_write(name_bit(k));
                            written.push(k.clone());
                        }
                    }
                    None => continue,
                }
            }
        }
        written
    }

    /// Evaluate a `#delay` expression to an integer number of simulator ticks.
    /// A real-valued delay (e.g. `#1.5`, or `#0.5` under a finer `\`timescale`)
    /// is rounded to the nearest tick — LRM §3.14.3 rounds delays to the time
    /// precision. Crucially this avoids reading a real Value's raw float *bits*
    /// as a huge integer (the old `to_u64()` did, scheduling the continuation
    /// astronomically far out so it never fired and the process was lost).
    pub(super) fn eval_delay_ticks(&mut self, d: &Expression) -> u64 {
        let (_, prec_exp) = self.current_timescale_exp();
        let precision_diff = prec_exp - Self::secs_to_exp(self.tick_s);
        self.eval_delay_ticks_with_precision(d, precision_diff)
    }

    pub(super) fn eval_delay_ticks_with_precision(
        &mut self,
        d: &Expression,
        precision_diff: i32,
    ) -> u64 {
        let v = self.eval_expr(d);
        self.delay_value_to_ticks(&v, d, precision_diff)
    }

    pub(super) fn delay_value_to_ticks(
        &mut self,
        v: &Value,
        d: &Expression,
        precision_diff: i32,
    ) -> u64 {
        let raw = if v.is_real {
            let f = v.to_f64();
            // Diagnose a pathological real delay: non-finite (NaN/inf) or
            // absurdly large. A huge `#delay` parks the process ~forever — which
            // reads as "the clock stopped" — and is almost always a bit-garbage
            // value (an IEEE-754 pattern read as a ~1e18 number) or a divide that
            // produced inf/NaN. Warn once per site so the offending delay names
            // itself in the log instead of silently freezing a clock.
            if (!f.is_finite() || f >= 1e15) && self.warned_delay_spikes.insert(d.span.start) {
                let loc = self
                    .span_file_line_in(d.span, None)
                    .map(|l| format!(" at {}", l))
                    .unwrap_or_default();
                eprintln!(
                    "[xezim][warning] a #delay evaluated to a pathological real value {:.4e}{} \
                         at time {} — the process is parked far in the future, so a clock driven \
                         by it will appear to stop. This usually means the delay expression \
                         produced NaN/inf or IEEE-754 bit garbage.",
                    f, loc, self.time
                );
            }
            if f.is_finite() && f > 0.0 { f } else { 0.0 }
        } else {
            v.to_u64().unwrap_or(0) as f64
        };
        // §3.14.3: a delay rounds to the CURRENT MODULE's time precision, not
        // the (finer) global tick — `#1.55` in a 1ns/100ps module is 1.6ns,
        // and `#0.04` collapses to 0. `raw` is already tick-denominated (the
        // elaborator pre-scales by the module unit), so quantize to the
        // module-precision-per-tick grid. The grid ratio comes from EXPONENT
        // arithmetic (10^n is exact in f64 for small n; dividing seconds
        // values is not), and a 1-ulp relative nudge keeps a half-grid delay
        // that arrived as x.49999999999996 from rounding the wrong way.
        if precision_diff > 0 {
            let q = 10f64.powi(precision_diff);
            let x = (raw / q) * (1.0 + 1e-12);
            (x.round() * q).round() as u64
        } else {
            raw.round() as u64
        }
    }

    /// IEEE 1800-2017 §9.4.5 intra-assignment delay: `lhs = #d rhs` /
    /// `lhs <= #d rhs` is canonicalized at source level (see
    /// `crate::intra_delay`) into `lhs = $__xz_intra_delay(d, rhs)`.
    /// Returns `(delay_expr, rhs_expr)` when `rvalue` is that marker.
    /// §14.16 `cb.out <= ##N rhs` marker: returns (count_expr, rhs).
    pub(super) fn intra_cycle_marker(rvalue: &Expression) -> Option<(&Expression, &Expression)> {
        match &rvalue.kind {
            ExprKind::SystemCall { name, args }
                if name == crate::intra_delay::INTRA_CYCLE_MARKER && args.len() == 2 =>
            {
                Some((&args[0], &args[1]))
            }
            _ => None,
        }
    }

    /// A blocking assignment's rvalue carries an intra-assignment delay or
    /// event control (the pre-parse markers), so executing it suspends.
    pub(super) fn intra_timing_suspends(rvalue: &Expression) -> bool {
        Self::intra_delay_marker(rvalue).is_some()
            || matches!(&rvalue.kind, ExprKind::SystemCall { name, args }
                if name == crate::intra_delay::INTRA_EVENT_MARKER && args.len() == 4)
    }

    pub(super) fn intra_delay_marker(rvalue: &Expression) -> Option<(&Expression, &Expression)> {
        match &rvalue.kind {
            ExprKind::SystemCall { name, args }
                if name == crate::intra_delay::INTRA_DELAY_MARKER && args.len() == 2 =>
            {
                Some((&args[0], &args[1]))
            }
            _ => None,
        }
    }

    /// Stash a §9.4.5 RHS value evaluated at statement-execution time and
    /// synthesize the `$__xz_intra_saved(idx)` expression that retrieves it
    /// when the delayed assignment finally runs — the pre-computed Value
    /// survives process rescheduling this way.
    pub(super) fn make_intra_saved_expr(
        &mut self,
        val: Value,
        span: crate::ast::Span,
    ) -> Expression {
        let idx = self.intra_saved_next;
        self.intra_saved_next += 1;
        self.intra_saved.insert(idx, val);
        Expression::new(
            ExprKind::SystemCall {
                name: INTRA_SAVED_MARKER.to_string(),
                args: vec![Expression::new(
                    ExprKind::Number(NumberLiteral::Integer {
                        size: None,
                        signed: false,
                        base: NumberBase::Decimal,
                        value: idx.to_string(),
                        cached_val: Cell::new(None),
                    }),
                    span,
                )],
            },
            span,
        )
    }

    /// Continue executing `cont` for `pid`. Normally recurses (fast path), but
    /// when the `run_process_stmts` recursion is deep — a long synchronous loop
    /// whose body never actually suspends — TRAMPOLINE through the event queue
    /// at the current simulation time to bound the stack. Same time + same
    /// continuation ⇒ identical semantics: `is_pid_suspended` sees the scheduled
    /// entry (via `event_queue.has_pid`) and `run_scheduled_process` snapshots /
    /// restores the process context, exactly as an immediate `@event` re-arm.
    pub(super) fn continue_stmts_or_trampoline(&mut self, pid: usize, cont: ProcCont) {
        if RPS_DEPTH.with(|c| c.get()) > RPS_TRAMPOLINE_DEPTH {
            // The context stays live; mark the entry as this pid's own
            // continuation (see `run_scheduled_process_inner`).
            if !self.local_stack.is_empty() || !self.this_stack.is_empty() {
                self.trampolined_pids.insert(pid);
            }
            self.event_queue.schedule(self.time, pid, cont);
        } else {
            self.run_process_stmts(pid, &cont);
        }
    }

    /// Phase 2 (§9.3.3 `break`/`continue` in suspend-aware loops): when a
    /// blocking loop is RE-ENTERED in the flattened process stream after its
    /// body ran, decide what to do with the loop-control flags the body set.
    ///
    /// Returns `true` if the caller should EXIT this loop now (`break`/
    /// `return` — skip to the statement after the loop); `false` if the loop
    /// should PROCEED (re-enter for the next iteration, or first entry).
    ///
    /// - `return_flag` (set with `break_flag` by a `return` in an inlined
    ///   task body): do NOT consume — unwind to the task's ScopePop is driven
    ///   by the top-of-loop `return_flag` skip. Exit this loop.
    /// - `break_flag` alone: the body's `break` exits this loop — consume it
    ///   (clear break+continue) and exit.
    /// - `continue_flag` alone: consume it and proceed (re-enter the next
    ///   iteration); the body statements after the `continue` were already
    ///   skipped by `exec_statement`'s flag-check at entry.
    ///
    /// First entry (all flags clear) is a no-op → proceed normally.
    pub(super) fn blocking_loop_flag_gate(&mut self) -> bool {
        if self.return_flag {
            return true;
        }
        if self.break_flag {
            self.break_flag = false;
            self.continue_flag = false;
            return true;
        }
        self.continue_flag = false;
        false
    }

    /// Execute a process's continuation: the current frame from its cursor, then
    /// each chained frame behind it (see `ProcCont`).
    ///
    /// `stmts`/`i` below are the CURRENT frame and an index within it, exactly as
    /// before. What changed is how a continuation is CAPTURED:
    /// `pc.resume_at(pc.start + i + 1)` names "the rest of my work" in O(1) where
    /// `pc.resume_at(pc.start + i + 1)` deep-cloned it, and
    /// `pc.pushed(body, pc.start + i + 1)` splices a task body or flattened block
    /// in front of the caller's tail without copying that tail.
    pub(super) fn try_exec_process_loop_bytecode(&mut self, pid: usize, stmt: &Statement) -> bool {
        // No call-frame gate: a process whose own variables were promoted to
        // the signal table has an EMPTY local_stack, and that is fine — there
        // are simply no frame locals to bind. Requiring a frame sent every
        // top-level stimulus loop (`repeat (N) begin @(posedge clk); for …`)
        // through the AST interpreter.
        //
        // `If` joins `For` because a per-cycle self-check
        // (`if (out !== expected(...)) fail++;`) re-executes as many times as
        // any loop — the (pid, span) cache amortizes its compile the same
        // way. The deep, call-following suspend check replaces the shallow
        // event-wait scan for it; anything the compiler cannot honestly
        // express (waits, loop control with no enclosing compiled loop,
        // returns) still bails to the AST path at compile time.
        // Compiled writes land directly in the signal table and would punch
        // through an active `force` (the AST path routes every assign through
        // the force filter). Forces are rare and usually short-lived: fall
        // back to the AST while ANY is active, per execution, keeping the
        // cache intact for when they lift.
        if !self.forced_signals.is_empty()
            || !self.forced_names.is_empty()
            || !self.active_force_exprs.is_empty()
        {
            return false;
        }
        // The vectorized packed-fill fast path beats an N-iteration compiled
        // loop and `exec_statement` would have tried it first — keep that
        // priority now that this runs ahead of `exec_statement`.
        if let StatementKind::For {
            init,
            condition,
            step,
            body,
        } = &stmt.kind
        {
            if self.try_exec_full_packed_fill(init, condition.as_ref(), step, body) {
                return true;
            }
        }
        let eligible = match &stmt.kind {
            StatementKind::For { .. } => !self.stmt_has_event_wait(stmt),
            _ => false,
        };
        if !eligible || stmt.span.start == 0 && stmt.span.end == 0 {
            static LOOP_STATS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
            if *LOOP_STATS.get_or_init(|| std::env::var_os("XEZIM_PROC_LOOP_STATS").is_some())
                && matches!(
                    stmt.kind,
                    StatementKind::For { .. } | StatementKind::If { .. }
                )
            {
                eprintln!(
                    "[PROC-LOOP] pid={} span={}..{} gate: eligible={} span0={}",
                    pid,
                    stmt.span.start,
                    stmt.span.end,
                    eligible,
                    stmt.span.start == 0 && stmt.span.end == 0
                );
            }
            return false;
        }
        let key = (pid, stmt.span.start, stmt.span.end);
        if !self.process_loop_bytecode.contains_key(&key) {
            let scope_hint = self.process_scope_hint.get(&pid).cloned();
            let mut compiler = super::bytecode::BytecodeCompiler::new(
                &self.signal_name_to_id,
                &self.signal_signed,
                &self.signal_widths,
                &self.module.arrays,
                &self.widths,
            );
            compiler.set_ast_fallback(false);
            compiler.set_scope_hint(scope_hint);
            compiler.set_tasks(&self.module.tasks);
            compiler.set_functions(&self.module.functions);
            compiler.set_params(&self.module.parameters);
            compiler.set_param_leaf_idx(self.param_leaf_index());
            compiler.set_cast_widths(&self.cast_widths);
            compiler.set_packed_struct_fields(
                self.bc_packed_fields
                    .as_ref()
                    .unwrap_or(&self.module.packed_struct_fields),
            );
            compiler.set_packed_elem_widths(&self.module.packed_signal_elem_widths);
            compiler.set_typedefs(&self.module.typedefs, &self.module.typedef_elem_widths);
            compiler.set_typedef_types(&self.module.typedef_types);
            compiler.set_assoc_elem_widths(&self.module.assoc_elem_widths);
            compiler.set_assoc_arrays(&self.module.associative_arrays);
            compiler.set_packed_full_dims(&self.module.packed_full_dims);
            compiler.set_packed_struct_fields(
                self.bc_packed_fields
                    .as_ref()
                    .unwrap_or(&self.module.packed_struct_fields),
            );
            compiler.set_multi_dim_arrays(&self.multi_dim_array_names);
            compiler.set_arrays_2d(&self.module.arrays_2d);
            compiler.set_collection_denies(&self.module.dynamic_arrays, &self.module.queue_vars);
            compiler.set_array_first_id(&self.array_first_id);
            compiler.set_string_signals(&self.module.string_signals);
            compiler.set_signal_real(&self.signal_real);
            compiler.top_module_name = Some(self.module.name.clone());
            if let Some(frame) = self.local_stack.last() {
                compiler.set_process_locals(frame);
            }
            // The feedback walk is per-SPAN (cached below), not per
            // activation — collect_stmt_reads is too heavy to re-run on a
            // 10k-cycle stimulus loop.
            let feedback = self.stmt_has_comb_feedback(stmt);
            let ok = !feedback && compiler.compile_stmt(stmt);
            if std::env::var_os("XEZIM_PROC_LOOP_STATS").is_some() {
                eprintln!(
                    "[PROC-LOOP] pid={} span={}..{} compiled={} reason={}",
                    pid,
                    stmt.span.start,
                    stmt.span.end,
                    ok,
                    if ok {
                        "-"
                    } else if feedback {
                        "comb_feedback"
                    } else {
                        compiler.bail_reason.unwrap_or("unknown")
                    }
                );
            }
            let compiled = ok.then(|| compiler.finish());
            self.process_loop_bytecode.insert(key, compiled);
        }
        let Some(Some(compiled)) = self.process_loop_bytecode.remove(&key) else {
            self.process_loop_bytecode.entry(key).or_insert(None);
            return false;
        };
        if self.vm_regs.len() < compiled.num_regs as usize {
            self.vm_regs
                .resize(compiled.num_regs as usize, Value::zero(1));
        }
        self.exec_insns(&compiled.instructions);
        self.process_loop_bytecode.insert(key, Some(compiled));
        true
    }

    /// Write→comb→read feedback check for the process statement caches: does
    /// this statement WRITE a signal whose (transitive) combinational fan-out
    /// produces a signal the statement also READS? The AST path settles
    /// between statements and loop iterations, so such feedback re-reads a
    /// FRESH value mid-statement (a mutable loop bound driven by always_comb
    /// from the loop body's own writes); a straight-line compiled block only
    /// settles at its boundary and would read stale state. Conservative:
    /// unresolvable names are ignored (they compile-bail elsewhere).
    pub(super) fn stmt_has_comb_feedback(&self, stmt: &Statement) -> bool {
        // Writes first: the read set names every element of a dynamically
        // indexed array (65,536 per testbench memory-load loop on c906), and
        // it only matters once some comb entry is downstream of a write.
        let mut writes: HashSet<String> = HashSet::default();
        Self::collect_stmt_rw(stmt, &self.module, None, &mut writes, None);
        if writes.is_empty() {
            return false;
        }
        // A DYNAMIC array is a collection, not a signal, so its name maps to
        // no id below: the dependency walk found nothing, the loop compiled,
        // and the compiled element store skipped the dirty-marking the AST
        // interpreter store does. `always_comb` readers were then never
        // re-evaluated and silently kept stale (or x) data --- `for (k) a[k] =
        // v;` on `logic [7:0] a [];` left a reader at x while `a[0]` really
        // held the written value. Treat any write to one as feedback so the
        // loop stays interpreted, which notifies correctly. Static arrays and
        // queues resolve through the walk normally and keep compiling.
        if writes.iter().any(|w| {
            let base = w.split('[').next().unwrap_or(w);
            self.module.dynamic_arrays.contains(base)
        }) {
            return true;
        }
        let to_id = |n: &String| self.signal_name_to_id.get(n.as_str()).copied();
        // Every signal the writes reach through comb entries.
        let mut reached: HashSet<usize> = HashSet::default();
        let mut frontier: Vec<usize> = writes.iter().filter_map(to_id).collect();
        let mut seen_entries: HashSet<u32> = HashSet::default();
        while let Some(id) = frontier.pop() {
            if id + 1 >= self.comb_dep_offsets.len() {
                continue;
            }
            let lo = self.comb_dep_offsets[id] as usize;
            let hi = self.comb_dep_offsets[id + 1] as usize;
            for &e in &self.comb_dep_entries[lo..hi] {
                if !seen_entries.insert(e) {
                    continue;
                }
                for &out in &self.comb_entries[e as usize].cold.write_signal_ids {
                    if reached.insert(out) {
                        frontier.push(out);
                    }
                }
            }
        }
        if reached.is_empty() {
            return false;
        }
        let mut reads: HashSet<String> = HashSet::default();
        writes.clear();
        Self::collect_stmt_reads(stmt, &self.module, &mut reads, &mut writes);
        reads
            .iter()
            .filter_map(to_id)
            .any(|id| reached.contains(&id))
    }

    /// Is this expression free of side effects, structurally? Conservative:
    /// unknown shapes are NOT effect-free. Calls may be allowed only when
    /// the bytecode compiler subsequently verifies their purity. Runtime
    /// shape probes must reject them before evaluating an index speculatively.
    /// System calls are limited to pure value readers.
    pub(super) fn cond_expr_is_effect_free(e: &Expression, compiler_checks_calls: bool) -> bool {
        let pure = |expr: &Expression| Self::cond_expr_is_effect_free(expr, compiler_checks_calls);
        match &e.kind {
            ExprKind::Number(_) | ExprKind::StringLiteral(_) => true,
            ExprKind::Ident(h) => h.path.iter().all(|seg| seg.selects.iter().all(pure)),
            ExprKind::Paren(i) => pure(i),
            ExprKind::Unary { op, operand } => {
                !matches!(
                    op,
                    UnaryOp::PreIncr | UnaryOp::PostIncr | UnaryOp::PreDecr | UnaryOp::PostDecr
                ) && pure(operand)
            }
            ExprKind::Binary { left, right, .. } => pure(left) && pure(right),
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => pure(condition) && pure(then_expr) && pure(else_expr),
            ExprKind::Concatenation(xs) => xs.iter().all(pure),
            ExprKind::Replication { count, exprs } => pure(count) && exprs.iter().all(pure),
            ExprKind::Index { expr, index } => pure(expr) && pure(index),
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => pure(expr) && pure(left) && pure(right),
            ExprKind::MemberAccess { expr, .. } => pure(expr),
            ExprKind::SystemCall { name, args } => {
                matches!(
                    name.as_str(),
                    "$signed"
                        | "$unsigned"
                        | "$bits"
                        | "$clog2"
                        | "$__xz_named_cast"
                        | "$__xz_size_cast"
                        | SVA_SIG_MARKER
                ) && args.iter().all(pure)
            }
            ExprKind::Call { func, args } => {
                // §7.9/§7.10: a built-in collection QUERY (`q.size()`,
                // `aa.num()`, `aa.exists(k)`, `s.len()`) only reads, so it
                // stays probe-safe. `slices[slices.size()-1] = s` must keep
                // its aggregate-shape probes or the struct copy is lost.
                if let ExprKind::MemberAccess { expr, member } = &func.kind {
                    if matches!(member.name.as_str(), "size" | "num" | "len" | "exists")
                        && pure(expr)
                        && args.iter().all(pure)
                    {
                        return true;
                    }
                }
                compiler_checks_calls
                    && matches!(func.kind, ExprKind::Ident(_))
                    && args.iter().all(pure)
            }
            _ => false,
        }
    }

    /// Evaluate a per-cycle `if` CONDITION through cached bytecode, for the
    /// self-check shape whose statement as a whole cannot compile (its fail
    /// branch may touch a name-keyed initial-scope variable the VM cannot
    /// address). The condition — a wide compare against an inlinable helper
    /// call — is where the time goes; the branch still runs via the AST.
    /// Returns the condition's truth, or None to fall back to `eval_expr`.
    /// Same honesty contract as the statement cache: if the expression does
    /// not fully compile (unknown name, un-inlinable call), the cache pins
    /// None and the AST path stays authoritative.
    pub(super) fn try_eval_process_cond_bytecode(
        &mut self,
        pid: usize,
        cond: &Expression,
    ) -> Option<bool> {
        if cond.span.start == 0 && cond.span.end == 0 {
            return None;
        }
        // FRAMELESS only: inside a subroutine frame a `static` local needs
        // the AST's dynamic scope (per-instance storage) that the compiled
        // lookup cannot reproduce.
        if !self.local_stack.is_empty() {
            return None;
        }
        let key = (pid, cond.span.start, cond.span.end);
        if !self.process_cond_bytecode.contains_key(&key) {
            // Side effects would DOUBLE-FIRE during the self-audit below —
            // only cache conditions that provably have none.
            if !Self::cond_expr_is_effect_free(cond, true) {
                self.process_cond_bytecode.insert(key, None);
                return None;
            }
            let scope_hint = self.process_scope_hint.get(&pid).cloned();
            let mut compiler = super::bytecode::BytecodeCompiler::new(
                &self.signal_name_to_id,
                &self.signal_signed,
                &self.signal_widths,
                &self.module.arrays,
                &self.widths,
            );
            compiler.set_ast_fallback(false);
            compiler.set_scope_hint(scope_hint);
            compiler.set_tasks(&self.module.tasks);
            compiler.set_functions(&self.module.functions);
            compiler.set_params(&self.module.parameters);
            compiler.set_param_leaf_idx(self.param_leaf_index());
            compiler.set_cast_widths(&self.cast_widths);
            compiler.set_packed_struct_fields(
                self.bc_packed_fields
                    .as_ref()
                    .unwrap_or(&self.module.packed_struct_fields),
            );
            compiler.set_packed_elem_widths(&self.module.packed_signal_elem_widths);
            compiler.set_typedefs(&self.module.typedefs, &self.module.typedef_elem_widths);
            compiler.set_typedef_types(&self.module.typedef_types);
            compiler.set_assoc_elem_widths(&self.module.assoc_elem_widths);
            compiler.set_assoc_arrays(&self.module.associative_arrays);
            compiler.set_packed_full_dims(&self.module.packed_full_dims);
            compiler.set_multi_dim_arrays(&self.multi_dim_array_names);
            compiler.set_arrays_2d(&self.module.arrays_2d);
            compiler.set_collection_denies(&self.module.dynamic_arrays, &self.module.queue_vars);
            compiler.set_array_first_id(&self.array_first_id);
            compiler.set_string_signals(&self.module.string_signals);
            compiler.set_signal_real(&self.signal_real);
            compiler.top_module_name = Some(self.module.name.clone());
            if let Some(frame) = self.local_stack.last() {
                compiler.set_process_locals(frame);
            }
            let result = compiler.compile_root_expr(cond);
            if std::env::var_os("XEZIM_PROC_LOOP_STATS").is_some() {
                eprintln!(
                    "[PROC-COND] pid={} span={}..{} compiled={} reason={}",
                    pid,
                    cond.span.start,
                    cond.span.end,
                    result.is_some(),
                    if result.is_some() {
                        "-"
                    } else {
                        compiler.bail_reason.unwrap_or("unknown")
                    }
                );
            }
            let compiled = result.map(|reg| (compiler.finish(), reg, COND_AUDIT_EVALS));
            if std::env::var_os("XEZIM_PROC_LOOP_STATS").is_some() {
                if let Some((cb, _, _)) = &compiled {
                    eprintln!(
                        "[PROC-COND-INSNS] {:?}",
                        cb.instructions
                            .iter()
                            .map(|i| format!("{:?}", i))
                            .collect::<Vec<_>>()
                    );
                }
            }
            self.process_cond_bytecode.insert(key, compiled);
        }
        let Some(Some((compiled, reg, audits))) = self.process_cond_bytecode.remove(&key) else {
            self.process_cond_bytecode.entry(key).or_insert(None);
            return None;
        };
        if self.vm_regs.len() < compiled.num_regs as usize {
            self.vm_regs
                .resize(compiled.num_regs as usize, Value::zero(1));
        }
        self.exec_insns(&compiled.instructions);
        let truth = self.vm_regs[reg as usize].is_true();
        // Self-audit: the first activations (and every one under the audit
        // env) also evaluate through the AST. A disagreement means the
        // bytecode compiled this expression with different semantics — pin
        // the span to the AST permanently and use the AST's answer.
        static COND_VERIFY: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if audits > 0
            || *COND_VERIFY.get_or_init(|| std::env::var_os("XEZIM_PROC_COND_VERIFY").is_some())
        {
            let ast = self.eval_expr(cond).is_true();
            if ast != truth {
                if std::env::var_os("XEZIM_PROC_LOOP_STATS").is_some()
                    || std::env::var_os("XEZIM_PROC_COND_VERIFY").is_some()
                {
                    eprintln!(
                        "[PROC-COND-DIVERGE] pid={} span={}..{} compiled={} ast={} — pinned to AST",
                        pid, cond.span.start, cond.span.end, truth, ast
                    );
                }
                self.process_cond_bytecode.insert(key, None);
                return Some(ast);
            }
            self.process_cond_bytecode
                .insert(key, Some((compiled, reg, audits.saturating_sub(1))));
            return Some(truth);
        }
        self.process_cond_bytecode
            .insert(key, Some((compiled, reg, 0)));
        Some(truth)
    }

    pub(super) fn run_process_stmts(&mut self, pid: usize, pc: &ProcCont) {
        // Install this activation's own scope (see `activation_scope`) —
        // the ratchet-proof twin of the hint reset performed inside. A
        // mid-block hierarchical reference (`u_child.x = 1` right before
        // a loop) ratchets the hint away, but a bare name this process's
        // own scope declares must still resolve there, never downward
        // into the instance. Saved and restored around the activation so
        // a re-entrant call (synchronous task re-entry) restores its
        // caller's scope, and compile-time evals after this activation
        // see None, not a stale scope. The body lives in
        // run_process_stmts_inner because a Drop guard holding
        // &self.activation_scope would block every &mut self call in it.
        let same_scope = match (
            self.process_scope_hint.get(&pid),
            self.activation_scope.borrow().as_deref(),
        ) {
            (Some(s), Some(a)) => s.as_str() == a,
            (None, Some("")) => true,
            _ => false,
        };
        if same_scope {
            return self.run_process_stmts_inner(pid, pc);
        }
        // A method task of a class declared inside a module runs in its
        // object's instance (`TaskCleanup::modclass`).
        let modclass = if self.modclass_depth.get() > 0 {
            self.task_cleanup
                .iter()
                .rev()
                .find_map(|t| t.modclass.clone())
        } else {
            None
        };
        let saved_activation_scope = self.activation_scope.replace(Some(
            modclass
                .or_else(|| self.process_scope_hint.get(&pid).cloned())
                .unwrap_or_default(),
        ));
        self.run_process_stmts_inner(pid, pc);
        *self.activation_scope.borrow_mut() = saved_activation_scope;
    }

    pub(super) fn run_process_stmts_inner(&mut self, pid: usize, pc: &ProcCont) {
        let stmts: &[Statement] = pc.frame();
        self.current_pid = pid;
        self.flush_deferred_asserts(pid);
        // Install THIS process's own instance scope as the resolution hint.
        // The hint is transient sibling-scope state: the previous process (or
        // a $display argument it evaluated) may have left its scope behind,
        // and a bare name in this process would then resolve into THAT
        // instance — `user`'s wildcard-imported enum member `C` read the
        // sibling `shadower`'s local `int C` (truncated to the enum's width)
        // purely because shadower's process happened to run first. The reset
        // helper already existed for loop re-entry; a fresh activation needs
        // it just as much.
        self.reset_hint_to_process_scope();
        // Track run_process_stmts recursion depth so the suspend-aware loop
        // handlers (While/For/Repeat below) can trampoline through the event
        // queue instead of recursing when a synchronous loop body never
        // suspends — otherwise a multi-iteration loop of non-blocking task calls
        // recurses deeply and overflows the stack cloning the continuation.
        // The Cell/guard is always on (a few ns per call); overflow-prevention
        // is not debug-only.
        let depth = RPS_DEPTH.with(|c| {
            let d = c.get();
            c.set(d + 1);
            d
        });
        struct DepthGuard;
        impl Drop for DepthGuard {
            fn drop(&mut self) {
                RPS_DEPTH.with(|c| c.set(c.get().saturating_sub(1)));
            }
        }
        let _dg = DepthGuard;
        // Only count TOP-LEVEL (depth == 0) activations toward the per-pid
        // zero-delay livelock limit. A re-entrant call (depth > 0) is a
        // synchronous sub-invocation fired from within this same activation —
        // e.g. a function/task the process called, or a trampolined
        // while/for/repeat body that re-entered run_process_stmts. Such
        // recursion is bounded (it returns) and is NOT the process re-arming
        // itself via a #0 / event wait, so inflating `stall_pid_hits` with it
        // would conflate ordinary recursion with genuine zero-delay livelock
        // (a process that keeps re-scheduling at one time and never lets the
        // clock advance). The livelock pattern only manifests at the outermost
        // dispatch, so the guard stays accurate while re-entrant work is free
        // to recurse up to the trampoline/stack limits above.
        if self.stall_limit > 0 && depth == 0 {
            let hits = self.stall_pid_hits.entry(pid).or_insert(0);
            // One process re-activated this many times at a SINGLE timestamp is
            // not a busy design, it is a livelock: it keeps re-arming itself and
            // time can never advance. (Counting per-process, rather than
            // counting activations at the timestamp, is what makes this safe for
            // a wide design that legitimately wakes thousands of DISTINCT
            // processes at time 0.) run_one_tick's inner drain loop re-reads the
            // queue at the current time, so a self-rescheduling process never
            // reaches the outer event loop — the check has to live here.
            // Check BEFORE counting: the activation that trips the guard is
            // re-parked UNEXECUTED and must not inflate "ran N times".
            if *hits >= self.stall_limit {
                // A single process re-arming this many times at one timestamp is
                // a zero-delay livelock (e.g. `always #(period) clk=~clk` whose
                // real `period` momentarily glitched to 0). Rather than abort the
                // whole run here, RE-PARK this un-executed activation at the
                // current time and ask the outer event loop to defer-and-advance
                // to the next scheduled event — a commercial simulator lets time
                // move past a #0 spinner once its driving value recovers (here,
                // once a reference-clock edge updates the measured period). The outer
                // loop declares it fatal only if nothing is scheduled ahead.
                self.event_queue.schedule(self.time, pid, pc.clone());
                self.zero_delay_defer_pending = true;
                return;
            }
            *hits += 1;
        }
        sim_dbg_eprintln!(
            "[DEBUG] running process {} ({} stmts) at time {}",
            pid,
            stmts.len(),
            self.time
        );
        let mut i = 0;
        while i < stmts.len() && !self.finished {
            // `resolve_hier_name` installs the parent scope as the resolution
            // hint whenever it resolves a DOTTED name, and nothing restored it.
            // So after a testbench statement read `u_dut.sig`, the hint stayed
            // "u_dut" and the NEXT statement's unqualified names resolved into
            // the DUT: `sel = 3'd4;` wrote `u_dut.sel` (promptly overwritten by
            // the port's continuous assign), so the stimulus silently stopped
            // reaching the design. Re-anchor to this process's own scope at
            // every statement boundary; a hint deliberately installed for a
            // statement is set inside that statement's own handler.
            self.reset_hint_to_process_scope();
            let stmt = &stmts[i];

            // §6.21: the trailing `Null` of a scoped block's frame closes the
            // block's scope, whatever flag is unwinding through it.
            if matches!(stmt.kind, StatementKind::Null)
                && i + 1 == stmts.len()
                && !self.block_scope_saves.is_empty()
            {
                self.block_scope_sentinel(pc);
            }

            // §35.5.2: the resume marker of a process parked inside an
            // imported task (dpi_task.rs). Ahead of the flag skips below: a
            // disable posted meanwhile unwinds the task before the block.
            if !self.dpi_task_names.is_empty() {
                if let Some(carry_on) = self.dpi_task_marker(pc, i, stmt) {
                    if !carry_on {
                        return;
                    }
                    i += 1;
                    continue;
                }
            }

            // An inlined task/method `return` (return_flag set) must unwind to
            // the enclosing task's `ScopePop` sentinel, skipping the rest of the
            // task body in between (including blocking statements after the `return`).
            // The ScopePop runs `unwind_task_frame`, which restores the caller's
            // saved break/continue/return flags — consuming the return. Without
            // this, return_flag/break_flag leaked past the ScopePop and skipped
            // the caller's next real statement.
            if self.return_flag {
                match &stmt.kind {
                    StatementKind::ScopePop => {
                        if let Some(c) = self.task_cleanup.pop() {
                            self.unwind_task_frame(c);
                        } else {
                            self.return_flag = false;
                        }
                    }
                    StatementKind::ForeachTail { fe_auto_len, .. } => {
                        self.auto_loop_vars.truncate(*fe_auto_len);
                    }
                    _ => {}
                }
                i += 1;
                continue;
            }

            // §9.6.2 `disable <block|task>`: skip to the end of the named
            // block's frame (or the task's `ScopePop`), running only the task
            // cleanups passed on the way.
            if self.break_flag && self.disable_target.is_some() && self.disable_unwinding(pid, pc) {
                if let StatementKind::ScopePop = &stmt.kind {
                    self.disable_pop_task_frame();
                }
                i += 1;
                continue;
            }

            // §12.7.2 for-step barrier (see `StatementKind::LoopStep`). Checked
            // HERE, ahead of the generic dispatch, because `exec_statement`
            // skips every statement while `continue_flag` is set — which is
            // exactly the flag this sentinel exists to consume.
            if let StatementKind::LoopStep = &stmt.kind {
                if !self.break_flag && !self.return_flag {
                    self.continue_flag = false;
                }
                i += 1;
                continue;
            }

            // §12.7 / §9.3.3: while a `break`/`continue` is pending, the REST of
            // the loop body must be skipped — INCLUDING blocking statements. The
            // synchronous `exec_statement` already no-ops every statement while
            // these flags are set (its top guard), and `blocking_loop_flag_gate`
            // relies on that: it assumes "the body statements after the `continue`
            // were already skipped". But a BLOCKING statement — a call to a
            // time-consuming task/method, a `#delay`, an `@event`, a `fork`, or a
            // blocking begin/end — is intercepted in THIS function AHEAD of
            // `exec_statement`, and inlining a blocking task even SAVES AND CLEARS
            // these flags for the callee's body (`bind_task_frame`). So without
            // this guard a `continue`/`break` guarding a call to a blocking
            // subroutine was ignored and the call ran for the skipped iterations
            // — bug 28: the mesh smoke seq's
            // `if (!has_neighbor(r,dir)) continue; send_directed(...);` leaked
            // off-mesh flits because `send_directed` blocks in
            // `start_item`/`finish_item`. Inline `#delay`s escaped the bug only
            // because the flag PERSISTS across suspend/resume, so the post-delay
            // assignment was still skipped; a called task clears it.
            //
            // Do NOT skip the statements that CONSUME the flag: `LoopStep`
            // (continue's barrier, handled just above), `ScopePop`
            // (task-frame cleanup), the `ForeachTail`/`ForeverTail` re-entry
            // sentinels (unambiguous — `Foreach`/`Forever` always lower to
            // one of these for re-entry, so the BASE kind is never itself a
            // tail and is never exempted here), and — ONLY at the exact
            // position `pc.loop_tail_at` names — a `While`/`Repeat` re-entry.
            //
            // That position check matters because those two kinds are
            // reused for BOTH the genuine re-entry AND an ordinary fresh
            // occurrence of the same kind reached for the first time right
            // after the `break`/`continue` (a nested loop in source, sitting
            // in the body BEFORE the re-appended tail). The two are the same
            // `StatementKind` variant, so matching on kind alone treats the
            // fresh one as if it were consuming ITS OWN flag: a `continue`
            // followed by a nested `for`/`while`/`repeat` ran anyway, and a
            // `break` followed by a nested `foreach`/loop got swallowed by
            // it instead of stopping the outer loop (PR #174 review, on top
            // of the original bug 28 fix — e.g. the mesh seq's
            // `if (!has_neighbor(...)) continue; for (dir2 ...) ...`).
            // `pc.loop_tail_at`, set only at the three sites that actually
            // re-append a `While`/`Repeat` as a tail (see its doc comment on
            // `ProcCont`; `For`/`DoWhile` lower to a `While` tail and are
            // themselves only ever reached fresh), is what tells the two
            // apart. A `disable` (disable_target set) drives its own unwind
            // through `break_flag` — leave that path untouched.
            if (self.break_flag || self.continue_flag) && self.disable_target.is_none() {
                let is_flag_consumer = matches!(
                    &stmt.kind,
                    StatementKind::ScopePop
                        | StatementKind::ForeachTail { .. }
                        | StatementKind::ForeverTail { .. }
                ) || (pc.loop_tail_at == Some(pc.start + i)
                    && matches!(
                        &stmt.kind,
                        StatementKind::While { .. } | StatementKind::Repeat { .. }
                    ));
                if !is_flag_consumer {
                    i += 1;
                    continue;
                }
            }
            // §38.36.1 cbStmt: statements this loop runs itself are reported
            // here (see `vpi_stmt_hook`).
            if self.vpi_cb_mask & vpi_cb::m(vpi_cb::CB_STMT) != 0 {
                self.vpi_stmt_hook(stmt, true);
            }

            // §35.5.2: a call of an imported task runs on its own stack, so
            // the process can park while the task waits (dpi_task.rs).
            if !self.dpi_task_names.is_empty() && self.dpi_cur_task.is_none() {
                if let Some(returned) = self.dpi_task_start(pid, pc, i, stmt) {
                    if !returned {
                        return;
                    }
                    i += 1;
                    continue;
                }
            }

            // Expand SeqBlocks: flatten begin/end so that timing controls and waits
            // inside them are properly handled with process suspension.
            if let StatementKind::SeqBlock { stmts: inner, name } = &stmt.kind {
                if self.stmts_have_blocking(inner) {
                    // §6.21: a block that declares variables opens a scope,
                    // closed by its frame's trailing `Null`.
                    let scope = self.block_scope_enter(inner);
                    // Immutable AST: share one Arc frame per AST node
                    // instead of cloning the whole list per process entry.
                    let frame = match name {
                        None if scope.is_some() => self.scoped_block_frame(&pc.stmts, stmt, inner),
                        None => self.blocking_cont_frame(&pc.stmts, stmt, 0, inner),
                        Some(n) => self.named_block_frame(&pc.stmts, stmt, inner, &n.name),
                    };
                    // §21.2.1.7: a NAMED block adds its name to the `%m`
                    // hierarchy while the process runs (and parks) in it;
                    // the frame's end pops it (see the frame-exhausted arm).
                    if let Some(n) = name {
                        self.m_scope_stack.push(n.name.clone());
                    }
                    if let Some(mut sv) = scope {
                        sv.frame_key = Self::frame_key(&frame);
                        self.block_scope_saves.push(sv);
                    }
                    self.run_process_stmts(pid, &pc.pushed_frame(frame, pc.start + i + 1));
                    return;
                }
            }

            // Stage 0: normalize a parenless task/method call (LRM §13.5
            // footnote 42 + §13.5.5: parentheses may be omitted for tasks, void
            // functions, and class methods). A call written without
            // parentheses — `t;`, `c.m;`, `m_run_phases;`, `run_test;` — parses
            // as a bare `Expr(Ident([name]))` or `Expr(MemberAccess{...})`, NOT
            // as `Expr(Call { func, args: [] })`. Every Stage 1/1b/1c inlining
            // guard below matches on `Call`, so without this rewrite a blocking
            // task called without parentheses bypasses inlining and falls
            // through to the synchronous `exec_statement`, which cannot honour
            // the body's `#delay`/`wait`/`fork` and silently drops it — the
            // caller never blocks (e.g. `task t; #10; endtask; ... t;` returns
            // at t=0 instead of t=10). Rewrite the parenless subroutine
            // reference into the equivalent zero-argument `Call` and
            // re-dispatch, so the existing guards fire uniformly. Only rewrite
            // when the callee resolves to a free task (`module.tasks`) or a
            // method (on the current `this` or on an object handle); a plain
            // variable reference `x;` is left to the normal expression path.
            // Non-blocking tasks: the rewritten Call still won't satisfy
            // `stmts_have_blocking`, so it falls through to `exec_statement`
            // unchanged — `t()` / `c.m()` with explicit parens already worked.
            //
            // Recognized parenless forms (LRM §13.5 `tf_call`/`method_call`):
            //   `t;`                    -> Ident([t])
            //   `m;`  (this-method)     -> Ident([m])
            //   `c.m;`                  -> MemberAccess{expr:Ident([c]), member:m}
            //   `c.m;` (flattened)      -> Ident([c, m])
            if let StatementKind::Expr(e) = &stmt.kind {
                let rewritten: Option<Expression> = match &e.kind {
                    // Bare name: free task or method on current `this`.
                    ExprKind::Ident(h) if h.path.len() == 1 => {
                        let nm = h.path[0].name.name.clone();
                        let is_free_task = self.module.tasks.contains_key(&nm);
                        let is_this_method = self
                            .this_stack
                            .last()
                            .copied()
                            .flatten()
                            .and_then(|hh| {
                                self.heap
                                    .get(hh)
                                    .and_then(|o| o.as_ref())
                                    .map(|inst| inst.class_name.clone())
                            })
                            .is_some_and(|cls| self.class_has_method(&cls, &nm));
                        if is_free_task || is_this_method {
                            Some(Expression::new(
                                ExprKind::Call {
                                    func: Box::new(e.clone()),
                                    args: vec![],
                                },
                                e.span,
                            ))
                        } else {
                            None
                        }
                    }
                    // `obj.method;` via member-access dot syntax.
                    ExprKind::MemberAccess { expr: recv, member } => {
                        let recv_ok = self
                            .eval_handle_expr(recv)
                            .and_then(|hh| {
                                self.heap
                                    .get(hh)
                                    .and_then(|o| o.as_ref())
                                    .map(|inst| inst.class_name.clone())
                            })
                            .is_some_and(|cls| self.class_has_method(&cls, &member.name));
                        if recv_ok {
                            Some(Expression::new(
                                ExprKind::Call {
                                    func: Box::new(e.clone()),
                                    args: vec![],
                                },
                                e.span,
                            ))
                        } else {
                            None
                        }
                    }
                    // `obj.method;` via flattened 2-segment ident.
                    ExprKind::Ident(h) if h.path.len() == 2 => {
                        let vn = h.path[0].name.name.clone();
                        let mn = h.path[1].name.name.clone();
                        let recv_ok = self
                            .eval_ident_handle(&vn)
                            .and_then(|hh| {
                                self.heap
                                    .get(hh)
                                    .and_then(|o| o.as_ref())
                                    .map(|inst| inst.class_name.clone())
                            })
                            .is_some_and(|cls| self.class_has_method(&cls, &mn));
                        if recv_ok {
                            Some(Expression::new(
                                ExprKind::Call {
                                    func: Box::new(e.clone()),
                                    args: vec![],
                                },
                                e.span,
                            ))
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                if let Some(call) = rewritten {
                    let call_stmt = Statement::new(StatementKind::Expr(call), stmt.span);
                    let cont = vec![call_stmt];
                    // Chain the caller's tail instead of copying it onto the end of
                    // the spliced body (ProcCont::pushed).
                    self.run_process_stmts(pid, &pc.pushed(cont, pc.start + i + 1));
                    return;
                }
            }

            // Stage 1: a call to a free task whose body blocks (top-level
            // `#delay`/`@event`/`wait`) is INLINED — bind the call frame, splice
            // the body + a ScopePop sentinel + the rest into this process's
            // stream, and recurse. The body's waits then suspend the process via
            // the machinery below instead of running synchronously (which spins
            // to a loop cap). Non-blocking task calls keep the synchronous path.
            if let StatementKind::Expr(expr) = &stmt.kind {
                if let ExprKind::Call { func, args } = &expr.kind {
                    if let ExprKind::Ident(h) = &func.kind {
                        if h.path.len() == 1 {
                            // A bare name that is ALSO a method on the current
                            // `this`'s class must inline via the method path
                            // (Stage 1b, with this-context), not as a free task —
                            // otherwise free task resolution without `this` causes
                            // member accesses to fail to resolve. Probe
                            // `task_decl_rc` first so the 99.9% of bare calls
                            // that are not module/package tasks skip the
                            // `this_stack` + `class_has_method` check entirely.
                            let tname = h.path[0].name.name.as_str();
                            // §23.8: in a method of a class declared inside a
                            // module the bare name is that instance's task
                            // (see `class_module_subroutine_key`); None
                            // elsewhere, leaving the free-task lookup as is.
                            let modclass_key = self.class_module_subroutine_key(tname);
                            let td = match &modclass_key {
                                Some(full) => self.task_decl_rc(full),
                                None => self.task_decl_rc(tname),
                            };
                            if let Some(td) = td {
                                let is_this_method = self
                                    .this_stack
                                    .last()
                                    .copied()
                                    .flatten()
                                    .and_then(|hh| self.heap.get(hh).and_then(|o| o.as_ref()))
                                    .map(|inst| inst.class_name.clone())
                                    .is_some_and(|cls| self.class_has_method(&cls, tname))
                                    // §8.10: in a static method a bare name binds
                                    // to the class's methods before any
                                    // compilation-unit task of the same name.
                                    || self
                                        .static_class_context()
                                        .is_some_and(|cls| self.class_has_method(&cls, tname));
                                if !is_this_method && self.stmts_have_blocking(&td.items) {
                                    let mut cleanup = self.bind_task_frame(&td, args, None);
                                    // §13.4/§13.3.2: a free task body must NOT
                                    // see the caller's class context — task-scope
                                    // `static` locals key off
                                    // `class_context_stack` at their declaration,
                                    // so an unshielded body silently gave every
                                    // CALLING CLASS its own copy of the static
                                    // cell (uvm_wait_for_nba_region's shared
                                    // nba/next_nba barrier split per caller
                                    // class). Self-guarded: only shields when
                                    // the caller actually has a `this`.
                                    self.push_instance_task_context(&mut cleanup);
                                    // The instance task's body resolves in its
                                    // instance across suspends, as Stage 1-hier.
                                    if let Some((scope, _)) =
                                        modclass_key.as_deref().and_then(|k| k.rsplit_once('.'))
                                    {
                                        cleanup.prev_hint = self.name_resolve_hint.borrow().clone();
                                        cleanup.frame_scope_hint = Some(scope.to_string());
                                        *self.name_resolve_hint.borrow_mut() =
                                            Some(scope.to_string());
                                    }
                                    self.task_cleanup.push(cleanup);
                                    let mut cont: Vec<Statement> = td.items.clone();
                                    cont.push(Statement::new(StatementKind::ScopePop, stmt.span));
                                    // Chain the caller's tail instead of copying it onto the end of
                                    // the spliced body (ProcCont::pushed).
                                    self.run_process_stmts(pid, &pc.pushed(cont, pc.start + i + 1));
                                    return;
                                }
                            }
                        }
                    }
                }
            }

            // Stage 1-pkg: a blocking PACKAGE task, `pkg::t(args)` — flattened
            // to Ident([pkg, t]) at module scope, a MemberAccess inside a
            // subroutine body. Stage 1 matches single-segment names only, so
            // these ran on the synchronous path, whose nested delay loop
            // never hands control back: a forked `pkg::t()` with a `forever`
            // body kept its parent's `#25 $finish` from ever running.
            if let StatementKind::Expr(expr) = &stmt.kind {
                if let ExprKind::Call { func, args } = &expr.kind {
                    if let Some(td) = self.package_task_target(func) {
                        if self.stmts_have_blocking(&td.items) {
                            let mut cleanup = self.bind_task_frame(&td, args, None);
                            // §13.4/§13.3.2 class-context shield — see the
                            // bare free-task arm above (Stage 1).
                            self.push_instance_task_context(&mut cleanup);
                            self.task_cleanup.push(cleanup);
                            let mut cont: Vec<Statement> = td.items.clone();
                            cont.push(Statement::new(StatementKind::ScopePop, stmt.span));
                            self.run_process_stmts(pid, &pc.pushed(cont, pc.start + i + 1));
                            return;
                        }
                    }
                }
            }

            // Stage 1-hier: a call to a blocking HIERARCHICAL or interface
            // task — `u_m.sample(args)` / `vif.sample(args)` (LRM §25.5.4,
            // §25.9) — flattens to Call{func: Ident([...])} with 2+ segments.
            // Stage 1 above only matches single-segment names, so these fell
            // through to the synchronous path, whose event controls ABORT the
            // body instead of suspending: a monitor BFM's `@(negedge pclk)`
            // returned immediately and the proxy's forever loop spun at t=0
            // (found running a public UVM AVIP). Resolve exactly like the
            // synchronous Call arm (vif alias on the head, then the joined
            // path, then hint-prefixed) and inline when the body blocks; the
            // frame carries the instance scope so the body's bare interface
            // member names resolve across suspends.
            if let StatementKind::Expr(expr) = &stmt.kind {
                if let ExprKind::Call { func, args } = &expr.kind {
                    {
                        {
                            let full = self.resolve_hier_task_target(func);
                            if let Some(full) = full {
                                let td = self.task_decl_rc(&full).unwrap();
                                if self.stmts_have_blocking(&td.items) {
                                    let scope = full
                                        .rsplit_once('.')
                                        .map(|(sc, _)| sc.to_string())
                                        .unwrap_or_default();
                                    let mut cleanup = self.bind_task_frame(&td, args, None);
                                    self.push_instance_task_context(&mut cleanup);
                                    cleanup.prev_hint = self.name_resolve_hint.borrow().clone();
                                    cleanup.frame_scope_hint = Some(scope.clone());
                                    *self.name_resolve_hint.borrow_mut() = Some(scope);
                                    self.task_cleanup.push(cleanup);
                                    let mut cont: Vec<Statement> = td.items.clone();
                                    cont.push(Statement::new(StatementKind::ScopePop, stmt.span));
                                    self.run_process_stmts(pid, &pc.pushed(cont, pc.start + i + 1));
                                    return;
                                }
                            }
                        }
                    }
                }
            }

            // Stage 1b: a call to a blocking CLASS METHOD (a `task` with a
            // top-level wait/#/@/forever in its body) — bare `m(args)` on the
            // current `this`, or `obj.m(args)` — is INLINED like the free-task
            // case above: bind the frame, splice body + ScopePop + the rest,
            // recurse. The method's waits then suspend the process instead of
            // spinning to the loop cap. Without this, run_phase bodies that call
            // e.g. `collect_data()` (`forever @clk`) or `seq.start()` run
            // synchronously and hang at t=0. Non-blocking methods (functions, or
            // tasks with no top-level wait) keep the synchronous path.
            if let StatementKind::Expr(expr) = &stmt.kind {
                if let ExprKind::Call { func, args } = &expr.kind {
                    // Preserve receivers evaluated by this probe, including
                    // selected elements, for the nonblocking fallback.
                    let mut call_recv: Option<(usize, String)> = None;
                    let resolved: Option<(usize, String)> = match &func.kind {
                        // (receiver_handle, method_name)
                        ExprKind::Ident(h) if h.path.len() == 1 => self
                            .this_stack
                            .last()
                            .copied()
                            .flatten()
                            .map(|hh| (hh, h.path[0].name.name.clone())),
                        // `a.m(args)` and deeper chains `a.b.m(args)` both
                        // parse as a flattened multi-segment Ident; the
                        // receiver is the path minus the trailing method
                        // segment. Only the 2-segment form was recognised, so
                        // a blocking method reached through a nested handle
                        // (`w.fifo.peek_it(x)` — the uvm_tlm_fifo shape) fell
                        // through to the SYNCHRONOUS executor, where a
                        // blocking mailbox peek cannot suspend and returned
                        // garbage immediately.
                        ExprKind::Ident(h) if h.path.len() >= 2 => {
                            let mut head = h.clone();
                            let last = head.path.pop().unwrap();
                            let recv = Expression::new(ExprKind::Ident(head), expr.span);
                            let selected = matches!(&recv.kind, ExprKind::Ident(hp)
                                if hp.path.iter().any(|s| !s.selects.is_empty()));
                            let handle = if selected {
                                if self
                                    .class_member_names()
                                    .tasks
                                    .contains(last.name.name.as_str())
                                {
                                    let handle = self.eval_live_object_receiver(&recv);
                                    call_recv = handle.map(|hh| (hh, last.name.name.clone()));
                                    handle
                                } else {
                                    None
                                }
                            } else {
                                match self.eval_handle_expr(&recv) {
                                    // §23.6: a hierarchical receiver
                                    // (`u_mem.obj.t()`) names another
                                    // instance's handle, which the
                                    // frame-local walk above cannot read;
                                    // the call ran synchronously and a
                                    // blocking body's event control was
                                    // dropped.
                                    None if self
                                        .class_member_names()
                                        .tasks
                                        .contains(last.name.name.as_str()) =>
                                    {
                                        let handle = self.eval_live_object_receiver(&recv);
                                        call_recv = handle.map(|hh| (hh, last.name.name.clone()));
                                        handle
                                    }
                                    hh => hh,
                                }
                            };
                            handle.map(|hh| (hh, last.name.name))
                        }
                        ExprKind::MemberAccess { expr: recv, member } => {
                            match self.eval_handle_expr(recv) {
                                None if !matches!(&recv.kind, ExprKind::Call { .. })
                                    && self
                                        .class_member_names()
                                        .tasks
                                        .contains(member.name.as_str()) =>
                                {
                                    let handle = self.eval_live_object_receiver(recv);
                                    call_recv = handle.map(|hh| (hh, member.name.clone()));
                                    call_recv.clone()
                                }
                                Some(hh) => Some((hh, member.name.clone())),
                                // A receiver reached through a call result
                                // (`h.pool.get("done").wait_trigger()`) has no
                                // side-effect-free handle form, so the call
                                // ran synchronously and a blocking task
                                // returned at once. For a name some class
                                // declares as a task, run the receiver call
                                // here and keep its result.
                                None if self
                                    .class_member_names()
                                    .tasks
                                    .contains(member.name.as_str()) =>
                                {
                                    match Self::recv_call_node(recv) {
                                        Some(call) => {
                                            let v = self.eval_expr(call);
                                            let hh = self.recv_chain_handle(
                                                v.to_u64().unwrap_or(0) as usize,
                                                recv,
                                            );
                                            call_recv = Some((hh, member.name.clone()));
                                            Some((hh, member.name.clone()))
                                        }
                                        None => None,
                                    }
                                }
                                None => None,
                            }
                        }
                        _ => None,
                    };
                    if let Some((rh, mn)) = resolved {
                        if rh != 0 {
                            let is_super = matches!(&func.kind, ExprKind::MemberAccess { expr: recv, .. }
                                if matches!(&recv.kind, ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].name.name == "super"));
                            let cls = if is_super {
                                self.class_context_stack
                                    .last()
                                    .cloned()
                                    .flatten()
                                    .and_then(|c| self.module.classes.get(&c))
                                    .and_then(|cd| cd.extends.clone())
                            } else {
                                self.heap
                                    .get(rh)
                                    .and_then(|o| o.as_ref())
                                    .map(|inst| inst.class_name.clone())
                            };
                            if let Some(cls) = cls {
                                if let Some((tm, mclass)) = self.resolve_class_task(&cls, &mn) {
                                    let crate::ast::decl::ClassMethodKind::Task(td) = &tm.kind
                                    else {
                                        unreachable!("resolve_class_task yields tasks only")
                                    };
                                    if self.stmts_have_blocking(&td.items) {
                                        let spec = self
                                            .task_method_spec(Some(rh), &mclass)
                                            .or_else(|| self.current_spec.clone());
                                        let mut cleanup = self.bind_task_frame(
                                            td,
                                            args,
                                            Some((spec, Some(rh), mclass.clone())),
                                        );
                                        // Even a same-object bare call changes
                                        // the defining-class context used by
                                        // `super`; push both stacks uniformly.
                                        self.push_task_method_this(
                                            Some(rh),
                                            mclass.clone(),
                                            &mut cleanup,
                                        );
                                        self.task_cleanup.push(cleanup);
                                        // Wait-free compiled fast path (see the
                                        // twin site below): a compiling task
                                        // body runs synchronously; skip the
                                        // process-frame machinery.
                                        if let Some(_v) =
                                            self.try_compiled_task_body(rh, &mclass, &mn, td)
                                        {
                                            if let Some(c) = self.task_cleanup.pop() {
                                                self.unwind_task_frame(c);
                                            }
                                            // The body ran synchronously; fall
                                            // through to the caller's next
                                            // statement (`i += 1`) instead of
                                            // returning — `return` would drop
                                            // the rest of this process's slice
                                            // (the `#5`/item_done tail of a
                                            // tx_driver loop), killing the
                                            // handshake pipeline.
                                            i += 1;
                                            continue;
                                        }
                                        self.refresh_bound_task_string_kinds(&td.ports);
                                        let frame = self.class_task_body_frame(&tm, stmt.span);
                                        // Chain the caller's tail instead of copying it onto the end of
                                        // the spliced body (ProcCont::pushed_frame).
                                        self.run_process_stmts(
                                            pid,
                                            &pc.pushed_frame(frame, pc.start + i + 1),
                                        );
                                        return;
                                    }
                                }
                            }
                        }
                    }
                    // Not inlined: run the call with the receiver value
                    // already computed above rather than evaluating it again.
                    if let Some((rh, member)) = call_recv {
                        let is_obj = rh != 0
                            && self.heap.get(rh).is_some_and(|o| o.is_some())
                            && !self.mailboxes.contains_key(&rh)
                            && !self.semaphores.contains_key(&rh);
                        if is_obj && self.cg_index(rh).is_none() {
                            self.exec_method_call(rh, &member, args);
                            i += 1;
                            continue;
                        }
                        // Not a class object (a mailbox, semaphore, process
                        // or null handle, or a collection property): leave
                        // the call to the later stages and the synchronous
                        // path, which evaluate the receiver themselves.
                    }
                }
            }

            // Stage 1c: a blocking STATIC method call
            // `Class::method()` with no receiver handle — Stage 1b skipped it
            // because `path[0]` is a class name, not a handle var. For a static
            // method containing blocking calls (e.g. `forever begin hopper.get(...); fork ... join_none; end`),
            // running synchronously prevents its blocking `get` from suspending.
            // Inline it (static context = the class, null `this`) so those waits
            // suspend the process.
            if let StatementKind::Expr(expr) = &stmt.kind {
                if let ExprKind::Call { func, args } = &expr.kind {
                    // `Class::method()` reaches here in two parse shapes:
                    // a flattened 2-segment Ident `[Class, method]`, OR a
                    // MemberAccess `{ expr: Ident(Class), member: member }`
                    // (the `::` static form). Stage 1b already tried — and
                    // failed — to resolve the receiver as a handle, so a
                    // bare class name reaching here is a static call.
                    let scoped: Option<(String, String, Option<(String, String)>)> =
                        match &func.kind {
                            // §8.10, §13.3: a bare `m(args)` inside a static
                            // method (no `this`) names a method of the
                            // enclosing class. Stage 1b needs a live `this`,
                            // so without this arm a blocking static task
                            // called by bare name ran synchronously and its
                            // waits (a mailbox `get`, `@`, `wait`) returned
                            // at once.
                            ExprKind::Ident(h) if h.path.len() == 1 => self
                                .static_class_context()
                                .map(|c| (c, h.path[0].name.name.clone(), None)),
                            ExprKind::Ident(h)
                                if h.path.len() == 2
                                    && self.module.classes.contains_key(&h.path[0].name.name) =>
                            {
                                Some((
                                    h.path[0].name.name.clone(),
                                    h.path[1].name.name.clone(),
                                    None,
                                ))
                            }
                            ExprKind::Ident(h) if h.path.len() == 2 => {
                                // A module/package-scoped TYPEDEF receiver
                                // (`T::method` where `T` aliases a class or a
                                // specialization, e.g. `typedef
                                // uvm_config_db#(uvm_bitstream_t)
                                // uvm_config_int;`). The typedef alias is not
                                // itself a class, so follow it to the real class
                                // (and its specialization) so a blocking static
                                // task call is recognized and inlined like the
                                // `Class::` / `Class#(params)::` spellings.
                                let tname = &h.path[0].name.name;
                                let spec = self.resolve_typedef_spec(tname);
                                spec.clone()
                                    .map(|(b, _s)| (b, h.path[1].name.name.clone(), spec))
                                    .or_else(|| {
                                        self.resolve_simple_typedef_class(tname)
                                            .map(|c| (c, h.path[1].name.name.clone(), None))
                                    })
                            }
                            ExprKind::MemberAccess { expr: recv, member } => match &recv.kind {
                                // `C#(T)::method(...)` — the receiver is a
                                // parameterized static class. Stage 1b's receiver
                                // is a HANDLE; a `Specialization` receiver is a
                                // TYPE, so it reaches here (static form). Build
                                // the concrete specialized class name and classify
                                // its blocking method like the plain static form.
                                ExprKind::Specialization {
                                    base,
                                    type_args_text,
                                } => {
                                    let bn = Self::leaf_ident_name(base).unwrap_or_default();
                                    if bn.is_empty() {
                                        None
                                    } else {
                                        let spec = format!(
                                            "{}#({})",
                                            bn,
                                            Self::normalize_spec_ws(type_args_text)
                                        );
                                        let mut cand = vec![spec.clone(), bn];
                                        cand.retain(|c| self.module.classes.contains_key(c));
                                        cand.first().map(|c| (c.clone(), member.name.clone(), None))
                                    }
                                }
                                ExprKind::Ident(h) if h.path.len() == 1 => {
                                    let bn = &h.path[0].name.name;
                                    if self.module.classes.contains_key(bn) {
                                        Some((bn.clone(), member.name.clone(), None))
                                    } else {
                                        // A module/package TYPEDEF receiver
                                        // (`uvm_config_int::wait_modified` where
                                        // `typedef uvm_config_db#(uvm_bitstream_t)
                                        // uvm_config_int;`): the typedef alias is
                                        // not itself a class, so the static-call
                                        // dispatcher must follow it to the real
                                        // class (and its specialization, so the
                                        // static-collection store keys match the
                                        // `Class#(params)::` spelling) before
                                        // inlining a blocking task. Otherwise the
                                        // call is treated as synchronous and its
                                        // event wait spins forever.
                                        let spec = self.resolve_typedef_spec(bn);
                                        let simple = if spec.is_none() {
                                            self.resolve_simple_typedef_class(bn)
                                                .map(|c| (c.clone(), member.name.clone(), None))
                                        } else {
                                            None
                                        };
                                        spec.clone()
                                            .map(|(b, _s)| (b, member.name.clone(), spec))
                                            .or(simple)
                                    }
                                }
                                _ => None,
                            },
                            _ => None,
                        };
                    if let Some((cls, mn, tf_spec)) = scoped {
                        // A `C#(params)::task` receiver dispatches on the
                        // bare class but must ALSO establish the receiver
                        // specialization as `current_spec`, or a bare type
                        // argument `T` in the task body (e.g.
                        // `tester#(uvm_object)::do_it()` constructing
                        // `my_cb#(T)` / `uvm_event#(T)`) resolves to the
                        // literal `T` instead of `uvm_object`. The static
                        // METHOD path seeds `current_spec` from the
                        // specialization (§8.25); this static-task path was
                        // missing it, so the body bound its parameterized
                        // locals with `type_bindings={T:"T"}`, casting to
                        // `uvm_callback` failed, and every `$cast` inside the
                        // callback `get_all` saw a plain `uvm_callback` with
                        // no callbacks. A TYPEDEF receiver (`T::task`, T a
                        // typedef alias of a parameterized class) has no
                        // `Specialization` receiver to seed `current_spec` from,
                        // so `tf_spec` (resolved when the typedef was followed to
                        // its class above) carries the same specialization here.
                        let mut recv_spec: Option<(String, String)> = tf_spec;
                        if recv_spec.is_none() {
                            if let ExprKind::MemberAccess { expr: mrecv, .. } = &func.kind {
                                if let ExprKind::Specialization { .. } = &mrecv.kind {
                                    recv_spec = self.resolve_call_spec_params(
                                        Self::extract_call_spec(&func.clone()),
                                        &self.current_spec,
                                    );
                                }
                            }
                        }
                        if let Some((tm, mclass)) = self.resolve_class_task(&cls, &mn) {
                            let crate::ast::decl::ClassMethodKind::Task(td) = &tm.kind else {
                                unreachable!("resolve_class_task yields tasks only")
                            };
                            if self.stmts_have_blocking(&td.items) {
                                let spec = recv_spec.clone().or_else(|| self.current_spec.clone());
                                let mut cleanup = self.bind_task_frame(
                                    td,
                                    args,
                                    Some((spec, None, mclass.clone())),
                                );
                                // Static context: push the declaring class for
                                // member/static resolution, with a null `this`.
                                self.m_scope_stack.clear();
                                self.this_stack.push(None);
                                self.class_context_stack.push(Some(mclass));
                                self.method_local_base
                                    .push(self.local_stack.len().saturating_sub(1));
                                cleanup.pushed_method_this = true;
                                let saved_spec = self.current_spec.clone();
                                // The frame's unwind restores the caller's
                                // specialization (§8.25).
                                cleanup.saved_spec = saved_spec.clone();
                                if let Some((sb, ss)) = recv_spec {
                                    self.ensure_spec_statics(&sb, &ss);
                                    self.current_spec = Some((sb, ss));
                                }
                                self.refresh_bound_task_string_kinds(&td.ports);
                                self.task_cleanup.push(cleanup);
                                let frame = self.class_task_body_frame(&tm, stmt.span);
                                // Chain the caller's tail instead of copying it onto the end of
                                // the spliced body (ProcCont::pushed_frame).
                                self.run_process_stmts(
                                    pid,
                                    &pc.pushed_frame(frame, pc.start + i + 1),
                                );
                                // A parked continuation is still executing
                                // inside its innermost task frame. Snapshot
                                // that binding; ScopePop restores the caller.
                                if !self.is_pid_suspended(pid) {
                                    self.current_spec = saved_spec;
                                }
                                return;
                            }
                        }
                    }
                }
            }

            // LRM §15.4.2: blocking mailbox.get(var) on an empty mailbox.
            // When the next statement is `<mb>.get(<lvalue>);` and the
            // resolved mailbox queue is empty, suspend this process: park
            // the destination expression + post-get continuation in
            // mailbox_get_waiters and return. A subsequent `put` will
            // assign + reschedule the continuation.
            if let StatementKind::Expr(expr) = &stmt.kind {
                if let ExprKind::Call { func, args } = &expr.kind {
                    // `mb.get(v)` parses as either
                    //   Call{ func: Ident(hier=[mb,get]), args=[v] }   (common)
                    //   Call{ func: MemberAccess{expr=mb, member=get} }
                    // Recognise both forms; the receiver in the hier-Ident
                    // case is the path with the last segment stripped.
                    let (recv_expr_opt, method): (Option<Expression>, String) = match &func.kind {
                        ExprKind::MemberAccess { expr: recv, member } => {
                            (Some((**recv).clone()), member.name.clone())
                        }
                        ExprKind::Ident(hier) if hier.path.len() >= 2 => {
                            let mut head = hier.clone();
                            let last = head.path.pop().unwrap();
                            let recv = Expression::new(ExprKind::Ident(head), expr.span);
                            (Some(recv), last.name.name)
                        }
                        _ => (None, String::new()),
                    };
                    // §25.9: an interface task may also be called `get` or
                    // `put`. Probing its getter as a mailbox receiver would
                    // run that function before the actual task dispatch.
                    let vif_receiver = match recv_expr_opt.as_ref().map(|e| &e.kind) {
                        Some(ExprKind::Call { func, .. }) => self
                            .call_return_type(func)
                            .is_some_and(|dt| self.is_virtual_iface_type(&dt)),
                        _ => false,
                    };
                    if !vif_receiver && (method == "get" || method == "peek") && !args.is_empty() {
                        if let Some(recv) = recv_expr_opt.clone() {
                            let recv_val = self.eval_expr(&recv);
                            let handle = recv_val.to_u64().unwrap_or(0) as usize;
                            // §15.4.2: `get`/`peek` on a mailbox handle that was
                            // never `new`ed. The parking path below is keyed on a
                            // LIVE handle, so a null one fell through to generic
                            // dispatch and returned immediately without blocking.
                            // Wrapped in the usual `forever` that reads a mailbox,
                            // that spins until the stall detector fires — which
                            // then blames a missing timing control, sending the
                            // user looking for a `#delay` in a loop whose real
                            // fault is an unconstructed mailbox. Fail where the
                            // fault actually is.
                            if handle == 0 && self.is_declared_mailbox(&recv) {
                                // Named from the receiver EXPRESSION, not from
                                // its span: the `Ident([mb, get])` parse shape
                                // is rebuilt with the whole call's span, so a
                                // source snippet would quote `mb.get(v)` back
                                // as if that were the handle's name.
                                let what = match &recv.kind {
                                    ExprKind::Ident(h) => h
                                        .path
                                        .iter()
                                        .map(|s| s.name.name.as_str())
                                        .collect::<Vec<_>>()
                                        .join("."),
                                    _ => self
                                        .span_source_snippet_in(recv.span, None)
                                        .unwrap_or_else(|| "<mailbox>".to_string()),
                                };
                                eprintln!(
                                    "[xezim][error] null mailbox handle at time {} — `{}.{}(...)` \
                                     on a mailbox that was never constructed.",
                                    self.time, what, method
                                );
                                eprintln!(
                                    "               A blocking `{}` cannot suspend on a null \
                                     handle, so a `forever` loop around it would spin forever.",
                                    method
                                );
                                eprintln!(
                                    "               Construct it (`{} = new();`) before the \
                                     first `{}`.",
                                    what, method
                                );
                                self.abort_run();
                                return;
                            }
                            let mbx_empty = self
                                .mailboxes
                                .get(&handle)
                                .map(|q| q.is_empty())
                                .unwrap_or(false);
                            if mbx_empty {
                                // §15.4.1/§15.4.2: a producer parked on a FULL
                                // bounded mailbox can proceed now that the box is
                                // empty. Only a consuming `get` used to admit it,
                                // so a get that PARKED on the empty box left both
                                // sides asleep — a zero-delay producer/consumer
                                // pair deadlocked after filling the bound once.
                                self.admit_mailbox_put_waiter(handle);
                            }
                            if let Some(q) = self.mailboxes.get(&handle) {
                                if q.is_empty() {
                                    // Blocking get/peek on an empty mailbox: park
                                    // until a put hands over a value. peek leaves
                                    // the item in the box (`m_req_fifo.peek` in
                                    // `get_next_item`, then `item_done`
                                    // try_gets it).
                                    let lvalue = args[0].clone();
                                    let cont = pc.resume_at(pc.start + i + 1);
                                    self.mailbox_get_waiters
                                        .entry(handle)
                                        .or_default()
                                        .push_back(MailboxGetWaiter {
                                            pid,
                                            lvalue,
                                            cont,
                                            is_peek: method == "peek",
                                        });
                                    if env_set_cached!("XEZIM_MBX_DBG") {
                                        eprintln!(
                                            "[MBX] t={} pid={} PARKED {} handle={}",
                                            self.time, pid, method, handle
                                        );
                                    }
                                    return;
                                }
                            }
                        }
                    }
                    // §15.4.1: blocking `mailbox.put(v)` on a FULL bounded
                    // mailbox. Capture the value now, park the producer, and
                    // return; a later get/try_get admits it and resumes here.
                    if !vif_receiver && method == "put" && !args.is_empty() {
                        if let Some(recv) = recv_expr_opt.clone() {
                            let recv_val = self.eval_expr(&recv);
                            let handle = recv_val.to_u64().unwrap_or(0) as usize;
                            let bound = self.mailbox_bound.get(&handle).copied().unwrap_or(0);
                            if bound > 0 {
                                let len = self.mailboxes.get(&handle).map(|q| q.len()).unwrap_or(0);
                                if len >= bound {
                                    let value = self.mbx_message(&args[0]);
                                    let cont = pc.resume_at(pc.start + i + 1);
                                    self.mailbox_put_waiters
                                        .entry(handle)
                                        .or_default()
                                        .push_back(MailboxPutWaiter { pid, value, cont });
                                    return;
                                }
                            }
                        }
                    }
                    // §15.3.3: blocking `semaphore.get(n)` on an under-full
                    // semaphore. Park until a `put` raises the count enough; the
                    // decrement happens at wake. A get that CAN proceed falls
                    // through to the method handler, which decrements there.
                    if !vif_receiver && method == "get" {
                        if let Some(recv) = recv_expr_opt {
                            let recv_val = self.eval_expr(&recv);
                            let handle = recv_val.to_u64().unwrap_or(0) as usize;
                            if let Some(&count) = self.semaphores.get(&handle) {
                                let n = args
                                    .first()
                                    .map(|a| self.eval_expr(a).to_u64().unwrap_or(1))
                                    .unwrap_or(1) as i64;
                                if count < n {
                                    let cont = pc.resume_at(pc.start + i + 1);
                                    self.semaphore_get_waiters
                                        .entry(handle)
                                        .or_default()
                                        .push_back(SemGetWaiter { pid, n, cont });
                                    return;
                                }
                            }
                        }
                    }
                }
            }

            // IEEE 1800-2017 §9.4.5 intra-assignment EVENT control
            // `lhs = [repeat(n)] @(edge sig) rhs` (canonicalized to
            // `$__xz_intra_ev(n, edge, sig, rhs)`): evaluate the RHS NOW,
            // then wait the event n times, then assign the saved value.
            // Expand into n chained event controls + the saved assign so the
            // existing top-level TimingControl::Event park machinery does
            // the waiting.
            if let StatementKind::BlockingAssign { lvalue, rvalue } = &stmt.kind {
                if let ExprKind::SystemCall { name, args } = &rvalue.kind {
                    if name == crate::intra_delay::INTRA_EVENT_MARKER && args.len() == 4 {
                        let n_val = self.eval_expr(&args[0]).to_u64().unwrap_or(0) as i64;
                        let edge_code = self.eval_expr(&args[1]).to_u64().unwrap_or(0);
                        let val = self.eval_target_rhs(lvalue, &args[3]);
                        let saved = self.make_intra_saved_expr(val, rvalue.span);
                        let assign = Statement::new(
                            StatementKind::BlockingAssign {
                                lvalue: lvalue.clone(),
                                rvalue: saved,
                            },
                            stmt.span,
                        );
                        let mut cont: Vec<Statement> = Vec::new();
                        // §9.4.5: a zero/negative repeat count degenerates to
                        // an immediate assignment (no event wait).
                        let edge = match edge_code {
                            1 => Some(Edge::Posedge),
                            2 => Some(Edge::Negedge),
                            _ => None,
                        };
                        for _ in 0..n_val.max(0) {
                            cont.push(Statement::new(
                                StatementKind::TimingControl {
                                    control: TimingControl::Event(EventControl::EventExpr(vec![
                                        EventExpr {
                                            edge,
                                            expr: args[2].clone(),
                                            iff: None,
                                            span: stmt.span,
                                        },
                                    ])),
                                    stmt: Box::new(Statement::new(StatementKind::Null, stmt.span)),
                                },
                                stmt.span,
                            ));
                        }
                        cont.push(assign);
                        // Chain the caller's tail instead of copying it onto the end of
                        // the spliced body (ProcCont::pushed).
                        self.run_process_stmts(pid, &pc.pushed(cont, pc.start + i + 1));
                        return;
                    }
                }
            }

            // IEEE 1800-2017 §9.4.5 intra-assignment delay `lhs = #d rhs`:
            // the RHS is evaluated NOW, the process suspends d time units,
            // then the pre-computed value is assigned — i.e. behave like
            // `#d;` followed by `lhs = <saved value>;`.
            if let StatementKind::BlockingAssign { lvalue, rvalue } = &stmt.kind {
                if let Some((d_expr, rhs)) = Self::intra_delay_marker(rvalue) {
                    let val = self.eval_expr(rhs);
                    let delay = self.eval_delay_ticks(d_expr);
                    let saved = self.make_intra_saved_expr(val, rvalue.span);
                    let cont = vec![Statement::new(
                        StatementKind::BlockingAssign {
                            lvalue: lvalue.clone(),
                            rvalue: saved,
                        },
                        stmt.span,
                    )];
                    // Chain the caller's tail rather than copying it (ProcCont::pushed).
                    let cont = pc.pushed(cont, pc.start + i + 1);
                    self.event_queue.schedule(self.time + delay, pid, cont);
                    return;
                }
            }

            // Check for timing control — delay or event
            // §15.5.3 wait_order (a, b, c) pass else fail — a state machine
            // over the event list. Each step parks the process on ALL not-yet-
            // expected events (so an out-of-order fire wakes us too), with the
            // continuation re-entering this statement `armed`. On wake the
            // `event_triggered_time` stamps identify which event fired: the
            // expected one advances the sequence (completing it runs `pass`),
            // any later one runs `fail` (or a loud warning without an else).
            if let StatementKind::WaitOrder {
                events,
                pass,
                fail,
                armed,
                idx,
                span,
            } = &stmt.kind
            {
                let k = *idx as usize;
                let n = events.len();
                let stamp_now = |sim: &Self, nm: &str| -> bool {
                    let now = sim.time;
                    let canon = sim.resolve_event_key(nm);
                    if sim.event_triggered_time.get(canon.as_str()) == Some(&now) {
                        return true;
                    }
                    let pref = format!("{}.{}", sim.module.name, canon);
                    sim.event_triggered_time.get(pref.as_str()) == Some(&now)
                };
                let mut next_k = k;
                let mut outcome: Option<bool> = None; // Some(true)=pass, Some(false)=fail
                if *armed {
                    let expected_fired = k < n && stamp_now(self, &events[k].name);
                    let later_fired = events[k.saturating_add(1)..]
                        .iter()
                        .any(|e| stamp_now(self, &e.name));
                    if expected_fired {
                        if k + 1 == n {
                            outcome = Some(true);
                        } else {
                            next_k = k + 1;
                        }
                    } else if later_fired {
                        outcome = Some(false);
                    }
                    // Neither fired this slot (spurious wake): re-park at k.
                }
                match outcome {
                    Some(ok) => {
                        let action = if ok { pass } else { fail };
                        if !ok && fail.is_none() {
                            eprintln!(
                                "[xezim][warning] wait_order sequence violation at time {}: an event fired out of order and the construct has no else clause (IEEE 1800-2017 §15.5.3)",
                                self.time
                            );
                        }
                        let mut cont: Vec<Statement> = Vec::new();
                        if let Some(a) = action {
                            match &a.kind {
                                StatementKind::SeqBlock { stmts: b, .. } => {
                                    cont.extend_from_slice(b)
                                }
                                _ => cont.push((**a).clone()),
                            }
                        }
                        // Chain the caller's tail instead of copying it onto the end of
                        // the spliced body (ProcCont::pushed).
                        self.run_process_stmts(pid, &pc.pushed(cont, pc.start + i + 1));
                        return;
                    }
                    None => {
                        // Park on events[next_k..]; continuation re-enters armed.
                        let evs: Vec<crate::ast::stmt::EventExpr> = events[next_k..]
                            .iter()
                            .map(|id| crate::ast::stmt::EventExpr {
                                edge: None,
                                expr: Expression::new(
                                    ExprKind::Ident(crate::ast::expr::HierarchicalIdentifier {
                                        root: None,
                                        path: vec![crate::ast::expr::HierPathSegment {
                                            name: id.clone(),
                                            selects: Vec::new(),
                                        }],
                                        span: *span,
                                        cached_signal_id: std::cell::Cell::new(None),
                                        cached_resolved_name: std::cell::OnceCell::new(),
                                    }),
                                    *span,
                                ),
                                iff: None,
                                span: *span,
                            })
                            .collect();
                        let sens =
                            self.event_to_sens(&crate::ast::stmt::EventControl::EventExpr(evs));
                        let cont = vec![Statement::new(
                            StatementKind::WaitOrder {
                                events: events.clone(),
                                pass: pass.clone(),
                                fail: fail.clone(),
                                armed: true,
                                idx: next_k as u32,
                                span: *span,
                            },
                            stmt.span,
                        )];
                        // Chain the caller's tail rather than copying it (ProcCont::pushed).
                        let cont = pc.pushed(cont, pc.start + i + 1);
                        if !sens.is_empty()
                            && sens.iter().any(|s| {
                                self.signal_name_to_id.contains_key(s.signal_name.as_str())
                            })
                        {
                            {
                                let w = self.make_event_waiter(pid, sens, cont);
                                self.event_waiters.push(w);
                            }
                        } else {
                            eprintln!(
                                "[xezim][warning] wait_order at time {}: none of the named events resolve to declared events; process will not resume (IEEE 1800-2017 §15.5.3)",
                                self.time
                            );
                        }
                        return;
                    }
                }
            }

            if let StatementKind::TimingControl {
                control,
                stmt: body,
            } = &stmt.kind
            {
                match control {
                    TimingControl::Delay(d) => {
                        let delay = self.eval_delay_ticks(d);
                        // A NON-LITERAL `#(expr)` that evaluates to 0 at the
                        // very start of time 0 almost always means its period
                        // signal has not settled yet: e.g. `assign xbOH =
                        // dTx*d0j;` feeding `always #(xbOH/2) clk=~clk;`, where
                        // the clock process (a lower pid, scheduled during
                        // classify_always_blocks) runs before the `initial`
                        // block that seeds `dTx`. Toggling now would drop a
                        // spurious edge at t=0 and INVERT the clock's phase for
                        // the whole run (so a later strobe samples the wrong
                        // half-cycle). Re-park the WHOLE timing control — not
                        // the post-`body` continuation — so `#(expr)` RE-
                        // EVALUATES after the time-0 active+NBA+settle has run
                        // the initial blocks and propagated the cont-assign.
                        // One-shot per pid (`t0_delay_deferred`): a genuine
                        // zero-period loop re-parks the continuation as usual on
                        // its second pass. Literal `#0` is untouched.
                        if delay == 0
                            && self.time == 0
                            && !matches!(d.kind, ExprKind::Number(_))
                            && self.t0_delay_deferred.insert(pid)
                        {
                            let whole = vec![Statement::new(
                                StatementKind::TimingControl {
                                    control: TimingControl::Delay(d.clone()),
                                    stmt: body.clone(),
                                },
                                stmt.span,
                            )];
                            // Chain the caller's tail rather than copying it (ProcCont::pushed).
                            let whole = pc.pushed(whole, pc.start + i + 1);
                            self.inactive_queue.push((pid, whole));
                            return;
                        }
                        // Chain the caller's tail rather than copying it (ProcCont::pushed_body).
                        let cont = pc.pushed_body(body, pc.start + i + 1);
                        if delay == 0 {
                            // `#0` — IEEE 1800-2017 §4.4.2.3: suspend into the
                            // Inactive region of the SAME time slot, not the
                            // Active region. Scheduling into event_queue at
                            // self.time would resume it in the same batch
                            // drain, BEFORE apply_nba — so an NBA posted
                            // before the `#0` would not be visible after it.
                            // Commercial consensus: it IS
                            // visible. Park here; run_one_tick promotes after
                            // the NBA region of this tick has been applied.
                            self.inactive_queue.push((pid, cont));
                        } else {
                            self.event_queue.schedule(self.time + delay, pid, cont);
                        }
                        return;
                    }
                    TimingControl::Event(event) => {
                        // §9.4.2 / §15.5: an OR of class events parks on
                        // their identities, not signal sensitivities (class
                        // events have no signal-table slot). One waiter owns
                        // the whole group and is removed after its first wake.
                        if let EventControl::EventExpr(terms) = event
                            && terms.len() > 1
                            && terms.iter().all(|t| {
                                t.edge.is_none()
                                    && t.iff.is_none()
                                    && Self::class_scope_chain(&t.expr).is_some()
                            })
                        {
                            let keys: Option<Vec<_>> = terms
                                .iter()
                                .map(|term| {
                                    self.expr_instance_event_field_general(&term.expr)
                                        .or_else(|| {
                                            Self::plain_ident_name(&term.expr)
                                                .and_then(|n| self.resolve_this_event_field(&n))
                                        })
                                        .or_else(|| self.static_class_event_key(&term.expr))
                                })
                                .collect();
                            if let Some(keys) = keys {
                                self.instance_event_waiters.push(InstanceEventWaiter {
                                    key: InstanceEventKeys::Any(keys),
                                    pid,
                                    continuation: pc.pushed_body(body, pc.start + i + 1),
                                });
                                return;
                            }
                        }
                        // §14.11 `##0`: SYNCHRONIZE to the default clocking
                        // event — a no-op when the process is already
                        // executing in the time slot of that event (its edge
                        // fired at the current time); otherwise park exactly
                        // like `@(__xz_default_clocking)`.
                        if let EventControl::Identifier(id) = event {
                            if id.name == "__xz_default_clocking0"
                                && self
                                    .default_clocking_cb
                                    .as_ref()
                                    .and_then(|cb| self.clocking_last_edge.get(cb))
                                    == Some(&self.time)
                            {
                                // The loop increments `i` at its bottom; a bare
                                // `continue` would re-execute this statement
                                // forever.
                                self.exec_statement(body);
                                i += 1;
                                continue;
                            }
                        }
                        // Suspend process until the event fires
                        // Class-field named event (`@m_event` inside a method
                        // on `this`): park on the per-instance event identity.
                        // A class `event` field has no backing signal, so
                        // without this it fell through to the delta-yield
                        // (NBA) branch below and `@m_event` returned
                        // immediately — breaking class event / heartbeat
                        // synchronization (a `start()`ed heartbeat died at t=0
                        // after one spurious check). Module-scope named events
                        // are backed by real signals and take the event_waiters
                        // path further below. Inside a class method the
                        // elaborator rewrites `@field` to a `HierIdentifier`,
                        // so accept both shapes.
                        if let Some(fname) = self.event_control_field_name(event) {
                            if let Some(key) = self.resolve_this_event_field(&fname) {
                                // Chain the caller's tail rather than copying it (ProcCont::pushed_body).
                                let cont = pc.pushed_body(body, pc.start + i + 1);
                                self.instance_event_waiters.push(InstanceEventWaiter {
                                    key: InstanceEventKeys::One(key),
                                    pid,
                                    continuation: cont,
                                });
                                return;
                            }
                        }
                        // `@(h.ce)`: a class-property event reached through a
                        // handle from OUTSIDE the class. `resolve_this_event_field`
                        // above only covers a bare field on `this`; the general
                        // resolver adds receivers with runtime selects
                        // (`@(m_events[obj].all_dropped)` — the UVM objection
                        // wait, #109) and chained handles.
                        if let Some(expr) = Self::event_control_single_expr(event) {
                            if let Some(key) = self.expr_instance_prop_field_general(&expr) {
                                // §9.4.2: a selected or nested ordinary
                                // property waits for a value change, too.
                                if !self.class_prop_is_event(key.0, &key.1) {
                                    let cond = self.value_change_cond(expr);
                                    let wait = Self::vc_wait_stmt(cond.clone(), body, stmt.span);
                                    let cont = pc.pushed(vec![wait], pc.start + i + 1);
                                    self.park_condition_waiter(pid, cont, &cond);
                                    return;
                                }
                                // Chain the caller's tail rather than copying it.
                                let cont = pc.pushed_body(body, pc.start + i + 1);
                                self.instance_event_waiters.push(InstanceEventWaiter {
                                    key: InstanceEventKeys::One(key),
                                    pid,
                                    continuation: cont,
                                });
                                return;
                            }
                        }
                        // §15.5: `@ev` / `@(C::ev)` on a STATIC class event.
                        let static_ev = if self.class_member_names().static_or_param.is_empty() {
                            None
                        } else {
                            match event {
                                EventControl::Identifier(id) => self
                                    .class_context_stack
                                    .last()
                                    .cloned()
                                    .flatten()
                                    .filter(|_| self.resolve_this_event_field(&id.name).is_none())
                                    .and_then(|c| self.static_class_event_key_named(&c, &id.name)),
                                _ => {
                                    // Only a name some class declares static
                                    // can be one; look before copying the term.
                                    let leaf = match event {
                                        EventControl::HierIdentifier(e) => Some(e),
                                        EventControl::EventExpr(t)
                                            if t.len() == 1 && t[0].edge.is_none() =>
                                        {
                                            Some(&t[0].expr)
                                        }
                                        _ => None,
                                    }
                                    .and_then(|e| {
                                        match &e.kind {
                                            ExprKind::Ident(h) => {
                                                h.path.last().map(|s| s.name.name.as_str())
                                            }
                                            ExprKind::MemberAccess { member, .. } => {
                                                Some(member.name.as_str())
                                            }
                                            _ => None,
                                        }
                                    });
                                    if leaf.is_some_and(|n| {
                                        self.class_member_names().static_or_param.contains(n)
                                    }) {
                                        Self::event_control_single_expr(event)
                                            .and_then(|e| self.static_class_event_key(&e))
                                    } else {
                                        None
                                    }
                                }
                            }
                        };
                        if let Some(key) = static_ev {
                            let cont = pc.pushed_body(body, pc.start + i + 1);
                            self.instance_event_waiters.push(InstanceEventWaiter {
                                key: InstanceEventKeys::One(key),
                                pid,
                                continuation: cont,
                            });
                            return;
                        }
                        // `@prop` / `@(h.prop)` on a NON-event class property:
                        // a value-change wait, resumed by an ordinary write
                        // of the property (never by `->`).
                        if let Some(cond) = self.class_prop_value_change_cond(event) {
                            let wait = Self::vc_wait_stmt(cond.clone(), body, stmt.span);
                            let cont = pc.pushed(vec![wait], pc.start + i + 1);
                            self.park_condition_waiter(pid, cont, &cond);
                            return;
                        }
                        // §15.5 a DECLARED named event — including an array
                        // element (`@ev[1]`) and one reached through an alias
                        // (`e1 = e2; @e1`). Both need the resolved key rather
                        // than the raw text: `event_to_sens` walks PAST an
                        // Index to the base ident, so `@ev[1]` armed on the
                        // array name `ev` (not a 1-bit signal) and fell into
                        // the delta-yield below, waking instantly at t=0. And
                        // an aliased waiter armed on its own name, so it never
                        // saw `-> e2` — only the TRIGGER side canonicalized
                        // (§15.5.4). Resolving here fixes both, and keeps the
                        // delta-yield fallback for genuine non-event locals.
                        if let Some(key) = self.event_control_event_key(event) {
                            let canon = self.resolve_event_key(&key);
                            if self.signal_name_to_id.contains_key(canon.as_str()) {
                                // Chain the caller's tail rather than copying it.
                                let cont = pc.pushed_body(body, pc.start + i + 1);
                                let sens = vec![Sensitivity {
                                    signal_name: canon,
                                    edge: EdgeKind::AnyEdge,
                                    iff: None,
                                    value_of: None,
                                }];
                                {
                                    let w = self.make_event_waiter_kind(pid, sens, cont, false);
                                    self.event_waiters.push(w);
                                }
                                return;
                            }
                        }
                        let is_star = matches!(event, EventControl::Star | EventControl::ParenStar);
                        let sens = if is_star {
                            self.star_sens_from_body(body)
                        } else {
                            self.event_to_sens(event)
                        };
                        let is_clk_ev = self.is_clocking_event(event);
                        // `@(*)` over a body that reads NOTHING can never
                        // trigger again — park the process instead of looping.
                        if is_star && sens.is_empty() {
                            return;
                        }
                        if !sens.is_empty() {
                            // Chain the caller's tail rather than copying it (ProcCont::pushed_body).
                            let cont = pc.pushed_body(body, pc.start + i + 1);
                            // Late-bind virtual-interface / handle-chained
                            // names to their concrete instance signals before
                            // deciding whether anything real is waited on.
                            let mut sens = sens;
                            for se in sens.iter_mut() {
                                if !self.signal_name_to_id.contains_key(se.signal_name.as_str()) {
                                    if let Some(resolved) =
                                        self.resolve_sens_name_via_handles(&se.signal_name)
                                    {
                                        se.signal_name = resolved;
                                    }
                                }
                            }
                            let has_real = sens.iter().any(|s| self.sens_term_resolves(s));
                            if has_real && is_clk_ev && self.clocking_event_due_now(event) {
                                if sens.iter().any(|s| s.iff.is_some()) {
                                    // §14.13: judge the `iff` after sampling.
                                    let w = self.make_event_waiter_kind(pid, sens, cont, true);
                                    self.park_clocking_guard(w);
                                } else {
                                    self.deferred_clocking_conts.push((pid, cont));
                                }
                            } else if has_real {
                                let w = self.make_event_waiter_kind(pid, sens, cont, is_clk_ev);
                                self.event_waiters.push(w);
                            } else {
                                // `@(x)` where x is not a real signal — a
                                // procedural local that was NBA-assigned then
                                // waited on (uvm_wait_for_nba_region:
                                // `nba <= next_nba; @(nba)`). Its event_waiter
                                // would resolve to no signal_id and park forever;
                                // park in nba_region_waiters so it resumes only
                                // after the active + inactive (#0) deltas have
                                // settled and the NBA region has committed.
                                self.nba_region_waiters.push((pid, cont));
                            }
                            return;
                        }
                        // Star/empty sensitivity — just execute body
                        if !is_star && env_set_cached!("XEZIM_TRACE_SPIN") {
                            eprintln!(
                                "[EMPTYSENS] pid={} t={} event={:?} span={:?}",
                                pid, self.time, event, stmt.span
                            );
                        }
                    }
                }
                self.exec_statement(body);
                i += 1;
                continue;
            }

            if let StatementKind::Wait {
                condition,
                stmt: body,
            } = &stmt.kind
            {
                if self.wait_condition_true(condition) {
                    self.vc_release(condition);
                    self.cond_progress = self.cond_progress.wrapping_add(1);
                    self.exec_statement(body);
                    i += 1;
                    continue;
                } else {
                    // IEEE 1800-2023 §9.7.4: `wait(cond)` is LEVEL-
                    // sensitive — the process resumes the moment the
                    // condition is true, including in the SAME timestep via a
                    // delta-cycle (#0) write from another process. The
                    // condition-waiter fixpoint in run_one_tick re-evaluates
                    // the actual expression every tick, so it handles same-
                    // timestep (delta), cross-timestep (#delay), and time-0
                    // init uniformly. Routing a signal-naming wait through the
                    // edge-triggered event_waiter path instead BREAKS delta-
                    // cycle wakeup: its registration-generation guard
                    // (correct for edge-sensitive `@()`) skips edges from the
                    // current snapshot window, so `wait(sig==v)` may miss a
                    // peer process writing `sig` at the same simtime. Level-sensitive
                    // condition handoffs depend on exactly this delta-cycle
                    // handoff. Always park in condition_waiters.
                    // `stmt` is already `pc.stmts[pc.start + i]`, so resuming
                    // `pc` at `pc.start + i` re-runs `stmt` followed by the
                    // tail without cloning `stmt` or allocating a wrapper frame.
                    let cont = pc.resume_at(pc.start + i);
                    self.park_condition_waiter(pid, cont, condition);
                    return;
                }
            }

            // Check for forever with delays/events. FIRST entry only — the
            // body's first iteration runs inside `exec_forever_sched`, which
            // re-appends a `ForeverTail` sentinel (not `Forever`) so that on
            // RESUME after a suspension the `ForeverTail` arm below runs the
            // break/continue/return gate. Gating here on first entry was the
            // old approach: it deadlocked a `forever` whose body only raises
            // a control flag AFTER its first iteration blocks (a stale or
            // transient flag fired before the
            // body ran even once. The sentinel split (first entry vs re-entry)
            // is what makes `break` safe now. (§9.3.3)
            if let StatementKind::Forever { body } = &stmt.kind {
                self.exec_forever_sched(pid, body, pc, i);
                return;
            }

            // INTERNAL: `ForeverTail` continuation sentinel — re-entry point
            // for a blocking-body `forever` after its body suspended (via
            // `exec_forever_sched`'s event/delay/blocking continuation). The
            // body statements up to the suspension point have just run; a
            // `break`/`continue`/`return` raised anywhere in that body slice
            // is honoured HERE (§9.3.3), which the old `Forever` re-append
            // silently dropped.
            if let StatementKind::ForeverTail { body } = &stmt.kind {
                // `return` from an inlined task body: do NOT consume — unwind
                // to the task's ScopePop (the top-of-loop return_flag skip
                // drives it). Exit this forever.
                if self.return_flag {
                    i += 1;
                    continue;
                }
                // `break` exits the forever; consume break+continue.
                if self.break_flag {
                    self.break_flag = false;
                    self.continue_flag = false;
                    i += 1;
                    continue;
                }
                // `continue` (or no flag): consume it and run the next
                // iteration. exec_forever_sched re-appends another
                // ForeverTail, so this gate runs again on the next resume.
                self.continue_flag = false;
                self.exec_forever_sched(pid, body, pc, i);
                return;
            }

            // Check for repeat with event waits inside
            if let StatementKind::Repeat { count, body } = &stmt.kind {
                let n = self.repeat_count(count);
                // Mirror the While/For arms: unroll not only when the body has
                // a direct `@event`/`#delay`, but also when it blocks via a
                // CALL to a task whose own body contains a `wait`/`#delay`/
                // `@event` (a loop like `repeat(N) obj.do_step();` where
                // do_step() blocks). Otherwise the repeat falls through to the
                // SYNCHRONOUS exec_statement, the body's calls are never
                // inlined (Stage 1b), and their nested `wait`s fall through
                // (IEEE 1800-2023 §9.7.4: a false `wait(cond)` must suspend)
                // instead of parking the process, busy-spinning the loop.
                if self.stmt_has_event_wait(body) || self.stmt_is_blocking(body) {
                    // n == 0 (initial count zero, or the natural-exhaustion
                    // sentinel re-entering after the final iteration's body):
                    // clear any loop-control flag left by that body — a
                    // trailing `continue` must not leak past the loop and
                    // suppress the statements after it. (§9.3.3)
                    if n == 0 {
                        self.break_flag = false;
                        self.continue_flag = false;
                        // §14.11: a RUNTIME `##(expr)` that evaluates to 0 —
                        // the desugared `repeat (0) @(__xz_default_clocking)`
                        // — SYNCHRONIZES to the clocking event like the
                        // literal ##0 (no-op when already at the edge, park
                        // otherwise). A plain zero repeat of any other body
                        // keeps falling straight through.
                        if let StatementKind::TimingControl {
                            control: TimingControl::Event(EventControl::Identifier(id)),
                            ..
                        } = &body.kind
                        {
                            if id.name == "__xz_default_clocking" {
                                let at_edge = self
                                    .default_clocking_cb
                                    .as_ref()
                                    .and_then(|cb| self.clocking_last_edge.get(cb))
                                    == Some(&self.time);
                                if !at_edge {
                                    let sync = Statement::new(
                                        StatementKind::TimingControl {
                                            control: TimingControl::Event(
                                                EventControl::Identifier(crate::ast::Identifier {
                                                    name: "__xz_default_clocking0".to_string(),
                                                    span: stmt.span,
                                                }),
                                            ),
                                            stmt: Box::new(Statement::new(
                                                StatementKind::Null,
                                                stmt.span,
                                            )),
                                        },
                                        stmt.span,
                                    );
                                    let cont = pc.pushed(vec![sync], pc.start + i + 1);
                                    self.event_queue.schedule(self.time, pid, cont);
                                    return;
                                }
                            }
                        }
                        i += 1;
                        continue;
                    }
                    // A `break`/`continue` set during a previous iteration's
                    // body is consumed here: break exits the repeat, continue
                    // proceeds to the next iteration. (§9.3.3)
                    if self.blocking_loop_flag_gate() {
                        i += 1;
                        continue;
                    }
                    // `repeat (N) @(event);` has no per-iteration action. Keep
                    // one counted waiter parked across the N events rather
                    // than cloning/evaluating a Repeat tail and resolving the
                    // same sensitivity after every edge. Clocking-block events
                    // retain the general path because their intermediate
                    // Reactive-region scheduling is observable.
                    if let StatementKind::TimingControl {
                        control: TimingControl::Event(event),
                        stmt: event_body,
                    } = &body.kind
                    {
                        if matches!(event_body.kind, StatementKind::Null)
                            && !self.is_clocking_event(event)
                        {
                            let sens = self.event_to_sens(event);
                            let has_real = sens.iter().any(|s| self.sens_term_resolves(s));
                            if !sens.is_empty() && has_real {
                                let mut waiter = self.make_event_waiter(
                                    pid,
                                    sens,
                                    pc.resume_at(pc.start + i + 1),
                                );
                                waiter.remaining_events = n;
                                self.event_waiters.push(waiter);
                                return;
                            }
                        }
                    }
                    // Unroll: execute body once, then schedule rest
                    let remaining_n = n - 1;
                    let mut cont = Vec::new();
                    // Expand body (may contain @event)
                    let body_stmts = Self::spliced_body(body);
                    cont.extend(body_stmts);
                    // Always re-schedule a Repeat tail — at remaining 0 it
                    // becomes the exhaustion sentinel whose n==0 arm above
                    // clears a trailing continue/break before `after` runs.
                    cont.push(Statement::new(
                        StatementKind::Repeat {
                            count: Expression::new(
                                ExprKind::Number(NumberLiteral::Integer {
                                    size: None,
                                    signed: false,
                                    base: NumberBase::Decimal,
                                    value: remaining_n.to_string(),
                                    cached_val: Cell::new(None),
                                }),
                                body.span,
                            ),
                            body: body.clone(),
                        },
                        stmt.span,
                    ));
                    // Chain the caller's tail rather than copying it (ProcCont::pushed).
                    // The just-appended Repeat IS this frame's genuine re-entry
                    // tail (see `loop_tail_at`'s doc comment) — it sits at the
                    // last index of `cont`.
                    let tail_idx = cont.len() - 1;
                    let cont = pc.pushed_with_tail(cont, pc.start + i + 1, tail_idx);
                    self.continue_stmts_or_trampoline(pid, cont);
                    return;
                }
            }

            // While loop with event/timing waits inside: unroll one iteration,
            // re-append the while statement so the condition is re-checked
            // after suspension.
            // If-statement whose chosen branch contains blocking stmts: descend
            // into the branch via run_process_stmts so repeat/while/@event
            // inside the branch can properly suspend the process.
            if let StatementKind::If {
                condition,
                then_stmt,
                else_stmt,
                ..
            } = &stmt.kind
            {
                // `if (e matches p)` binds pattern variables, which only
                // `exec_statement` does; evaluating it here would produce the
                // match result without the bindings. Leave it alone.
                if !matches!(condition.kind, ExprKind::Matches { .. }) {
                    // The condition is evaluated EXACTLY ONCE. Deciding whether
                    // the chosen branch blocks used to evaluate it here and then
                    // fall through to `exec_statement`, which evaluated it again
                    // — so any side effect in an `if` condition (`$random()`,
                    // `$urandom()`, a VPI `$systf`, `i++`) happened twice.
                    // The compiled-condition cache preserves the once-only
                    // contract: the bytecode runs once per activation.
                    let cond_true = self
                        .try_eval_process_cond_bytecode(pid, condition)
                        .unwrap_or_else(|| self.eval_expr(condition).is_true());
                    let chosen: Option<&Statement> = if cond_true {
                        Some(then_stmt.as_ref())
                    } else {
                        else_stmt.as_ref().map(|b| b.as_ref())
                    };
                    match chosen {
                        Some(branch) if self.stmt_is_blocking(branch) => {
                            // The chosen branch is immutable AST: share one
                            // Arc frame per (process, span) instead of cloning
                            // its statements on every activation (a clocked
                            // monitor whose else-chain ends in a rare `#delay`
                            // did this every cycle — 3% of c906).
                            let frame = match &branch.kind {
                                StatementKind::SeqBlock { stmts, .. }
                                    if !Self::stmts_declare_vars(stmts) =>
                                {
                                    self.blocking_cont_frame(&pc.stmts, branch, 1, stmts)
                                }
                                _ => self.blocking_cont_frame(
                                    &pc.stmts,
                                    branch,
                                    4,
                                    std::slice::from_ref(branch),
                                ),
                            };
                            self.run_process_stmts(pid, &pc.pushed_frame(frame, pc.start + i + 1));
                            return;
                        }
                        // Run the branch we already selected, rather than
                        // re-entering the whole `if`.
                        Some(branch) => self.exec_statement(branch),
                        None => {}
                    }
                    i += 1;
                    continue;
                }
            }

            // Suspend-aware Case (mirror of the If handler above). An inlined
            // task body that is a `case` containing blocking statements
            // (`case(op) wait(...); endcase`) must select its arm HERE — falling
            // through to `exec_statement(Case)` would run the matched arm's
            // `wait` on the SYNCHRONOUS path, where a false condition with no
            // parking continuation silently falls through instead of
            // blocking (IEEE 1800-2023 §9.7.4). Evaluate the selector ONCE
            // (side-effect conditions must not double-fire), pick the first
            // matching item or the default, and if the chosen arm blocks,
            // flatten its body and recurse so its waits reach the suspend-aware
            // Wait arm.
            if let StatementKind::Case {
                kind,
                expr,
                items,
                unique_priority: _,
            } = &stmt.kind
            {
                // Pattern case (`case(x) matches p: ...`) binds variables;
                // delegate to the synchronous path which performs the bindings.
                // Likewise a case with NO blocking arm: `exec_statement(Case)`
                // owns the §12.5.3 unique/unique0/priority violation checks,
                // so only intercept when some arm can actually suspend.
                if !items.iter().any(|i| i.pattern.is_some()) && self.stmt_is_blocking(stmt) {
                    let val = self.case_selector_value(kind, expr, items);
                    let chosen: Option<&Statement> = items
                        .iter()
                        .find_map(|item| {
                            if item.is_default {
                                return None;
                            }
                            for pat in &item.patterns {
                                let hit = match kind {
                                    CaseKind::CaseInside => self.case_inside_match(&val, pat),
                                    CaseKind::Casez => {
                                        let pv = self.eval_expr(pat);
                                        val.casez_eq(&pv).is_true()
                                    }
                                    CaseKind::Casex => {
                                        let pv = self.eval_expr(pat);
                                        val.casex_eq(&pv).is_true()
                                    }
                                    _ => {
                                        let pv = self.eval_expr(pat);
                                        val.case_eq(&pv).is_true()
                                    }
                                };
                                if hit {
                                    return Some(&item.stmt);
                                }
                            }
                            None
                        })
                        .or_else(|| {
                            items
                                .iter()
                                .find(|item| item.is_default)
                                .map(|item| &item.stmt)
                        });
                    match chosen {
                        Some(arm) if self.stmt_is_blocking(arm) => {
                            let frame = self.spliced_body_frame(&pc.stmts, arm);
                            // Chain the caller's tail instead of copying it onto the end of
                            // the spliced body (ProcCont::pushed_frame).
                            self.run_process_stmts(pid, &pc.pushed_frame(frame, pc.start + i + 1));
                            return;
                        }
                        Some(arm) => self.exec_statement(arm),
                        None => {}
                    }
                    i += 1;
                    continue;
                }
            }

            // Suspend-aware randcase (§18.16), as for `case` above: a chosen
            // branch that blocks (`1: seq.start(sqr);`) must run on this path,
            // or its first wait returned at once and the loop around it
            // started the same sequence again while it was still running.
            if let StatementKind::RandCase { items } = &stmt.kind {
                if self.stmt_is_blocking(stmt) {
                    if let Some(k) = self.randcase_pick(items) {
                        let arm = &items[k].1;
                        if self.stmt_is_blocking(arm) {
                            let frame = self.spliced_body_frame(&pc.stmts, arm);
                            self.run_process_stmts(pid, &pc.pushed_frame(frame, pc.start + i + 1));
                            return;
                        }
                        self.exec_statement(arm);
                    }
                    i += 1;
                    continue;
                }
            }

            // A `for` loop with a blocking body (e.g. `for (i=0; i<n; i++) task_call()`)
            // must iterate via the suspend-aware path — otherwise the synchronous
            // exec runs the body once and the loop never advances. Run the init now,
            // then lower to `while (cond) { body; step; }` and recurse.
            if let StatementKind::For {
                init,
                condition,
                step,
                body,
            } = &stmt.kind
            {
                if condition.is_some()
                    && (self.stmt_has_event_wait(body) || self.stmt_is_blocking(body))
                {
                    // §12.7.1: retain the enclosing declaration, and close the
                    // automatic loop scope after the final resumed iteration.
                    let names: Vec<String> = init
                        .iter()
                        .filter_map(|fi| match fi {
                            ForInit::VarDecl { name, .. } => Some(name.name.clone()),
                            _ => None,
                        })
                        .collect();
                    let saved = self.snapshot_loop_vars(&names);
                    let owns_loop_frame = self.local_stack.is_empty() && !names.is_empty();
                    if owns_loop_frame {
                        self.push_loop_frame();
                    }
                    for fi in init {
                        match fi {
                            ForInit::VarDecl {
                                data_type,
                                name,
                                init: e,
                            } => {
                                let rv = self.declare_for_init_var(data_type, &name.name, e);
                                if let Some(frame) = self.local_stack.last_mut() {
                                    frame.insert(name.name.clone(), rv);
                                } else {
                                    // §12.7.1: a variable declared in the for-init
                                    // has AUTOMATIC lifetime — it is local to the
                                    // loop. With no call frame (an initial block or
                                    // a fork child) it used to land in the GLOBAL
                                    // signal map, so two concurrent processes each
                                    // running `for (int i ...)` shared one counter
                                    // and clobbered each other's index. Only the
                                    // suspend-aware path can interleave, so give
                                    // the process its own frame here.
                                    let mut f: HashMap<String, Value> = HashMap::default();
                                    f.insert(name.name.clone(), rv);
                                    self.push_local_frame(f);
                                    // Not a call frame: the process's block
                                    // locals stay static (§6.21).
                                    if let Some(&g) = self.local_gen_stack.last() {
                                        self.loop_frame_gens.insert(g);
                                    }
                                }
                            }
                            ForInit::Assign { lvalue, rvalue } => {
                                let v = self.eval_expr(rvalue);
                                self.assign_value(lvalue, &v);
                            }
                        }
                    }
                    let for_key = (stmt as *const Statement as usize, 6u8);
                    let frame = if let Some((_, f)) = self.blocking_cont_cache.get(&for_key) {
                        f.clone()
                    } else {
                        let mut body_stmts = Self::spliced_body(body);
                        // §12.7.2: a `continue` skips the rest of the body but the
                        // step STILL runs. Without this barrier the step — appended
                        // to the body by this very lowering — was skipped along
                        // with it, so the index never advanced and the loop hung.
                        if !step.is_empty() {
                            body_stmts.push(Statement::new(StatementKind::LoopStep, stmt.span));
                        }
                        for s in step {
                            body_stmts
                                .push(Statement::new(StatementKind::Expr(s.clone()), stmt.span));
                        }
                        let while_body = Statement::new(
                            StatementKind::SeqBlock {
                                name: None,
                                stmts: body_stmts,
                            },
                            stmt.span,
                        );
                        let while_stmt = Statement::new(
                            StatementKind::While {
                                condition: condition.clone().unwrap(),
                                body: Box::new(while_body),
                            },
                            stmt.span,
                        );
                        let f: Arc<[Statement]> = Arc::from(vec![
                            while_stmt,
                            Statement::new(StatementKind::Null, stmt.span),
                        ]);
                        self.blocking_cont_cache
                            .insert(for_key, (Arc::clone(&pc.stmts), f.clone()));
                        f
                    };
                    if !saved.is_empty() {
                        self.block_scope_saves.push(BlockScopeSave {
                            frame_key: Self::frame_key(&frame),
                            frame_gen: self.local_gen_stack.last().copied().unwrap_or(0),
                            loop_vars: saved,
                            owns_loop_frame,
                            ..Default::default()
                        });
                    }
                    // Chain the caller's tail rather than copying it (ProcCont::pushed_frame).
                    let cont = pc.pushed_frame(frame, pc.start + i + 1);
                    self.continue_stmts_or_trampoline(pid, cont);
                    return;
                }
            }

            if let StatementKind::While { condition, body } = &stmt.kind {
                // Descend into a blocking while-body (event waits, #delays, or
                // blocking method calls) so each iteration suspends via the
                // suspend-aware path instead of spinning synchronously.
                if self.stmt_has_event_wait(body) || self.stmt_is_blocking(body) {
                    if self.blocking_loop_flag_gate() {
                        // `break`/`return` exits the while loop.
                        i += 1;
                        continue;
                    }
                    self.reset_hint_to_process_scope();
                    let cond_val = self.eval_expr(condition).is_true();
                    if cond_val {
                        let while_key = (stmt as *const Statement as usize, 7u8);
                        let frame = if let Some((_, f)) = self.blocking_cont_cache.get(&while_key) {
                            f.clone()
                        } else {
                            let mut prepared = Self::spliced_body(body);
                            prepared.push(stmt.clone());
                            let frame: Arc<[Statement]> = Arc::from(prepared);
                            let tail_ptr = &frame[frame.len() - 1] as *const Statement as usize;
                            self.blocking_cont_cache
                                .insert(while_key, (Arc::clone(&pc.stmts), Arc::clone(&frame)));
                            self.blocking_cont_cache
                                .insert((tail_ptr, 7u8), (Arc::clone(&frame), Arc::clone(&frame)));
                            frame
                        };
                        // `frame` always ends with the re-appended `stmt.clone()`
                        // (freshly built above, or the identical shape pulled
                        // from `blocking_cont_cache` — same span, same tail
                        // position either way): that last slot is this frame's
                        // genuine re-entry (see `loop_tail_at`'s doc comment).
                        let tail_idx = frame.len() - 1;
                        let cont = pc.pushed_frame_with_tail(frame, pc.start + i + 1, tail_idx);
                        self.continue_stmts_or_trampoline(pid, cont);
                        return;
                    } else {
                        i += 1;
                        continue;
                    }
                }
            }

            // do...while with a blocking body: run the body once, then continue
            // as a regular while(cond) body (above). Without this, a blocking
            // do...while body spins synchronously.
            if let StatementKind::DoWhile { condition, body } = &stmt.kind {
                if self.stmt_is_blocking(body) {
                    let body_stmts = Self::spliced_body(body);
                    let mut cont: Vec<Statement> = body_stmts;
                    cont.push(Statement::new(
                        StatementKind::While {
                            condition: condition.clone(),
                            body: body.clone(),
                        },
                        stmt.span,
                    ));
                    // Chain the caller's tail instead of copying it onto the end of
                    // the spliced body (ProcCont::pushed). The appended `While`
                    // is this frame's genuine re-entry tail.
                    let tail_idx = cont.len() - 1;
                    self.run_process_stmts(
                        pid,
                        &pc.pushed_with_tail(cont, pc.start + i + 1, tail_idx),
                    );
                    return;
                }
            }

            // Check for ParBlock (fork...join)
            if let StatementKind::ParBlock {
                stmts: sub_stmts,
                join_type,
                name: block_name,
                ..
            } = &stmt.kind
            {
                // Fork-block declarations run in THIS process first (§9.3.2), so
                // the children snapshot them instead of racing over them.
                let (spawnable, saved_auto_len) = self.exec_fork_block_decls(sub_stmts);
                let mut child_pids = HashSet::default();
                // §9.3.2 children start in THIS time slot, ahead of whatever
                // is already queued for it; collected here and pushed to the
                // slot front in reverse below so siblings keep source order.
                let mut start_now: Vec<(usize, Arc<[Statement]>)> = Vec::new();
                for s in spawnable {
                    let pid_child = self.next_pid;
                    self.next_pid += 1;
                    self.process_parents.insert(pid_child, pid);
                    self.note_fork_spawn(pid_child, pc);
                    // §9.3.2: a fork child executes in the forking process's
                    // scope, so it inherits the parent's instance-scope hint
                    // (additive — a child previously had none at all).
                    if let Some(h) = self.process_scope_hint.get(&pid).cloned() {
                        self.process_scope_hint.insert(pid_child, h);
                    }
                    self.process_origin
                        .insert(pid_child, (s.span, "fork child"));
                    // §9.6.2: `disable <label>` where the label names this
                    // child's own top-level `begin : name` block terminates
                    // the child. `disable_labels` was populated ONLY for
                    // initial blocks at start-up, so a fork child's label was
                    // unknown: the disable found no target, fell through to
                    // the self-unwind path, and the child kept running — a
                    // later `wait fork` then blocked on it forever.
                    if let StatementKind::SeqBlock { name: Some(n), .. } = &s.kind {
                        self.disable_labels.insert(n.name.clone(), pid_child);
                    }
                    self.inherit_fork_child_context(pid_child);
                    self.inherit_fork_m_scope(pid_child, pid, block_name.as_ref());
                    // §9.4.5: a child that IS an intra-assignment delay
                    // (`fork lhs = #d rhs; join_none`) captures its RHS at the
                    // fork point — a join_none parent keeps running and may
                    // overwrite RHS operands before the child would start, so
                    // deferring the capture to child start-up reads the
                    // post-fork values. Evaluate now and schedule the
                    // pre-computed assignment directly at t+d.
                    if let StatementKind::BlockingAssign { lvalue, rvalue } = &s.kind {
                        if let Some((d_expr, rhs)) = Self::intra_delay_marker(rvalue) {
                            let val = self.eval_expr(rhs);
                            let delay = self.eval_delay_ticks(d_expr);
                            let saved = self.make_intra_saved_expr(val, rvalue.span);
                            let assign = Statement::new(
                                StatementKind::BlockingAssign {
                                    lvalue: lvalue.clone(),
                                    rvalue: saved,
                                },
                                s.span,
                            );
                            self.event_queue.schedule(
                                self.time + delay,
                                pid_child,
                                vec![assign].into(),
                            );
                            child_pids.insert(pid_child);
                            continue;
                        }
                    }
                    // Schedule children to run at current time, sharing one
                    // cached Arc<[Statement]> per static fork branch node.
                    let frame = self.blocking_cont_frame(&pc.stmts, s, 5, std::slice::from_ref(s));
                    start_now.push((pid_child, frame));
                    child_pids.insert(pid_child);
                }
                for (pid_child, frame) in start_now.into_iter().rev() {
                    self.event_queue.schedule_front(
                        self.time,
                        pid_child,
                        ProcCont::from_frame(frame),
                    );
                }
                self.auto_loop_vars.truncate(saved_auto_len);
                if let Some(nm) = block_name {
                    self.fork_block_children
                        .insert(nm.name.clone(), child_pids.clone());
                }

                // An empty fork (or one whose only items were declarations)
                // has nothing to wait for and completes at once, whatever the
                // join type. A JoinWaiter is re-checked only when a child
                // finishes, so a waiter with no children would never fire —
                // `fork join` with an empty body used to drop the entire
                // continuation.
                if *join_type == JoinType::JoinNone || child_pids.is_empty() {
                    // Continue immediately
                    i += 1;
                    continue;
                } else {
                    // Suspend current process and wait for children
                    let cont = pc.resume_at(pc.start + i + 1);
                    self.join_waiters.push(JoinWaiter {
                        parent_pid: pid,
                        child_pids,
                        join_type: *join_type,
                        continuation: cont,
                        finished_children: HashSet::default(),
                        wait_fork: false,
                    });
                    return;
                }
            } else {
                // §9.7: `process_handle.await()` blocks the calling process
                // until the target terminates. Must be intercepted here (not
                // in exec_method_call) because the continuation is needed.
                if let StatementKind::Expr(expr) = &stmt.kind {
                    if let Some(target_pid) = self.extract_proc_await_target(expr) {
                        let cont = pc.resume_at(pc.start + i + 1);
                        if self.proc_await(target_pid, pid, cont) {
                            return; // caller suspended — don't execute await
                        }
                        // target already terminated — fall through
                    }
                }
                // INTERNAL: `ForeachTail` continuation sentinel — run the
                // next iteration (or exit) of a blocking-body `foreach`.
                // §9.4.3: a process blocked inside the loop body resumes HERE,
                // at the next iteration, not by restarting the whole foreach.
                if let StatementKind::ForeachTail {
                    loop_var,
                    var_scope,
                    body,
                    keys,
                    is_str,
                    key_type,
                    idx,
                    fe_auto_len,
                    live_size_name,
                } = &stmt.kind
                {
                    // A `return` from an inlined task body propagates to the
                    // task's ScopePop (handled by the top-of-loop return_flag
                    // skip). Just exit this foreach without consuming flags.
                    if self.return_flag {
                        self.auto_loop_vars.truncate(*fe_auto_len);
                        i += 1;
                        continue;
                    }
                    // A `break` (set WITHOUT return_flag) exits this loop —
                    // consume it, mirroring the synchronous `while` at L30181.
                    if self.break_flag {
                        self.break_flag = false;
                        self.continue_flag = false;
                        self.auto_loop_vars.truncate(*fe_auto_len);
                        i += 1;
                        continue;
                    }
                    self.continue_flag = false;
                    // Bounds check. For dynamic arrays/queues whose size can
                    // change between iterations (because the body suspended
                    // and another process shrunk the collection), re-check the
                    // LIVE size instead of the frozen key count.
                    let exhausted = if let Some(ln) = live_size_name {
                        // §12.7.3: the iteration set is fixed when the loop
                        // starts. The live size is re-read ONLY so a collection
                        // shrunk by another process (while the body was
                        // suspended) exits early instead of indexing past the
                        // end — growth must not extend the loop. A `push_back`
                        // in the body grew the bound exactly as fast as `idx`
                        // advanced, so the loop never exhausted: it ran to max
                        // simulation time with no diagnostic at all. Clamping to
                        // the entry count makes ForeachTail provably terminate.
                        *idx as u64 >= self.get_queue_size(ln).min(keys.len() as u64)
                    } else {
                        *idx >= keys.len()
                    };
                    if exhausted {
                        // loop exhausted — restore automatic-loop-var scope
                        self.auto_loop_vars.truncate(*fe_auto_len);
                        i += 1;
                        continue;
                    }
                    // advance the loop variable to keys[idx]
                    if let Some(vn) = loop_var {
                        // For a live-size collection, synthesize the index key
                        // from `idx` (the frozen keys may be stale).
                        let key_str = if live_size_name.is_some() {
                            idx.to_string()
                        } else {
                            keys[*idx].clone()
                        };
                        let kv = if *is_str {
                            Value::from_string(&key_str)
                        } else if key_type.1 {
                            let mut v = Value::from_u64(
                                key_str.parse::<i64>().unwrap_or(0) as u64,
                                key_type.0,
                            );
                            v.is_signed = true;
                            v
                        } else {
                            // §7.9 + §7.8.2: parse SIGNED (a negative key
                            // must not collapse to 0), then land at the
                            // DECLARED index width/signedness when recorded.
                            let kw = if key_type.0 > 0 { key_type.0 } else { 32 };
                            let mut v =
                                Value::from_u64(key_str.parse::<i64>().unwrap_or(0) as u64, kw);
                            v.is_signed = if key_type.0 > 0 { key_type.1 } else { true };
                            v
                        };
                        self.set_loop_var_aliased(var_scope.as_deref(), vn, kv);
                    }
                    let body_id = body.span.start;
                    let body_frame = self
                        .blocking_cont_cache
                        .get(&(body_id, 8u8))
                        .map(|(_, f)| f.clone())
                        .unwrap_or_else(|| self.spliced_body_frame(&pc.stmts, body));
                    let tail = pc.pushed(
                        vec![Statement::new(
                            StatementKind::ForeachTail {
                                loop_var: loop_var.clone(),
                                var_scope: var_scope.clone(),
                                body: body.clone(),
                                keys: keys.clone(),
                                is_str: *is_str,
                                key_type: *key_type,
                                idx: idx + 1,
                                fe_auto_len: *fe_auto_len,
                                live_size_name: live_size_name.clone(),
                            },
                            stmt.span,
                        )],
                        pc.start + i + 1,
                    );
                    let cont = tail.pushed_frame(body_frame, 0);
                    self.continue_stmts_or_trampoline(pid, cont);
                    return;
                }
                // Blocking-body `foreach` (single loop variable): unroll one
                // iteration and re-append a `ForeachTail` sentinel, exactly
                // like the `while`/`for` unroll above, so a `wait`/`#delay`
                // inside the body (often nested several inlined-task frames
                // deep) parks and resumes at the NEXT iteration instead of
                // restarting from index 0. The previous `exec_park_cont` replay
                // re-ran the whole loop (and its bodies' pre-wait side effects),
                // corrupting consuming handshakes and never actually blocking.
                if let StatementKind::Foreach { array, vars, body } = &stmt.kind {
                    if self.stmt_is_blocking(body) {
                        if let Some((keys, is_str, var_scope, live_size_name, key_type)) =
                            self.foreach_materialize_keys_1d(array, vars)
                        {
                            // §12.7.3 (blocking-body trampoline): this path
                            // bypasses `exec_stmt_foreach`, so the loop
                            // variable's class binding (assoc KEY class, then
                            // element class) must be recorded HERE — a
                            // `foreach (m_successors[succ])` body writing
                            // `succ.m_state = …` dropped every write without
                            // it, because the id-collision receiver gate
                            // could not classify the bare name. Recorded into
                            // the task's live frame, the binding rides the
                            // process-context snapshot through every
                            // park/restore cycle of the ForeachTail unrolling.
                            if let Some(arr_name) = Self::foreach_array_root_name(array) {
                                if let Some(cls) = self
                                    .assoc_key_class_of_foreach(&arr_name, array)
                                    .or_else(|| self.elem_class_of_arr(&arr_name))
                                {
                                    if self.module.classes.contains_key(&cls)
                                        && let Some(v) = vars.first().and_then(|v| v.as_ref())
                                    {
                                        self.record_local_class_type(&v.name, &cls);
                                    }
                                }
                            }
                            let fe_names: Vec<String> = vars
                                .iter()
                                .filter_map(|v| v.as_ref().map(|id| id.name.clone()))
                                .collect();
                            let fe_auto_len = self.auto_loop_vars.len();
                            // foreach index vars are automatic (§9.7.3); like
                            // for-init vars, record for fork capture.
                            if self.local_stack.last().is_none() {
                                for nm in &fe_names {
                                    self.auto_loop_vars.push(nm.clone());
                                }
                            }
                            let loop_var = vars
                                .first()
                                .and_then(|v| v.as_ref().map(|id| id.name.clone()));
                            if let Some(vn) = &loop_var {
                                self.widths.insert(vn.clone(), key_type.0);
                            }
                            if keys.is_empty() {
                                self.auto_loop_vars.truncate(fe_auto_len);
                                i += 1;
                                continue;
                            }
                            // run the FIRST iteration now
                            if let Some(vn) = &loop_var {
                                let kv = if is_str {
                                    Value::from_string(&keys[0])
                                } else if key_type.1 {
                                    let mut v = Value::from_u64(
                                        keys[0].parse::<i64>().unwrap_or(0) as u64,
                                        key_type.0,
                                    );
                                    v.is_signed = true;
                                    v
                                } else {
                                    // §7.9 + §7.8.2: signed parse, declared width.
                                    let kw = if key_type.0 > 0 { key_type.0 } else { 32 };
                                    let mut v = Value::from_u64(
                                        keys[0].parse::<i64>().unwrap_or(0) as u64,
                                        kw,
                                    );
                                    v.is_signed = if key_type.0 > 0 { key_type.1 } else { true };
                                    v
                                };
                                self.set_loop_var_aliased(var_scope.as_deref(), vn, kv);
                            }
                            self.continue_flag = false;
                            let body_frame = self.spliced_body_frame(&pc.stmts, body);
                            let body_id = (&**body) as *const Statement as usize;
                            let marker = Box::new(Statement::new(
                                StatementKind::Null,
                                crate::ast::Span {
                                    start: body_id,
                                    end: 0,
                                },
                            ));
                            let tail = pc.pushed(
                                vec![Statement::new(
                                    StatementKind::ForeachTail {
                                        loop_var,
                                        var_scope,
                                        body: marker,
                                        keys,
                                        is_str,
                                        key_type,
                                        idx: 1,
                                        fe_auto_len,
                                        live_size_name,
                                    },
                                    stmt.span,
                                )],
                                pc.start + i + 1,
                            );
                            let cont = tail.pushed_frame(body_frame, 0);
                            self.continue_stmts_or_trampoline(pid, cont);
                            return;
                        }
                        // else: multi-var / unhandled shape → fall through to
                        // the synchronous exec_statement (exec_park_cont) below.
                    }
                }
                // Set up a parking continuation for blocking waits inside
                // loop bodies (foreach) processed by exec_statement's
                // synchronous path. exec_statement's Wait handler reads this
                // to park the process with `[stmt, rest]` when a wait
                // condition is false, so the process re-runs this statement
                // when resumed.
                //
                // FALLBACK only: the common single-loop-variable case is
                // handled by the suspend-aware unroll above (which resumes at
                // the NEXT iteration per §9.4.3). This replay-from-zero path
                // remains for multi-variable / unhandled shapes that
                // `foreach_materialize_keys_1d` declined.
                if self.try_exec_process_loop_bytecode(pid, stmt) {
                    i += 1;
                    continue;
                }
                let is_foreach_fallback = matches!(&stmt.kind, StatementKind::Foreach { .. });
                if is_foreach_fallback {
                    self.exec_park_cont = Some({
                        let c = vec![stmt.clone()];
                        // Chain the caller's tail rather than copying it (ProcCont::pushed).
                        let c = pc.pushed(c, pc.start + i + 1);
                        c
                    });
                }
                self.exec_statement(stmt);
                // NO-PROGRESS GUARD. This fallback replays the foreach from
                // index 0 on every resume, so a body that parks here makes no
                // forward progress: it re-runs iteration 0, parks again, and
                // repeats — historically forever, with no diagnostic. One or
                // two parks may still complete legitimately (the wait is
                // satisfied on resume and the loop finishes synchronously), so
                // only a run of them is treated as stuck. Naming the array and
                // its source offset turns an unbounded hang into one line.
                if is_foreach_fallback {
                    if self.parked_from_exec {
                        let key = (pid, stmt.span.start);
                        let n = self.foreach_replay_parks.entry(key).or_insert(0);
                        *n += 1;
                        if self.foreach_replay_limit > 0 && *n > self.foreach_replay_limit {
                            let what = match &stmt.kind {
                                StatementKind::Foreach { array, .. } => match &array.kind {
                                    ExprKind::Ident(h) => h
                                        .path
                                        .iter()
                                        .map(|seg| seg.name.name.as_str())
                                        .collect::<Vec<_>>()
                                        .join("."),
                                    _ => "<expr>".to_string(),
                                },
                                _ => "<foreach>".to_string(),
                            };
                            eprintln!(
                                "Error: `foreach ({what}[...])` at offset {} made no progress \
                                 across {} suspensions.",
                                stmt.span.start, self.foreach_replay_limit
                            );
                            eprintln!(
                                "               Its index shape could not be materialized, so \
                                 each resume restarts the loop at index 0."
                            );
                            eprintln!(
                                "               Multi-variable `foreach` over a blocking body is \
                                 the usual cause. Raise or disable with \
                                 XEZIM_FOREACH_REPLAY_LIMIT=<n> (0 = never check)."
                            );
                            self.abort_run();
                        }
                    } else {
                        // Completed without parking — clear any earlier streak
                        // so a foreach inside an outer loop cannot accumulate.
                        self.foreach_replay_parks.remove(&(pid, stmt.span.start));
                    }
                }
                self.exec_park_cont = None;
                self.parked_from_exec = false;
            }

            // Check for WaitFork
            if let StatementKind::WaitFork = &stmt.kind {
                // §9.6.1: `wait fork` blocks until the IMMEDIATE child
                // subprocesses of the calling process complete — and no
                // further. The asymmetry with `disable fork` (which kills all
                // DESCENDANTS) is the LRM's, not an accident. Waiting on the
                // transitive closure made a `join_none` grandchild extend the
                // wait arbitrarily — or forever, for a persistent monitor
                // spawned by a child, which is exactly the shape UVM drivers
                // use.
                let children: HashSet<usize> = self
                    .process_parents
                    .iter()
                    .filter(|&(_, &parent)| parent == pid)
                    .map(|(&child, _)| child)
                    .collect();

                if children.is_empty() {
                    i += 1;
                    continue;
                } else {
                    let cont = pc.resume_at(pc.start + i + 1);
                    self.join_waiters.push(JoinWaiter {
                        parent_pid: pid,
                        child_pids: children,
                        join_type: JoinType::Join,
                        continuation: cont,
                        finished_children: HashSet::default(),
                        wait_fork: true,
                    });
                    return;
                }
            }

            i += 1;
        }
        // This frame is exhausted. Follow the chain: a spliced task body or a
        // flattened block runs with the caller's tail linked behind it
        // (ProcCont::pushed), so finishing the body means continuing into the
        // caller rather than returning.
        //
        // Recursive, and deliberately so: the previous code recursed once per
        // splice too (`self.run_process_stmts(pid, &expanded); return;`), so the
        // depth is the same splice nesting it always was and RPS_DEPTH still
        // bounds it. What changed is that a level costs a pointer instead of a
        // copy of everything the caller had left.
        if !self.finished {
            // §21.2.1.7: leaving a flattened named block drops its `%m` name.
            if !self.named_block_frames.is_empty() {
                if let Some(n) = self.named_block_frames.get(&Self::frame_key(&pc.stmts)) {
                    if self.m_scope_stack.last() == Some(n) {
                        self.m_scope_stack.pop();
                    }
                }
            }
            // §9.6.2: a `disable` of THIS named block ends here.
            if self.break_flag
                && self.disable_target.is_some()
                && (self.disable_check_pending || !self.named_block_frames.is_empty())
                && self.disable_unwinding(pid, pc)
            {
                if let Some(n) = self.named_block_frames.get(&Self::frame_key(&pc.stmts)) {
                    if self
                        .disable_target
                        .as_deref()
                        .is_some_and(|t| Self::block_name_matches(t, n))
                        && !self.killed_pids.contains(&pid)
                    {
                        self.disable_target = None;
                        self.break_flag = false;
                        self.continue_flag = false;
                    }
                }
            }
            if let Some(frame) = pc.next.clone() {
                self.run_process_stmts(pid, &frame);
            }
        }
    }
}
