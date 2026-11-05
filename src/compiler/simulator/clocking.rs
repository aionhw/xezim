//! Simulator clocking.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

impl Simulator {
    /// §14.16: commit a clocking drive of a SELECT of an output clockvar
    /// (`cb.v[2] <= 1`, `cb.u[1] <= 1`) after `skew` ticks, as a nonblocking
    /// update of just the selected bits or element. `target` is the net with
    /// the drive's indices frozen when it executed.
    pub(super) fn drive_clocking_select(&mut self, target: &Expression, val: Value, skew: u64) {
        let span = target.span;
        let rvalue = self.make_intra_saved_expr(val, span);
        let delay = (skew > 0).then(|| {
            Expression::new(
                ExprKind::Number(NumberLiteral::Integer {
                    size: Some(64),
                    signed: false,
                    base: NumberBase::Decimal,
                    value: skew.to_string(),
                    cached_val: crate::ast::cache::LiteralCache::new(None),
                }),
                span,
            )
        });
        self.exec_statement(&Statement::new(
            StatementKind::NonblockingAssign {
                lvalue: target.clone(),
                delay,
                rvalue,
            },
            span,
        ));
    }

    /// §14.3: the element signals (`net[i]`, with `i`) of clockvar net `net`
    /// when it is a one-dimensional unpacked array, which has no signal under
    /// its own name; `None` for any other net.
    pub(super) fn clocking_array_elems(&self, net: &str) -> Option<Vec<(i64, String)>> {
        if self.signal_name_to_id.contains_key(net) {
            return None;
        }
        let &(lo, hi, _) = self.module.arrays.get(net)?;
        Some((lo..=hi).map(|i| (i, format!("{}[{}]", net, i))).collect())
    }

    /// LRM §14.3 clocking-block input snapshot. Each registered cb
    /// has a clock signal; on every posedge transition of that clock
    /// we snapshot the current values of input-direction signals. The
    /// snapshot is the value `cb.<sig>` returns for reads until the
    /// next posedge updates it — this is the `#1step` input-skew
    /// behavior (IEEE 1800-2023 §14.13 / §14.16) that lets clocking block
    /// monitors sample stable pre-edge state.
    /// §14.4: the per-signal output skew of the clockvar bound to `net`.
    /// The skew map is keyed by the clockvar's own name; a renamed clockvar
    /// (`output #2 q = q_drv;`) queues its drive under the bound net, so the
    /// alias map is consulted the other way round (#824).
    fn clocking_net_out_skew(&self, cb: &str, net: &str) -> Option<u64> {
        let per_sig = self.clocking_sig_out_skew.get(cb)?;
        if let Some(&s) = per_sig.get(net) {
            return Some(s);
        }
        let aliases = self.clocking_sig_alias.get(cb)?;
        aliases
            .iter()
            .find(|(_, bound)| bound.as_str() == net)
            .and_then(|(vis, _)| per_sig.get(vis).copied())
    }

    /// §14.5: `sel` (`bus[3:0]`, `mem[2][7:4]`) applied to `base`, the
    /// sampled value of the net under the selects: the base is swapped for
    /// the saved value and the select chain evaluated as usual.
    fn apply_bound_select(&mut self, sel: &Expression, base: Value) -> Value {
        fn swap_base(e: &mut Expression, saved: Expression) {
            match &mut e.kind {
                ExprKind::Index { expr, .. } | ExprKind::RangeSelect { expr, .. }
                    if Simulator::select_chain_base(expr).is_some() =>
                {
                    swap_base(expr, saved)
                }
                ExprKind::Index { expr, .. } | ExprKind::RangeSelect { expr, .. } => {
                    **expr = saved;
                }
                _ => *e = saved,
            }
        }
        let saved = self.make_intra_saved_expr(base, sel.span);
        let mut e = sel.clone();
        swap_base(&mut e, saved);
        self.eval_expr(&e)
    }

    pub(super) fn tick_clocking_blocks(&mut self) {
        if self.clocking_meta.is_empty() {
            return;
        }
        // This runs after EVERY tick. The poll table resolves each block's
        // clock to a signal-table id once and keeps the previous level, so a
        // quiet tick costs one indexed read per block instead of cloning the
        // block's signal list and resolving its clock by name. Rebuilt when a
        // block is registered (the map only ever grows); iteration order is
        // the map's, exactly as before.
        if self.clocking_tick_cache.len() != self.clocking_meta.len() {
            let names: Vec<String> = self.clocking_meta.keys().cloned().collect();
            self.clocking_tick_cache = names
                .into_iter()
                .map(|cb| {
                    let clk = self.clocking_meta[&cb].0.clone();
                    let id = self.signal_name_to_id.get(clk.as_str()).copied();
                    let edge = self
                        .clocking_edge
                        .get(&cb)
                        .copied()
                        .unwrap_or(EdgeKind::Posedge);
                    let prev = self.clocking_prev_clock.get(&cb).copied().unwrap_or(2);
                    ClockingTick {
                        cb,
                        clk,
                        id,
                        edge,
                        prev,
                    }
                })
                .collect();
        }
        let mut cache = std::mem::take(&mut self.clocking_tick_cache);
        for ent in cache.iter_mut() {
            let cur = match ent.id {
                Some(id) => self.signal_table[id]
                    .to_u64()
                    .map(|u| (u & 1) as u8)
                    .unwrap_or(2),
                None => self
                    .get_signal_value_by_name(&ent.clk)
                    .and_then(|v| v.to_u64())
                    .map(|u| (u & 1) as u8)
                    .unwrap_or(2),
            };
            let prev = ent.prev;
            ent.prev = cur;
            // §14.3: advance on the block's declared clock edge (posedge by
            // default; negedge / any-edge when so declared).
            let fired = match ent.edge {
                EdgeKind::Negedge => prev == 1 && cur == 0,
                EdgeKind::Posedge => prev == 0 && cur == 1,
                // `edge` / any: any resolved 0↔1 transition.
                _ => prev != cur && prev != 2 && cur != 2,
            };
            if !fired {
                continue;
            }
            let cb = ent.cb.clone();
            let sigs: Vec<(String, bool)> = self
                .clocking_meta
                .get(&cb)
                .map(|(_, s)| s.clone())
                .unwrap_or_default();
            // §14.16.1: record the edge time — a drive executed AT this time
            // matures at edge + output skew instead of waiting a full cycle.
            self.clocking_last_edge.insert(cb.clone(), self.time);
            // §14.16 `##N` drives: one clocking event elapsed — mature any
            // whose countdown hits zero (honoring output skew), keep the rest.
            if let Some(mut pend) = self.clocking_cycle_pending.remove(&cb) {
                let dskew = self.clocking_out_skew.get(&cb).copied().unwrap_or(0);
                let mut keep: Vec<(u64, String, Value, Option<Expression>)> = Vec::new();
                for (n, net, val, target) in pend.drain(..) {
                    if n <= 1 {
                        let skew = self.clocking_net_out_skew(&cb, &net).unwrap_or(dskew);
                        if let Some(t) = target {
                            self.drive_clocking_select(&t, val, skew);
                        } else if let Some(&id) = self.signal_name_to_id.get(net.as_str()) {
                            let w = self.signal_widths[id];
                            self.schedule_delayed_with_delay(id, val.resize(w), skew);
                        }
                    } else {
                        keep.push((n - 1, net, val, target));
                    }
                }
                if !keep.is_empty() {
                    self.clocking_cycle_pending.insert(cb.clone(), keep);
                }
            }
            // LRM §14.4 output skew: apply any pending output drives
            // queued by `cb.<out_sig> <= val` since the last edge.
            // Route through `assign_value` (with a synthesised Ident
            // lvalue) so width/signedness handling matches a normal
            // procedural assignment.
            // NOTE: Drive outputs BEFORE sampling inputs so that @(cb) waiters
            // see the new output values when they run in the Reactive region.
            if let Some(pending) = self.clocking_output_pending.remove(&cb) {
                let dskew = self.clocking_out_skew.get(&cb).copied().unwrap_or(0);
                for (sig, val, target) in pending {
                    // §14.4 output skew for drives maturing at THIS edge.
                    let skew = self.clocking_net_out_skew(&cb, &sig).unwrap_or(dskew);
                    if let Some(t) = target {
                        if skew > 0 {
                            self.drive_clocking_select(&t, val, skew);
                        } else {
                            self.assign_value(&t, &val);
                        }
                        continue;
                    }
                    if skew > 0 {
                        if let Some(&id) = self.signal_name_to_id.get(sig.as_str()) {
                            let w = self.signal_widths[id];
                            self.schedule_delayed_with_delay(id, val.resize(w), skew);
                            continue;
                        }
                    }
                    let lval = Expression::new(
                        ExprKind::Ident(crate::ast::expr::HierarchicalIdentifier {
                            root: None,
                            path: vec![crate::ast::expr::HierPathSegment {
                                name: crate::ast::Identifier {
                                    name: sig.clone(),
                                    span: crate::ast::Span::dummy(),
                                },
                                selects: Vec::new(),
                            }],
                            span: crate::ast::Span::dummy(),
                            cached_signal_id: crate::ast::cache::SignalCache::new(None),
                            cached_resolved_name: crate::ast::cache::NameCache::new(),
                        }),
                        crate::ast::Span::dummy(),
                    );
                    self.assign_value(&lval, &val);
                }
            }
            // Refresh the input samples. IEEE 1800-2023 §14.4 input skew:
            // - `#1step` (also the §14.3 default): the Preponed value, before
            //   this edge's updates;
            // - `#0`: the Observed-region value, after this edge's NBAs;
            // - `#N`: the value N time units before the edge (`ClockingHist`).
            // A per-signal skew overrides the block default. The `##N` /
            // default-clocking aliases share the named block's skews.
            let skey: String = if cb.starts_with("__xz_default_clocking") {
                self.default_clocking_cb
                    .clone()
                    .unwrap_or_else(|| cb.clone())
            } else {
                cb.clone()
            };
            let skey = skey.as_str();
            let in_dskew = self
                .clocking_in_skew
                .get(skey)
                .copied()
                .unwrap_or(CB_SKEW_1STEP);
            let vis_list: Vec<String> = self.clocking_vis.get(skey).cloned().unwrap_or_default();
            let mut snap = HashMap::default();
            for (i, (sig, is_input)) in sigs.iter().enumerate() {
                if *is_input {
                    let vis = vis_list.get(i).unwrap_or(sig);
                    let skew = self
                        .clocking_sig_in_skew
                        .get(skey)
                        .and_then(|m| m.get(vis))
                        .copied()
                        .unwrap_or(in_dskew);
                    // §14.3: an unpacked-array clockvar samples each element,
                    // published as `vis[i]` for `cb.sig[i]` reads.
                    if let Some(elems) = self.clocking_array_elems(sig) {
                        let leaf = vis.rsplit('.').next().unwrap_or(vis);
                        if !self.clocking_array_leaves.contains(leaf) {
                            self.clocking_array_leaves.insert(leaf.to_string());
                        }
                        for (idx, e) in elems {
                            let v = if skew == CB_SKEW_1STEP {
                                self.clocking_preponed.get(&e).cloned()
                            } else if skew == 0 {
                                None
                            } else {
                                self.clocking_hist_value(&e, self.time.saturating_sub(skew))
                            }
                            .or_else(|| self.get_signal_value_by_name(&e));
                            if let Some(v) = v {
                                snap.insert(format!("{}[{}]", vis, idx), v);
                            }
                        }
                        continue;
                    }
                    let v = if skew == CB_SKEW_1STEP {
                        self.clocking_preponed
                            .get(sig)
                            .cloned()
                            .or_else(|| self.get_signal_value_by_name(sig))
                    } else if skew == 0 {
                        self.get_signal_value_by_name(sig)
                    } else {
                        self.clocking_hist_value(sig, self.time.saturating_sub(skew))
                            .or_else(|| self.get_signal_value_by_name(sig))
                    };
                    // §14.5: a clockvar bound to a select samples that select
                    // of the base net's sample (#823).
                    let bound_sel = self
                        .clocking_sig_bound_sel
                        .get(skey)
                        .and_then(|m| m.get(vis))
                        .cloned();
                    let v = match (v, bound_sel) {
                        (Some(v), Some(sel)) => Some(self.apply_bound_select(&sel, v)),
                        (v, _) => v,
                    };
                    if let Some(v) = v {
                        snap.insert(vis.clone(), v);
                    }
                }
            }
            // §14.13: publish each fresh sample to its hidden mirror signal so
            // `@(cb.sig)` fires at THIS edge, and only on an actual change.
            let mirror: Vec<(String, Value)> = snap
                .iter()
                .map(|(vis, v)| (vis.clone(), v.clone()))
                .collect();
            for (vis, v) in mirror {
                let hidden = format!("{}{}.{}", CB_SAMPLE_PREFIX, cb, vis);
                if let Some(&id) = self.signal_name_to_id.get(hidden.as_str()) {
                    let w = self.signal_widths[id];
                    let nv = v.resize(w);
                    if self.signal_table[id] != nv {
                        write_sig!(self, id, nv);
                        self.mark_dirty_id(id);
                        self.table_modified = true;
                        self.clocking_mirror_dirty = true;
                    }
                }
            }
            self.clocking_snapshots.insert(cb.clone(), snap);
        }
        self.clocking_tick_cache = cache;
    }

    /// §9.4.2.3/§14.13: judge the `iff` guards of the clocking-event waiters
    /// whose edge fired this slot (held by `drain_triggered_event_waiters`).
    /// The clocking event occurs after the block samples, so the guard reads
    /// this edge's clockvars (and post-NBA design state). A true guard
    /// resumes the waiter with the other clocking continuations; a false one
    /// re-arms it for the block's next clocking event.
    #[cold]
    #[inline(never)]
    pub(super) fn resolve_deferred_clocking_guards(&mut self) {
        let pending = std::mem::take(&mut self.deferred_clocking_guarded);
        for (mut waiter, g) in pending {
            // A prior guard can call a function that kills another waiter
            // already extracted into this batch. Do not evaluate or re-arm it.
            if self.killed_pids.contains(&waiter.pid) {
                continue;
            }
            let saved_hint = waiter
                .value_ctx
                .as_ref()
                .map(|c| self.name_resolve_hint.replace(c.scope.clone()));
            let ok = if !self.process_contexts.contains_key(&waiter.pid)
                && self.process_context_is_default()
            {
                self.eval_expr(&g).is_true()
            } else {
                self.eval_waiter_condition(waiter.pid, &g)
            };
            if self.killed_pids.contains(&waiter.pid) {
                if let Some(h) = saved_hint {
                    *self.name_resolve_hint.borrow_mut() = h;
                }
                continue;
            }
            if ok && waiter.remaining_events <= 1 {
                let cont = std::mem::replace(&mut waiter.continuation, ProcCont::empty());
                self.deferred_clocking_conts.push((waiter.pid, cont));
            } else {
                if ok {
                    waiter.remaining_events -= 1;
                }
                // Re-arm from the current levels: the edge just consumed
                // must not fire again.
                for (i, sid) in waiter.resolved_sensitivities.iter().enumerate() {
                    waiter.captured_prev[i] = self.term_raw_bits(sid.signal_id);
                    if let Some(v) = self.term_wide_value(sid.signal_id) {
                        waiter.captured_prev_wide[i] = Some(v);
                    }
                }
                if waiter.value_ctx.is_some() {
                    self.rebase_value_terms(&mut waiter);
                }
                self.event_waiters.push(waiter);
            }
            if let Some(h) = saved_hint {
                *self.name_resolve_hint.borrow_mut() = h;
            }
        }
    }

    /// The guard of term `ix` decides whether this clocking edge counts: the
    /// waiter (with its continuation and event count) moves aside until
    /// `resolve_deferred_clocking_guards` judges it.
    #[cold]
    #[inline(never)]
    pub(super) fn hold_clocking_guard(&mut self, waiter: &mut EventWaiter, ix: usize) {
        let Some(g) = waiter.resolved_sensitivities[ix].iff.clone() else {
            return;
        };
        let cont = std::mem::replace(&mut waiter.continuation, ProcCont::empty());
        let mut held = waiter.clone();
        held.continuation = cont;
        self.deferred_clocking_guarded.push((held, g));
    }

    /// A clocking-event wait reached after its block's edge in the same
    /// slot (`clocking_event_due_now`): resume with this slot's clocking
    /// waiters, judging an `iff` guard after the block samples.
    pub(super) fn defer_due_clocking_wait(
        &mut self,
        pid: usize,
        terms: Vec<SensitivityId>,
        cont: ProcCont,
    ) {
        if terms.iter().any(|t| t.iff.is_some()) {
            let w = self.make_event_waiter_resolved(pid, terms, cont, true);
            self.park_clocking_guard(w);
        } else {
            self.deferred_clocking_conts.push((pid, cont));
        }
    }

    /// Hold `w` for `resolve_deferred_clocking_guards` under its first
    /// term's `iff` guard.
    #[cold]
    pub(super) fn park_clocking_guard(&mut self, w: EventWaiter) {
        match w.resolved_sensitivities.iter().find_map(|t| t.iff.clone()) {
            Some(g) => self.deferred_clocking_guarded.push((w, g)),
            None => self.event_waiters.push(w),
        }
    }

    /// Resume the clocking-event waiters (`@(cb)` / `##N`) that fired this slot,
    /// held aside by `drain_triggered_event_waiters` and run here in the
    /// Reactive region — after `apply_nba` + `tick_clocking_blocks` — so their
    /// continuations read post-edge design state and this cycle's `cb.<in>`
    /// samples (§14.13). Mirrors the Active-region waiter run loop.
    pub(super) fn drain_deferred_clocking_conts(&mut self) {
        if !self.deferred_clocking_guarded.is_empty() {
            self.resolve_deferred_clocking_guards();
        }
        let mut guard = 0u32;
        let mut ran_any = false;
        while !self.deferred_clocking_conts.is_empty() {
            let conts = std::mem::take(&mut self.deferred_clocking_conts);
            self.late_resume_epoch += conts.len() as u64;
            for (pid, stmts) in conts {
                if self.finished {
                    break;
                }
                // A prior continuation may kill a sibling in this batch.
                // Dispatch already rejects killed pids; skip completion too.
                if self.killed_pids.contains(&pid) {
                    continue;
                }
                ran_any = true;
                self.run_scheduled_process(pid, &stmts);
                if !self.killed_pids.contains(&pid) && !self.is_pid_suspended(pid) {
                    self.child_finished(pid);
                }
                self.break_flag = false;
                self.continue_flag = false;
                self.return_flag = false;
            }
            guard += 1;
            if self.finished || guard > 10_000 {
                break;
            }
        }
        // These continuations run after the tick's edge pass. A named event
        // or signal they write (`##2; -> ev;`) had no pass left to wake its
        // `@(ev)` / `@(sig)` waiters, which then resumed one tick late
        // (a clocking-synchronised trigger was seen a clock toggle after
        // it fired). Give their writes the same edge pass the clocking
        // mirror gets above.
        if ran_any && !self.finished {
            self.settle_combinatorial();
            let saved = self.edge_dispatch_phase;
            self.edge_dispatch_phase = "clocking-conts";
            self.check_edges();
            self.edge_dispatch_phase = saved;
            let limit = self.cascade_limit;
            let _ = self.drain_edge_cascade(limit);
        }
    }

    /// §16.4: record a deferred assertion's outcome; its action block runs
    /// when the report matures. Arguments of a subroutine-call action are
    /// evaluated now, as §16.4 requires.
    pub(super) fn queue_deferred_assert(
        &mut self,
        a: &crate::ast::stmt::AssertionStatement,
        kind: crate::ast::stmt::DeferredAssertion,
        passed: bool,
        label: Option<&str>,
    ) {
        let kind_tag: u8 = match a.kind {
            crate::ast::stmt::AssertionKind::Assert => 0,
            crate::ast::stmt::AssertionKind::Assume => 1,
            crate::ast::stmt::AssertionKind::Cover => 2,
        };
        let src = self.current_src_file();
        let stat_key = Self::assert_stat_key(a.span.start, src);
        self.note_assert_label(stat_key, label);
        let entry = self
            .assertion_stats
            .entry(stat_key)
            .or_insert(AssertionStat {
                kind: kind_tag,
                loc: Some((src, a.span.start)),
                pass_count: 0,
                fail_count: 0,
            });
        if passed {
            entry.pass_count += 1;
        } else {
            entry.fail_count += 1;
        }
        // An `always_comb` / `always @*` the settle runs as a comb entry is
        // not the process that happens to be current (whose resumption, a
        // §16.4.2 flush point, would drop the report): its reports belong to
        // no process, keyed by the block's scope instead.
        let (pid, comb_scope) = if self.settling {
            (usize::MAX, self.name_resolve_hint.borrow().clone())
        } else {
            (self.current_pid, None)
        };
        // A re-executed assertion of the same activation (an always_comb
        // evaluated twice in one slot) supersedes its earlier report.
        self.deferred_asserts
            .retain(|d| !(d.pid == pid && d.span == a.span && d.comb_scope == comb_scope));
        let action = if passed { &a.action } else { &a.else_action };
        let action = action.as_deref().map(|st| self.capture_deferred_action(st));
        // §16.3: a failure without an `else` clause calls `$error` (a cover
        // has no failure action).
        let default_error_scope =
            (action.is_none() && !passed && kind_tag != 2).then(|| self.m_path());
        if action.is_none() && default_error_scope.is_none() {
            return;
        }
        let ctl = self.imm_assert_ident(a);
        self.deferred_asserts.push(DeferredReport {
            pid,
            kind,
            span: a.span,
            passed,
            action,
            default_error_scope,
            comb_scope,
            this_handle: self.this_stack.last().copied().flatten(),
            class_context: self.class_context_stack.last().cloned().flatten(),
            ctl,
        });
    }

    /// A deferred action with the arguments of its subroutine call fixed at
    /// their current values (literals stay, so a format string still reads
    /// as one).
    pub(super) fn capture_deferred_action(&mut self, st: &Statement) -> Statement {
        let StatementKind::Expr(e) = &st.kind else {
            return st.clone();
        };
        let fix = |sim: &mut Self, args: &[Expression]| -> Vec<Expression> {
            args.iter()
                .map(|a| match &a.kind {
                    ExprKind::StringLiteral(_) | ExprKind::Number(_) => a.clone(),
                    _ => {
                        let v = sim.eval_expr(a);
                        sim.make_intra_saved_expr(v, a.span)
                    }
                })
                .collect()
        };
        let kind = match &e.kind {
            ExprKind::SystemCall { name, args } => ExprKind::SystemCall {
                name: name.clone(),
                args: fix(self, args),
            },
            ExprKind::Call { func, args } => ExprKind::Call {
                func: func.clone(),
                args: fix(self, args),
            },
            _ => return st.clone(),
        };
        Statement::new(StatementKind::Expr(Expression::new(kind, e.span)), st.span)
    }

    /// §16.4.2 flush point: process `pid` resumed, dropping its pending
    /// deferred reports.
    #[inline]
    pub(super) fn flush_deferred_asserts(&mut self, pid: usize) {
        if !self.deferred_asserts.is_empty() {
            self.deferred_asserts.retain(|d| d.pid != pid);
        }
        // §16.14.6.2: a flush point drops the process's pending procedural
        // concurrent assertion instances too.
        if !self.sva_proc_pending.is_empty() {
            self.sva_proc_pending.retain(|p| p.1 != pid);
        }
    }

    /// Mature the pending deferred reports at the end of the time slot:
    /// `#0` ones (Observed region) first, then `final` ones (Postponed).
    /// After `$finish` the action blocks do not run; a failure is noted.
    pub(super) fn mature_deferred_asserts(&mut self) {
        let mut pending = std::mem::take(&mut self.deferred_asserts);
        pending.sort_by_key(|d| d.kind == crate::ast::stmt::DeferredAssertion::Final);
        for d in pending {
            if self.finished {
                if !d.passed {
                    let loc = self
                        .span_file_line_in(d.span, None)
                        .unwrap_or_else(|| "?".to_string());
                    eprintln!(
                        "[xezim] note: deferred assertion at {} failed at time {}; \
                         its action block did not run before $finish",
                        loc, self.time
                    );
                }
                continue;
            }
            // §20.12 PassOff/FailOff as the report matures.
            if self.assert_ctl_on {
                let st = self.assert_ctl_state(&d.ctl);
                if !(if d.passed {
                    st.pass_action(false)
                } else {
                    st.fail_action()
                }) {
                    continue;
                }
            }
            let Some(action) = d.action else {
                if let Some(scope) = d.default_error_scope {
                    self.emit_severity_text_in("Error", ASSERT_DEFAULT_ERROR, &scope);
                }
                continue;
            };
            let saved_pid = self.current_pid;
            self.current_pid = d.pid;
            self.this_stack.push(d.this_handle);
            self.class_context_stack.push(d.class_context);
            self.exec_statement(&action);
            self.class_context_stack.pop();
            self.this_stack.pop();
            self.current_pid = saved_pid;
        }
    }

    pub(super) fn drain_observed_region(&mut self) {
        if self.pending_observed.is_empty() {
            return;
        }
        let probes = std::mem::take(&mut self.pending_observed);
        for probe in probes {
            // Push `this` so member references in the predicate resolve
            // against the property's owning class instance (if any).
            self.this_stack.push(probe.this_handle);
            let outcome = self.eval_expr(&probe.predicate).is_true();
            self.this_stack.pop();
            // Fold into assertion_stats so the coverage DB reflects
            // concurrent-property hits without a separate channel, on the
            // site's own entry: it was keyed by a hash of a synthetic name,
            // tagged `cover` whatever its kind, and had no location.
            let stat = self
                .assertion_stats
                .entry(probe.stat_key)
                .or_insert_with(|| AssertionStat {
                    kind: probe.kind,
                    loc: Some(probe.loc),
                    pass_count: 0,
                    fail_count: 0,
                });
            if outcome {
                stat.pass_count += 1;
            } else {
                stat.fail_count += 1;
            }
        }
    }

    /// A `$monitor`/`$fmonitor` argument that must NOT drive re-printing:
    /// `$time`, `$stime`, `$realtime` change every step (LRM §21.2.3).
    pub(super) fn is_monitor_time_arg(e: &Expression) -> bool {
        matches!(&e.kind, ExprKind::SystemCall { name, .. }
            if matches!(name.as_str(), "$time" | "$stime" | "$realtime"))
    }

    /// The design root is the synthetic multi-top wrapper (§23.3.3): its
    /// children are the real tops, and it never appears in a printed path.
    pub fn root_is_multi_top(&self) -> bool {
        self.module.name == xezim_core::MULTI_TOP_WRAPPER
    }

    /// Printed hierarchical path of an instance scope in signal-key form
    /// (`u_a.u_b`; empty = the root): `<top>.u_a.u_b`. Under the multi-top
    /// wrapper the scope already starts with its own top.
    pub fn hier_path(&self, scope: &str) -> String {
        if scope.is_empty() {
            self.module.name.clone()
        } else if self.root_is_multi_top() {
            scope.to_string()
        } else {
            format!("{}.{}", self.module.name, scope)
        }
    }

    /// The instance scope currently executing — the same precedence `%m` uses:
    /// a sensitivity-driven block records `m_block_scope`, a procedural process
    /// records `current_scope`.
    /// Strip the top module's name from a `%m`-form scope (`tb.u_w` ->
    /// `u_w`; `tb` -> ``), giving the instance path signal keys use.
    pub(super) fn instance_relative_scope(&self, scope: &str) -> String {
        let top = self.module.name.as_str();
        if scope == top {
            return String::new();
        }
        match scope.strip_prefix(top) {
            Some(rest) if rest.starts_with('.') => rest[1..].to_string(),
            _ => scope.to_string(),
        }
    }

    /// The module a class (or an ancestor of it) is declared inside, if any.
    pub(super) fn class_enclosing_module(&self, handle: usize) -> Option<String> {
        let class_name = self
            .heap
            .get(handle)
            .and_then(|o| o.as_ref())
            .map(|i| i.class_name.as_str())?;
        if let Some(hit) = self.class_enclosing_cache.borrow().get(class_name) {
            return hit.clone();
        }
        let v = self.class_enclosing_module_walk(class_name);
        self.class_enclosing_cache
            .borrow_mut()
            .insert(class_name.to_string(), v.clone());
        v
    }

    /// §8/§23.9: the instance whose class an object of a class declared
    /// inside a module (not the top) belongs to — the instance holding the
    /// object's creation scope, or the module's only instance. None
    /// otherwise. Callers check `any_modclass` first.
    #[cold]
    #[inline(never)]
    pub(super) fn modclass_instance(&self, handle: usize) -> Option<String> {
        let enc = self.class_enclosing_module(handle)?;
        if enc == self.module.name {
            return None;
        }
        let paths: Vec<&str> = self
            .module
            .instances
            .iter()
            .filter(|i| i.def_name == enc)
            .map(|i| i.path.as_str())
            .collect();
        if let [only] = paths[..] {
            return Some(only.to_string());
        }
        let birth = self.heap.get(handle)?.as_ref()?.creation_scope.as_str();
        let birth = self.instance_relative_scope(birth);
        paths
            .into_iter()
            .filter(|p| birth == *p || birth.strip_prefix(*p).is_some_and(|r| r.starts_with('.')))
            .max_by_key(|p| p.len())
            .map(str::to_string)
    }

    /// §8/§23.9: the instance whose copy of class `cname` runs here, when
    /// `cname` is declared inside a (non-top) module with several instances:
    /// the instance a module-class method runs in, else the one holding the
    /// executing process. Instance-qualified class keys already own their
    /// storage and need no extra instance suffix.
    #[cold]
    #[inline(never)]
    pub(super) fn modclass_static_inst(&self, cname: &str) -> Option<String> {
        let cached = self.modclass_insts.borrow().get(cname).cloned();
        let insts = match cached {
            Some(v) => v,
            None => {
                let top = self.module.name.as_str();
                let paths = self.module.classes.get(cname).and_then(|cd| {
                    let enc = cd.declaring_module.as_deref().filter(|m| *m != top)?;
                    let paths = self
                        .module
                        .instances
                        .iter()
                        .filter(|i| i.def_name == enc)
                        .map(|i| i.path.clone())
                        .collect::<Vec<_>>();
                    // §8.3 / §23.6: canonical module-class names already
                    // include their owner. Adding it again made reads miss
                    // the initialized cell and execute its initializer twice.
                    if cd.name == cname
                        && paths.iter().any(|owner| {
                            cname
                                .strip_prefix(owner.as_str())
                                .is_some_and(|rest| rest.starts_with('.'))
                        })
                    {
                        return None;
                    }
                    (paths.len() > 1).then(|| std::rc::Rc::new(paths))
                });
                self.modclass_insts
                    .borrow_mut()
                    .insert(cname.to_string(), paths.clone());
                paths
            }
        }?;
        let scope = {
            let a = self.activation_scope.borrow();
            match a.as_deref().filter(|s| !s.is_empty()) {
                Some(s) => s.to_string(),
                None => self.name_resolve_hint.borrow().clone().unwrap_or_default(),
            }
        };
        let scope = self.instance_relative_scope(&scope);
        insts
            .iter()
            .filter(|p| {
                scope == **p
                    || scope
                        .strip_prefix(p.as_str())
                        .is_some_and(|r| r.starts_with('.'))
            })
            .max_by_key(|p| p.len())
            .cloned()
    }

    /// `inst.name` when, in a method of a class declared inside a module,
    /// the bare `name` is an item (unpacked struct, variable) of the instance
    /// `inst` the method runs in — not a local or a property of `this`.
    pub(super) fn modclass_item(&self, name: &str) -> Option<String> {
        if self
            .local_stack
            .last()
            .is_some_and(|l| l.contains_key(name))
        {
            return None;
        }
        if let Some(th) = self.this_stack.last().copied().flatten()
            && self
                .heap
                .get(th)
                .and_then(|o| o.as_ref())
                .is_some_and(|i| i.properties.contains_key(name))
        {
            return None;
        }
        let q = self.modclass_scoped(name)?;
        (self.module.struct_members.contains_key(&q)
            || self.module.var_decl_types.contains_key(&q)
            || self.signal_name_to_id.contains_key(q.as_str())
            || self.signals.contains_key(&q))
        .then_some(q)
    }

    /// Is any class declared inside a module with several instances
    /// (computed once)? Only then do statics key per instance.
    #[inline]
    pub(super) fn any_multi_modclass(&self) -> bool {
        if !self.any_modclass() {
            return false;
        }
        *self.multi_modclass.get_or_init(|| {
            let top = self.module.name.as_str();
            self.module.classes.values().any(|c| {
                c.declaring_module.as_deref().is_some_and(|m| {
                    m != top
                        && self
                            .module
                            .instances
                            .iter()
                            .filter(|i| i.def_name == m)
                            .nth(1)
                            .is_some()
                })
            })
        })
    }

    /// `C::p` / `C#sig::p` -> the instance `inst`'s own cell (`C#@inst::p`,
    /// `C#sig@inst::p`); the head before `#` still names the class.
    pub(super) fn per_instance_static_key(key: &str, prop: &str, inst: &str) -> String {
        let head = key
            .strip_suffix(prop)
            .and_then(|h| h.strip_suffix("::"))
            .unwrap_or(key);
        let sep = if head.contains('#') { "@" } else { "#@" };
        format!("{}{}{}::{}", head, sep, inst, prop)
    }

    /// `inst.name` while a method of a class declared inside a module runs
    /// in instance `inst` (see `modclass_depth`); None otherwise.
    pub(super) fn modclass_scoped(&self, name: &str) -> Option<String> {
        if self.modclass_depth.get() == 0 {
            return None;
        }
        let inst = self.activation_scope.borrow();
        let inst = inst.as_deref().filter(|i| !i.is_empty())?;
        Some(format!("{}.{}", inst, name))
    }

    /// Enter the collections instance `inst` declares into the innermost
    /// rename frame as `name -> inst.name`, ahead of the frame's formals and
    /// locals (they shadow them), except names the object's class declares.
    pub(super) fn modclass_rename_frame(&mut self, handle: usize, cname: &str, inst: &str) {
        let cached = self.modclass_colls.borrow().get(inst).cloned();
        let names = match cached {
            Some(n) => n,
            None => {
                let pre = format!("{}.", inst);
                let m = &self.module;
                let mut v: Vec<String> = m
                    .arrays
                    .keys()
                    .chain(m.arrays_2d.keys())
                    .chain(m.arrays_nd.keys())
                    .chain(m.associative_arrays.keys())
                    .chain(m.dynamic_arrays.iter())
                    .chain(m.queue_vars.iter())
                    .chain(m.struct_members.keys())
                    .chain(m.instances.iter().map(|i| &i.path))
                    .filter_map(|k| k.strip_prefix(pre.as_str()))
                    .filter(|r| !r.contains('.') && !r.contains('['))
                    .map(str::to_string)
                    .collect();
                v.sort_unstable();
                v.dedup();
                let n = std::rc::Rc::new(v);
                self.modclass_colls
                    .borrow_mut()
                    .insert(inst.to_string(), n.clone());
                n
            }
        };
        if names.is_empty() {
            return;
        }
        let props = self.class_coll_member_names(cname);
        let own = self.heap.get(handle).and_then(|o| o.as_ref());
        let entries: Vec<(String, String)> = names
            .iter()
            .filter(|n| !props.contains(*n) && !own.is_some_and(|i| i.properties.contains_key(*n)))
            .map(|n| (n.clone(), format!("{}.{}", inst, n)))
            .collect();
        if let Some(frame) = self.local_dyn.last_mut() {
            frame.splice(0..0, entries);
        }
    }

    pub(super) fn class_enclosing_module_walk(&self, class_name: &str) -> Option<String> {
        let mut cur: Option<String> = Some(class_name.to_string());
        let mut hops = 0;
        while let Some(cn) = cur {
            hops += 1;
            if hops > 64 {
                break;
            }
            let cd = self.module.classes.get(&cn)?;
            if let Some(m) = &cd.declaring_module {
                return Some(m.clone());
            }
            if let Some(enc) = &cd.enclosing {
                return Some(enc.clone());
            }
            cur = cd.extends.clone();
        }
        None
    }

    pub(super) fn active_instance_scope(&self) -> String {
        if !self.m_block_scope.is_empty() {
            self.m_block_scope.clone()
        } else {
            self.current_scope.clone()
        }
    }

    /// §21.2.1.7 `%m`: the hierarchical name of the scope holding the running
    /// statement — instance path, then the process's block label and the
    /// named-block chain. A subroutine or class method names its DECLARING
    /// scope (`pk.f`, `tb.u.mf`, `pk.C.show`), not the caller's.
    pub(super) fn m_path(&self) -> String {
        if self.monitor_m_active {
            // Slot-end $monitor / $strobe re-render: the path captured when
            // the task executed.
            return self.monitor_m_suffix.clone();
        }
        let first = self.m_scope_stack.first().map(|s| s.as_str());
        // A subroutine declared in a package or another instance: its
        // entry carries the absolute declaring path.
        if let Some(root) = first.and_then(|f| f.strip_prefix(M_ROOT_MARK)) {
            let mut out = root.to_string();
            for sc in &self.m_scope_stack[1..] {
                out.push('.');
                out.push_str(sc.trim_start_matches(M_SUBR_MARK));
            }
            return out;
        }
        let in_subroutine = first.is_some_and(|f| f.starts_with(M_SUBR_MARK));
        // A class method body (its entry clears the lexical chain, so any
        // subroutine it calls shows up as a marked entry instead).
        if !in_subroutine {
            if let Some(Some(cname)) = self.class_context_stack.last() {
                let mut out = self.class_m_root(cname);
                if let Some((method, _)) = self.static_local_syncs.last() {
                    out.push('.');
                    out.push_str(method.rsplit("::").next().unwrap_or(method));
                }
                for sc in &self.m_scope_stack {
                    out.push('.');
                    out.push_str(sc);
                }
                return out;
            }
        }
        // A comb/edge block names its own instance (empty = the root); a
        // process names the scope `run_process` installed.
        let inst = if self.m_block_active || self.current_scope.is_empty() {
            self.m_block_scope.as_str()
        } else {
            self.current_scope.as_str()
        };
        let mut out = self.hier_path(inst);
        if !self.m_block_active && !in_subroutine && self.func_call_stack.is_empty() {
            if let Some(l) = self.process_m_label.get(&self.current_pid) {
                out.push('.');
                out.push_str(l);
            }
        }
        for sc in &self.m_scope_stack {
            out.push('.');
            out.push_str(sc.trim_start_matches(M_SUBR_MARK));
        }
        out
    }
}
