//! Simulator event loop.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

impl Simulator {
    pub(super) fn eval_waiter_condition(&mut self, waiter_pid: usize, cond: &Expression) -> bool {
        let saved_hint = self.name_resolve_hint.borrow().clone();
        let saved_scope = self.current_scope.clone();
        let saved_pid = self.current_pid;
        self.current_pid = waiter_pid;
        if let Some(scope) = self.process_scope_hint.get(&waiter_pid).cloned() {
            *self.name_resolve_hint.borrow_mut() = Some(scope.clone());
            self.current_scope = scope;
        }
        let cond_held = if let Some(ctx) = self.process_contexts.remove(&waiter_pid) {
            // The caller's context is moved aside, not cloned: the waiter's
            // context replaces it wholesale and the original comes back
            // untouched afterwards.
            let saved = self.take_process_context();
            self.restore_process_context(ctx);
            let val = self.wait_condition_true(cond);
            let ctx = self.take_process_context();
            self.process_contexts.insert(waiter_pid, ctx);
            self.restore_process_context(saved);
            val
        } else {
            let saved = self.snapshot_process_context();
            let val = self.wait_condition_true(cond);
            self.restore_process_context(saved);
            val
        };
        *self.name_resolve_hint.borrow_mut() = saved_hint;
        self.current_scope = saved_scope;
        self.current_pid = saved_pid;
        cond_held
    }

    pub(super) fn check_condition_waiters_for_write(&mut self, lhs: &Expression) {
        if self.condition_waiters.is_empty() {
            return;
        }
        // Cheap reject first: nothing the lvalue names is read by any parked
        // waiter. The union is rebuilt only after the waiter set changed.
        if self.cond_read_union_dirty {
            self.cond_read_union.clear();
            for reads in self.cond_waiter_reads.values() {
                self.cond_read_union.extend(reads.iter().cloned());
            }
            self.cond_read_union_dirty = false;
        }
        if !Self::lvalue_hits_names(lhs, &self.cond_read_union) {
            return;
        }
        let mut target_names: Vec<String> = Vec::new();
        Self::collect_lvalue_target_names(lhs, &mut target_names);
        if target_names.is_empty() {
            return;
        }
        let any_match = self.condition_waiters.iter().any(|(pid, _)| {
            self.cond_waiter_reads
                .get(pid)
                .is_some_and(|reads| target_names.iter().any(|tn| reads.contains(tn)))
        });
        if !any_match {
            return;
        }

        self.promote_condition_waiters_for_names(&target_names);
    }

    /// Re-check parked condition waiters whose condition names one of
    /// `target_names`, promoting those now true to `ready_condition_waiters`.
    /// Shared by `check_condition_waiters_for_write` (a blocking procedural
    /// write) and the fork-child merge path (a child's write propagating back
    /// into a parked parent's activation). The promotion is what unblocks the
    /// waiter inside a long-running edge cascade, which drains only READY
    /// waiters (`ready_condition_waiters`) and would otherwise starve a parked
    /// (§9.4.2) condition-maintained handshake.
    pub(super) fn promote_condition_waiters_for_names(&mut self, target_names: &[String]) {
        if self.condition_waiters.is_empty() {
            return;
        }
        let any_match = self.condition_waiters.iter().any(|(pid, _)| {
            self.cond_waiter_reads
                .get(pid)
                .is_some_and(|reads| target_names.iter().any(|tn| reads.contains(tn)))
        });
        if !any_match {
            return;
        }

        let mut i = 0;
        while i < self.condition_waiters.len() {
            let (pid, _) = self.condition_waiters[i];
            let matches = self
                .cond_waiter_reads
                .get(&pid)
                .is_some_and(|reads| target_names.iter().any(|tn| reads.contains(tn)));
            if !matches {
                i += 1;
                continue;
            }
            let cond_opt = self.cond_waiter_conditions.get(&pid).cloned();
            let Some(cond) = cond_opt else {
                i += 1;
                continue;
            };

            let is_true = self.eval_waiter_condition(pid, &cond);
            if is_true {
                let (wpid, wcont) = self.condition_waiters.remove(i);
                self.cond_waiter_gate.remove(&wpid);
                self.cond_waiter_reads.remove(&wpid);
                self.cond_read_union_dirty = true;
                self.cond_waiter_conditions.remove(&wpid);
                self.ready_condition_waiters.push((wpid, wcont));
            } else {
                i += 1;
            }
        }
    }

    /// Settle after a procedural blocking write (outside edge blocks).
    ///
    /// §5.5 + §10.3 say the driven continuous assignments' re-evaluation is a
    /// separate ACTIVE-region update event: the writing process does NOT
    /// yield, so its own later reads see PRE-update net values until it hits a
    /// delay/wait. The reference simulator does exactly that
    /// (`a=5; s1=b;` reads the OLD b). xezim settles EAGERLY here, so such a
    /// read observes the propagated value one statement early — a real
    /// divergence for testbench/BFM code that drives a signal and immediately
    /// reads a continuous-assign-derived handshake.
    ///
    /// Eager remains the DEFAULT: switching wholesale also changes t=0
    /// gate/UDP initialization ordering, which regressed two
    /// reference-validated traces (`udp_primitives::edge_shorthands` q2 at
    /// t=0, `dep_reg_entry_synth`). The correct end state is lazy propagation
    /// with t=0 initialization kept eager; until that separation exists,
    /// `XEZIM_LAZY_PROC_SETTLE=1` opts into the LRM/reference ordering so a
    /// design that depends on it can be run (and diffed) without a rebuild.
    pub(super) fn settle_after_proc_write(&mut self) {
        if self.in_edge_block {
            return;
        }
        static LAZY: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let lazy = *LAZY
            .get_or_init(|| std::env::var("XEZIM_LAZY_PROC_SETTLE").ok().as_deref() == Some("1"));
        // #35: the DEFAULT is the per-evaluator split — gates/UDPs settle now
        // (dep.sv's reference-eager netlist shape), cont-assign entries defer
        // to the yield point (c1b.sv's reference-lazy RTL shape).
        // XEZIM_LAZY_PROC_SETTLE=1 still forces the all-lazy mode;
        // XEZIM_EAGER_PROC_SETTLE=1 restores the old all-eager mode.
        static EAGER: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let eager = *EAGER
            .get_or_init(|| std::env::var("XEZIM_EAGER_PROC_SETTLE").ok().as_deref() == Some("1"));
        if lazy {
            return;
        }
        if eager {
            self.settle_combinatorial();
            return;
        }
        let __sc_before = self.settle_calls;
        self.proc_settle_defer = true;
        self.settle_combinatorial();
        self.proc_settle_defer = false;
        // The deferred updates are PENDING WORK: without this, a caller that
        // gates its settle on dirty_any (the tick top) skips the drain and a
        // process in the NEXT slot reads stale nets.
        if !self.deferred_proc_entries.is_empty() {
            self.dirty_any = true;
        }
        if env_set_cached!("XEZIM_PSETTLE_STATS") {
            self.prof_psettle_calls += 1;
            if self.settle_calls != __sc_before {
                self.prof_psettle_deferred += 1;
            }
            if self.prof_psettle_calls % 20_000 == 0 {
                eprintln!(
                    "[PSETTLE] proc_writes={} did_work={} ({:.1}%) settle_calls={} deferred_entries={} dirty_list={} comb_entries={}",
                    self.prof_psettle_calls,
                    self.prof_psettle_deferred,
                    100.0 * self.prof_psettle_deferred as f64 / self.prof_psettle_calls as f64,
                    self.settle_calls,
                    self.deferred_proc_entries.len(),
                    self.dirty_list.len(),
                    self.comb_entries.len(),
                );
            }
        }
    }

    /// Promote `#0` continuations parked in the Inactive-region queue
    /// (IEEE 1800-2017 §4.4.2.3) back into the event queue at the current
    /// time. Called only AFTER `apply_nba` has committed this pass's NBA
    /// region, so when the caller's loop re-drains same-time events the
    /// promoted continuations observe post-NBA values — matching the
    /// commercial consensus: an NBA posted before
    /// a `#0` is visible after it in the same time slot.
    pub(super) fn promote_inactive_to_active(&mut self) -> bool {
        if self.inactive_queue.is_empty() {
            return false;
        }
        let moved = std::mem::take(&mut self.inactive_queue);
        for (pid, cont) in moved {
            self.event_queue.schedule(self.time, pid, cont);
        }
        true
    }

    /// Drain pending NBAs and repeatedly snapshot → apply_nba → settle →
    /// check_edges until a round produces neither new edges nor new NBAs.
    ///
    /// Needed when a signal transition commits via NBA whose settled effect
    /// triggers further edge-sensitive blocks. The canonical case is the c910
    /// reset chain: async_cpurst_b negedge fires the sync-chain FF → queues
    /// `cpurst_3ff <= 0` NBA → settle propagates cpurst_b X→0 → every
    /// sub-module's local cpurst_b port (driven by a port-connection cont-
    /// assign) transitions X→0 → their async-reset FFs must fire within the
    /// same iter, otherwise the next iter's snapshot captures prev=0 and the
    /// X→0 edge is lost forever (FF stays X, instructions never retire).
    ///
    /// `cascade_limit` bounds the loop at well-behaved designs; a legitimate
    /// design requiring more than this suggests a combinational loop through
    /// the sync chain that the user needs to address.
    ///
    /// Returns (t_snap, t_nba, t_settle, t_edges) deltas in ns for profiling;
    /// callers that don't need them can ignore the return value.

    /// §4.5: drain `#0` continuations (Inactive region) INLINE until empty,
    /// BEFORE the caller commits the NBA region. A `#0` parked by an
    /// edge-waiter-resumed process previously promoted only at end-of-tick —
    /// after the cascade's apply_nba — so `@(posedge clk); #0 x = r;` read
    /// POST-NBA r (one cycle "early" vs the reference simulator; the exact
    /// checker/BFM divergence shape of the rptr race-through report).
    pub(super) fn drain_inactive_pre_nba(&mut self) {
        let mut fuel = 1000u32;
        // §9.2: a `#0` continuation's write happens AFTER this slot's edge
        // detection; without recording it for the rescan drain, the caller
        // either breaks without another check_edges (no NBAs pending) or
        // snapshots prev over the change before the next detect — the edge
        // is silently lost and an AnyEdge block never wakes. Record like
        // edge-block/continuation writes (save/restore for nesting).
        let __prev_edge_cont = self.in_edge_cont;
        self.in_edge_cont = true;
        while !self.inactive_queue.is_empty() && fuel > 0 {
            fuel -= 1;
            let moved = std::mem::take(&mut self.inactive_queue);
            for (pid, cont) in moved {
                if self.finished {
                    self.in_edge_cont = __prev_edge_cont;
                    return;
                }
                self.run_scheduled_process(pid, &cont);
                if !self.is_pid_suspended(pid) {
                    self.child_finished(pid);
                }
            }
            if self.dirty_any {
                self.settle_combinatorial();
            }
        }
        self.in_edge_cont = __prev_edge_cont;
        // Re-detect the recorded positions NOW, against the still-pre-write
        // prev — before any snapshot can erase them. No-op when nothing was
        // recorded.
        self.drain_edge_exec_rescan();
    }

    pub(super) fn drain_edge_cascade(&mut self, cascade_limit: u32) -> (u64, u64, u64, u64) {
        let (mut t_snap, mut t_nba, mut t_settle, mut t_edges) = (0u64, 0u64, 0u64, 0u64);
        let mut cascade_iter = 0u32;
        while cascade_iter < cascade_limit && !self.finished {
            // Inactive region precedes the NBA region (§4.5): `#0`
            // continuations parked by this round's active-region work run
            // before its NBAs commit.
            self.drain_inactive_pre_nba();
            // §9.7.4: `wait(expr)` is a same-delta, same-timestep handoff. A
            // waiter whose condition an ACTIVE-region continuation just made
            // true (an inline edge resume above — e.g. a grant handshake
            // writing the watched variable) must resume BEFORE this
            // generation's NBA region commits and edge-dispatches its own
            // waiters again; otherwise a peer parked on the very NBA the
            // resumer queued (`nba <= next_nba; @(nba)`) wakes first and
            // observes pre-handoff state (reference-verified: the granted
            // sequence must put its item before the peeker's one-NBA peek).
            // `pre_inactive=true`: the waiter's own queued NBAs commit in the
            // apply_nba below — after the snapshot — so their edges stay
            // detectable; `in_edge_cont` records its blocking signal writes
            // for the §9.2 rescan drain.
            if !self.ready_condition_waiters.is_empty() {
                let __prev_edge_cont = self.in_edge_cont;
                self.in_edge_cont = true;
                self.drain_condition_waiters(true, false);
                self.in_edge_cont = __prev_edge_cont;
            }
            if self.nba_fast.is_empty()
                && self.nba_queue.is_empty()
                && self.packed_nba.is_empty()
                && self.inactive_queue.is_empty()
            {
                break;
            }
            let t0 = self.profile_timing.then(std::time::Instant::now);
            self.snapshot_edge_signals();
            if let Some(t) = t0 {
                t_snap += t.elapsed().as_nanos() as u64;
            }
            let t0 = self.profile_timing.then(std::time::Instant::now);
            self.apply_nba();
            if let Some(t) = t0 {
                t_nba += t.elapsed().as_nanos() as u64;
            }
            if self.dirty_any {
                let t0 = self.profile_timing.then(std::time::Instant::now);
                self.settle_combinatorial();
                if let Some(t) = t0 {
                    t_settle += t.elapsed().as_nanos() as u64;
                }
            }
            if !self.delayed_updates.is_empty() {
                self.apply_due_delayed_now();
            }
            let t0 = self.profile_timing.then(std::time::Instant::now);
            let edges_before = self.prof_edges_fired;
            self.edge_dispatch_phase = "cascade";
            self.check_edges();
            self.edge_dispatch_phase = "active";
            if let Some(t) = t0 {
                t_edges += t.elapsed().as_nanos() as u64;
            }
            if self.prof_edges_fired == edges_before
                && self.nba_fast.is_empty()
                && self.nba_queue.is_empty()
                && self.packed_nba.is_empty()
            {
                break;
            }
            cascade_iter += 1;
        }
        // Exiting at the limit with NBAs STILL pending is a zero-delay
        // livelock through the NBA region (`always @(fb) fb <= ~fb;` — the
        // classic PLL-feedback model). The old behavior silently DISCARDED the
        // pending writes and carried on with stale state — worse than a hang,
        // because nothing ever said so. Report it like the other delta-loop
        // detectors and stop. The pending targets ARE the feedback signals.
        if cascade_iter >= cascade_limit
            && !(self.nba_fast.is_empty()
                && self.nba_queue.is_empty()
                && self.packed_nba.is_empty())
            && !self.finished
        {
            eprintln!(
                "[xezim][error] simulation STALLED at time {} — the edge/NBA cascade did not converge after {} delta cycles (XEZIM_CASCADE_LIMIT).",
                self.time, cascade_iter
            );
            eprintln!(
                "               This is a zero-delay (delta) livelock: non-blocking writes keep"
            );
            eprintln!("               re-triggering edge-sensitive blocks at the same timestamp.");
            let mut names: Vec<String> = Vec::new();
            for e in &self.nba_fast {
                if let Some(n) = self.name_opt(e.signal_id) {
                    names.push(n.to_string());
                }
            }
            for e in &self.nba_queue {
                if let Some(n) = e.resolved_id.and_then(|i| self.name_opt(i)) {
                    names.push(n.to_string());
                }
            }
            names.sort();
            names.dedup();
            if !names.is_empty() {
                eprintln!("               Pending non-blocking writes that keep the loop alive:");
                for n in names.iter().take(5) {
                    eprintln!("                 {} <= ... (re-armed every cycle)", n);
                }
            }
            eprintln!(
                "               Raise XEZIM_CASCADE_LIMIT=<n> if the design legitimately needs deeper same-time NBA cascades."
            );
            self.abort_run();
        }
        // Final propagation pass: level-sensitive always blocks fired by the
        // last check_edges may have done blocking-writes to regs that feed
        // downstream cont-assigns (e.g. cr_iu_decd's ill_expt16 always block
        // → decd_ill_expt → decd_special_sel chain). Without this final
        // settle, those cont-assign dependents stay stale until the next
        // clock tick — symptom: e902 part6 decoder outputs lag a reference simulator by
        // one cycle on inst-class transitions. settle alone is sufficient
        // (no further check_edges); the always-block sens lists read from
        // signals already captured in the loop's last snapshot, so they
        // wouldn't re-trigger anyway.
        if self.dirty_any {
            let t0 = self.profile_timing.then(std::time::Instant::now);
            self.settle_combinatorial();
            if let Some(t) = t0 {
                t_settle += t.elapsed().as_nanos() as u64;
            }
        }
        (t_snap, t_nba, t_settle, t_edges)
    }

    /// One simulation tick: snap → delayed → fire-clocks → process batch →
    /// apply_nba → settle → check_edges → cascade drain → monitor/trace.
    /// Factored out of `event_loop`.  Pure refactor — preserves
    /// single-threaded behavior exactly.
    ///
    /// `#[inline(always)]` because LLVM's auto-inline heuristic refused
    /// the body's size and the un-inlined version regressed hello by 15×
    /// (function-call overhead × 8938 ticks + inner-call inlining
    /// disabled by the LLVM threshold drop).  See diff against the
    /// pre-Phase-3 inline body for context.

    /// How deep `exec_forever_sched` may re-enter itself before it stops
    /// recursing and trampolines through the event queue instead. Small: real
    /// designs suspend on the first iteration, so any depth beyond a handful
    /// means the loop is not actually blocking.
    pub(super) const FOREVER_RECURSION_LIMIT: u32 = 32;

    /// A zero-delay livelock: `self.stall_limit` delta cycles have run at one
    /// timestamp and time never moved. Rather than spin forever (the user just
    /// sees "stuck at time 0"), say so and name the processes responsible —
    /// the ones the scheduler keeps re-arming — including, per offender, the
    /// RTL construct it came from (kind + file:line + instance path) and WHY
    /// it keeps re-arming (its pending continuation's head).
    ///
    /// `running` is the currently-executing process when the limit tripped
    /// inside `run_process_stmts` (its continuation lives on the Rust stack,
    /// not in any queue, so the caller must hand it over). `None` when the
    /// outer event loop tripped the delta-cycle limit instead.
    /// "process N — <kind> at <file:line> (<instance path>, module <m>)" — the
    /// attribution prefix shared by the spinner list and the parked-process
    /// list of the stall report.
    /// The source text of the block `pid` originates from. The line number in
    /// `stall_pid_identity` can be unusable when an `\`include` or a multi-line
    /// macro has shifted the preprocessed numbering (see `span_file_line_in`),
    /// and quoting the offending code identifies the process regardless.
    pub(super) fn stall_pid_source(&mut self, pid: usize) -> Option<String> {
        let src_file = self.stall_pid_src_file(pid);
        let (span, _) = *self.process_origin.get(&pid)?;
        self.span_source_snippet_in(span, src_file)
    }

    /// Is the scheduler trace armed for the CURRENT simulation time?
    #[inline]
    pub(super) fn sched_trace_on(&self) -> bool {
        match self.trace_sched {
            Some((lo, hi)) => self.time >= lo && self.time <= hi,
            None => false,
        }
    }

    /// "<kind> at <file>:<line>" for a pid, or just the pid when unknown.
    pub(super) fn sched_pid_label(&self, pid: usize) -> String {
        match self.process_origin.get(&pid) {
            Some(&(span, kind)) => {
                match self.span_file_line_in(span, self.stall_pid_src_file(pid)) {
                    Some(loc) => format!("{kind} at {loc}"),
                    None => kind.to_string(),
                }
            }
            None => String::new(),
        }
    }

    pub(super) fn stall_pid_identity(&mut self, pid: usize) -> String {
        let mut line = format!("                 process {}", pid);
        let src_file = self.stall_pid_src_file(pid);
        if let Some(&(span, kind)) = self.process_origin.get(&pid) {
            line.push_str(" — ");
            line.push_str(kind);
            if let Some(loc) = self.span_file_line_in(span, src_file) {
                line.push_str(" at ");
                line.push_str(&loc);
            }
        }
        match self.process_scope_hint.get(&pid) {
            Some(s) if !s.is_empty() => {
                let s = s.clone();
                let def_mod = self
                    .stall_pid_def_module(pid)
                    .filter(|m| *m != self.module.name);
                match def_mod {
                    Some(m) => line.push_str(&format!(" ({}, module {})", self.hier_path(&s), m)),
                    None => line.push_str(&format!(" ({})", self.hier_path(&s))),
                }
            }
            _ => line.push_str(&format!(" ({})", self.module.name)),
        }
        line
    }

    /// Attempt to break a zero-delay (delta) livelock by deferring every
    /// process stuck at the current timestamp to the next scheduled event and
    /// advancing time to it. Returns true if it advanced, false if NOTHING is
    /// scheduled ahead (a genuine unbreakable livelock — the caller then
    /// reports and finishes). Shared by the outer-loop stall detector and the
    /// per-pid spinner check inside `run_process_stmts`.
    ///
    /// The clock-generator and delayed-update candidates are computed FRESH
    /// here rather than taken from the caller: when this runs from inside a
    /// tick (the per-pid path), `fire_clock_generators` has already toggled the
    /// clocks and advanced their `next_toggle_time`, so any value the outer
    /// loop computed before the tick is stale (it would filter out the clock's
    /// already-advanced edge and skip past it, stranding the clock in the past).
    pub(super) fn try_break_zero_delay_livelock(&mut self) -> bool {
        let next_clk_time = self
            .clock_generators
            .iter()
            .map(|c| c.next_toggle_time)
            .min();
        let next_delayed = self.next_delayed_time();
        let future = self
            .event_queue
            .next_time_after(self.time)
            .into_iter()
            .chain(next_clk_time.filter(|&t| t > self.time))
            .chain(next_delayed.filter(|&t| t > self.time))
            .min()
            .filter(|&t| t <= self.max_time);
        let Some(ft) = future else {
            return false;
        };
        self.report_zero_delay_recovery(ft);
        let stuck = self.event_queue.remove(self.time);
        for (p, c) in stuck {
            self.event_queue.schedule(ft, p, c);
        }
        let inact = std::mem::take(&mut self.inactive_queue);
        for (p, c) in inact {
            self.event_queue.schedule(ft, p, c);
        }
        // Close out the slot being abandoned before the clock jumps. Every
        // other time advance services the postponed region first; this
        // recovery path is the last one that did not, and skipping it would
        // re-date the livelocked slot's changes onto `ft` in both `$monitor`
        // and the dump.
        self.time = ft;
        publish_sim_time(ft);
        self.stall_iters = 0;
        self.stall_time = ft;
        self.stall_pid_hits.clear();
        self.stall_edge_block_hits.clear();
        true
    }

    pub(super) fn edge_block_stall_identity(&self, bi: usize, count: u64) -> Option<String> {
        let block = self.edge_blocks.get(bi)?;
        let kind = match block.kind {
            AlwaysKind::AlwaysFf => "always_ff block",
            AlwaysKind::AlwaysComb => "always_comb block",
            AlwaysKind::AlwaysLatch => "always_latch block",
            AlwaysKind::Always => "always block",
        };
        let scope = (!block.scope.is_empty()).then_some(block.scope.as_str());
        let src_file = self.scope_src_file(scope);
        let mut line = format!("                 {}", kind);
        if let Some(loc) = self.span_file_line_in(block.stmt.span, src_file) {
            line.push_str(" at ");
            line.push_str(&loc);
        }
        if let Some(s) = scope {
            match self.scope_def_module(Some(s)) {
                Some(m) if m != self.module.name => {
                    line.push_str(&format!(" ({}, module {})", s, m));
                }
                _ => line.push_str(&format!(" ({})", s)),
            }
        }
        let sens = block
            .resolved_sensitivities
            .iter()
            .map(|si| {
                let edge = si.edge.print_str();
                format!("{}{}", edge, self.name_for_id(si.signal_id))
            })
            .collect::<Vec<_>>()
            .join(" or ");
        if !sens.is_empty() {
            line.push_str(&format!(" — sensitive to @({})", sens));
        }
        line.push_str(&format!(" — ran {} times at this timestamp", count));
        Some(line)
    }

    pub(super) fn hottest_stall_edge_block(&self) -> Option<(usize, u64)> {
        self.stall_edge_block_hits
            .iter()
            .map(|(&bi, &count)| (bi, count))
            .max_by_key(|&(bi, count)| (count, std::cmp::Reverse(bi)))
    }

    /// A recoverable zero-delay spinner has a future timed event which may
    /// repair its zero-valued delay (common in behavioral PLL models). Keep
    /// that compatibility behavior, but make the forced time advance visible
    /// and attribute the work that prevented normal delta convergence.
    pub(super) fn report_zero_delay_recovery(&mut self, future: u64) {
        eprintln!(
            "[xezim][warning] simulation made no time progress at {} for {} delta cycles; deferring stuck work to future event at {}.",
            self.time, self.stall_iters, future
        );
        if let Some((&pid, &count)) = self
            .stall_pid_hits
            .iter()
            .max_by_key(|&(pid, count)| (*count, std::cmp::Reverse(*pid)))
        {
            let mut line = self.stall_pid_identity(pid);
            if let Some(reason) =
                self.describe_stall_offender(pid, None, self.stall_pid_src_file(pid))
            {
                line.push_str(" — ");
                line.push_str(&reason);
            }
            line.push_str(&format!(" — ran {} times at this timestamp", count));
            eprintln!("               Most active scheduled process:");
            eprintln!("{}", line);
        }
        if let Some((bi, count)) = self.hottest_stall_edge_block() {
            if let Some(line) = self.edge_block_stall_identity(bi, count) {
                eprintln!("               Most active edge block:");
                eprintln!("{}", line);
            }
        }
        eprintln!(
            "               Lower XEZIM_STALL_LIMIT to diagnose sooner; 0 disables this check."
        );
    }

    pub(super) fn report_zero_delay_stall(&mut self, running: Option<(usize, &[Statement])>) {
        let mut top: Vec<(usize, u64)> =
            self.stall_pid_hits.iter().map(|(p, c)| (*p, *c)).collect();
        top.sort_by_key(|&(_, c)| std::cmp::Reverse(c));
        // A process that ran once or twice at this timestamp is not a spinner —
        // it belongs in the "parked" section below (it is usually the very
        // process the spinner is waiting for). Keep low-count entries only if
        // NOTHING spun (so the report always names someone).
        if top.iter().any(|&(_, c)| c > 2) {
            top.retain(|&(_, c)| c > 2);
        }
        top.truncate(5);

        // Report whichever count actually tripped: the outer loop's delta count,
        // or (when a process re-armed inside one tick's drain loop) how many
        // times that process ran. Quoting "1 delta cycle" for a process that ran
        // 100_000 times would just be confusing.
        let worst = top.first().map(|&(_, c)| c).unwrap_or(0);
        eprintln!(
            "[xezim][error] simulation STALLED at time {} — no progress after {} process activations \
             at this timestamp ({} delta cycles).",
            self.time,
            worst.max(self.stall_iters),
            self.stall_iters
        );
        eprintln!(
            "               This is a zero-delay (delta) livelock: one or more processes keep"
        );
        eprintln!(
            "               re-arming at the same timestamp, so simulated time can never move."
        );
        if !top.is_empty() {
            eprintln!("               Most frequently scheduled processes:");
            for (pid, count) in &top {
                let mut line = self.stall_pid_identity(*pid);
                let src_file = self.stall_pid_src_file(*pid);
                // Re-arm reason, from the pid's pending continuation.
                if let Some(reason) = self.describe_stall_offender(*pid, running, src_file) {
                    line.push_str(" — ");
                    line.push_str(&reason);
                }
                line.push_str(&format!(" — ran {} times at this timestamp", count));
                eprintln!("{}", line);
            }
        }
        // The OTHER half of the story: processes that are NOT spinning. The
        // classic livelock has the spinner's fix parked right here — e.g. the
        // initial block that would set the zero-valued clock period, scheduled
        // for a future time the spin never reaches, or blocked on an event the
        // spin never produces.
        {
            let spinner: HashSet<usize> = top.iter().map(|(p, _)| *p).collect();
            let mut parked: Vec<String> = Vec::new();
            let mut seen: HashSet<usize> = HashSet::default();
            for (pid, t) in self.event_queue.pending_pid_times() {
                if t > self.time && !spinner.contains(&pid) && seen.insert(pid) {
                    let id = self.stall_pid_identity(pid);
                    parked.push(format!("{} — next event at time {}", id, t));
                }
            }
            let waiter_pids: Vec<usize> = self
                .event_waiters
                .iter()
                .map(|w| w.pid)
                .chain(self.condition_waiters.iter().map(|(p, _)| *p))
                .collect();
            for pid in waiter_pids {
                if spinner.contains(&pid) || !seen.insert(pid) {
                    continue;
                }
                let src_file = self.stall_pid_src_file(pid);
                let id = self.stall_pid_identity(pid);
                let why = self
                    .describe_stall_offender(pid, None, src_file)
                    .unwrap_or_else(|| "blocked".to_string());
                // The ping-pong hint is for SPINNERS; a parked waiter is just
                // waiting.
                let why = why.replace(" — event ping-pong suspected", "");
                parked.push(format!("{} — {}", id, why));
            }
            if !parked.is_empty() {
                eprintln!(
                    "               Parked (not spinning) — a spinner above may be waiting on one of these:"
                );
                for l in parked.iter().take(8) {
                    eprintln!("{}", l);
                }
                if parked.len() > 8 {
                    eprintln!("                 ... and {} more", parked.len() - 8);
                }
            }
        }
        eprintln!("               Common causes:");
        eprintln!("                 * `forever` / `always` with `#0` or no timing control at all");
        eprintln!("                 * two processes triggering each other's events with no delay");
        eprintln!("                 * `wait (cond)` inside a `forever` where cond is already true");
        eprintln!(
            "               Raise or disable the check with XEZIM_STALL_LIMIT=<n> (0 = never stall-check)."
        );
    }

    /// Which retained source file the spans of `pid`'s statements index
    /// into. A span is a byte offset into its OWN file's preprocessed text,
    /// so a multi-file design needs this to resolve one at all: the pid's
    /// scope hint names the elaborated instance it runs in (empty = the top
    /// module itself), `module.instances` maps that path to the DEFINING
    /// module, and `src_file_of_module` records which file defined it.
    /// `None` when any link is missing (library module, class/object scope,
    /// .xzb run) — callers then fall back to the exact-fit rule.
    pub(super) fn stall_pid_src_file(&self, pid: usize) -> Option<u32> {
        self.stall_pid_def_module(pid)
            .and_then(|m| self.module.src_file_of_module.get(&m).copied())
    }

    /// The module whose definition `pid`'s originating block was written in,
    /// via the same scope→instance lookup as `stall_pid_src_file`.
    pub(super) fn stall_pid_def_module(&self, pid: usize) -> Option<String> {
        self.scope_def_module(self.process_scope_hint.get(&pid).map(String::as_str))
    }

    /// The module DEFINING the instance at `scope` (empty/None = the top
    /// module itself). Shared by pid-based attribution (stall report) and
    /// scope-hint-based attribution (settle-limit warning, edge blocks).
    pub(super) fn scope_def_module(&self, scope: Option<&str>) -> Option<String> {
        match scope {
            None | Some("") => Some(self.module.name.clone()),
            Some(scope) => self
                .module
                .instances
                .iter()
                .find(|inst| inst.path == scope)
                .map(|inst| inst.def_name.clone()),
        }
    }

    /// The retained-source index for a construct inlined under `scope` —
    /// scope → defining module → file. `None` when any link is missing;
    /// `span_file_line_in` then falls back to the exact-fit rule.
    pub(super) fn scope_src_file(&self, scope: Option<&str>) -> Option<u32> {
        self.scope_def_module(scope)
            .and_then(|m| self.module.src_file_of_module.get(&m).copied())
    }

    /// `assertion_stats` key of an assertion statement: its span start,
    /// tagged with its source file so statements of different files at the
    /// same offset stay apart.
    pub(super) fn assert_stat_key(span_start: usize, src_file: Option<u32>) -> usize {
        span_start | (src_file.map_or(0, |f| f as usize + 1) << 40)
    }

    /// Source file of the running process's module (memoized per process).
    pub(super) fn current_src_file(&mut self) -> Option<u32> {
        let pid = self.current_pid;
        if let Some(&f) = self.pid_src_file.get(&pid) {
            return f;
        }
        let f = self.stall_pid_src_file(pid);
        self.pid_src_file.insert(pid, f);
        f
    }

    /// File and line of a span, resolved like `span_file_line_in`.
    pub(super) fn span_file_and_line(
        &self,
        span: crate::ast::Span,
        src_file: Option<u32>,
    ) -> Option<(String, usize)> {
        let texts = &self.module.source_texts;
        let i = match src_file
            .map(|s| s as usize)
            .filter(|&s| texts.get(s).is_some_and(|t| span.start < t.len()))
        {
            Some(i) => i,
            None => {
                let mut fits = texts
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| span.start < t.len());
                let (i, _) = fits.next()?;
                if fits.next().is_some() {
                    return None;
                }
                i
            }
        };
        if let Some(loc) = self
            .module
            .source_line_maps
            .get(i)
            .and_then(|m| m.as_ref())
            .and_then(|m| m.locate(&texts[i], span))
        {
            return Some((loc.file, loc.line as usize));
        }
        let line = 1 + texts[i].as_bytes()[..span.start]
            .iter()
            .filter(|&&b| b == b'\n')
            .count();
        let file = self
            .module
            .source_files
            .get(i)
            .filter(|f| !f.is_empty())?
            .clone();
        Some((file, line))
    }

    /// Best-effort `file:line` for a statement span. A span is a byte offset
    /// into its OWN file's preprocessed text; `src_file` says which file
    /// that is when the offender's originating module is known (see
    /// `stall_pid_src_file`). Without it — untagged origins — resolve only
    /// when it is unambiguous: exactly ONE retained source is long enough to
    /// contain the offset (trivially true for the single-source case). When
    /// several files could contain it, print nothing rather than risk a
    /// wrong location.
    pub(super) fn span_file_line_in(
        &self,
        span: crate::ast::Span,
        src_file: Option<u32>,
    ) -> Option<String> {
        if span.start == 0 && span.end == 0 {
            return None; // Span::dummy() — synthesized statement, no source
        }
        let texts = &self.module.source_texts;
        let i = match src_file
            .map(|s| s as usize)
            .filter(|&s| texts.get(s).is_some_and(|t| span.start < t.len()))
        {
            Some(i) => i,
            None => {
                // Exact-fit fallback: unambiguous only.
                let mut hit: Option<usize> = None;
                for (i, t) in texts.iter().enumerate() {
                    if span.start < t.len() {
                        if hit.is_some() {
                            return None; // ambiguous across files
                        }
                        hit = Some(i);
                    }
                }
                hit?
            }
        };
        // The line map names the file the text really came from (an
        // `include`d one) and its own line and column.
        if let Some(loc) = self
            .module
            .source_line_maps
            .get(i)
            .and_then(|m| m.as_ref())
            .and_then(|m| m.locate(&texts[i], span))
        {
            return Some(loc.short());
        }
        let line = 1 + texts[i].as_bytes()[..span.start]
            .iter()
            .filter(|&&b| b == b'\n')
            .count();
        let file = self
            .module
            .source_files
            .get(i)
            .filter(|f| !f.is_empty())
            .cloned()
            .unwrap_or_else(|| format!("<source {}>", i + 1));
        // `texts` are PREPROCESSED, with every `\`include` spliced in whole, so
        // this line can point well past the end of the file it names — a
        // 10-line file including 3000 lines reports line 3005. Quoting that
        // bare sends the reader hunting for a line their file does not have.
        // Say so, and give them the number that at least locates it in the
        // expanded text.
        if let Some(&orig) = self.module.source_orig_lines.get(i) {
            if orig > 0 && line as u32 > orig {
                return Some(format!(
                    "{}:{} [preprocessed line; the file has {} lines, so this code came from an \
                     `include or a multi-line macro]",
                    file, line, orig
                ));
            }
        }
        Some(format!("{}:{}", file, line))
    }

    /// The source text a span covers: the `src_file`'s text when the
    /// offender's file is known, else exactly-one-file fallback (same rule
    /// as `span_file_line_in`). Whitespace-collapsed and capped, for inline
    /// display of e.g. a `wait (...)` condition or a `#(...)` delay.
    pub(super) fn span_source_snippet_in(
        &self,
        span: crate::ast::Span,
        src_file: Option<u32>,
    ) -> Option<String> {
        if span.start == 0 && span.end == 0 || span.start >= span.end {
            return None;
        }
        let texts = &self.module.source_texts;
        let i = match src_file
            .map(|s| s as usize)
            .filter(|&s| texts.get(s).is_some_and(|t| span.end <= t.len()))
        {
            Some(i) => i,
            None => {
                let mut hit: Option<usize> = None;
                for (i, t) in texts.iter().enumerate() {
                    if span.end <= t.len() {
                        if hit.is_some() {
                            return None;
                        }
                        hit = Some(i);
                    }
                }
                hit?
            }
        };
        let raw = texts[i].get(span.start..span.end)?;
        let mut s = raw.split_whitespace().collect::<Vec<_>>().join(" ");
        if s.len() > 60 {
            s.truncate(57);
            s.push('…');
        }
        Some(s)
    }

    pub(super) fn expr_snippet_in(&self, e: &Expression, src_file: Option<u32>) -> String {
        self.span_source_snippet_in(e.span, src_file)
            .unwrap_or_else(|| "…".to_string())
    }

    /// Why is `pid` re-arming at this timestamp? Finds its pending
    /// continuation — the running process's own statements, the `#0`
    /// inactive queue, the event/condition waiter lists, or the event
    /// queue — and classifies its head. Terminal-path only.
    pub(super) fn describe_stall_offender(
        &mut self,
        pid: usize,
        running: Option<(usize, &[Statement])>,
        src_file: Option<u32>,
    ) -> Option<String> {
        // The process that tripped the limit is mid-execution: its
        // continuation is on the Rust stack, not in any queue.
        if let Some((rpid, rstmts)) = running {
            if rpid == pid {
                return Some(
                    self.classify_stall_stmts(rstmts, 0, src_file)
                        .unwrap_or_else(|| "currently executing".to_string()),
                );
            }
        }
        // Retained dynamic-delay assignments use an empty scheduler marker,
        // so their pending queue entry has no cloned Forever AST to inspect.
        // Attribute a zero-delay spin from the retained expression itself.
        if let Some(delay) = self
            .fast_delay_always
            .get(&pid)
            .map(|fast| fast.delay.clone())
        {
            if !Self::expr_contains_call(&delay) && self.eval_delay_ticks(&delay) == 0 {
                if let Some(src) = self.span_source_snippet_in(delay.span, src_file) {
                    if src != "0" {
                        return Some(format!("re-arming via #({}) — currently 0", src));
                    }
                }
                return Some("re-arming via #0 delay".to_string());
            }
        }
        // Parked in the Inactive region — only a `#0` puts a process here.
        // Its continuation usually loops back through the timing control that
        // re-arms it (a `forever`/`always` body), so classify that first to
        // recover the actual delay expression (`#(p/2) — currently 0`);
        // degrade to the plain form when the continuation says nothing.
        if let Some((_, cont)) = self.inactive_queue.iter().find(|(p, _)| *p == pid) {
            let cont = cont.clone();
            if let Some(r) = self.classify_stall_stmts(&cont.to_vec(), 0, src_file) {
                // Being parked here PROVES the re-arm was a zero delay; take
                // the classification only when it agrees (it then carries the
                // delay's source text), lest a later suspension point in the
                // continuation (an @(...) after the #0) misattribute the spin.
                if r.starts_with("re-arming via #") {
                    return Some(r);
                }
            }
            return Some("re-arming via #0 delay".to_string());
        }
        // Blocked on @(...) — if it is also a top spinner, something keeps
        // re-triggering it at this timestamp.
        if let Some(w) = self.event_waiters.iter().find(|w| w.pid == pid) {
            let sens = w
                .resolved_sensitivities
                .iter()
                .map(|s| {
                    let edge = s.edge.print_str();
                    format!("{}{}", edge, self.name_for_id(s.signal_id))
                })
                .collect::<Vec<_>>()
                .join(" or ");
            return Some(format!(
                "waiting on @({}) — event ping-pong suspected",
                sens
            ));
        }
        // Blocked on a signal-less `wait (cond)` (condition-waiter fixpoint).
        if let Some((_, cont)) = self.condition_waiters.iter().find(|(p, _)| *p == pid) {
            let cont = cont.clone();
            return self.classify_stall_stmts(&cont.to_vec(), 0, src_file);
        }
        // A delta continuation sitting in the event queue.
        if let Some(stmts) = self.event_queue.pending_stmts_for(pid) {
            let stmts = stmts.to_vec();
            return self.classify_stall_stmts(&stmts, 0, src_file);
        }
        None
    }

    /// Classify the first suspension point in a pending continuation: a `#0`
    /// re-arm, an `@(...)` event wait, a `wait (cond)` (noting whether the
    /// condition is ALREADY true — the classic spin), or a `forever` whose
    /// body never yields. Descends into `forever` / `begin..end` bodies a
    /// few levels; evaluates only call-free expressions so a terminal
    /// diagnostic can't run user side effects.
    pub(super) fn classify_stall_stmts(
        &mut self,
        stmts: &[Statement],
        depth: u32,
        src_file: Option<u32>,
    ) -> Option<String> {
        if depth > 4 {
            return None;
        }
        for s in stmts.iter().take(64) {
            match &s.kind {
                StatementKind::TimingControl { control, .. } => match control {
                    TimingControl::Delay(d) => {
                        if !Self::expr_contains_call(d) && self.eval_delay_ticks(d) == 0 {
                            // Quote the delay EXPRESSION when it is more than
                            // a literal `0` (mirroring how the wait case
                            // quotes its condition) — `#(p/2)` evaluating to
                            // zero is the classic uninitialized-period clock,
                            // and the report should make that legible.
                            if let Some(src) = self.span_source_snippet_in(d.span, src_file) {
                                if src != "0" {
                                    return Some(format!("re-arming via #({}) — currently 0", src));
                                }
                            }
                            return Some("re-arming via #0 delay".to_string());
                        }
                        // A real (nonzero/unknown) delay ahead would advance
                        // time — the spin is something else; stop guessing.
                        return None;
                    }
                    TimingControl::Event(ev) => {
                        return Some(format!(
                            "waiting on @({}) — event ping-pong suspected",
                            self.render_event_control(ev, src_file)
                        ));
                    }
                },
                StatementKind::Wait { condition, .. } => {
                    let cond_src = self.expr_snippet_in(condition, src_file);
                    let already_true =
                        !Self::expr_contains_call(condition) && self.eval_expr(condition).is_true();
                    return Some(if already_true {
                        format!("`wait ({})` with the condition already true", cond_src)
                    } else {
                        format!("blocked on `wait ({})`", cond_src)
                    });
                }
                StatementKind::Forever { body } => {
                    let inner: &[Statement] = match &body.kind {
                        StatementKind::SeqBlock { stmts, .. } => stmts,
                        _ => std::slice::from_ref(&**body),
                    };
                    return Some(
                        self.classify_stall_stmts(inner, depth + 1, src_file)
                            .unwrap_or_else(|| {
                                "loop body has no timing control — never yields".to_string()
                            }),
                    );
                }
                // `ForeverTail` carries the same forever body as `Forever`
                // (it is the resume sentinel), so classify it identically.
                StatementKind::ForeverTail { body } => {
                    let inner: &[Statement] = match &body.kind {
                        StatementKind::SeqBlock { stmts, .. } => stmts,
                        _ => std::slice::from_ref(&**body),
                    };
                    return Some(
                        self.classify_stall_stmts(inner, depth + 1, src_file)
                            .unwrap_or_else(|| {
                                "loop body has no timing control — never yields".to_string()
                            }),
                    );
                }
                StatementKind::SeqBlock { stmts: inner, .. } => {
                    if let Some(r) = self.classify_stall_stmts(inner, depth + 1, src_file) {
                        return Some(r);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Render an `@(...)` event control for the stall report.
    pub(super) fn render_event_control(&self, ev: &EventControl, src_file: Option<u32>) -> String {
        match ev {
            EventControl::Star | EventControl::ParenStar => "*".to_string(),
            EventControl::Identifier(id) => id.name.clone(),
            EventControl::HierIdentifier(e) => self.expr_snippet_in(e, src_file),
            EventControl::EventExpr(terms) => terms
                .iter()
                .map(|t| {
                    let edge = match t.edge {
                        Some(Edge::Posedge) => "posedge ",
                        Some(Edge::Negedge) => "negedge ",
                        Some(Edge::Edge) => "edge ",
                        None => "",
                    };
                    format!("{}{}", edge, self.expr_snippet_in(&t.expr, src_file))
                })
                .collect::<Vec<_>>()
                .join(" or "),
        }
    }

    /// Resume level-sensitive `wait(expr)` waiters parked in
    /// `condition_waiters` whose condition has just become true. IEEE
    /// 1800-2023 §9.7.4: `wait(cond)` is level-sensitive — the process
    /// resumes as soon as the condition is true, INCLUDING in the same
    /// timestep via a delta-cycle (#0) write from another process.
    ///
    /// Because `wait` on non-signal state (class members / locals) cannot use
    /// the edge-triggered event_waiter path, waiters park here and are
    /// re-checked as a bounded fixpoint: reschedule each, run the ones whose
    /// condition now holds (its continuation may free another waiter), apply
    /// any NBA work those runs produced, and iterate until a round advances
    /// no waiter (the remainder are blocked on a genuinely future-time
    /// change).
    ///
    /// The caller invokes this BOTH (a) inside `run_one_tick`'s active loop,
    /// immediately before the INACTIVE (`#0`) region is promoted — so a
    /// waiter is resumed in the SAME delta as the write that satisfied it,
    /// before a `#0`-parked peer's continuation overwrites the value — and
    /// (b) at the end of the tick, after the NBA region has settled, to
    /// catch waiters satisfied by non-blocking/edge-propagated state.
    /// Variant of `drain_condition_waiters` called in `run_one_tick` right BEFORE
    /// the INACTIVE (`#0`) region is promoted:
    /// 1. Does NOT commit NBA (IEEE 1800-2017 §4.5 places the NBA region strictly
    ///    AFTER the Inactive region).
    /// 2. If `#0` continuations are waiting in `inactive_queue`, stops after this
    ///    round so those `#0` continuations execute in this delta rather than
    ///    being starved by multi-generation cascades of condition waiters.
    /// Drain level-sensitive `wait(expr)` waiters until none can proceed at
    /// the current timestamp, re-resuming any that a same-tick write newly
    /// moved into `ready_condition_waiters` even when `cond_progress` did not
    /// step (moving to ready does not itself fire the waiter's body).
    ///
    /// `pre_inactive` selects the two behaviors that differ between the two
    /// call sites:
    ///   * `true`  — the pre-INACTIVE(`#0`) drain: do NOT apply NBA (the NBA
    ///     region stays strictly after the inactive region, IEEE 1800-2017
    ///     §4.5) and yield as soon as a `#0` continuation is pending, so a
    ///     multi-generation condition-waiter cascade cannot starve it.
    ///   * `false` — the end-of-active drain: flush any pending NBA before
    ///     settling, and run to the full same-time fixpoint.
    ///   * `from_repass` — called from the end-of-tick late-region re-pass
    ///     loop. An UNGATED parked waiter (its condition reads names whose
    ///     writes are not noted — RTL signals) is unconditionally re-scheduled
    ///     by every ordinary drain; inside the re-pass fixpoint that schedule
    ///     bumps `late_resume_epoch` without any real progress, so the loop
    ///     would re-drain, re-schedule, re-park the still-false waiter up to
    ///     its 10 000-pass bound every tick (`wait(done >= N)` in a datapath
    ///     regression cost >100× wall-clock). A re-pass drain re-checks each
    ///     ungated waiter at most ONCE per timestamp: if it re-parks, the
    ///     next wake is the ordinary drain outside the re-pass loop (same
    ///     timing as before the re-pass loop existed). Gated waiters (class
    ///     properties / method locals) keep exact same-timestamp semantics —
    ///     their read-bits intersect the store-write mask the moment a late
    ///     region writes them.
    pub(super) fn drain_condition_waiters(&mut self, pre_inactive: bool, from_repass: bool) {
        let mut guard = 0u32;
        while (!self.condition_waiters.is_empty() || !self.ready_condition_waiters.is_empty())
            && !self.finished
            && !self.zero_delay_defer_pending
        {
            guard += 1;
            if guard > 10000 {
                break;
            }
            let prog_before = self.cond_progress;
            let ready = std::mem::take(&mut self.ready_condition_waiters);
            for (cpid, cont) in ready {
                self.cond_waiter_gate.remove(&cpid);
                self.cond_waiter_reads.remove(&cpid);
                self.cond_read_union_dirty = true;
                self.cond_waiter_conditions.remove(&cpid);
                self.event_queue.schedule(self.time, cpid, cont);
            }
            // Parked waiters used to be resumed wholesale on every drain:
            // eight UVM housekeeping waits (`wait(m_premature_end)`, the
            // phase hopper's `.size() != 0`, `wait(0)`) each restored a
            // process, re-evaluated, merged fork frames and parked again —
            // ~40 µs per tick, most of a 5 GHz-clock UVM run. A gated
            // waiter (`cond_gate_for`) is skipped until one of the names it
            // reads is written.
            let parked = std::mem::take(&mut self.condition_waiters);
            let written = take_store_writes();
            let mut still_parked: Vec<(usize, ProcCont)> = Vec::new();
            for (cpid, cont) in parked {
                if let Some(Some(bits)) = self.cond_waiter_gate.get(&cpid) {
                    if !bits.iter().any(|&b| written & b == b) {
                        still_parked.push((cpid, cont));
                        continue;
                    }
                } else if from_repass && self.cond_repass_recheck.get(&cpid) == Some(&self.time) {
                    still_parked.push((cpid, cont));
                    continue;
                }
                if from_repass {
                    // Entries only matter for the current time slot; drop
                    // older ones so the map stays bounded.
                    if self.cond_repass_recheck.len() > 4096 {
                        let now = self.time;
                        self.cond_repass_recheck.retain(|_, t| *t == now);
                    }
                    self.cond_repass_recheck.insert(cpid, self.time);
                }
                self.cond_waiter_gate.remove(&cpid);
                self.cond_waiter_reads.remove(&cpid);
                self.cond_read_union_dirty = true;
                self.cond_waiter_conditions.remove(&cpid);
                self.event_queue.schedule(self.time, cpid, cont);
            }
            let newly = std::mem::take(&mut self.condition_waiters);
            self.condition_waiters = still_parked;
            self.condition_waiters.extend(newly);
            while self.event_queue.next_time() == Some(self.time)
                && !self.finished
                && !self.zero_delay_defer_pending
            {
                let Some((bpid, stmts)) = self.event_queue.pop_front(self.time) else {
                    break;
                };
                self.late_resume_epoch += 1;
                self.run_scheduled_process(bpid, &stmts);
                if !self.is_pid_suspended(bpid) {
                    self.child_finished(bpid);
                }
            }
            if !pre_inactive
                && (!self.nba_fast.is_empty()
                    || !self.nba_queue.is_empty()
                    // §15.5.2: a bare `->>` (deferred trigger) must flush with the
                    // NBA region even when no NBA DATA is queued.
                    || !self.pending_nba_triggers.is_empty()
                    || !self.packed_nba.is_empty()
                    || !self.pending_nba_instance_triggers.is_empty()
                    || self.delayed_nba_due())
            {
                self.apply_nba();
            }
            if self.dirty_any {
                self.settle_combinatorial();
            }
            // Pre-inactive: yield to a pending `#0` continuation rather than
            // running more cascade generations ahead of it.
            if pre_inactive && !self.inactive_queue.is_empty() {
                break;
            }
            // No waiter proceeded this round → the remainder are genuinely
            // blocked (future-time change); stop re-checking until the next
            // tick. But a waiter newly moved to `ready_condition_waiters` by
            // a same-tick write is forward progress whether or not
            // `cond_progress` stepped (moving to ready does not itself fire
            // the waiter's body), so it must be consumed before we can treat
            // the set as blocked.
            if self.cond_progress == prog_before && self.ready_condition_waiters.is_empty() {
                break;
            }
        }
    }

    // Kept out of line: inlined into the event loop, it pushed the loop's
    // other callees out of line, which cost instructions on designs without
    // assertions.
    // Out of line. Once the sampled array cells made the assertion code it
    // used to inline longer, it was inlined into `event_loop_singlethread`
    // instead, which cost a design without assertions 1.1% more
    // instructions.
    #[inline(never)]
    pub(super) fn run_one_tick(
        &mut self,
        accum: &mut PerTickAccum,
        cascade_limit: u32,
        iters: u64,
        trace_loop: bool,
    ) {
        // Track whether anything other than a clock toggle moved a
        // non-clock edge signal this iter.  Same fast-path predicate as
        // the prior inline version — see event_loop history.
        let mut non_clock_change = false;
        self.nba_touched_edge_non_clock = false;

        let profile_timing = self.profile_timing;
        let _t = profile_timing.then(std::time::Instant::now);
        if iters > 1 {
            self.snapshot_edge_signals();
        }
        if let Some(t) = _t {
            accum.t_snap += t.elapsed().as_nanos() as u64;
        }
        // §14.4: #1step samples precede every callback/active-region write
        // at this time, and remain fixed across this slot's later deltas.
        self.refresh_clocking_preponed();
        // LRM §16.5.1: the same holds for the Preponed samples of every
        // SVA-referenced signal: taken before the slot's Pre-Active VPI
        // callbacks (cbAtStartOfSimTime, cbAfterDelay; §4.4.3.2) and its
        // active/NBA regions, and kept by the slot's later deltas, so a
        // clocked assertion firing later in the slot (a clock written from
        // cbReadWriteSynch, or one a later delta raises) samples pre-edge
        // values rather than values written earlier in the slot.
        self.refresh_sva_preponed();
        // §38.36 cbAfterDelay (cocotb's `Timer`) fires HERE: after this slot's
        // edge snapshot and before `check_edges`, so a clock the callback
        // drives registers as a real transition. Firing it before the snapshot
        // meant the snapshot captured the value the callback had just written,
        // so `always @(posedge clk)` compared 1 against 1 and saw no edge — a
        // cocotb-driven DUT sat at x forever while level-sensitive
        // `always @(clk)` blocks worked fine.
        //
        // The slot's start callbacks (cbNextSimTime, cbAtStartOfSimTime) come
        // first, in the first delta of the time, at the same point for the
        // same reason.
        if self.vpi_slot_started != self.time {
            if self.vpi_cb_mask & vpi_cb::MASK_SLOT_START != 0 && self.vpi_slot_start() {
                self.dirty_any = true;
            }
            self.vpi_slot_started = self.time;
        }
        if self.fire_due_after_delay_cbs() {
            self.dirty_any = true;
        }
        self.fire_timing_timers();

        if self.sched_trace_on() {
            eprintln!("[sched] t={} +--- tick ---", self.time);
        }

        if self.apply_delayed_updates() {
            self.settle_combinatorial();
            non_clock_change = true;
        }

        // Clock generators fire at their FIFO rank inside the first wheel
        // drain below — see the interleave note at the call site.
        let mut clocks_fired = false;
        self.toggled_clock_positions.clear();
        let mut clocks_held = self
            .clock_generators
            .iter()
            .any(|c| c.next_toggle_time == self.time);
        // Pops are ranked while a generator waits for its turn, and at time 0
        // while `initial` generators still need their source position.
        let mut rank_pops = clocks_held || (self.time == 0 && !self.clock_generators.is_empty());

        let _t = profile_timing.then(std::time::Instant::now);
        let has_active = self.event_queue.next_time() == Some(self.time);
        if let Some(t) = _t {
            accum.t_sched += t.elapsed().as_nanos() as u64;
        }
        if has_active {
            non_clock_change = true;
        }
        let _t = profile_timing.then(std::time::Instant::now);
        if trace_loop {
            eprintln!(
                "[xezim] iter={} time={} active={}",
                iters, self.time, has_active
            );
        }
        loop {
            while self.event_queue.next_time() == Some(self.time) {
                if self.finished || self.zero_delay_defer_pending {
                    break;
                }
                if clocks_held {
                    clocks_held = self.fire_due_clocks(true);
                    rank_pops = clocks_held || self.time == 0;
                }
                let Some((pid, stmts)) = self.event_queue.pop_front(self.time) else {
                    break;
                };
                if rank_pops && !clocks_fired {
                    self.clock_rank_on_pop(pid);
                }
                if self.sched_trace_on() {
                    eprintln!(
                        "[sched] t={} | proc pid={} {}",
                        self.time,
                        pid,
                        self.sched_pid_label(pid)
                    );
                }
                if trace_loop {
                    eprintln!("[xezim]   running pid={} stmts={}", pid, stmts.len());
                    for (idx, s) in stmts.to_vec().iter().enumerate() {
                        eprintln!(
                            "[xezim]     stmt[{}]: {:?}",
                            idx,
                            std::mem::discriminant(&s.kind)
                        );
                    }
                }
                self.run_scheduled_process(pid, &stmts);
                if !self.is_pid_suspended(pid) {
                    self.child_finished(pid);
                }
            }
            // §4.4.2 interleave (reference-simulator verified): a clock
            // generator is a process whose toggle was scheduled at its
            // PREVIOUS toggle, so events due in the same slot keep FIFO
            // order around it. A `#delay` queued before that point resumes
            // and runs to its next timing control BEFORE the toggle — its
            // `@(posedge clk)` catches this slot's edge and a `clk` read sees
            // the pre-toggle value. One queued after it (a process the last
            // edge woke, or a time-0 process later in source order than an
            // `initial` generator) resumes AFTER the toggle and waits for the
            // next edge. The in-drain firing above releases each generator at
            // its rank; whatever is still held fires here.
            if !clocks_fired {
                clocks_fired = true;
                if self.time == 0 {
                    self.settle_clock_ranks();
                }
                self.fire_due_clocks(false);
            }
            if self.finished || self.zero_delay_defer_pending {
                // Pending same-time activations remain in the timing wheel.
                break;
            }
            // IEEE 1800-2017 §4.5: once the ACTIVE region empties, the
            // INACTIVE region (`#0` continuations) activates and runs BEFORE
            // the NBA region — a `#0`-resumed read must see pre-NBA values
            // (reference-simulator verified; the previous NBA-first order
            // followed a differing commercial camp).
            //
            // Before handing control to the INACTIVE (`#0`) continuations,
            // let any `wait(expr)` whose condition this ACTIVE batch just made
            // true resume FIRST. Level-sensitive `wait` is a same-delta, same-
            // timestep handoff (IEEE 1800-2023 §9.7.4); a `#0`-parked peer's
            // continuation runs in the NEXT delta and may overwrite the value
            // the waiter latches on — e.g. `x = 1; #0; x = 0;` with a peer
            // `wait(x==1)` must observe the 1 and proceed, not skip straight
            // to the 0 (the passthrough-socket AT handshake stalls when the
            // slave's `wait(state!=BEGIN_RESP)` wakes on the clobbered value).
            // The end-of-tick fixpoint alone is too late: the `#0` resume has
            // already committed the overwrite by then.
            self.drain_condition_waiters(true, false);
            if self.promote_inactive_to_active() && self.event_queue.next_time() == Some(self.time)
            {
                // Cont-assign propagation and edge-sensitive blocks are
                // ACTIVE-region work (§4.4.2): complete them so the resumed
                // `#0` observes settled combinational state (a $deposit
                // through a port, a blocking write's fanout). NO NBA flush
                // here (drain_edge_cascade would apply_nba) — the NBA region
                // stays strictly after the inactive region (§4.5).
                if self.dirty_any {
                    self.settle_combinatorial();
                }
                self.check_edges();
                continue;
            }
            break;
        }
        if let Some(t) = _t {
            accum.t_process += t.elapsed().as_nanos() as u64;
        }

        if self.finished {
            return;
        }
        // IEEE 1800-2017 §9.2.2.2: an always_comb with an empty inferred
        // sensitivity list triggers exactly once at time zero, AFTER all
        // initial/always procedures have started (run to their first delay).
        // We deferred these out of the pre-initial settle; fire them now — at
        // the end of the time-0 active region, once the initial blocks above
        // have run and suspended — so a same-time pre-delay read of the output
        // still observes X (matches a reference simulator).
        if self.time == 0 && !self.comb_time0_deferred_done && !self.comb_time0_deferred.is_empty()
        {
            self.comb_time0_deferred_done = true;
            let deferred = std::mem::take(&mut self.comb_time0_deferred);
            for entry in &deferred {
                self.eval_comb_entry_full(entry);
            }
            self.comb_time0_deferred = deferred;
        }
        // Deliver `-> ev` triggers that fired during the pre-process settle
        // now that every process has started and parked its waiters.
        if self.time == 0 && !self.pending_t0_triggers.is_empty() {
            let evs = std::mem::take(&mut self.pending_t0_triggers);
            for ev in evs {
                self.fire_named_event(&ev);
            }
        }

        let _t = profile_timing.then(std::time::Instant::now);
        // §4.5: inactive (`#0`) continuations run before this slot's NBAs.
        self.drain_inactive_pre_nba();
        if self.finished {
            return;
        }
        // §38.36.2 cbNBASynch: the Pre-NBA region.
        if self.vpi_cb_mask & vpi_cb::m(vpi_cb::CB_NBA_SYNCH) != 0 {
            self.vpi_nba_synch();
            if self.finished {
                return;
            }
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
            if self.nba_touched_edge_non_clock {
                non_clock_change = true;
            }
        }
        if let Some(t) = _t {
            accum.t_nba += t.elapsed().as_nanos() as u64;
        }
        let _t = profile_timing.then(std::time::Instant::now);
        if self.dirty_any {
            self.settle_combinatorial();
            non_clock_change = true;
        }
        if !self.delayed_updates.is_empty() && self.apply_due_delayed_now() {
            non_clock_change = true;
        }
        if let Some(t) = _t {
            accum.t_settle += t.elapsed().as_nanos() as u64;
        }
        let _t = profile_timing.then(std::time::Instant::now);
        if !non_clock_change && !self.toggled_clock_positions.is_empty() {
            self.check_edges_clocks_only();
        } else {
            self.check_edges();
        }
        if let Some(t) = _t {
            accum.t_edges += t.elapsed().as_nanos() as u64;
        }
        if self.finished {
            return;
        }
        let (ds, dn, dse, de) = self.drain_edge_cascade(cascade_limit);
        accum.t_snap += ds;
        accum.t_nba += dn;
        accum.t_settle += dse;
        accum.t_edges += de;
        if self.finished {
            return;
        }

        // LRM §4.4 region ordering — observed runs after the NBA region
        // has settled but BEFORE the postponed region (which is where
        // $strobe and $monitor live). No-op when no probes are registered.
        self.drain_observed_region();
        if self.finished {
            return;
        }
        // LRM §16.5 SVA clocked property evaluation. Detects edges on
        // each registered clock signal and ticks pending consequents.
        // A cbNBASynch or cbReadWriteSynch due at a clock edge runs first
        // (`sva_post_nba`).
        self.sva_observed();
        if self.finished {
            return;
        }
        // LRM §14.3 clocking-block input snapshot — refresh per-cycle
        // sample state so subsequent `cb.<sig>` reads see the pre-edge
        // value (`#1step` input skew).
        self.tick_clocking_blocks();
        // §14.13: deliver `@(cb.sig)` events for samples just published. The
        // publish happens after this slot's edge pass, so run one more.
        if self.clocking_mirror_dirty {
            self.clocking_mirror_dirty = false;
            self.edge_dispatch_phase = "clocking-mirror";
            self.check_edges();
            self.edge_dispatch_phase = "active";
            let _ = self.drain_edge_cascade(cascade_limit);
        }
        // §14.13: now that this edge's NBA updates have committed and the
        // clocking input snapshots are refreshed, resume the clocking-event
        // waiters (`@(cb)` / `##N`) that fired this slot — in the Reactive
        // region — so they observe post-edge state and this cycle's samples.
        let tick_late_epoch_start = self.late_resume_epoch;
        self.drain_deferred_clocking_conts();
        if self.finished {
            return;
        }
        self.drain_reactive_region();
        if self.finished {
            return;
        }
        self.drain_nba_region_waiters();
        if self.finished {
            return;
        }
        self.fire_vpi_synch_cbs();
        if self.finished {
            return;
        }
        self.check_monitor();
        self.drain_pending_strobes();
        self.dump_write_changes();

        // Condition-waiter fixpoint (level-sensitive `wait(expr)` resume).
        self.drain_condition_waiters(false, false);

        // LRM §4.4/§4.7: every late-region resume above (clocking conts,
        // reactive initials, Re-NBA waiters, condition waiters re-queued at
        // the same time) still belongs to THIS time slot. A continuation
        // resumed there can toggle a signal or fire a named event, and a bare
        // flip schedules nothing — without another edge/waiter pass its
        // `@(e)` / `@(sig)` waiters wake only in the NEXT slot's tail pass,
        // one full time unit late. Reference-verified shape: a sequencer
        // grant (`uvm_wait_for_nba_region`: `nba <= next_nba; @(nba)`, resumed
        // by drain_nba_region_waiters, then re-parked and run again by the
        // condition-waiter drain) delivers the item and the driver's `-> ev`
        // must wake the DUT's `@(ev)` at the same timestamp — register
        // backdoor mirrors read X otherwise. Loop while the re-pass itself
        // produces further late resumes; bounded like every fixpoint here.
        //
        // Gated on tick_late_epoch_start so that ticks where no late-region
        // resumes ran avoid permanent extra scanning after the first late resume.
        if self.late_resume_epoch != tick_late_epoch_start
            && !self.finished
            && !self.zero_delay_defer_pending
        {
            let mut late_passes = 0u32;
            loop {
                let epoch_before = self.late_resume_epoch;
                if self.dirty_any {
                    self.settle_combinatorial();
                }
                self.check_edges();
                let _ = self.drain_edge_cascade(cascade_limit);
                self.drain_deferred_clocking_conts();
                if self.finished {
                    return;
                }
                self.drain_reactive_region();
                if self.finished {
                    return;
                }
                self.drain_nba_region_waiters();
                if self.finished {
                    return;
                }
                self.drain_condition_waiters(false, true);
                late_passes += 1;
                if self.finished || late_passes > 10_000 || self.late_resume_epoch == epoch_before {
                    break;
                }
            }
        }

        // `#0` continuations parked during the POST-active stages of this
        // tick (edge cascades, reactive work) resume in the next same-time
        // delta: promote so the outer event_loop re-enters run_one_tick,
        // where they run at the top — BEFORE that delta's NBA region (§4.5).
        self.promote_inactive_to_active();

        // `--warn-x` / `X_WARN`: format anything the write hook captured this
        // tick. Deferred to here because the hook runs under a foreign `&mut`
        // borrow and can only record, not print.
        if self.warn_x {
            self.drain_warn_x();
            self.drain_value_trace();
        }

        self.loop_iters += 1;
    }

    /// `start`: a `cbStartOfSimulation` routine wrote a value, so the time-0
    /// slot runs even with nothing else due in it.
    pub(super) fn event_loop(&mut self, start: bool) {
        self.event_loop_singlethread(start);
    }

    /// Ask the event loop to stop at the next slot boundary and finalize.
    ///
    /// Ctrl-C used to kill the process where it stood, and for FST that means
    /// losing EVERYTHING: value changes live in an in-memory block that is only
    /// written by `fst_finish`, so an interrupted run left a header, hierarchy
    /// and geometry with no data at all — a 17MB run reduced to 577 bytes. VCD
    /// and XTrace fare better (they stream) but still lose their tail and any
    /// closing time record.
    ///
    /// The handler itself only stores a flag, which is async-signal-safe; the
    /// loop below polls it and exits through the normal path so `run()` reaches
    /// `vcd_finish`/`xtrace_finish`/`fst_finish`. A SECOND signal restores the
    /// default disposition and re-raises, so an interrupt is never ignored if
    /// finalizing is itself what is stuck.
    pub(super) fn install_interrupt_handler() {
        super::interrupt::install_interrupt_handler();
    }

    /// Take a Ctrl-C / SIGTERM: disarm the backstop and stop the run, so it
    /// leaves through the normal exit and `run()` finalizes the dumps. Called
    /// at slot boundaries and from loop back-edges (a long loop inside one
    /// time slot never reaches a boundary); the first call wins.
    #[cold]
    #[inline(never)]
    pub(crate) fn take_interrupt(&mut self) {
        if self.finished {
            return;
        }
        acknowledge_interrupt();
        eprintln!(
            "[xezim] interrupted at time {} — finalizing waveform dumps",
            self.time
        );
        if self.vpi_cb_mask != 0 {
            self.vpi_notify_signal();
        }
        self.finished = true;
    }

    pub(super) fn event_loop_singlethread(&mut self, mut start: bool) {
        Self::install_interrupt_handler();
        let sim_start = std::time::Instant::now();
        // Realtime twin for the human-facing report (see crate::WallTimer).
        let sim_start_rt = crate::WallTimer::now();
        // Sampled method profiler: spawn a low-rate sampler thread that
        // records whatever method the hot thread is executing. Gated by
        // XEZIM_METHOD_PROFILE=1 so production runs pay nothing.
        sampler_reset();
        let _sampler_thr = if std::env::var_os("XEZIM_METHOD_PROFILE").is_some() {
            SAMPLER_READY.store(true, Ordering::Relaxed);
            let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let d2 = done.clone();
            let handle = std::thread::Builder::new()
                .name("xz-method-sampler".into())
                .spawn(move || {
                    while !d2.load(Ordering::Relaxed) {
                        sampler_sample();
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                });
            Some((done, handle))
        } else {
            None
        };
        let mut iters: u64 = 0;
        let max_iters = self.max_time.saturating_mul(1000);
        let mut accum = PerTickAccum::default();
        let cascade_limit = self.cascade_limit;
        // JIT-redesign Stage 1: optional invariant check on
        // signal_inline_bits.  Walks every signal (~O(num_signals)),
        // gated by XEZIM_VERIFY_INLINE_BITS=1.
        // Late-init writes (memory image loads, all_z nets initialized after
        // the mirror allocation) bypass write_sig!; one O(n) re-snapshot at
        // simulation start makes the planes exact before anything reads them.
        if !self.signal_inline_bits.is_empty() {
            for (i, v) in self.signal_table.iter().enumerate() {
                let (vb, xb) = v.raw_bits();
                self.signal_inline_bits[i] = [vb, xb];
                if i < self.signal_has_xz.len() {
                    self.signal_has_xz[i] = if v.has_xz() { 1 } else { 0 };
                }
            }
        }
        let verify_inline_bits =
            std::env::var("XEZIM_VERIFY_INLINE_BITS").ok().as_deref() == Some("1");
        if verify_inline_bits {
            let mismatches = self.verify_inline_bits_invariant(0);
            if mismatches == 0 && !self.signal_inline_bits.is_empty() {
                eprintln!(
                    "[INLINE_BITS] startup invariant clean ({} signals tracked)",
                    self.signal_inline_bits.len()
                );
            }
        }
        // Optional periodic progress log — set XEZIM_PROGRESS=<seconds> to
        // emit `[PROGRESS]` lines showing sim_time + wall + iters every N
        // seconds. Useful for investigating long-running designs like c910
        // where $display output is sparse.
        let progress_interval = std::env::var("XEZIM_PROGRESS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        let mut next_progress = if progress_interval > 0 {
            std::time::Duration::from_secs(progress_interval)
        } else {
            std::time::Duration::MAX
        };
        // ── Dead-clock watchdog ─────────────────────────────────────────────
        // Signature of a dead-clock hang (undriven net / unresolved cell /
        // ungenerated behavioral PLL VCO): a process is parked on a clock/reset
        // that never changes value, while the rest of the design keeps churning
        // edges (X-propagation) — the sim burns huge wall time making no
        // functional progress. We track whether the OLDEST parked waiter's
        // awaited signals stay frozen across a window of (sim ticks, edges,
        // wall seconds); when all three thresholds are exceeded with the value
        // unchanged, we emit a prominent actionable diagnostic (and optionally
        // abort). Keyed on the signal VALUE, not the park cycle, so a waiter
        // that repeatedly wakes-and-re-parks on a still-dead clock is caught.
        //
        //   XEZIM_STUCK_CLOCK = off | warn (default) | abort
        //   XEZIM_STUCK_CLOCK_TICKS = <sim ticks>   (default 1000, a low floor)
        //   XEZIM_STUCK_CLOCK_EDGES = <edges>       (default 1000000)
        //   XEZIM_STUCK_CLOCK_WALL  = <seconds>     (default 20)
        // All three must be exceeded together, so a healthy sim (its awaited
        // signal toggles) or a quiet one (few edges) or a fast one (little
        // wall) never trips. WALL + EDGES are the real gates — in a dead-clock
        // churn the sim advances very LITTLE sim-time (each tick is expensive),
        // so TICKS is only a low floor confirming time moved at all (not a
        // same-timestamp delta storm, which the separate stall detector owns).
        // Default is WARN — never changes results — with opt-in `abort` for a
        // fast fail in CI/regressions.
        let sc_mode = std::env::var("XEZIM_STUCK_CLOCK")
            .ok()
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_else(|| "warn".to_string());
        let sc_enabled = sc_mode != "off" && sc_mode != "0";
        let sc_abort = sc_mode == "abort";
        let sc_ticks: u64 = std::env::var("XEZIM_STUCK_CLOCK_TICKS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1_000);
        let sc_edges: u64 = std::env::var("XEZIM_STUCK_CLOCK_EDGES")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1_000_000);
        let sc_wall = std::time::Duration::from_secs(
            std::env::var("XEZIM_STUCK_CLOCK_WALL")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(20),
        );
        // Reference snapshot of the monitored waiter's frozen signals.
        let mut wd_sigs: Vec<usize> = Vec::new();
        let mut wd_bits: Vec<(u64, u64)> = Vec::new();
        let mut wd_ref_time: u64 = self.time;
        let mut wd_ref_edges: u64 = self.prof_edges_fired;
        let mut wd_ref_wall = sim_start.elapsed();
        // Event-phase high-water mark when the window opened. A clock returns to
        // the same value each period, so equal *sampled* bits does NOT prove the
        // awaited signal is frozen — it may have toggled 0→1→0→1 between checks.
        // sig_last_change records the phase of each signal's last transition, so
        // a value > this mark means the signal genuinely moved since the window
        // opened (a live clock), suppressing the dead-clock false positive.
        let mut wd_ref_phase: u64 = self.event_phase;
        // Did any monitored signal move while THIS window was open? The
        // `sig_last_change` mark above is only stamped under
        // `event_measure && !armed_edge`, and ARMED mode is the default on a
        // design this size — there the array never advances, so it can never
        // witness a live clock and every window looks frozen. Sampling the
        // window's own signals on every loop iteration (below) is immune to
        // that and to the phase aliasing the mark was added for: a toggling
        // clock is caught whatever phase the periodic check lands on.
        let mut wd_toggled = false;
        let mut wd_warned = false;
        // O1 flop-fire skip is default-ON (correct-by-construction snapshot
        // compare; ~1.1-1.6x on c910). Set XEZIM_EVENT_EDGE=0 to disable.
        let event_skip = std::env::var("XEZIM_EVENT_EDGE").ok().as_deref() != Some("0");
        let event_meas = std::env::var("XEZIM_EVENT_EDGE_MEASURE").ok().as_deref() == Some("1");
        if event_skip || event_meas {
            self.event_measure = true; // enables stamping + state + phase clock
            self.event_skip = event_skip;
            self.event_after = std::env::var("XEZIM_EVENT_AFTER")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            self.build_event_measure_state();
            if event_skip {
                chatter!(
                    "[EVENT-EDGE] SKIP mode ON (sim_time >= {}) — skipping gateable flop fires with no data-input change",
                    self.event_after
                );
            }
        }
        let trace_loop = std::env::var("XEZIM_TRACE_INIT").ok().as_deref() == Some("1");
        if trace_loop {
            eprintln!(
                "[xezim] event_loop start: finished={} eq.empty={} waiters={} clocks={}",
                self.finished,
                self.event_queue.is_empty(),
                self.event_waiters.len(),
                self.clock_generators.len()
            );
        }
        // §38.36.3: a value a VPI client writes before time 0 (from
        // cbStartOfSimulation, where cocotb starts its first test, so its
        // `Immediate` writes there) is processed in the time-0 slot, like one
        // written at time 0. With nothing else due at time 0 the loop used to
        // skip that slot, so the combinational logic read the old value until
        // the next one and toggle coverage never saw the written value as a
        // baseline. `start` holds the loop at time 0 until that slot has run.
        while !self.finished && iters < max_iters {
            iters += 1;
            if HANG_REPORT_REQUESTED.load(std::sync::atomic::Ordering::Relaxed)
                && HANG_REPORT_REQUESTED.swap(false, std::sync::atomic::Ordering::Relaxed)
            {
                eprintln!(
                    "[xezim][hang-report] on-demand report (SIGUSR1) at sim time {}:",
                    self.time
                );
                self.report_parked_waiters(12);
            }
            // Dead-clock watchdog, liveness half: witness a toggle on the
            // monitored signals EVERY iteration. The periodic check below only
            // samples every 1024 iterations, and equal sampled bits do not mean
            // frozen — a clock returns to the same value each period. At most a
            // couple of signals, so this is noise next to a loop iteration.
            if sc_enabled && !wd_warned && !wd_toggled && !wd_sigs.is_empty() {
                wd_toggled = wd_sigs
                    .iter()
                    .zip(wd_bits.iter())
                    .any(|(&id, &b)| self.term_raw_bits(id) != b);
            }
            // Dead-clock watchdog: cheap periodic check (every 1024 iters).
            if sc_enabled && !wd_warned && (iters & 1023) == 0 && !self.event_waiters.is_empty() {
                if let Some(w) = self.event_waiters.iter().min_by_key(|w| w.parked_time) {
                    let cur_sigs: Vec<usize> = w
                        .resolved_sensitivities
                        .iter()
                        .map(|s| s.signal_id)
                        .collect();
                    let cur_bits: Vec<(u64, u64)> =
                        cur_sigs.iter().map(|&id| self.term_raw_bits(id)).collect();
                    // A live clock periodically returns to the same value, so
                    // comparing sampled bits alone false-positives on a healthy
                    // toggling clock whenever the 1024-iter sampling aligns with
                    // its phase. Confirm with sig_last_change (precise per-signal
                    // transition phase): if any awaited signal moved since the
                    // window opened, it is a LIVE clock — treat as progress.
                    let toggled_since_ref = self.event_measure
                        && !self.armed_edge
                        && cur_sigs.iter().any(|&id| {
                            id < self.sig_last_change.len()
                                && self.sig_last_change[id] > wd_ref_phase
                        });
                    // Same monitored signals AND all still at their reference
                    // value AND no transition since the window opened = the
                    // awaited clock/reset is genuinely frozen.
                    if !cur_sigs.is_empty()
                        && cur_sigs == wd_sigs
                        && cur_bits == wd_bits
                        && !toggled_since_ref
                        && !wd_toggled
                    {
                        let d_ticks = self.time.saturating_sub(wd_ref_time);
                        let d_edges = self.prof_edges_fired.saturating_sub(wd_ref_edges);
                        let d_wall = sim_start.elapsed().saturating_sub(wd_ref_wall);
                        if d_ticks >= sc_ticks && d_edges >= sc_edges && d_wall >= sc_wall {
                            let sig = self.name_for_id(cur_sigs[0]).to_string();
                            let pid = w.pid;
                            eprintln!(
                                "\n=================== xezim: DEAD-CLOCK WATCHDOG ==================="
                            );
                            eprintln!(
                                "Process pid {} has been parked on '{}' for {} sim tick(s) with NO\n\
                                 change to its awaited signal(s), while the design fired {} edges in\n\
                                 {:.0}s of wall time. '{}' looks like a DEAD clock/reset.",
                                pid,
                                sig,
                                d_ticks,
                                d_edges,
                                d_wall.as_secs_f64(),
                                sig
                            );
                            eprintln!(
                                "Likely cause: an undriven net, an unresolved/black-boxed cell, or an\n\
                                 ungenerated behavioral clock (PLL VCO) upstream — search the log for\n\
                                 '[IMPLICIT NET' and '-v resolution summary' / 'unresolved definition'.\n\
                                 The simulation is making no functional progress (X-churn)."
                            );
                            self.report_parked_waiters(8);
                            if sc_abort {
                                eprintln!(
                                    "Aborting (XEZIM_STUCK_CLOCK=abort). Fix the dead clock/reset, or\n\
                                           set XEZIM_STUCK_CLOCK=warn to continue."
                                );
                                eprintln!(
                                    "=================================================================\n"
                                );
                                self.finished = true;
                                self.stuck_clock_aborted = true;
                            } else {
                                eprintln!(
                                    "Continuing (set XEZIM_STUCK_CLOCK=abort to stop here; =off to silence)."
                                );
                                eprintln!(
                                    "=================================================================\n"
                                );
                            }
                            wd_warned = true; // report once
                        }
                    } else {
                        // Progress (signal changed or a different waiter is
                        // oldest): reset the reference window.
                        wd_sigs = cur_sigs;
                        wd_bits = cur_bits;
                        wd_ref_time = self.time;
                        wd_ref_edges = self.prof_edges_fired;
                        wd_ref_wall = sim_start.elapsed();
                        wd_ref_phase = self.event_phase;
                        wd_toggled = false;
                    }
                }
            }
            if progress_interval > 0 && sim_start.elapsed() >= next_progress {
                eprintln!(
                    "[PROGRESS] wall={:.1}s sim_time={} iters={} delta_cycles={} edges_fired={} nba_q={} waiters={}",
                    sim_start.elapsed().as_secs_f64(),
                    self.time,
                    iters,
                    self.stall_iters,
                    self.prof_edges_fired,
                    self.nba_fast.len() + self.nba_queue.len(),
                    self.event_waiters.len()
                );
                // Oldest parked waiter one-liner: an ancient waiter whose
                // signals never moved is the signature of a dead-clock hang.
                if let Some(w) = self.event_waiters.iter().min_by_key(|w| w.parked_time) {
                    let age = self.time.saturating_sub(w.parked_time);
                    let all_dead = w
                        .resolved_sensitivities
                        .iter()
                        .zip(w.arm_bits.iter())
                        .all(|(sid, &armed)| self.term_raw_bits(sid.signal_id) == armed);
                    let first_sig = w
                        .resolved_sensitivities
                        .first()
                        .map(|sid| self.name_for_id(sid.signal_id).to_string())
                        .unwrap_or_else(|| "?".to_string());
                    eprintln!(
                        "[PROGRESS] oldest waiter: pid {} on '{}' for {} tick(s), remaining_events={}{}",
                        w.pid,
                        first_sig,
                        age,
                        w.remaining_events,
                        if all_dead {
                            " — awaited signal(s) UNCHANGED since parked"
                        } else {
                            ""
                        }
                    );
                }
                if let Some((bi, count)) = self.hottest_stall_edge_block() {
                    if let Some(line) = self.edge_block_stall_identity(bi, count) {
                        eprintln!("[PROGRESS] hottest edge block:");
                        eprintln!("{}", line);
                    }
                }
                // Live PROF breakdown: the end-of-run [PROF] summary never
                // prints for a run that doesn't finish (the exact case we need
                // to profile), so emit the cumulative phase split with each
                // PROGRESS tick. The percentages localize the hot phase —
                // edge_exec (bytecode) vs settle (combinational) vs process
                // (procedural/waiter) vs sched/nba — without a profiler build.
                // Only meaningful when the phase timers are armed, so gate on
                // XEZIM_PROFILE_TIMING (else the numbers would all read 0).
                if self.profile_timing {
                    let wall_ns = sim_start.elapsed().as_nanos().max(1) as f64;
                    let pct = |ns: u64| 100.0 * ns as f64 / wall_ns;
                    let ms = |ns: u64| ns as f64 / 1e6;
                    eprintln!(
                        "[PROGRESS][PROF] settle={:.0}ms({:.0}%) edges={:.0}ms({:.0}%) nba={:.0}ms({:.0}%) process={:.0}ms({:.0}%) sched={:.0}ms({:.0}%) snap={:.0}ms({:.0}%)",
                        ms(accum.t_settle),
                        pct(accum.t_settle),
                        ms(accum.t_edges),
                        pct(accum.t_edges),
                        ms(accum.t_nba),
                        pct(accum.t_nba),
                        ms(accum.t_process),
                        pct(accum.t_process),
                        ms(accum.t_sched),
                        pct(accum.t_sched),
                        ms(accum.t_snap),
                        pct(accum.t_snap),
                    );
                    eprintln!(
                        "[PROGRESS][PROF] edge_exec={:.0}ms({:.0}%) edge_detect={:.0}ms({:.0}%) waiters={:.0}ms({:.0}%) insns={} ns/insn={:.1} edges_fired={}",
                        ms(self.prof_edge_exec),
                        pct(self.prof_edge_exec),
                        ms(self.prof_edge_detect),
                        pct(self.prof_edge_detect),
                        ms(self.prof_edge_waiters),
                        pct(self.prof_edge_waiters),
                        self.prof_insns_executed,
                        if self.prof_insns_executed > 0 {
                            self.prof_edge_exec as f64 / self.prof_insns_executed as f64
                        } else {
                            0.0
                        },
                        self.prof_edges_fired,
                    );
                }
                next_progress += std::time::Duration::from_secs(progress_interval);
            }

            // Ctrl-C / SIGTERM: leave through the normal exit so the dumps are
            // finalized rather than truncated.
            if interrupt_requested() {
                self.take_interrupt();
                break;
            }
            let has_timed = !self.event_queue.is_empty();
            let has_waiters = !self.event_waiters.is_empty();
            let has_clocks = !self.clock_generators.is_empty();
            let has_reactive = !self.pending_reactive.is_empty() || !self.reactive_conts.is_empty();

            if !has_timed
                && !has_waiters
                && !has_clocks
                && self.delayed_updates.is_empty()
                && self.delayed_nba.is_empty()
                && self.delayed_nba_queue.is_empty()
                && !has_reactive
                // A pending VPI timer (cbAfterDelay, a future cbReadWriteSynch,
                // ...) is real future work even when the HDL side is idle — a
                // cocotb testbench drives everything from VPI timers, so
                // without this the run ends at time 0 and every `Timer`
                // reports as never set up.
                && !self.vpi_timed_future()
                && !start
            {
                // The slot still ends: its cbAtEndOfSimTime/cbReadOnlySynch
                // run, and may schedule more.
                if self.vpi_cb_mask != 0 && self.vpi_slot_end() {
                    continue;
                }
                break;
            }
            // Deadlock: only waiters remain but nothing can ever wake them.
            // A pending phase/drain timer still needs the loop to advance to its
            // deadline, so don't treat that as a deadlock.
            if has_waiters
                && !has_timed
                && !has_clocks
                && self.delayed_updates.is_empty()
                && self.delayed_nba.is_empty()
                && self.delayed_nba_queue.is_empty()
                && !self.vpi_timed_future()
                && !start
            {
                if self.vpi_cb_mask != 0 && self.vpi_slot_end() {
                    continue;
                }
                break;
            }

            // Determine next time: minimum of event queue, clock generators, and delayed updates
            let next_eq_time = self.event_queue.next_time();
            let next_clk_time = if has_clocks {
                self.clock_generators
                    .iter()
                    .map(|c| c.next_toggle_time)
                    .min()
            } else {
                None
            };
            let next_delayed = self.next_delayed_time();
            let next_nba_time = if !self.nba_fast.is_empty()
                || !self.nba_queue.is_empty()
                || !self.packed_nba.is_empty()
                || !self.nba_region_waiters.is_empty()
            {
                Some(self.time)
            } else {
                None
            };
            let next_time = [
                next_eq_time,
                next_clk_time,
                next_delayed,
                next_nba_time,
                self.next_timer_time(),
            ]
            .into_iter()
            .flatten()
            .min()
            .unwrap_or({
                if has_waiters || has_reactive {
                    self.time
                } else {
                    u64::MAX
                }
            });
            let next_time = if start { self.time } else { next_time };

            if next_time > self.max_time {
                if self.vpi_cb_mask != 0 && self.vpi_slot_end() {
                    continue;
                }
                if !self.finished
                    && !RUN_LENGTH_REQUESTED.load(std::sync::atomic::Ordering::Relaxed)
                {
                    eprintln!(
                        "[xezim][hang-report] simulation reached --max-time ({} ticks) without $finish",
                        self.max_time
                    );
                    self.report_parked_waiters(8);
                }
                break;
            }
            // §16.4: the time slot is over — deferred assertion reports
            // mature. Their actions may schedule same-time work, so look
            // again before advancing.
            if next_time > self.time && !self.deferred_asserts.is_empty() {
                self.mature_deferred_asserts();
                continue;
            }
            // §38.36.2: the slot ends — cbAtEndOfSimTime, cbReadOnlySynch —
            // before time moves. A routine may add work to this slot or an
            // earlier timer, so look again when any ran.
            if next_time > self.time && self.vpi_cb_mask != 0 && self.vpi_slot_end() {
                continue;
            }
            if next_time > self.time {
                self.dump_slot_end();
                self.time = next_time;
                publish_sim_time(next_time);
                // Lines printed near the end of a burst stay buffered until the
                // next line; release them once they are stale (core #49).
                if let Some(sink) = self.stdout_sink.as_mut() {
                    sink.flush_if_stale();
                }
                self.stall_iters = 0;
                self.stall_time = next_time;
                self.stall_pid_hits.clear();
                self.stall_edge_block_hits.clear();
                // Time advanced on its own — any spinner that asked to be
                // deferred resolved itself, so DON'T service a stale defer
                // request (it would fire at the new time and jump past a clock
                // toggle that is due there). Let run_one_tick process the new
                // time; if the spinner re-arms, the per-pid check re-flags it
                // with fresh clock/queue state.
                self.zero_delay_defer_pending = false;
            } else {
                // Time did NOT advance this iteration. That is normal for a few
                // delta cycles; it is a livelock if it never ends.
                if self.time == self.stall_time {
                    self.stall_iters += 1;
                } else {
                    self.stall_time = self.time;
                    self.stall_iters = 1;
                }
                if self.stall_limit > 0 && self.stall_iters > self.stall_limit {
                    // Try to BREAK the zero-delay livelock by deferring the stuck
                    // current-time processes to the next scheduled event. Only a
                    // livelock with NOTHING ahead is fatal.
                    if !self.try_break_zero_delay_livelock() {
                        self.report_zero_delay_stall(None);
                        self.abort_run();
                        break;
                    }
                }
            }
            // A per-pid spinner inside run_one_tick (an `always #(period)` block
            // whose real period glitched to 0) asked the outer loop to break the
            // livelock rather than abort. Attempt the same defer-and-advance;
            // fatal only if nothing is scheduled ahead.
            if self.zero_delay_defer_pending {
                self.zero_delay_defer_pending = false;
                if !self.try_break_zero_delay_livelock() {
                    self.report_zero_delay_stall(None);
                    self.abort_run();
                    break;
                }
            }

            // Complex synchronization handshakes legitimately spend many
            // delta cycles at one timestamp: every `#0` wait and each state resume
            // advances `cond_progress`. The zero-delay stall detector exists to catch
            // livelocks (a process re-arming forever with nothing changing),
            // NOT finite delta-dense settling. So only count an iteration
            // toward `stall_iters` when the tick made NO wait-driven progress:
            // reset the counter whenever `cond_progress` advanced. (The
            // per-process `stall_pid_hits` detector still catches a single
            // spinning process; `drain_edge_cascade`'s `cascade_limit` still
            // catches NBA feedback loops.)
            let cond_prog_before = self.cond_progress;
            self.run_one_tick(&mut accum, cascade_limit, iters, trace_loop);
            start = false;
            if self.cond_progress != cond_prog_before {
                self.stall_iters = 0;
            }
            // The genuine library owns the full phase schedule
            // Periodic invariant check — every 1000 iters; bail on
            // mismatch to surface bugs early.
            if verify_inline_bits && iters % 1000 == 0 {
                let mismatches = self.verify_inline_bits_invariant(iters);
                if mismatches > 0 {
                    eprintln!(
                        "[INLINE_BITS] ABORTING at iter={} due to {} mismatches",
                        iters, mismatches
                    );
                    self.abort_run();
                    break;
                }
            }
        }
        // The last slot ends too, unless `$finish` ended it: §20.2 stops the
        // run inside that slot, and toggle coverage counts nothing written
        // there, as before.
        if !self.finished {
            self.dump_slot_end();
        }
        if verify_inline_bits && !self.signal_inline_bits.is_empty() {
            let mismatches = self.verify_inline_bits_invariant(iters);
            if mismatches == 0 {
                eprintln!("[INLINE_BITS] final invariant clean ({} iters)", iters);
            }
        }
        // PerTickAccum → local vars for the existing PROF print code.
        let t_settle = accum.t_settle;
        let t_edges = accum.t_edges;
        let t_nba = accum.t_nba;
        let t_process = accum.t_process;
        let t_snap = accum.t_snap;
        let t_sched = accum.t_sched;
        let sim_elapsed = sim_start_rt.elapsed();
        chatter!(
            "[PROF] settle={:.1}ms edges={:.1}ms nba={:.1}ms process={:.1}ms snap={:.1}ms sched={:.1}ms",
            t_settle as f64 / 1e6,
            t_edges as f64 / 1e6,
            t_nba as f64 / 1e6,
            t_process as f64 / 1e6,
            t_snap as f64 / 1e6,
            t_sched as f64 / 1e6
        );
        if std::env::var("XEZIM_CACHE_FIT").is_ok() {
            let mb = |bytes: usize| bytes as f64 / (1024.0 * 1024.0);
            let vs = std::mem::size_of::<Value>();
            let ce = std::mem::size_of::<CombEntry>();
            // Wide-storage heap: only signals wider than 64b carry a Vec<LogicBit>.
            let mut wide_heap = 0usize;
            let mut wide_cnt = 0usize;
            // Is `signal_widths` (134 MiB at 35M signals) redundant with the
            // `width` already carried inside each `Value`? They are seeded
            // together at construction; this reports whether they ever diverge.
            let mut width_mismatch = 0usize;
            let mut first_mismatch: Option<(usize, u32, u32)> = None;
            for (i, v) in self.signal_table.iter().enumerate() {
                if v.width > 64 {
                    wide_cnt += 1;
                    wide_heap += v.width as usize;
                }
                if let Some(&w) = self.signal_widths.get(i) {
                    if w != v.width {
                        width_mismatch += 1;
                        if first_mismatch.is_none() {
                            first_mismatch = Some((i, w, v.width));
                        }
                    }
                }
            }
            // Per-entry heap (read/write id vecs, scope_hint strings).
            let mut entry_heap = 0usize;
            for e in &self.comb_entries {
                entry_heap += e.cold.read_signal_ids.capacity() * 8
                    + e.cold.write_signal_ids.capacity() * 8
                    + e.cold.scope_hint.as_ref().map_or(0, |s| s.capacity());
            }
            let st = self.signal_table.capacity() * vs;
            let ceb = self.comb_entries.capacity() * ce;
            let sw = self.signal_widths.capacity() * 4;
            let sr = self.signal_real.capacity();
            let s2 = self.signal_two_state.capacity();
            let dof = self.comb_dep_offsets.capacity() * 4;
            let den = self.comb_dep_entries.capacity() * 4;
            let strig = self.settle_triggered.capacity();
            let dsig = self.dirty_signals.capacity();
            let tog = self.signal_toggle_counts.capacity() * 8;
            // Read the ACTUAL cache geometry. The previous hardcoded
            // "L1d=32KiB L2=1MiB L3=16.5MiB" was wrong on this machine (Tiger
            // Lake: 48 KiB / 1.25 MiB / 12 MiB), which silently skewed every
            // "x L3" ratio below by ~37%.
            let rd_cache = |lvl: u32, ty: &str| -> Option<usize> {
                for idx in 0..8 {
                    let base = format!("/sys/devices/system/cpu/cpu0/cache/index{}", idx);
                    let l = std::fs::read_to_string(format!("{}/level", base)).ok()?;
                    let t = std::fs::read_to_string(format!("{}/type", base)).ok()?;
                    if l.trim() == lvl.to_string() && t.trim().starts_with(ty) {
                        let sz = std::fs::read_to_string(format!("{}/size", base)).ok()?;
                        let sz = sz.trim();
                        let n: usize = sz
                            .trim_end_matches(|c: char| c.is_alphabetic())
                            .parse()
                            .ok()?;
                        return Some(if sz.ends_with('M') {
                            n * 1024 * 1024
                        } else {
                            n * 1024
                        });
                    }
                }
                None
            };
            let l1d = rd_cache(1, "Data").unwrap_or(32 * 1024);
            let l2 = rd_cache(2, "Unified").unwrap_or(1024 * 1024);
            let l3 = rd_cache(3, "Unified").unwrap_or(16 * 1024 * 1024);
            eprintln!(
                "[CACHE-FIT] L1d={:.0}KiB/core L2={:.2}MiB/core L3={:.1}MiB shared line=64B",
                l1d as f64 / 1024.0,
                mb(l2),
                mb(l3)
            );
            eprintln!(
                "[CACHE-FIT] signals={} entries={} sizeof(Value)={} sizeof(CombEntry)={}",
                self.signal_table.len(),
                self.comb_entries.len(),
                vs,
                ce
            );
            eprintln!("[CACHE-FIT] --- per-signal arrays (indexed by signal id) ---");
            eprintln!(
                "[CACHE-FIT]   signal_table       {:>9.2} MiB  ({} B/sig, {} sig/line)",
                mb(st),
                vs,
                64 / vs.max(1)
            );
            eprintln!("[CACHE-FIT]   signal_widths(u32) {:>9.2} MiB", mb(sw));
            eprintln!("[CACHE-FIT]   signal_real(bool)  {:>9.2} MiB", mb(sr));
            eprintln!("[CACHE-FIT]   signal_two_state   {:>9.2} MiB", mb(s2));
            eprintln!("[CACHE-FIT]   dirty_signals      {:>9.2} MiB", mb(dsig));
            eprintln!("[CACHE-FIT]   signal_toggle(u64) {:>9.2} MiB", mb(tog));
            eprintln!("[CACHE-FIT]   dep_offsets(u32)   {:>9.2} MiB", mb(dof));
            eprintln!(
                "[CACHE-FIT]   Wide heap          {:>9.2} MiB  ({} wide signals)",
                mb(wide_heap),
                wide_cnt
            );
            eprintln!("[CACHE-FIT] --- per-entry arrays (indexed by comb entry) ---");
            eprintln!("[CACHE-FIT]   comb_entries       {:>9.2} MiB", mb(ceb));
            eprintln!(
                "[CACHE-FIT]   entry heap(r/w ids){:>9.2} MiB",
                mb(entry_heap)
            );
            eprintln!("[CACHE-FIT]   settle_triggered   {:>9.2} MiB", mb(strig));
            eprintln!("[CACHE-FIT]   dep_entries(u32)   {:>9.2} MiB", mb(den));
            if width_mismatch == 0 {
                eprintln!(
                    "[CACHE-FIT] signal_widths vs Value.width: IDENTICAL for all {} signals => the {:.2} MiB array is REDUNDANT",
                    self.signal_table.len(),
                    mb(sw)
                );
            } else {
                eprintln!(
                    "[CACHE-FIT] signal_widths vs Value.width: {} MISMATCHES (first: id={} arr={} val={}) => NOT redundant",
                    width_mismatch,
                    first_mismatch.unwrap().0,
                    first_mismatch.unwrap().1,
                    first_mismatch.unwrap().2
                );
            }
            let per_sig_hot = st + sw + sr + s2 + dsig;
            eprintln!(
                "[CACHE-FIT] hot per-signal working set (table+width+real+2st+dirty) = {:.2} MiB  => {:.0}x L3",
                mb(per_sig_hot),
                per_sig_hot as f64 / l3 as f64
            );
            // These 39 bytes live in FIVE SEPARATE arrays, so touching all of
            // them costs up to five distinct cache lines. The old formula
            // divided the byte total by the line size, which answers the
            // contiguous-layout question — the opposite of what "if scattered"
            // means, and it made a 5-line access look like 1.
            eprintln!(
                "[CACHE-FIT] one signal's hot bytes = {} B across 5 arrays => up to 5 lines scattered (1 line if packed)",
                vs + 4 + 1 + 1 + 1
            );
        }
        let unresolved = self
            .comb_entries
            .iter()
            .filter(|e| e.has_unresolved_reads)
            .count();
        chatter!(
            "[PROF] edge_waiters={:.1}ms edge_cg={:.1}ms waiter_iters={}",
            self.prof_edge_waiters as f64 / 1e6,
            self.prof_edge_cg as f64 / 1e6,
            self.prof_waiter_iters
        );
        chatter!(
            "[PROF] clocks_only_detect={} nba_elided={} (iters with clocks-only scan / NBAs dropped at eval because value matched signal_table)",
            self.prof_clocks_only_detect,
            self.prof_nba_elided
        );
        if self.prof_par_ticks > 0 {
            chatter!(
                "[PROF] parallel_dispatch ticks={} blocks={} (their insns are NOT counted in insns= below — they run on worker threads)",
                self.prof_par_ticks,
                self.prof_par_blocks
            );
        }
        self.dump_comb_paths();
        self.dump_edge_paths();
        self.dump_comb_evals();
        self.dump_template_census();
        self.dump_island_census();
        self.dump_cycle_census();
        self.dump_chain_census();
        self.dump_coact_census();
        chatter!(
            "[PROF] edge_detect={:.1}ms edge_exec={:.1}ms edges_fired={} insns={} ns_per_insn={:.1} fallbacks={}",
            self.prof_edge_detect as f64 / 1e6,
            self.prof_edge_exec as f64 / 1e6,
            self.prof_edges_fired,
            self.prof_insns_executed,
            if self.prof_insns_executed > 0 {
                self.prof_edge_exec as f64 / self.prof_insns_executed as f64
            } else {
                0.0
            },
            self.prof_fallback_insns
        );
        // Static count of `LoadSignal;LoadArrayElem;NbaAssign` triples the
        // peephole collapsed into `Insn::NbaAssignArrayRead` (see
        // `BytecodeCompiler::fuse_array_read_nba`).
        chatter!(
            "[FUSE] array-read-NBA fusions (static sites): {}",
            super::bytecode::array_read_nba_fusions()
        );
        chatter!(
            "[FUSE] packed-loop NBA copies (static sites): {}",
            super::bytecode::packed_loop_nba_copies()
        );
        chatter!(
            "[FUSE] packed blocking fills (dynamic executions): {}",
            self.packed_blocking_fill_collapses
        );
        // Static count of `LoadConst;<binop>` pairs the peephole collapsed
        // into `Insn::BinOpConst` (see `BytecodeCompiler::fuse_binop_const`).
        {
            let f = super::bytecode::binop_const_fusions();
            chatter!(
                "[FUSE] const-operand ALU fusions (static sites): Add={} Eq={} CaseEq={} Xor={} And={} Or={} Mul={} Sub={} total={}",
                f[0],
                f[1],
                f[2],
                f[3],
                f[4],
                f[5],
                f[6],
                f[7],
                f.iter().sum::<u64>()
            );
            chatter!(
                "[FUSE] Move-into-assign forwards (static sites): {}",
                super::bytecode::census_pair_fusions()
            );
            chatter!(
                "[FUSE] copies forwarded into readers (static sites): {}",
                super::bytecode::copy_forward_count()
            );
            chatter!(
                "[FUSE] provably-unsigned scrubs elided (static sites): {}",
                super::bytecode::elided_scrub_count()
            );
            chatter!(
                "[FUSE] AddC2 superinstructions (static sites): {}",
                super::bytecode::addc2_count()
            );
        }
        // XEZIM_OPCODE_CENSUS=1: dynamic opcode + adjacent-pair histogram for
        // fusion planning. Top-20 of each, with % of total executed insns.
        if self.census_enabled && !self.census_counts.is_empty() {
            let total: u64 = self.census_counts.iter().sum::<u64>().max(1);
            let name = |op: usize| {
                format!("{:?}", unsafe {
                    std::mem::transmute::<u8, super::dispatch::Opcode>(op as u8)
                })
            };
            let mut singles: Vec<(usize, u64)> = self
                .census_counts
                .iter()
                .enumerate()
                .filter(|(_, c)| **c > 0)
                .map(|(i, c)| (i, *c))
                .collect();
            singles.sort_by_key(|&(_, c)| std::cmp::Reverse(c));
            eprintln!("[CENSUS] total_insns={}", total);
            for &(op, c) in singles.iter().take(20) {
                eprintln!(
                    "[CENSUS] op {:>28} {:>14} ({:.1}%)",
                    name(op),
                    c,
                    100.0 * c as f64 / total as f64
                );
            }
            let mut pairs: Vec<(usize, u64)> = self
                .census_pairs
                .iter()
                .enumerate()
                .filter(|(_, c)| **c > 0)
                .map(|(i, c)| (i, *c))
                .collect();
            pairs.sort_by_key(|&(_, c)| std::cmp::Reverse(c));
            for &(k, c) in pairs.iter().take(20) {
                eprintln!(
                    "[CENSUS] pair {:>26} -> {:<26} {:>13} ({:.1}%)",
                    name(k / super::dispatch::NUM_OPCODES),
                    name(k % super::dispatch::NUM_OPCODES),
                    c,
                    100.0 * c as f64 / total as f64
                );
            }
        }
        #[cfg(feature = "opcode-census")]
        if self.census_enabled && !self.ts_census.is_empty() {
            let total: u64 = self.ts_census.values().sum::<u64>().max(1);
            let mut singles: std::collections::HashMap<&'static str, u64> =
                std::collections::HashMap::new();
            for (&(_, b), &c) in self.ts_census.iter() {
                *singles.entry(b).or_insert(0) += c;
            }
            let mut sv: Vec<_> = singles.into_iter().collect();
            sv.sort_by_key(|&(_, c)| std::cmp::Reverse(c));
            eprintln!("[TS-CENSUS] total_insns={}", total);
            for (&(a, b), &c) in self.ts_census.iter() {
                if a == "XBAIL" {
                    eprintln!("[TS-CENSUS] xbail after {:>16} {:>14}", b, c);
                }
            }
            {
                let mut v: Vec<(usize, u64, u8)> = self
                    .ts_xbail_by_entry
                    .iter()
                    .map(|(&e, &(c, k))| (e, c, k))
                    .collect();
                v.sort_by_key(|&(_, c, _)| std::cmp::Reverse(c));
                let n_entries = v.len();
                let tot: u64 = v.iter().map(|x| x.1).sum();
                eprintln!("[TS-CENSUS] xbail entries={} total={}", n_entries, tot);
                for &(e, c, k) in v.iter().take(16) {
                    let evals = self.prof_entry_counts.get(e).copied().unwrap_or(0);
                    let label = self
                        .comb_entries
                        .get(e)
                        .map(|en| self.comb_entry_trace_label(e, en))
                        .unwrap_or_default();
                    let label: String = label.chars().take(110).collect();
                    eprintln!(
                        "[TS-CENSUS] xbail entry eidx={} kind={} bails={} evals={} {}",
                        e, k, c, evals, label
                    );
                }
            }
            for &(n, c) in sv.iter().take(60) {
                eprintln!(
                    "[TS-CENSUS] op {:>16} {:>14} ({:.1}%)",
                    n,
                    c,
                    100.0 * c as f64 / total as f64
                );
            }
            let mut pv: Vec<_> = self.ts_census.iter().map(|(k, c)| (*k, *c)).collect();
            pv.sort_by_key(|&(_, c)| std::cmp::Reverse(c));
            for &((a, b), c) in pv.iter().take(40) {
                eprintln!(
                    "[TS-CENSUS] pair {:>16} -> {:<16} {:>13} ({:.1}%)",
                    a,
                    b,
                    c,
                    100.0 * c as f64 / total as f64
                );
            }
        }
        if self.event_measure && self.event_gateable_total > 0 {
            chatter!(
                "[EVENT-EDGE] would-skip {}/{} gateable main-clk flop-fires ({:.1}%) had NO data-input change since last posedge => event-driven edge could skip them",
                self.event_would_skip,
                self.event_gateable_total,
                100.0 * self.event_would_skip as f64 / self.event_gateable_total as f64,
            );
            if self.event_skip {
                chatter!(
                    "[EVENT-EDGE] adaptive epoch-fast-exec={} snapshot-checks={} healed={}",
                    self.event_epoch_fast_exec,
                    self.event_snapshot_checks,
                    self.event_healed
                );
                if let Some(c) = &self.arm_census {
                    let mut rows: Vec<(usize, u64, u64)> = c
                        .iter()
                        .enumerate()
                        .filter(|(_, v)| v.1 > 0)
                        .map(|(i, v)| (i, v.0, v.1))
                        .collect();
                    rows.sort_by(|a, b| b.2.cmp(&a.2));
                    let total: u64 = rows.iter().map(|r| r.2).sum();
                    let writes: u64 = rows.iter().map(|r| r.1).sum();
                    eprintln!(
                        "[ARM-CENSUS] signals={} writes={} block_arms={}",
                        rows.len(),
                        writes,
                        total
                    );
                    for (i, w, a) in rows.iter().take(25) {
                        eprintln!(
                            "[ARM-CENSUS]   arms={} writes={} fanout={} {}",
                            a,
                            w,
                            a / w.max(&1),
                            self.name_opt(*i).unwrap_or("?")
                        );
                    }
                }
                if self.armed_edge {
                    chatter!(
                        "[EVENT-EDGE] armed-fast-skips={} shadow-checks={}",
                        self.armed_fast_skips,
                        self.armed_shadow_checks
                    );
                }
            }
        }
        if self.prof_par_dispatch_partition + self.prof_par_dispatch_legacy > 0 {
            chatter!(
                "[PROF] par_dispatch partition={} legacy={} (partition k={})",
                self.prof_par_dispatch_partition,
                self.prof_par_dispatch_legacy,
                self.edge_block_partition_count
            );
        }
        let mut reasons: Vec<(&str, u64, u64)> = self
            .prof_fallback_by_reason
            .iter()
            .map(|(k, v)| (k.as_ref(), v.0, v.1))
            .collect();
        reasons.sort_by_key(|(_, _, ns)| std::cmp::Reverse(*ns));
        for (reason, count, ns) in reasons.iter().take(15) {
            chatter!(
                "[PROF] fallback_reason {:>30}: count={:>8} total={:>8.1}ms avg={:>7.1}µs",
                reason,
                count,
                *ns as f64 / 1e6,
                *ns as f64 / *count as f64 / 1e3
            );
        }
        chatter!(
            "[PROF] settle_dc={:.1}ms({}) settle_ca={:.1}ms({}) settle_ab={:.1}ms({})",
            self.prof_settle_dc_ns as f64 / 1e6,
            self.prof_settle_dc_count,
            self.prof_settle_ca_ns as f64 / 1e6,
            self.prof_settle_ca_count,
            self.prof_settle_ab_ns as f64 / 1e6,
            self.prof_settle_ab_count
        );
        chatter!(
            "[PROF] engine={}",
            if self.cycle_mode { "cycle" } else { "event" }
        );
        chatter!(
            "[PROF] tables: signals={} sig_to_edge_pos={} edge_signals={} armed_input_ranges={} dirty_signals={} gate_lane_ops={}",
            self.signal_table.len(),
            self.sig_to_edge_pos.len(),
            self.edge_signal_ids.len(),
            self.armed_input_ranges.len(),
            self.dirty_signals.len(),
            self.gate_ops.len()
        );
        chatter!("[PROF] vm_insns_total={}", self.prof_insns_executed);
        chatter!(
            "[PROF] write_observer: quiet={} observed={}",
            self.prof_quiet_stores,
            self.prof_observed_stores
        );
        #[cfg(feature = "jit")]
        if let Some(m) = self.ts_jit.as_ref() {
            chatter!(
                "[PROF] ts_jit: compiled={} rejected={} code_bytes={} pending={}",
                m.compiled,
                m.rejected,
                m.code_bytes,
                m.pending_len()
            );
        }
        chatter!(
            "[PROF] clock_tree: roots={} entries={} eager_evals={}",
            self.clock_tree_by_root.len(),
            self.is_clock_tree_entry.iter().filter(|&&b| b).count(),
            self.prof_clock_tree_evals
        );
        chatter!(
            "[PROF] settle_calls={} settle_iters={} max_iters={} entry_evals={} unresolved_entries={}/{}",
            self.settle_calls,
            self.settle_iters,
            self.max_settle_iters,
            self.entry_evals,
            unresolved,
            self.comb_entries.len()
        );
        let quiet_evals = self.entry_evals.saturating_sub(self.prof_settle_writes);
        chatter!(
            "[PROF] settle_writes={} ({:.1}% of evals changed a signal, {} quiet) dep_edges={} ({:.1} per write) repass_evals={}",
            self.prof_settle_writes,
            if self.entry_evals > 0 {
                100.0 * self.prof_settle_writes as f64 / self.entry_evals as f64
            } else {
                0.0
            },
            quiet_evals,
            self.prof_settle_dep_edges,
            if self.prof_settle_writes > 0 {
                self.prof_settle_dep_edges as f64 / self.prof_settle_writes as f64
            } else {
                0.0
            },
            self.prof_settle_repass_evals
        );
        chatter!(
            "[PROF] two_state_evals={} ({} entries lowered)",
            self.prof_ts_evals,
            self.ts_comb
                .iter()
                .filter(|s| matches!(s, TsSlot::Yes(_)))
                .count()
        );
        if self.prof_ts_bail_warnx
            + self.prof_ts_bail_forced
            + self.prof_ts_bail_xread
            + self.prof_ts_bail_abort
            > 0
        {
            chatter!(
                "[PROF] two_state_bail warn_x={} forced_write={} x_read={} run_abort={} x_plane_runs={}",
                self.prof_ts_bail_warnx,
                self.prof_ts_bail_forced,
                self.prof_ts_bail_xread,
                self.prof_ts_bail_abort,
                self.prof_tsx_evals
            );
        }
        if self.profile_report {
            let tally = self.prof_sampler.take().map(|s| s.finish());
            self.print_profile_report(t_nba, t_process, t_sched, tally.as_ref());
        }
        {
            const NAMES: [&str; 12] = [
                "noop",
                "fastcopy",
                "dircopy",
                "fastfanout",
                "busfanout",
                "andfanout",
                "gate",
                "udp",
                "contassign_c",
                "alwaysblk_c",
                "contassign_ast",
                "other",
            ];
            let mut hist = [0u64; 12];
            for (e, &c) in self.prof_entry_counts.iter().enumerate() {
                if c != 0 {
                    if let Some(ent) = self.comb_entries.get(e) {
                        hist[Self::entry_hist_kind(&ent.item)] += c;
                    }
                }
            }
            self.prof_entry_hist = hist;
            let tot: u64 = self.prof_entry_hist.iter().sum();
            if tot > 0 {
                let mut v: Vec<(usize, u64)> = self
                    .prof_entry_hist
                    .iter()
                    .copied()
                    .enumerate()
                    .filter(|(_, c)| *c > 0)
                    .collect();
                v.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
                let parts: Vec<String> = v
                    .iter()
                    .map(|(i, c)| {
                        format!(
                            "{}={} ({:.1}%)",
                            NAMES[*i],
                            c,
                            *c as f64 * 100.0 / tot as f64
                        )
                    })
                    .collect();
                eprintln!("[PROF] entry_hist {}", parts.join(" "));
            }
        }
        if !self.prof_entry_counts.is_empty() {
            let mut scatter_evals = 0u64;
            let mut scatter_lanes = 0u64;
            let mut scatter_entries = 0usize;
            for (eidx, ent) in self.comb_entries.iter().enumerate() {
                let lanes = match &ent.item {
                    CombItem::ScatterGate { op } => match op {
                        ScatterGate::Buf { dsts, .. } | ScatterGate::Bin { dsts, .. } => {
                            dsts.len() as u64
                        }
                    },
                    _ => continue,
                };
                let evals = self.prof_entry_counts.get(eidx).copied().unwrap_or(0);
                scatter_entries += 1;
                scatter_evals += evals;
                scatter_lanes += evals.saturating_mul(lanes);
            }
            if scatter_entries != 0 {
                eprintln!(
                    "[PROF] scatter entries={} evals={} lane_evals={}",
                    scatter_entries, scatter_evals, scatter_lanes
                );
            }
            // Attribute CompiledContAssign evaluations to RHS SHAPES, so
            // the fusion work targets what actually runs rather than what
            // is common in the netlist. Key = opcode sequence.
            let mut by_shape: std::collections::HashMap<String, (u64, usize)> =
                std::collections::HashMap::new();
            let mut cc_total = 0u64;
            for (eidx, ent) in self.comb_entries.iter().enumerate() {
                let n = self.prof_entry_counts.get(eidx).copied().unwrap_or(0);
                if n == 0 {
                    continue;
                }
                if let CombItem::CompiledContAssign { compiled } = &ent.item {
                    cc_total += n;
                    let sig = compiled
                        .instructions
                        .iter()
                        .map(super::bytecode::insn_opcode_name)
                        .collect::<Vec<_>>()
                        .join(",");
                    let e = by_shape.entry(sig).or_insert((0, 0));
                    e.0 += n;
                    e.1 += 1;
                }
            }
            // Aggregate OPCODE execution counts (eval-weighted, all
            // entries). Shapes say what is common; this says where the
            // interpreter actually spends its dispatches — the two rank
            // very differently, and only this one predicts a speedup.
            {
                let mut by_op: std::collections::HashMap<&'static str, u64> =
                    std::collections::HashMap::new();
                let mut total_insns = 0u64;
                for (eidx, ent) in self.comb_entries.iter().enumerate() {
                    let n = self.prof_entry_counts.get(eidx).copied().unwrap_or(0);
                    if n == 0 {
                        continue;
                    }
                    let blk = match &ent.item {
                        CombItem::CompiledContAssign { compiled } => Some(compiled),
                        CombItem::CompiledAlwaysBlock { compiled, .. } => Some(compiled),
                        _ => None,
                    };
                    if let Some(b) = blk {
                        for i in b.instructions.iter() {
                            *by_op
                                .entry(super::bytecode::insn_opcode_name(i))
                                .or_insert(0) += n;
                            total_insns += n;
                        }
                    }
                }
                let mut ov: Vec<(&str, u64)> = by_op.into_iter().collect();
                ov.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
                #[cfg(feature = "jit")]
                eprintln!(
                    "[PROF] comb_jit native={} xz_bail={}",
                    self.prof_comb_jit[0], self.prof_comb_jit[1]
                );
                if !self.cone_chain_member.is_empty() {
                    let mut chain_evals = 0u64;
                    let mut all_evals = 0u64;
                    for (i, &c) in self.prof_entry_counts.iter().enumerate() {
                        all_evals += c;
                        if self.cone_chain_member.get(i).copied().unwrap_or(false) {
                            chain_evals += c;
                        }
                    }
                    eprintln!(
                        "[CONE] eval-weighted: chain-member evals {}/{} ({:.1}%)",
                        chain_evals,
                        all_evals,
                        chain_evals as f64 * 100.0 / all_evals.max(1) as f64
                    );
                    // Does merging actually save work? A merged kernel fires when
                    // ANY member's inputs change and then recomputes every member.
                    // Its fire count is at least max(member evals), so best-case
                    // work is max*len against sum(evals) unmerged. Ratio 1.0 means
                    // members always co-fire (pure win); >1.0 is recomputation the
                    // saved dispatch has to pay for. Dispatches saved is
                    // sum(evals) - max(evals) per chain, i.e. what merging removes
                    // from the worklist even in that best case.
                    let mut before = 0u64; // sum of member evals
                    let mut after = 0u64; // best-case merged work = max*len
                    let mut fires = 0u64; // best-case merged dispatch count
                    let mut skew_hist = [0usize; 5]; // ratio <1.1,<1.5,<2,<4,>=4
                    let mut sel_before = [0u64; 5];
                    let mut sel_after = [0u64; 5];
                    let mut sel_fires = [0u64; 5];
                    for ch in self.cone_chains.iter() {
                        let evals: Vec<u64> = ch
                            .iter()
                            .map(|&e| self.prof_entry_counts.get(e as usize).copied().unwrap_or(0))
                            .collect();
                        let s: u64 = evals.iter().sum();
                        let m: u64 = evals.iter().copied().max().unwrap_or(0);
                        if s == 0 {
                            continue;
                        }
                        let w = m * ch.len() as u64;
                        before += s;
                        after += w;
                        fires += m;
                        let r = w as f64 / s as f64;
                        let b = if r < 1.1 {
                            0
                        } else if r < 1.5 {
                            1
                        } else if r < 2.0 {
                            2
                        } else if r < 4.0 {
                            3
                        } else {
                            4
                        };
                        skew_hist[b] += 1;
                        sel_before[b] += s;
                        sel_after[b] += w;
                        sel_fires[b] += m;
                    }
                    // Selective merge: only fold chains whose members co-fire, so
                    // the recomputation stays bounded. Report the running totals by
                    // threshold to show where the knee is.
                    let names = ["<1.1x", "<1.5x", "<2x", "<4x", "all"];
                    let (mut cb, mut ca, mut cf) = (0u64, 0u64, 0u64);
                    for b in 0..5 {
                        cb += sel_before[b];
                        ca += sel_after[b];
                        cf += sel_fires[b];
                        eprintln!(
                            "[CONE] selective merge thresh={:>5}: dispatches removed {} ({:.1}% of all evals), \
                         extra compute {} ({:+.1}% of all evals)",
                            names[b],
                            cb.saturating_sub(cf),
                            cb.saturating_sub(cf) as f64 * 100.0 / all_evals.max(1) as f64,
                            ca.saturating_sub(cb),
                            ca.saturating_sub(cb) as f64 * 100.0 / all_evals.max(1) as f64,
                        );
                    }
                    eprintln!(
                        "[CONE] merge economics: evals before={} best-case-after={} ({:+.1}% compute), \
                     dispatches {}->{} ({:.1}% removed)",
                        before,
                        after,
                        (after as f64 - before as f64) * 100.0 / before.max(1) as f64,
                        before,
                        fires,
                        (before.saturating_sub(fires)) as f64 * 100.0 / before.max(1) as f64
                    );
                    eprintln!(
                        "[CONE] per-chain recompute ratio (max*len/sum): <1.1x={} <1.5x={} <2x={} <4x={} >=4x={}",
                        skew_hist[0], skew_hist[1], skew_hist[2], skew_hist[3], skew_hist[4]
                    );
                }
                eprintln!(
                    "[PROF] opcode_exec total={} (static-estimate, straight-line)",
                    total_insns
                );
                let line: Vec<String> = ov
                    .iter()
                    .take(14)
                    .map(|(k, c)| {
                        format!(
                            "{}={:.1}%",
                            k,
                            *c as f64 * 100.0 / total_insns.max(1) as f64
                        )
                    })
                    .collect();
                eprintln!("[PROF]   {}", line.join(" "));
            }
            let mut v: Vec<(String, (u64, usize))> = by_shape.into_iter().collect();
            v.sort_by_key(|(_, (c, _))| std::cmp::Reverse(*c));
            eprintln!(
                "[PROF] contassign_shapes total_evals={} distinct={}",
                cc_total,
                v.len()
            );
            for (sig, (c, ents)) in v.iter().take(15) {
                eprintln!(
                    "[PROF]   {:>6.2}% n={} entries={} :: {}",
                    *c as f64 * 100.0 / cc_total.max(1) as f64,
                    c,
                    ents,
                    sig
                );
            }
        }
        chatter!(
            "[PROF] simulation_loop={:.1}ms iters={} avg={:.2}µs/iter",
            sim_elapsed.as_secs_f64() * 1000.0,
            self.loop_iters,
            sim_elapsed.as_secs_f64() * 1e6 / self.loop_iters.max(1) as f64
        );
        if let Some((done, handle)) = _sampler_thr {
            done.store(true, Ordering::Relaxed);
            if let Ok(thr) = handle {
                let _ = thr.join();
            }
            sampler_report(200);
        }
        if std::env::var_os("XEZIM_XZ_STATS").is_some() {
            // Sparse-X/Z opportunity: how many signals carry X/Z at sim end
            // (steady-state proxy). If X/Z is rare, a sparse X/Z store + 2-state
            // fast value path would shrink the dominant 4-state storage traffic.
            let n = self.signal_table.len();
            let mut xz = 0usize;
            let mut xz_scalar = 0usize;
            let named = self.id_to_name.legacy_len();
            for i in 0..n {
                if self.signal_table[i].has_xz() {
                    xz += 1;
                    if i < named {
                        xz_scalar += 1;
                    }
                }
            }
            eprintln!(
                "[XZ-STATS] signals with X/Z at end: {}/{} ({:.3}%); of {} named: {} ({:.3}%); \
                 → {:.1}% are 2-state (sparse-X/Z candidates)",
                xz,
                n,
                100.0 * xz as f64 / n.max(1) as f64,
                named,
                xz_scalar,
                100.0 * xz_scalar as f64 / named.max(1) as f64,
                100.0 * (n - xz) as f64 / n.max(1) as f64
            );
        }
        if self.bsp_widths_on {
            let b = &self.bsp_width_buckets;
            let tot: u64 = b.iter().sum();
            let pct = |x: u64| 100.0 * x as f64 / tot.max(1) as f64;
            eprintln!(
                "[BSP-WIDTHS] steady-state entry-evals by level width: \
                 w=1:{} ({:.0}%) 2-15:{} ({:.0}%) 16-63:{} ({:.0}%) 64-255:{} ({:.0}%) \
                 256-1023:{} ({:.0}%) 1024+:{} ({:.0}%); parallelizable(>=64)={:.0}%",
                b[0],
                pct(b[0]),
                b[1],
                pct(b[1]),
                b[2],
                pct(b[2]),
                b[3],
                pct(b[3]),
                b[4],
                pct(b[4]),
                b[5],
                pct(b[5]),
                pct(b[3] + b[4] + b[5])
            );
        }
        if self.clktree_probe {
            eprintln!(
                "[CLKTREE-PROBE] groups={} memo-eligible visits={} MISMATCHES={} \
                 (dedup is sound iff 0)",
                self.edge_group_count, self.clktree_probe_hits, self.clktree_probe_mismatch
            );
            for &sid in &self.clktree_probe_sids {
                eprintln!(
                    "[CLKTREE-PROBE]   mismatching signal: {}",
                    self.name_for_id(sid)
                );
            }
        }
        if self.edge_scan_stats {
            let s = self.edge_scan_scanned;
            let c = self.edge_scan_changed;
            eprintln!(
                "[EDGE-SCAN] full-scan positions scanned={} changed(fired)={} ({:.1}%); \
                 dirty-driven ceiling = skip {:.1}% of detect work",
                s,
                c,
                100.0 * c as f64 / s.max(1) as f64,
                100.0 * (s - c) as f64 / s.max(1) as f64
            );
        }
        if std::env::var("XEZIM_NAME_STATS").is_ok() {
            let n = &self.name_stats;
            eprintln!(
                "[NAME-STATS] get_signal_by_name={} resolve_hier_name={} frames_pushed={} class_prop_reads={} (mean {:.2}/resolve) frame_gets={}",
                n[0].get(),
                n[1].get(),
                n[2].get(),
                n[3].get(),
                n[3].get() as f64 / n[1].get().max(1) as f64,
                n[4].get()
            );
        }
        // §16.12.2: strong properties still pending fail at the end of
        // simulation, before the summary counts them.
        self.sva_end_of_sim();
        // Assertion + coverage summary (always when any data was recorded;
        // dump the JSON database iff XEZIM_COV_DB=<path> is set, else default
        // path xezim_cov.json when at least one stat exists). A run with code
        // coverage writes it after the `final` blocks, which count too.
        if self.code_cov.is_none() {
            self.emit_coverage_summary();
        }
        if self.edge_block_stats_enabled {
            self.print_edge_block_stats();
        }

        // Activity monitor report
        if self.activity_mon {
            self.print_activity_report();
        }
    }
}
