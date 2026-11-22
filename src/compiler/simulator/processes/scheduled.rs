//! Running scheduled processes and deferred comb blocks.

use super::*;

impl Simulator {
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
    pub(in crate::compiler::simulator) fn compiled_block_write_ids(
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

    pub(in crate::compiler::simulator) fn note_comb_ran_in_process(
        &mut self,
        eidx: usize,
        entry: &CombEntry,
    ) {
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
    pub(in crate::compiler::simulator) fn drain_deferred_comb(&mut self) {
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
                    // §13.4: see `enter_private_regs`.
                    let caller_regs = (!self.settling).then(|| self.enter_private_regs());
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
                    self.timescale_scope_override =
                        Some(entry.cold.scope_hint.clone().unwrap_or_default());
                    let saved_hint = if compiled.has_fallback {
                        let prev = self.name_resolve_hint.borrow().clone();
                        *self.name_resolve_hint.borrow_mut() = entry.cold.scope_hint.clone();
                        Some(prev)
                    } else {
                        None
                    };
                    self.exec_insns(insns);
                    if let Some(prev) = saved_hint {
                        *self.name_resolve_hint.borrow_mut() = prev;
                    }
                    self.timescale_scope_override = saved_ts;
                    if let Some(caller_regs) = caller_regs {
                        self.leave_private_regs(caller_regs);
                    }
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
    pub(in crate::compiler::simulator) fn run_scheduled_process(
        &mut self,
        pid: usize,
        stmts: &ProcCont,
    ) {
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
        // §23.6: nested scheduled work must restore the caller's lexical
        // scope, including a task body currently suspended on this stack.
        let outer_scope = self.current_scope.clone();

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

        self.current_scope = outer_scope;
        let mine = std::mem::replace(&mut self.m_scope_stack, outer_m_scope);
        // Non-empty ⇒ suspended mid-task; keep it for the resume. A process
        // that ran to completion unwound its own pushes and parks nothing.
        if !mine.is_empty() {
            self.process_m_scope.insert(pid, mine);
        }
    }

    pub(in crate::compiler::simulator) fn run_scheduled_process_inner(
        &mut self,
        pid: usize,
        stmts: &ProcCont,
    ) {
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
        let sibling_baseline = self.fork_baselines.get(&pid).cloned();
        let fork_parent = self.process_parents.get(&pid).copied();
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
        // §6.21, §9.3.2: a variable a block of the child declared is the
        // child's own, even when it has the name of a variable in a frame
        // the child shares with its parent (the block's declaration hides
        // it): its writes reach neither the parent nor the siblings, and the
        // value its block's exit puts back is no write either (#943).
        let child_private =
            |g: u64, k: &str| Self::block_declares(&child_ctx.block_scope_saves, g, k);
        let baseline = sibling_baseline.as_deref();
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
                        &child_private,
                    )
                } else {
                    // Parent is the active process — its context is `saved`.
                    Self::merge_fork_writes(
                        &mut saved.local_stack,
                        &saved.local_gen_stack,
                        child_frames,
                        child_frames_gen,
                        baseline,
                        &child_private,
                    )
                };
            }
        }
        // (b) signal-capture write-back: procedural block-locals that live in
        // `self.signals`. Write only keys the child CHANGED (relative to the
        // fork-time baseline) so an inherited-unchanged value doesn't
        // clobber a sibling's or the parent's concurrent write.
        let mut stored: Vec<(String, Value)> = Vec::new();
        if let Some(caps) = signal_caps {
            let top = child_frames.last();
            let top_gen = child_frames_gen.last().copied().unwrap_or(0);
            for nm in caps {
                let Some(v) = top.and_then(|f| f.get(nm)) else {
                    continue;
                };
                if child_private(top_gen, nm) {
                    continue;
                }
                let inherited_unchanged = baseline
                    .and_then(|b| b.last())
                    .and_then(|f| f.get(nm))
                    .is_some_and(|old| old == v);
                if !inherited_unchanged {
                    self.signals.insert(nm.clone(), v.clone());
                    stored.push((nm.clone(), v.clone()));
                }
            }
        }
        // The other children of a fork share these block-locals too, but each
        // holds a copy in its own capture frame, under its own activation, so
        // the frame broadcast below never reaches them: hand them the value
        // and make it their baseline, so a later write of the fork-time value
        // by one of them is still a write.
        if !stored.is_empty() {
            let others: Vec<usize> = self
                .fork_signal_captures
                .iter()
                .filter(|(p, caps)| **p != pid && stored.iter().any(|(n, _)| caps.contains(n)))
                .map(|(p, _)| *p)
                .collect();
            for other_pid in others {
                let Some(other) = self.process_contexts.get_mut(&other_pid) else {
                    continue;
                };
                let top_gen = other.local_gen_stack.last().copied().unwrap_or(0);
                let Some(top) = other.local_stack.last_mut() else {
                    continue;
                };
                for (n, v) in &stored {
                    if Self::block_declares(&other.block_scope_saves, top_gen, n) {
                        continue;
                    }
                    if let Some(slot) = top.get_mut(n) {
                        *slot = v.clone();
                    }
                }
                if let Some(base) = self.fork_baselines.get_mut(&other_pid) {
                    Self::sync_fork_baseline(
                        base,
                        &other.local_stack,
                        &other.local_gen_stack,
                        &other.block_scope_saves,
                    );
                }
            }
        }
        // Match both task activation and lexical block activation. Repeated
        // loop-block declarations share a flattened frame but not storage.
        if let Some(sibling_baseline) = &sibling_baseline {
            let has_changes = child_frames.iter().enumerate().any(|(i, frame)| {
                frame.iter().any(|(k, v)| {
                    sibling_baseline.get(i).and_then(|base| base.get(k))
                        .is_some_and(|old| old != v)
                })
            });
            if has_changes {
                let mut sibling_scopes = self.fork_local_scopes.get(&pid).cloned().unwrap_or_default();
                Self::overlay_fork_local_scopes(&mut sibling_scopes, &child_ctx.block_scope_saves);
                let changes: Vec<HashMap<String, Value>> = child_frames.iter().enumerate()
                    .map(|(i, frame)| frame.iter().filter(|(k, v)| {
                        sibling_baseline.get(i).and_then(|base| base.get(*k))
                            .is_some_and(|old| old != *v)
                    }).map(|(k, v)| (k.clone(), v.clone())).collect()).collect();
                // The child's own block variables are no sibling's (#943).
                let mut changes = changes;
                if !child_ctx.block_scope_saves.is_empty() {
                    for (frame, g) in changes.iter_mut().zip(child_frames_gen) {
                        frame.retain(|k, _| !child_private(*g, k));
                    }
                }
                // Only a fork child holding one of the changed frames' own
                // activations can receive a write (`merge_fork_writes`
                // matches frames by generation), so visit just those, not
                // every parked process.
                let mut candidates: Vec<usize> = changes
                    .iter()
                    .enumerate()
                    .filter(|(_, frame)| !frame.is_empty())
                    .filter_map(|(i, _)| self.fork_gen_pids.get(child_frames_gen.get(i)?))
                    .flatten()
                    .copied()
                    .filter(|&p| p != pid && Some(p) != fork_parent)
                    .collect();
                candidates.sort_unstable();
                candidates.dedup();
                let has_any_block_saves = !child_ctx.block_scope_saves.is_empty()
                    || !sibling_scopes.is_empty()
                    || candidates.iter().any(|p| {
                        self.fork_local_scopes.get(p).is_some_and(|s| !s.is_empty())
                            || self.process_contexts.get(p).is_some_and(|c| !c.block_scope_saves.is_empty())
                    });
                for other_pid in candidates {
                    let Some(other) = self.process_contexts.get_mut(&other_pid) else {
                        continue;
                    };
                    if !self.fork_baselines.contains_key(&other_pid) {
                        continue;
                    }
                    let other_pid = &other_pid;
                    let visible_storage;
                    let visible: &[HashMap<String, Value>] = if !has_any_block_saves {
                        &changes
                    } else {
                        let base_scopes = self.fork_local_scopes.get(other_pid);
                        let has_other_blocks = !other.block_scope_saves.is_empty();
                        let (other_scopes_storage, other_scopes_ref) = if has_other_blocks {
                            let mut sc = base_scopes.cloned().unwrap_or_default();
                            Self::overlay_fork_local_scopes(&mut sc, &other.block_scope_saves);
                            (Some(sc), None)
                        } else {
                            (None, base_scopes)
                        };
                        let lookup_other = |f_gen: &u64, key: &str| -> Option<u64> {
                            if let Some(sc) = &other_scopes_storage {
                                sc.get(f_gen).and_then(|m| m.get(key)).copied()
                            } else if let Some(sc) = other_scopes_ref {
                                sc.get(f_gen).and_then(|m| m.get(key)).copied()
                            } else {
                                None
                            }
                        };
                        let mut filtered = changes.clone();
                        for (i, frame) in filtered.iter_mut().enumerate() {
                            let frame_generation = child_frames_gen.get(i).copied().unwrap_or(0);
                            frame.retain(|key, _| {
                                sibling_scopes.get(&frame_generation).and_then(|m| m.get(key)).copied()
                                    == lookup_other(&frame_generation, key)
                            });
                        }
                        visible_storage = filtered;
                        &visible_storage
                    };
                    let updates = Self::merge_fork_writes(
                        &mut other.local_stack,
                        &other.local_gen_stack,
                        visible,
                        child_frames_gen,
                        None,
                        &|g, k| Self::block_declares(&other.block_scope_saves, g, k),
                    );
                    // §6.21: where a block of the sibling hides the variable,
                    // the value is for the hidden one, which the block's exit
                    // puts back.
                    // A write of a variable the sibling shares at the fork
                    // (the writer's and the sibling's fork-time scopes agree).
                    let hidden = if other.block_scope_saves.is_empty() {
                        Vec::new()
                    } else {
                        let base_scopes = self.fork_local_scopes.get(other_pid);
                        let shared = |g: u64, k: &str| {
                            sibling_scopes.get(&g).and_then(|m| m.get(k))
                                == base_scopes.and_then(|sc| sc.get(&g)).and_then(|m| m.get(k))
                        };
                        Self::deliver_to_hidden(
                            &mut other.block_scope_saves,
                            &other.local_gen_stack,
                            &changes,
                            child_frames_gen,
                            &shared,
                        )
                    };
                    // The sibling now holds these values: a later write of
                    // the value it held at the fork is a write too.
                    if let Some(base) = self.fork_baselines.get_mut(other_pid) {
                        Self::sync_fork_baseline(
                            base,
                            &other.local_stack,
                            &other.local_gen_stack,
                            &other.block_scope_saves,
                        );
                        if !hidden.is_empty() {
                            let frames = Arc::make_mut(base);
                            for (i, k, v) in hidden {
                                if let Some(b) = frames.get_mut(i).and_then(|f| f.get_mut(&k)) {
                                    *b = v;
                                }
                            }
                        }
                    }
                    written.extend(updates);
                }
            }
        }
        // §9.3.2: this child's writes are published (to the parent and its
        // siblings), so they become its baseline. A later write is then
        // compared with what the child last published, not with the value at
        // the fork: `x = 0; #10; x = 5;` with 5 the fork-time value used to
        // read as "unchanged" and be dropped, the parent keeping 0.
        drop(sibling_baseline);
        if let Some(base) = self.fork_baselines.get_mut(&pid) {
            Self::sync_fork_baseline(
                base,
                child_frames,
                child_frames_gen,
                &child_ctx.block_scope_saves,
            );
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

    /// #35/#t0-comb-defer: the full settle that consumes
    /// `deferred_proc_entries` (the cont-assign updates a yielding process
    /// deferred). The drained entries must evaluate in a NEUTRAL scope: a
    /// process's name-resolve hint (its instance path) would anchor a
    /// same-named cont-assign LHS into the instance — the u_h.t
    /// self-assign hazard (see tests/hierarchy/same_name_port_hop.rs).
    pub(in crate::compiler::simulator) fn drain_deferred_proc_settle(&mut self) {
        if self.deferred_proc_entries.is_empty() || self.proc_settle_defer {
            return;
        }
        let saved_hint = self.name_resolve_hint.borrow_mut().take();
        self.settle_combinatorial();
        *self.name_resolve_hint.borrow_mut() = saved_hint;
    }

    /// Dispatch a scheduled payload, recognizing the empty marker used by a
    /// retained dynamic-delay always assignment.
    pub(in crate::compiler::simulator) fn run_process_payload(
        &mut self,
        pid: usize,
        stmts: &ProcCont,
    ) {
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
        } else if stmts.is_empty()
            && !self.lazy_initial.is_empty()
            && let Some(p) = self.lazy_initial.remove(&pid)
        {
            // An initial block queued by its pending form (see `compile`):
            // its body is materialized for this instance now, with the
            // block locals given storage of their own as an instance's
            // always block gets (`materialize_pending_always`), and is
            // dropped again when the process ends.
            let mut ib = p.materialize();
            Self::rename_instance_locals(&mut ib.stmt, &ib.scope);
            let stmts = match ib.stmt.kind {
                StatementKind::SeqBlock { stmts, .. } => stmts,
                other => vec![Statement::new(other, ib.stmt.span)].into(),
            };
            self.run_process_stmts(pid, &ProcCont::from_vec(stmts.into_vec()));
        } else {
            self.run_process_stmts(pid, stmts);
        }
        // #35: the process just YIELDED (parked or finished) — the
        // cont-assign update events its writes scheduled become runnable
        // now (§10.3), so the deferred entries drain before anything else
        // in the slot observes the nets — EXCEPT during the time-0 start
        // batch (#t0-comb-defer): a commercial simulator keeps those update
        // events behind the ENTIRE batch (active-region FIFO), so a
        // source-later blocking-body `always` is parked and armed before
        // the propagation fires. Draining per-process here would fire the
        // edge while later starts have not run yet and silently lose it.
        if !self.deferred_proc_entries.is_empty()
            && !self.proc_settle_defer
            && (self.time != 0 || self.t0_starts_remaining == 0)
        {
            self.drain_deferred_proc_settle();
        }
    }
}
