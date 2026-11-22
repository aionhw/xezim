//! Process state machines and fast delayed always blocks.

use super::*;

impl Simulator {
    /// §3.14.3 delay quantization for a process-FSM WaitDelayReg value —
    /// the register-value half of `delay_value_to_ticks`, without the
    /// pathological-real diagnostics (no source expression at hand).
    pub(in crate::compiler::simulator) fn fsm_delay_ticks(
        &self,
        v: &Value,
        precision_diff: i32,
    ) -> u64 {
        if !v.is_real {
            return Self::quantize_integral_delay_ticks(v.to_u64().unwrap_or(0), precision_diff);
        }
        let f = v.to_f64();
        let raw = if f.is_finite() && f > 0.0 { f } else { 0.0 };
        Self::quantize_delay_ticks(raw, precision_diff)
    }

    /// Roadmap step 13 (XEZIM_AOT=1): compile every process FSM body that
    /// the rustc emitter covers into ONE native dylib and install the fn
    /// pointers. Bodies the emitter rejects (or that touch real-typed /
    /// >64-bit signals) stay on the bytecode executor.
    #[cfg(feature = "jit")]
    pub(in crate::compiler::simulator) fn aot_compile_proc_fsms(&mut self) {
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
    pub(in crate::compiler::simulator) fn lower_proc_fsm_ts(
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
    pub(in crate::compiler::simulator) fn run_proc_fsm_ts_segment(
        &mut self,
        f: &mut ProcFsm,
    ) -> bool {
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

    pub(in crate::compiler::simulator) fn run_proc_fsm(&mut self, pid: usize) {
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
    pub(in crate::compiler::simulator) fn fsm_wait_terms(
        &mut self,
        f: &mut ProcFsm,
        ix: usize,
    ) -> Vec<SensitivityId> {
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

    pub(in crate::compiler::simulator) fn run_proc_fsm_inner(&mut self, pid: usize) {
        self.current_pid = pid;
        self.flush_deferred_asserts(pid);
        // Same per-process zero-delay guard as `run_process_stmts` /
        // `run_fast_delay_always`: a `#0` wait parks the FSM on the inactive
        // queue, which the scheduler drains inside one delta, so the outer
        // loop's stall counter never sees `initial forever begin #0; ... end`
        // spin — the run_proc_fsm path hung the stall-report test.
        if self.stall_limit > 0 && self.stall_pid_tripped(pid, None) {
            self.event_queue.schedule(self.time, pid, Vec::new().into());
            self.zero_delay_defer_pending = true;
            return;
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
                            self.park_event_waiter(w);
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
                            self.park_event_waiter(w);
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

    pub(in crate::compiler::simulator) fn run_fast_delay_always(&mut self, pid: usize) {
        self.current_pid = pid;
        self.flush_deferred_asserts(pid);

        // Match run_process_stmts' per-process zero-delay protection. The
        // marker was removed from the timing wheel before dispatch, so park it
        // again before asking the outer loop to seek a future event.
        // Check BEFORE counting (inside `stall_pid_tripped`): the activation
        // that trips the guard is re-parked UNEXECUTED, so it must not inflate
        // the "ran N times" attribution in the stall report.
        if self.stall_limit > 0 && self.stall_pid_tripped(pid, None) {
            self.event_queue.schedule(self.time, pid, Vec::new().into());
            self.zero_delay_defer_pending = true;
            return;
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
}
