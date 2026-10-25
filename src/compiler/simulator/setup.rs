//! Simulator setup.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

impl Simulator {
    /// `XEZIM_INIT_REG=0|random` — initialize REGISTER-class storage at t=0
    /// via REAL writes, semantically "initial q = <v>;" per flop.
    ///
    /// This replaces the retired construction-time X->0 coercion, which was
    /// invisible to the event system (no dirty marking, no X->0 transition,
    /// nets forced into unreachable states) — measured to distort c906
    /// firmware 1.64x and, after the Aug 2026 upstream event-semantics fixes,
    /// to wedge it entirely. Industry precedent: commercial register-init
    /// options cover registers only; nets derive by evaluation.
    ///
    /// Selection: NBA targets of COMPILED edge blocks (the flop set), minus
    /// continuously-driven nets, minus anything already non-X. AST-only edge
    /// blocks are not scanned (rare; their flops stay X). Writes go through
    /// `write_sig!` + dirty marking, so the time-0 settle derives all comb
    /// values and edge detection sees a legitimate X->0 (negedge, not
    /// posedge). `random` fills inline-width flops from a per-id hash mixed
    /// with the run's seed (`+seed=`/`-sv_seed`), so different seeds give
    /// different patterns (reset-bug hunting); wide flops get 0 in both modes.
    ///
    /// `XEZIM_INIT_ZERO=1` is kept as a deprecated alias for `=0` (its array
    /// zeroing lives on separately as `XEZIM_INIT_MEM`).
    pub(super) fn init_registers(&mut self) {
        let mode = match std::env::var("XEZIM_INIT_REG").ok().as_deref() {
            Some("0") => 0u8,
            Some("random") => 1u8,
            _ => {
                if std::env::var("XEZIM_INIT_ZERO").ok().as_deref() == Some("1") {
                    0u8
                } else {
                    return;
                }
            }
        };
        // Flop set: every whole/partial NBA destination in compiled edge blocks.
        use super::bytecode::Insn as BI;
        let mut ids: Vec<u32> = Vec::new();
        for cb in self.compiled_edge_blocks.iter().flatten() {
            for i in &cb.instructions {
                match i {
                    BI::NbaAssign(id, _, _)
                    | BI::NbaAssignConst(id, _, _)
                    | BI::NbaAssignRange(id, _, _, _)
                    | BI::NbaAssignRangeDyn(id, _, _, _)
                    | BI::NbaAssignBitDyn(id, _, _)
                    | BI::NbaAssignArrayRead(id, _, _, _) => ids.push(*id),
                    _ => {}
                }
            }
        }
        ids.sort_unstable();
        ids.dedup();
        let mut inited = 0usize;
        for id in ids {
            let id = id as usize;
            if id >= self.signal_table.len()
                || self.cont_driven.contains(&id)
                || !self.signal_table[id].has_xz()
            {
                continue;
            }
            let w = self.signal_widths[id];
            let mut val = if mode == 1 && w <= 64 {
                // SplitMix64 over the signal id, offset by the run's seed
                // (`+seed=` / `-sv_seed`) so the X-init pattern changes with
                // the seed. No seed, or the default seed 1, gives the
                // historical values.
                let salt = self
                    .run_seed
                    .filter(|&s| s != SvRng::DEFAULT_SEED)
                    .map_or(0, |s| s.wrapping_add(1).wrapping_mul(0xd1b5_4a32_d192_ed03));
                let mut z = (id as u64)
                    .wrapping_add(0x9e3779b97f4a7c15)
                    .wrapping_add(salt);
                z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
                z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
                let bits = (z ^ (z >> 31)) & if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
                Value::from_inline(bits, 0, w)
            } else {
                Value::zero(w)
            };
            val.is_signed = self.signal_signed[id];
            if self.signal_real.get(id).copied().unwrap_or(false) {
                continue; // real-typed flops keep their own init semantics
            }
            if !self.dirty_signals[id] {
                self.dirty_signals[id] = true;
                self.dirty_list.push(id);
            }
            self.dirty_any = true;
            write_sig!(self, id, val);
            self.table_modified = true;
            inited += 1;
        }
        if inited > 0 {
            eprintln!(
                "[INIT-REG] initialized {} register signals to {}",
                inited,
                if mode == 1 { "random" } else { "0" }
            );
        }
    }

    /// Advise the kernel to back the largest per-signal / dependency arrays
    /// with transparent huge pages (2 MiB), cutting TLB page-walk cost on the
    /// scattered, GiB-scale working set. See the call site for the rationale.
    pub(super) fn advise_hugepages(&self) {
        if std::env::var("XEZIM_HUGEPAGE").ok().as_deref() == Some("0") {
            return;
        }
        // A parent process can disable THP for its whole descendant tree with
        // prctl(PR_SET_THP_DISABLE, 1); the flag is inherited across fork+exec
        // and silently overrides MADV_HUGEPAGE — madvise still returns 0 and
        // sets VM_HUGEPAGE, but the fault handler never attempts a huge page
        // and MADV_COLLAPSE fails EINVAL. Some launchers (CI runners, agent
        // harnesses, container supervisors) set it. Clearing it for ourselves
        // needs no privilege and affects only this process.
        //
        // Measured on c906 memcpy x50 with it wrongly left set: dTLB-load-misses
        // 161.1M vs 65.2M, cycles +3.7%, wall +3.8%. Worth reclaiming.
        #[cfg(target_os = "linux")]
        unsafe {
            const PR_SET_THP_DISABLE: libc::c_int = 41;
            libc::prctl(PR_SET_THP_DISABLE, 0, 0, 0, 0);
        }
        // Only worth it for spans >= one huge page.
        const HP: usize = 2 * 1024 * 1024;
        // Both madvise() results were previously discarded, so a total failure
        // was indistinguishable from success. XEZIM_HUGEPAGE_STATS=1 reports
        // what the kernel actually did.
        let stats = std::env::var("XEZIM_HUGEPAGE_STATS").is_ok();
        #[cfg(target_os = "linux")]
        if stats {
            const PR_GET_THP_DISABLE: libc::c_int = 42;
            let d = unsafe { libc::prctl(PR_GET_THP_DISABLE, 0, 0, 0, 0) };
            eprintln!(
                "[HUGEPAGE] PR_THP_DISABLE now {} (1 here would block every huge page)",
                d
            );
        }
        // MADV_COLLAPSE is in `libc` for glibc targets only, so a musl build
        // (also `target_os = "linux"`) keeps the stub below (#290).
        #[cfg(all(target_os = "linux", target_env = "gnu"))]
        fn advise<T>(s: &[T], name: &str, stats: bool) {
            let bytes = std::mem::size_of_val(s);
            if bytes < HP {
                if stats {
                    eprintln!(
                        "[HUGEPAGE] {:<20} {:>8.2} MiB  SKIPPED (< 2 MiB)",
                        name,
                        bytes as f64 / 1048576.0
                    );
                }
                return;
            }
            let page = 4096usize;
            let start = s.as_ptr() as usize;
            let end = start + bytes;
            // madvise needs a page-aligned start; round the span inward.
            let astart = (start + page - 1) & !(page - 1);
            let aend = end & !(page - 1);
            if aend > astart {
                let p = astart as *mut libc::c_void;
                let len = aend - astart;
                unsafe {
                    // Hint future faults, then SYNCHRONOUSLY collapse the
                    // already-faulted 4 KiB pages into 2 MiB pages. MADV_HUGEPAGE
                    // alone only helps new faults / lazy khugepaged, but these
                    // arrays are fully populated by `compile()` before we get
                    // here, so nothing would change without the collapse.
                    // MADV_COLLAPSE is Linux 6.1+; on older kernels it returns
                    // EINVAL and we simply keep the (advisory) hint.
                    let r1 = libc::madvise(p, len, libc::MADV_HUGEPAGE);
                    let e1 = if r1 == 0 {
                        0
                    } else {
                        *libc::__errno_location()
                    };
                    let r2 = libc::madvise(p, len, libc::MADV_COLLAPSE);
                    let e2 = if r2 == 0 {
                        0
                    } else {
                        *libc::__errno_location()
                    };
                    if stats {
                        let al = if astart % HP == 0 {
                            "2MiB-aligned"
                        } else {
                            "UNALIGNED"
                        };
                        eprintln!(
                            "[HUGEPAGE] {:<20} {:>8.2} MiB  HUGEPAGE={} COLLAPSE={} ({}, start%2MiB={})",
                            name,
                            len as f64 / 1048576.0,
                            if r1 == 0 {
                                "ok".to_string()
                            } else {
                                format!("errno {}", e1)
                            },
                            if r2 == 0 {
                                "ok".to_string()
                            } else {
                                format!("errno {}", e2)
                            },
                            al,
                            astart % HP
                        );
                    }
                }
            }
        }
        #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
        fn advise<T>(_s: &[T], _n: &str, _st: bool) {}

        advise(&self.signal_table, "signal_table", stats);
        advise(&self.signal_widths, "signal_widths", stats);
        advise(&self.signal_signed, "signal_signed", stats);
        advise(&self.signal_real, "signal_real", stats);
        advise(&self.signal_two_state, "signal_two_state", stats);
        advise(&self.dirty_signals, "dirty_signals", stats);
        advise(&self.comb_dep_offsets, "comb_dep_offsets", stats);
        advise(&self.comb_dep_entries, "comb_dep_entries", stats);
        advise(&self.comb_entries, "comb_entries", stats);
        // The packed memories are accessed at random addresses just like the
        // per-signal arrays (a CPU's RAM model reads wherever the program
        // points it).
        advise(self.packed.data(), "packed_arena", stats);
    }

    pub fn simulate(&mut self) {
        if !self.compiled {
            self.compile();
        }
        // A compile-time failure means simulation must not run at all.
        if !self.compile_errors.is_empty() {
            return;
        }
        // Back the big, stable, randomly-accessed arrays with 2 MiB transparent
        // huge pages. On c910 the per-signal arrays span ~1.3 GiB / ~340k 4 KiB
        // pages accessed with no spatial locality, so page-table walks burned
        // ~15% of all cycles (dtlb_load_misses.walk_active) with 0 walks landing
        // on huge pages. `madvise(MADV_HUGEPAGE)` collapses each 2 MiB span to a
        // single TLB entry. Advisory + safe: a no-op where THP is off/unset, and
        // it never changes results. `XEZIM_HUGEPAGE=0` opts out.
        self.advise_hugepages();
        self.init_registers();
        // `--dump-timescales`: report every module's timescale before the run.
        if dump_timescales_enabled() {
            self.dump_module_timescales();
        }
        let prev_active = ACTIVE_SIMULATOR.with(|cell| cell.get());
        let self_ptr = self as *mut Simulator;
        ACTIVE_SIMULATOR.with(|cell| cell.set(self_ptr));
        let prev_global =
            GLOBAL_ACTIVE_SIMULATOR.swap(self_ptr, std::sync::atomic::Ordering::AcqRel);
        self.vpi_self_ptr = self_ptr;
        // §38.36.3 cbEndOfCompile, then cbStartOfSimulation, once each. The
        // model was built before the VPI startup routines ran, so both come
        // here, before the time-0 slot. (cbStartOfReset/cbEndOfReset never
        // fire: xezim has no `$reset`.)
        let mut start = false;
        if self.vpi_cb_mask != 0 {
            self.vpi_start_of_simulation();
            // A value a routine wrote there left work for the time-0 slot.
            start = self.dirty_any;
        }
        if !self.wake_rank_built {
            self.build_sig_wake_rank();
        }
        self.event_loop(start);
        if self.ended_waiting_in_c {
            // Stopped, not finished: no `$finish` was called.
            self.finished = false;
        }
        // Loops in the `final` blocks below run to completion again.
        super::interrupt::end_loop_stop();
        // §16.4: reports of the last time slot (or, after $finish, notes),
        // before the clock moves to a `run <time>` stop point.
        if !self.deferred_asserts.is_empty() {
            self.mature_deferred_asserts();
        }
        // `run <time>` advances the current time to the end of the run even
        // when nothing is scheduled there, so `final` blocks and the closing
        // report see the stop time rather than the last event's.
        if !self.finished
            && RUN_LENGTH_REQUESTED.load(std::sync::atomic::Ordering::Relaxed)
            && self.time < self.max_time
        {
            self.time = self.max_time;
        }
        // LRM §9.2.3 — `final` procedures run after the event loop terminates
        // but BEFORE any trace flush. May not contain time-consuming
        // statements (no `#`, `@`, `wait`). We honor that by running each
        // body through `run_process_stmts` with `finished` momentarily
        // cleared; if a final block tries to suspend it will simply terminate
        // when the runtime next checks `finished`.
        let finals = std::mem::take(&mut self.module.final_blocks);
        if !finals.is_empty() {
            let was_finished = self.finished;
            self.finished = false;
            for fb in finals {
                let span = fb.stmt.span;
                let stmts = match fb.stmt.kind {
                    StatementKind::SeqBlock { stmts, .. } => stmts,
                    other => vec![Statement::new(other, fb.stmt.span)],
                };
                let pid = self.next_pid;
                self.next_pid += 1;
                // A submodule's final must resolve names, %m, and $time/%t
                // under ITS instance scope, like initial blocks do.
                if !fb.scope.is_empty() {
                    self.process_scope_hint.insert(pid, fb.scope.clone());
                    self.current_scope = fb.scope.clone();
                } else {
                    self.current_scope.clear();
                }
                self.process_origin.insert(pid, (span, "final block"));
                self.run_process_stmts(pid, &ProcCont::from_vec(stmts));
                // A final block's $finish should NOT stop subsequent finals;
                // re-clear after each.
                self.finished = false;
            }
            self.finished = was_finished;
        }
        // §9.2.3: end-of-simulation callbacks observe all final procedures.
        // cbEnterInteractive has already been delivered if $stop ended the run.
        if self.vpi_cb_mask != 0 {
            self.vpi_end_of_simulation();
        }
        if self.code_cov.is_some() {
            self.emit_coverage_summary();
        }
        // Flush any `--warn-x` reports captured during the final region.
        self.drain_warn_x();
        self.drain_value_trace();
        self.vcd_finish();
        self.xtrace_finish();
        self.fst_finish();
        // Barrier before the caller prints its own summary. `$display` output
        // is on a writer thread; the `[PROF]`/`[PHASE]` lines and the trailing
        // "Simulation finished at time N" are not, and under `--log` all of them
        // land in ONE file (stdout and stderr are dup2'd together). Without this
        // the tail of the simulation's own output can be overtaken by the
        // summary that is supposed to follow it.
        self.flush_stdout();
        self.flush_file_writes();
        if std::env::var("XEZIM_RS_STATS").is_ok() {
            xezim_core::value::Value::dump_range_select_stats();
        }
        ACTIVE_SIMULATOR.with(|cell| cell.set(prev_active));
        GLOBAL_ACTIVE_SIMULATOR.store(prev_global, std::sync::atomic::Ordering::Release);
    }

    /// Detect `initial begin VAR = CONST; forever #d VAR = ~VAR; end`
    /// and turn it into a ClockGen. With concurrent assertions, the caller
    /// schedules the seed in Active after Preponed sampling; otherwise seed
    /// in-place. Return the periodic generator, or None
    /// if the initial block's shape doesn't match.
    pub(super) fn try_extract_initial_clock_gen(
        &mut self,
        stmts: &[Statement],
    ) -> Option<ClockGen> {
        if stmts.len() != 2 {
            return None;
        }
        // Stmt 0: BlockingAssign(VAR, CONST)
        let (seed_lhs_name, seed_val) = match &stmts[0].kind {
            StatementKind::BlockingAssign { lvalue, rvalue } => {
                let n = match &lvalue.kind {
                    ExprKind::Ident(h) => Self::clock_gen_ident(h)?,
                    _ => return None,
                };
                // RHS must be a compile-time constant (plain integer,
                // Paren-wrapped, or parameter reference). CRUCIAL: a clock seed
                // is never a function call — bail before evaluating if the RHS
                // contains one, so this *detection* pass can't run a
                // side-effecting function (the real execution evaluates it
                // again, which double-ran every `x = side_effecting_F();`).
                if Self::expr_contains_call(rvalue) {
                    return None;
                }
                let v = self.eval_expr(rvalue);
                // Reject X/Z seeds; we need a determinate start value.
                if v.has_xz() {
                    return None;
                }
                (n, v)
            }
            _ => return None,
        };
        // Stmt 1: Forever containing #d VAR = ~VAR
        let (half_period, tog_hier) = match &stmts[1].kind {
            StatementKind::Forever { body } => {
                let inner = match &body.kind {
                    StatementKind::SeqBlock { stmts: s, .. } if s.len() == 1 => &s[0],
                    StatementKind::SeqBlock { .. } => return None,
                    _ => &**body,
                };
                // inner must be TimingControl::Delay(d) with BlockingAssign body
                let (d_expr, assign_body) = match &inner.kind {
                    StatementKind::TimingControl {
                        control: TimingControl::Delay(d),
                        stmt,
                    } => (d, stmt.as_ref()),
                    _ => return None,
                };
                // Evaluate the delay at extraction time. Route through
                // `eval_delay_ticks`, which handles a REAL-valued half-period
                // (`#(real_period/2)`) — a plain `eval_expr(..).to_u64()` reads
                // a real's raw IEEE-754 bits as the tick count (a huge number,
                // so the clock never fired) — and applies §3.14.3 module-
                // precision rounding, matching the normal `#delay` path. A
                // zero (or unevaluable) delay is NOT a valid clock-gen
                // half-period; fall back to the general forever path.
                let delay = self.eval_delay_ticks(d_expr);
                if delay == 0 {
                    return None;
                }
                // A runtime-variable delay (`forever #(half) clk = ~clk` with
                // `half` reprogrammed at runtime) must re-evaluate each toggle,
                // so it cannot be a fixed-period ClockGen — fall back to the
                // general forever path (which re-reads the delay every loop).
                if self.delay_expr_is_dynamic(d_expr) {
                    return None;
                }
                // assign_body (or inner SeqBlock): BA VAR = ~VAR
                let ba_target = match &assign_body.kind {
                    StatementKind::BlockingAssign { lvalue, rvalue } => (lvalue, rvalue),
                    StatementKind::SeqBlock { stmts: s, .. } if s.len() == 1 => match &s[0].kind {
                        StatementKind::BlockingAssign { lvalue, rvalue } => (lvalue, rvalue),
                        _ => return None,
                    },
                    _ => return None,
                };
                let (lhs, rhs) = ba_target;
                let lh = match &lhs.kind {
                    ExprKind::Ident(h) => h,
                    _ => return None,
                };
                let ln = Self::clock_gen_ident(lh)?;
                // rhs must be ~LHS or !LHS
                let rn = match &rhs.kind {
                    ExprKind::Unary {
                        op: UnaryOp::BitNot,
                        operand,
                    }
                    | ExprKind::Unary {
                        op: UnaryOp::LogNot,
                        operand,
                    } => match &operand.kind {
                        ExprKind::Ident(h) => Self::clock_gen_ident(h)?,
                        _ => return None,
                    },
                    _ => return None,
                };
                if ln != rn {
                    return None;
                }
                (delay, lh)
            }
            _ => return None,
        };
        let tog_name = Self::clock_gen_ident(tog_hier)?;
        if seed_lhs_name != tog_name {
            return None;
        }
        // Resolve the whole dotted path under this block's scope (the caller
        // set the hint). Keying on the leaf alone let `BUS.clk` bind to any
        // signal ending in `.clk` — another interface's `clk` — so the real
        // clock never toggled.
        let sid = self.clock_gen_signal_id(tog_hier)?;
        // Apply seed: sets signal_table[sid] + marks dirty so settle sees it.
        let width = self.signal_widths[sid];
        let seed = seed_val.resize(width);
        if self.sva_sites.is_empty() && self.signal_table[sid] != seed {
            write_sig!(self, sid, seed);
            self.mark_dirty_id(sid);
            self.table_modified = true;
        }
        Some(ClockGen {
            signal_id: sid,
            half_period,
            next_toggle_time: half_period,
            edge_signal_position: usize::MAX,
            ahead: None,
            t0_rank: usize::MAX,
        })
    }

    /// The flat dotted name of a clock-generator operand (`clk`, `BUS.clk`),
    /// or None when a segment carries a select (a bit of a vector is not a
    /// whole-signal clock).
    pub(super) fn clock_gen_ident(h: &HierarchicalIdentifier) -> Option<String> {
        if h.path.iter().any(|s| !s.selects.is_empty()) {
            return None;
        }
        Some(
            h.path
                .iter()
                .map(|s| s.name.name.as_str())
                .collect::<Vec<_>>()
                .join("."),
        )
    }

    /// Signal id of a clock-generator target, resolved like any other
    /// reference under the current name-resolution hint.
    pub(super) fn clock_gen_signal_id(&self, h: &HierarchicalIdentifier) -> Option<usize> {
        let name = self.resolve_hier_name(h);
        self.signal_name_to_id.get(name.as_ref()).copied()
    }

    /// Try to detect `always #N var = ~var` pattern and extract as a ClockGen.
    /// True if a `#(delay)` expression reads a runtime signal/variable (so its
    /// value can CHANGE during simulation — e.g. `always #(half) clk = ~clk`
    /// with `half` reconfigured by a PLL testbench). Such a clock must NOT be
    /// baked into a fixed-period `ClockGen`; it has to re-evaluate the delay on
    /// every toggle (the `FastDelayAlways` / general Forever path). A delay of
    /// only literals/parameters (folded at elaboration, not in
    /// `signal_name_to_id`) is constant and safe to freeze.
    pub(super) fn delay_expr_is_dynamic(&self, d: &Expression) -> bool {
        let mut reads: HashSet<String> = HashSet::default();
        Self::collect_expr_reads(d, &self.module, &mut reads);
        reads.iter().any(|name| {
            self.signal_name_to_id.contains_key(name.as_str()) || {
                let suffix = format!(".{}", name);
                self.signal_name_to_id.any_name_ends_with(&suffix)
                    // An elided input port ending in this name counted
                    // before it was left out; keep the same verdict.
                    || (!self.module.elided_port_leaves.is_empty()
                        && self
                            .module
                            .elided_port_leaves
                            .contains(name.rsplit('.').next().unwrap_or(name)))
            }
        })
    }

    pub(super) fn try_extract_clock_gen(
        &self,
        body: &Statement,
        half_period: u64,
    ) -> Option<ClockGen> {
        // Body should be: var = ~var (blocking assign)
        let assign = match &body.kind {
            StatementKind::BlockingAssign { lvalue, rvalue, .. } => Some((lvalue, rvalue)),
            StatementKind::SeqBlock { stmts, .. } if stmts.len() == 1 => {
                if let StatementKind::BlockingAssign { lvalue, rvalue, .. } = &stmts[0].kind {
                    Some((lvalue, rvalue))
                } else {
                    None
                }
            }
            _ => None,
        }?;
        let (lhs, rhs) = assign;

        // LHS must be a simple identifier
        let lhs_hier = match &lhs.kind {
            ExprKind::Ident(hier) => hier,
            _ => return None,
        };
        let lhs_name = Self::clock_gen_ident(lhs_hier)?;
        let signal_id = self.clock_gen_signal_id(lhs_hier)?;

        // RHS must be ~LHS or !LHS
        match &rhs.kind {
            ExprKind::Unary {
                op: UnaryOp::BitNot,
                operand,
            }
            | ExprKind::Unary {
                op: UnaryOp::LogNot,
                operand,
            } => {
                if let ExprKind::Ident(hier) = &operand.kind {
                    let rhs_name = Self::clock_gen_ident(hier)?;
                    if rhs_name == lhs_name {
                        return Some(ClockGen {
                            signal_id,
                            half_period,
                            next_toggle_time: half_period, // first toggle at t=half_period
                            edge_signal_position: usize::MAX,
                            ahead: None,
                            t0_rank: usize::MAX,
                        });
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// Toggle clock generators that fire at the current time.
    /// Toggle every clock generator whose next_toggle_time matches the
    /// current sim time.  Fills `toggled_clock_positions` with the
    /// `edge_signal_ids` positions of clocks that toggled this call —
    /// `event_loop` uses that to restrict the next check_edges scan to just
    /// those positions when no other state changed this iter.
    pub(super) fn fire_clock_generators(&mut self) {
        self.toggled_clock_positions.clear();
        self.fire_due_clocks(false);
    }

    /// Toggle the generators due now — with `ready_only`, only those whose
    /// FIFO rank (`ClockGen::ahead`) has been reached. Returns whether a due
    /// generator was left for later in this slot.
    pub(super) fn fire_due_clocks(&mut self, ready_only: bool) -> bool {
        if self.finished {
            return false;
        }
        let mut held = false;
        let trace = self.sched_trace_on();
        // Collected inside the &mut borrow of `clock_generators`, printed
        // after it ends (the names live in `id_to_name`).
        let mut fired: Vec<(usize, u64)> = Vec::new();
        let mut tree_roots = std::mem::take(&mut self.cg_tree_roots_buf);
        tree_roots.clear();
        for cg in &mut self.clock_generators {
            if cg.next_toggle_time == self.time {
                if ready_only && cg.ahead != Some(0) {
                    held = true;
                    continue;
                }
                let w = self.signal_widths[cg.signal_id];
                // `always #N clk = ~clk` must apply the real 4-state NOT
                // (LRM §11.5.1): 0->1, 1->0, X->X, Z->X. The old shortcut
                // treated any non-One (including X / Z on an uninitialized
                // 4-state `logic`) as 0 and flipped it to 1, synthesising a
                // clock out of a signal that per LRM stays X forever and so
                // never produces an edge. Preserving X keeps the clockgen
                // from inventing posedges where the reference idles. Keep the
                // O(1) fast path for genuine 2-state 0/1 clocks.
                let cur = self.signal_table[cg.signal_id].bits_first();
                let new_val = match cur {
                    LogicBit::Zero => Value::from_u64(1, w),
                    LogicBit::One => Value::from_u64(0, w),
                    _ => Value::all_x(w), // ~X = X ; ~Z = X
                };
                // Unknown clock: ~X == X is a no-op, but ~Z == X is a REAL
                // value change (anyedge waiters, VCD), so write only when the
                // value differs — and keep RE-ARMING rather than retiring:
                // the extraction consumed the always block, so a retired
                // generator could never run again, yet the standard
                // start-the-clock-after-reset idiom seeds an X clock from the
                // testbench later (`initial begin #20; clk = 0; end`) and
                // must revive it. One bit compare per half-period is noise.
                if !cur.is_known() {
                    if self.signal_table[cg.signal_id] != new_val {
                        write_sig!(self, cg.signal_id, new_val);
                        if !self.dirty_signals[cg.signal_id] {
                            self.dirty_signals[cg.signal_id] = true;
                            self.dirty_list.push(cg.signal_id);
                        }
                        self.dirty_any = true;
                        self.table_modified = true;
                    }
                    cg.next_toggle_time += cg.half_period;
                    cg.ahead = Some(self.event_queue.len_at(cg.next_toggle_time) as u32);
                    continue;
                }
                write_sig!(self, cg.signal_id, new_val);
                if !self.dirty_signals[cg.signal_id] {
                    self.dirty_signals[cg.signal_id] = true;
                    self.dirty_list.push(cg.signal_id);
                }
                self.dirty_any = true;
                self.table_modified = true;
                cg.next_toggle_time += cg.half_period;
                cg.ahead = Some(self.event_queue.len_at(cg.next_toggle_time) as u32);
                if trace {
                    fired.push((cg.signal_id, u64::from(cur != LogicBit::One)));
                }
                if cg.edge_signal_position != usize::MAX {
                    self.toggled_clock_positions.push(cg.edge_signal_position);
                }
                tree_roots.push(cg.signal_id);
            }
        }
        for &root in tree_roots.iter() {
            if self
                .is_clock_tree_signal
                .get(root)
                .copied()
                .unwrap_or(false)
            {
                self.eval_clock_tree(root);
            }
        }
        self.cg_tree_roots_buf = tree_roots;
        for (sig, v) in fired {
            eprintln!(
                "[sched] t={} | clockgen {} -> {}",
                self.time,
                self.name_opt(sig).unwrap_or("?"),
                v
            );
        }
        held
    }

    /// Time-0 activation order of the processes queued at time 0 (§4.4.2
    /// leaves it unspecified; this is the reference simulator's). Each root
    /// runs as one block, roots in elaboration order. Within a root the
    /// processes follow the depth-first source order of the instance tree —
    /// a child instance's processes at its instantiation, a bound instance's
    /// after all of its host's own items — except that the `initial` blocks
    /// that cannot suspend all run as one group, at the position of the
    /// first of them. Static initializers keep running first. Clock
    /// generators take their slot in the same order (`ClockGen::t0_rank`).
    pub(super) fn order_time0_processes(&mut self) {
        let pids = self.event_queue.slot_pids(0);
        let (rank, gen_ranks) = {
            // instance path -> (parent path, position in the parent, index)
            let mut inst: HashMap<&str, (&str, usize, usize)> = HashMap::default();
            for (i, e) in self.module.instances.iter().enumerate() {
                inst.insert(e.path.as_str(), (e.parent.as_str(), e.src_pos, i));
            }
            // Several tops elaborate under a synthetic wrapper whose
            // children are the roots.
            let multi_top = matches!(
                self.module.name.as_str(),
                "__xezim_multi_top" | "__xz_multitop__"
            );
            // The innermost instance enclosing a scope (generate scopes are
            // part of their instance's source).
            fn nearest<'a>(
                inst: &HashMap<&str, (&str, usize, usize)>,
                mut s: &'a str,
            ) -> Option<&'a str> {
                loop {
                    if s.is_empty() {
                        return None;
                    }
                    if inst.contains_key(s) {
                        return Some(s);
                    }
                    s = s.rfind('.').map_or("", |i| &s[..i]);
                }
            }
            // (root, source positions from the root down to the process)
            let position = |scope: &str, span: usize| -> (usize, Vec<usize>) {
                let mut chain = vec![span];
                let mut root = 0;
                let mut cur = nearest(&inst, scope);
                while let Some(p) = cur {
                    let (parent, src_pos, idx) = inst[p];
                    if multi_top && parent.is_empty() {
                        root = idx + 1;
                        break;
                    }
                    chain.push(src_pos);
                    cur = nearest(&inst, parent);
                }
                chain.reverse();
                (root, chain)
            };
            let mut placed: Vec<(usize, Option<(usize, Vec<usize>, bool)>)> =
                Vec::with_capacity(pids.len());
            let mut anchor: HashMap<usize, Vec<usize>> = HashMap::default();
            for &pid in &pids {
                let info = match self.process_origin.get(&pid) {
                    Some((span, kind @ ("initial block" | "always block"))) => {
                        let scope = self.process_scope_hint.get(&pid).map_or("", String::as_str);
                        let (root, pos) = position(scope, span.start);
                        let grouped =
                            *kind == "initial block" && self.t0_timing_free.contains(&pid);
                        if grouped {
                            let a = anchor.entry(root).or_insert_with(|| pos.clone());
                            if pos < *a {
                                *a = pos.clone();
                            }
                        }
                        Some((root, pos, grouped))
                    }
                    _ => None,
                };
                placed.push((pid, info));
            }
            type Key = (u8, usize, Vec<usize>, Vec<usize>, usize);
            let key_of = |i: usize, info: &Option<(usize, Vec<usize>, bool)>| -> Key {
                match info {
                    None => (0, 0, Vec::new(), Vec::new(), i),
                    Some((root, pos, grouped)) => {
                        let at = if *grouped {
                            anchor[root].clone()
                        } else {
                            pos.clone()
                        };
                        (1, *root, at, pos.clone(), i)
                    }
                }
            };
            let mut keyed: Vec<(Key, usize)> = placed
                .iter()
                .enumerate()
                .map(|(i, (pid, info))| (key_of(i, info), *pid))
                .collect();
            keyed.sort();
            let gen_ranks: Vec<(usize, usize)> = self
                .clock_gen_src
                .iter()
                .map(|(gi, scope, span)| {
                    let (root, pos) = position(scope, *span);
                    let k: Key = (1, root, pos.clone(), pos, usize::MAX);
                    (*gi, keyed.partition_point(|(kk, _)| *kk < k))
                })
                .collect();
            let rank: HashMap<usize, usize> = keyed
                .iter()
                .enumerate()
                .map(|(r, (_, pid))| (*pid, r))
                .collect();
            (rank, gen_ranks)
        };
        self.event_queue.reorder_slot(0, &rank);
        for (gi, r) in gen_ranks {
            if let Some(cg) = self.clock_generators.get_mut(gi) {
                cg.t0_rank = r;
            }
        }
        self.t0_pid_ord = rank;
        self.t0_timing_free = HashSet::default();
    }

    /// Account one event popped from the current slot against the FIFO rank
    /// of the generators due now, and settle the time-0 rank of `initial`
    /// generators once the drain reaches their source position. Called
    /// BEFORE the popped process runs, so its own new events queue behind.
    pub(super) fn clock_rank_on_pop(&mut self, pid: usize) {
        if self.event_queue.last_pop_front {
            return;
        }
        let now = self.time;
        // A process spawned during the drain runs after every time-0 one.
        let ord = self.t0_pid_ord.get(&pid).copied().unwrap_or(usize::MAX - 1);
        for cg in &mut self.clock_generators {
            match cg.ahead {
                Some(ref mut a) if cg.next_toggle_time == now => {
                    *a = a.saturating_sub(1);
                }
                None if ord >= cg.t0_rank => {
                    cg.ahead = Some(self.event_queue.len_at(cg.next_toggle_time) as u32);
                }
                _ => {}
            }
        }
    }

    /// Settle every rank still unknown at the end of the time-0 drain.
    pub(super) fn settle_clock_ranks(&mut self) {
        self.t0_pid_ord = HashMap::default();
        for cg in &mut self.clock_generators {
            if cg.ahead.is_none() {
                cg.ahead = Some(self.event_queue.len_at(cg.next_toggle_time) as u32);
            }
        }
    }

    /// Classify a single always-block: route to edge_blocks / clock_generators
    /// / event_queue / clinic, returning Some(rest) if the block remained
    /// "comb-equivalent" and should stay in `module.always_blocks`. Extracted
    /// from `classify_always_blocks` so the lazy-prefix streaming path (#7)
    /// can materialize one PendingAlways at a time and feed it through here
    /// without staging the full materialized vec.
    /// Whether a body READS something it also WRITES (`cnt++`, `x = x + 1`).
    /// Such a block is not idempotent, so re-running it on every settle
    /// iteration — as the comb path does — diverges: each run dirties its own
    /// input and schedules another, up to `settle_limit`.
    pub(super) fn body_is_self_referential(stmt: &Statement) -> bool {
        fn base_of(e: &Expression) -> Option<&str> {
            match &e.kind {
                ExprKind::Ident(h) => h.path.last().map(|s| s.name.name.as_str()),
                ExprKind::Index { expr, .. }
                | ExprKind::RangeSelect { expr, .. }
                | ExprKind::MemberAccess { expr, .. } => base_of(expr),
                _ => None,
            }
        }
        /// Does `e` read a variable whose base name is `target`?
        fn reads_name(e: &Expression, target: &str) -> bool {
            match &e.kind {
                ExprKind::Ident(h) => h.path.last().is_some_and(|s| s.name.name == target),
                ExprKind::Paren(i) => reads_name(i, target),
                ExprKind::Unary { operand, .. } => reads_name(operand, target),
                ExprKind::Binary { left, right, .. } => {
                    reads_name(left, target) || reads_name(right, target)
                }
                ExprKind::Conditional {
                    condition,
                    then_expr,
                    else_expr,
                } => {
                    reads_name(condition, target)
                        || reads_name(then_expr, target)
                        || reads_name(else_expr, target)
                }
                ExprKind::Index { expr, index } => {
                    reads_name(expr, target) || reads_name(index, target)
                }
                ExprKind::RangeSelect {
                    expr, left, right, ..
                } => {
                    reads_name(expr, target)
                        || reads_name(left, target)
                        || reads_name(right, target)
                }
                ExprKind::MemberAccess { expr, .. } => reads_name(expr, target),
                ExprKind::Concatenation(parts) => parts.iter().any(|p| reads_name(p, target)),
                ExprKind::Replication { count, exprs } => {
                    reads_name(count, target) || exprs.iter().any(|p| reads_name(p, target))
                }
                ExprKind::Call { args, .. } => args.iter().any(|a| reads_name(a, target)),
                ExprKind::SystemCall { args, .. } => args.iter().any(|a| reads_name(a, target)),
                _ => false,
            }
        }
        fn walk(st: &Statement) -> bool {
            match &st.kind {
                // `cnt++` / `--cnt` read and write the same variable.
                StatementKind::Expr(e) => matches!(
                    &e.kind,
                    ExprKind::Unary { op, .. }
                        if matches!(op, UnaryOp::PreIncr | UnaryOp::PreDecr
                                       | UnaryOp::PostIncr | UnaryOp::PostDecr)
                ),
                StatementKind::BlockingAssign { lvalue, rvalue }
                | StatementKind::NonblockingAssign { lvalue, rvalue, .. } => {
                    let Some(target) = base_of(lvalue) else {
                        return false;
                    };
                    reads_name(rvalue, target)
                }
                StatementKind::SeqBlock { stmts, .. } => stmts.iter().any(walk),
                StatementKind::If {
                    then_stmt,
                    else_stmt,
                    ..
                } => walk(then_stmt) || else_stmt.as_ref().is_some_and(|e| walk(e)),
                StatementKind::TimingControl { stmt, .. } => walk(stmt),
                _ => false,
            }
        }
        walk(stmt)
    }

    /// The shape of an always block's leading event control. Only an
    /// explicit list of PLAIN identifiers (edge qualifier allowed) can be
    /// compared with the body's read set; a term carrying a select or an
    /// expression (`@(vco_tap[c_ph_val[0]])`, `@(sig[5:0])`, `@({a,b})`) fires
    /// on that value, which the edge path narrows or a value waiter
    /// evaluates (tests/prtest/pr2011429.v: `c_ph_val[0]` selects which tap
    /// is watched, so the waiter follows the index).
    pub(super) fn event_list_shape(stmt: &Statement) -> EventListShape {
        fn check(control: &TimingControl) -> EventListShape {
            let TimingControl::Event(EventControl::EventExpr(exprs)) = control else {
                // `@*`, a named event, or any other form: leave the existing
                // routing untouched.
                return EventListShape::Implicit;
            };
            let plain = exprs.iter().all(|ee| {
                let mut e = &ee.expr;
                while let ExprKind::Paren(inner) = &e.kind {
                    e = inner;
                }
                matches!(&e.kind, ExprKind::Ident(_))
            });
            if plain {
                EventListShape::Plain
            } else {
                EventListShape::NonPlain
            }
        }
        match &stmt.kind {
            StatementKind::TimingControl { control, .. } => check(control),
            StatementKind::SeqBlock { stmts, .. } => match stmts.first().map(|s| &s.kind) {
                Some(StatementKind::TimingControl { control, .. }) => check(control),
                _ => EventListShape::Implicit,
            },
            _ => EventListShape::Implicit,
        }
    }

    /// §9.4.2: the PHYSICAL bits of signal `sid` a level event term `e`
    /// watches, when `e` is a constant bit- or part-select of that signal as
    /// a plain vector (`v[3]`, `v[W-1]`, `v[7:4]`, `v[b +: 2]`), read through
    /// its declared range. None for every other shape — a runtime index, an
    /// element of a packed multi-D vector or of an array — which must wait on the
    /// expression's value instead.
    pub(super) fn narrow_event_mask(
        &self,
        e: &Expression,
        sid: usize,
        scope: &str,
    ) -> Option<event_bits::EventMask> {
        let mut e = e;
        while let ExprKind::Paren(inner) = &e.kind {
            e = inner;
        }
        match &e.kind {
            ExprKind::Index { expr, .. } | ExprKind::RangeSelect { expr, .. }
                if matches!(expr.kind, ExprKind::Ident(_)) => {}
            _ => return None,
        }
        Some(event_bits::EventMask::from_bits(
            self.select_chain_bits(e, sid, scope)?,
        ))
    }

    /// The PHYSICAL bits of signal `sid` a constant select chain rooted at
    /// it reads, LSB first: `v[7:4]`, or the port-substitution shapes
    /// `v[32:0][31]` and `v[15:8][3:2]`, where an outer select counts
    /// positions in the value of the inner one. None when an index is not
    /// constant, a select leaves its operand, or the root is no plain vector.
    pub(super) fn select_chain_bits(
        &self,
        e: &Expression,
        sid: usize,
        scope: &str,
    ) -> Option<Vec<i64>> {
        let (base, kind, l, r) = match &e.kind {
            ExprKind::Paren(x) => return self.select_chain_bits(x, sid, scope),
            ExprKind::Index { expr, index } => {
                let k = self.event_const_index(index, scope)?;
                (expr, RangeKind::Constant, k, k)
            }
            ExprKind::RangeSelect {
                expr,
                kind,
                left,
                right,
            } => (
                expr,
                *kind,
                self.event_const_index(left, scope)?,
                self.event_const_index(right, scope)?,
            ),
            _ => return None,
        };
        let mut b = &**base;
        while let ExprKind::Paren(x) = &b.kind {
            b = x;
        }
        if let ExprKind::Ident(h) = &b.kind {
            if h.path.last().is_some_and(|s| !s.selects.is_empty()) {
                return None;
            }
            let width = *self.signal_widths.get(sid)?;
            if self.signal_real.get(sid).copied().unwrap_or(false) {
                return None;
            }
            let dim = event_bits::vector_decl_dim(&self.module, self.name_for_id(sid), width)?;
            let (lo, hi) = event_bits::select_label_span(dim, kind, l, r)?;
            let mut bits: Vec<i64> = Vec::new();
            for label in lo..=hi {
                let phys = event_bits::label_to_phys(dim, label);
                if !(0..width as i64).contains(&phys) || bits.len() > 4096 {
                    return None;
                }
                bits.push(phys);
            }
            bits.sort_unstable();
            return Some(bits);
        }
        // A select of an unnamed value counts positions from its LSB.
        let inner = self.select_chain_bits(b, sid, scope)?;
        let (lo, hi) = event_bits::select_label_span((i64::MAX, 0), kind, l, r)?;
        if lo < 0 || hi >= inner.len() as i64 {
            return None;
        }
        Some(inner[lo as usize..=hi as usize].to_vec())
    }

    /// The bits each signal of an all-level event list is watched through:
    /// `(signal id, mask)` for a signal named only by constant selects
    /// (`@(a[1:0] or b)` gives `a`'s two bits; `b`, named whole, needs
    /// none). None when some term is neither a plain name nor such a select
    /// (`@(mem[i])`, `@(a + b)`) or names no signal.
    pub(super) fn comb_select_masks(
        &self,
        sens: &[Sensitivity],
        scope: &str,
    ) -> Option<Vec<(usize, Vec<u64>)>> {
        let mut masks: Vec<(usize, Vec<u64>)> = Vec::new();
        let mut whole: Vec<usize> = Vec::new();
        for s in sens {
            let sid = self.sens_term_sid(&s.signal_name, scope)?;
            match &s.value_of {
                None => whole.push(sid),
                Some(e) => {
                    let bits = self.select_chain_bits(e, sid, scope)?;
                    let words = (*self.signal_widths.get(sid)? as usize).div_ceil(64);
                    let entry = match masks.iter().position(|(id, _)| *id == sid) {
                        Some(k) => &mut masks[k].1,
                        None => {
                            masks.push((sid, vec![0u64; words]));
                            &mut masks.last_mut().unwrap().1
                        }
                    };
                    for b in bits {
                        entry[b as usize / 64] |= 1u64 << (b % 64);
                    }
                }
            }
        }
        masks.retain(|(id, _)| !whole.contains(id));
        Some(masks)
    }

    /// Does an always body read the signals of `masks` only through
    /// constant selects inside their masks, and do nothing a repeated run
    /// could observe — only blocking/nonblocking assignments, `if` and
    /// `case`, with no call other than a pure system function? `resolve`
    /// maps a name the way the comb path's read set does.
    pub(super) fn body_reads_within_masks<'m>(
        &self,
        body: &Statement,
        masks: &'m [(usize, Vec<u64>)],
        scope: &str,
        resolve: &dyn Fn(&str) -> Vec<usize>,
    ) -> bool {
        let masked = |h: &HierarchicalIdentifier| -> Option<(usize, &'m [u64])> {
            let raw = Self::resolve_hier_name_static(h, &self.module);
            resolve(&raw).into_iter().find_map(|id| {
                masks
                    .iter()
                    .find(|(m, _)| *m == id)
                    .map(|(m, w)| (*m, w.as_slice()))
            })
        };
        fn expr_ok<'m>(
            sim: &Simulator,
            e: &Expression,
            masks: &'m [(usize, Vec<u64>)],
            scope: &str,
            masked: &dyn Fn(&HierarchicalIdentifier) -> Option<(usize, &'m [u64])>,
        ) -> bool {
            let ok = |x: &Expression| expr_ok(sim, x, masks, scope, masked);
            match &e.kind {
                ExprKind::Number(_) | ExprKind::StringLiteral(_) => true,
                ExprKind::Ident(h) => masked(h).is_none(),
                ExprKind::Paren(x) => ok(x),
                ExprKind::Unary { operand, .. } => ok(operand),
                ExprKind::Binary { left, right, .. } => ok(left) && ok(right),
                ExprKind::Conditional {
                    condition,
                    then_expr,
                    else_expr,
                } => ok(condition) && ok(then_expr) && ok(else_expr),
                ExprKind::Concatenation(parts) => parts.iter().all(ok),
                ExprKind::Replication { count, exprs } => ok(count) && exprs.iter().all(ok),
                ExprKind::Inside { expr, ranges } => ok(expr) && ranges.iter().all(ok),
                ExprKind::Range(l, r) => ok(l) && ok(r),
                ExprKind::Index { .. } | ExprKind::RangeSelect { .. } => {
                    // A select chain rooted at a listed signal may read only
                    // listed bits; any other select is an ordinary read.
                    let mut root = e;
                    loop {
                        match &root.kind {
                            ExprKind::Index { expr, .. } | ExprKind::RangeSelect { expr, .. } => {
                                root = expr
                            }
                            ExprKind::Paren(x) => root = x,
                            _ => break,
                        }
                    }
                    match &root.kind {
                        ExprKind::Ident(h) if masked(h).is_some() => {
                            let (sid, want) = masked(h).unwrap();
                            sim.select_chain_bits(e, sid, scope).is_some_and(|bits| {
                                bits.iter()
                                    .all(|&b| want[b as usize / 64] >> (b % 64) & 1 == 1)
                            })
                        }
                        _ => match &e.kind {
                            ExprKind::Index { expr, index } => ok(expr) && ok(index),
                            ExprKind::RangeSelect {
                                expr, left, right, ..
                            } => ok(expr) && ok(left) && ok(right),
                            _ => false,
                        },
                    }
                }
                ExprKind::SystemCall { name, args } => {
                    matches!(
                        name.as_str(),
                        "$signed"
                            | "$unsigned"
                            | "$bits"
                            | "$clog2"
                            | "$countones"
                            | "$onehot"
                            | "$onehot0"
                            | "$isunknown"
                    ) && args.iter().all(ok)
                }
                _ => false,
            }
        }
        fn stmt_ok<'m>(
            sim: &Simulator,
            st: &Statement,
            masks: &'m [(usize, Vec<u64>)],
            scope: &str,
            masked: &dyn Fn(&HierarchicalIdentifier) -> Option<(usize, &'m [u64])>,
        ) -> bool {
            let e_ok = |x: &Expression| expr_ok(sim, x, masks, scope, masked);
            let s_ok = |x: &Statement| stmt_ok(sim, x, masks, scope, masked);
            // A target's indices are reads; the target itself must not be a
            // listed signal.
            let lv_ok = |lv: &Expression| -> bool {
                fn base_and_indices<'a>(
                    e: &'a Expression,
                    idx: &mut Vec<&'a Expression>,
                ) -> Option<&'a HierarchicalIdentifier> {
                    match &e.kind {
                        ExprKind::Ident(h) => Some(h),
                        ExprKind::Paren(x) => base_and_indices(x, idx),
                        ExprKind::Index { expr, index } => {
                            idx.push(index);
                            base_and_indices(expr, idx)
                        }
                        ExprKind::RangeSelect {
                            expr, left, right, ..
                        } => {
                            idx.push(left);
                            idx.push(right);
                            base_and_indices(expr, idx)
                        }
                        _ => None,
                    }
                }
                let parts: Vec<&Expression> = match &lv.kind {
                    ExprKind::Concatenation(parts) => parts.iter().collect(),
                    _ => vec![lv],
                };
                parts.into_iter().all(|p| {
                    let mut idx = Vec::new();
                    base_and_indices(p, &mut idx)
                        .is_some_and(|h| masked(h).is_none() && idx.into_iter().all(e_ok))
                })
            };
            match &st.kind {
                StatementKind::Null => true,
                StatementKind::BlockingAssign { lvalue, rvalue } => lv_ok(lvalue) && e_ok(rvalue),
                StatementKind::NonblockingAssign {
                    lvalue,
                    delay: None,
                    rvalue,
                } => lv_ok(lvalue) && e_ok(rvalue),
                StatementKind::SeqBlock { stmts, .. } => stmts.iter().all(s_ok),
                StatementKind::If {
                    condition,
                    then_stmt,
                    else_stmt,
                    ..
                } => e_ok(condition) && s_ok(then_stmt) && else_stmt.as_deref().is_none_or(s_ok),
                StatementKind::Case { expr, items, .. } => {
                    e_ok(expr)
                        && items.iter().all(|it| {
                            it.pattern.is_none() && it.patterns.iter().all(e_ok) && s_ok(&it.stmt)
                        })
                }
                _ => false,
            }
        }
        stmt_ok(self, body, masks, scope, &masked)
    }

    /// Count the statement-level timing controls in a subtree (waits an FSM
    /// would own). Intra-assignment delays are canonicalized into marker
    /// CALLS by the elaborator and do not appear as TimingControl nodes.
    /// Does the subtree contain a `#delay` STATEMENT control? Used to route
    /// edge-sensitive always bodies to the process path: on the edge path a
    /// body delay executes synchronously and advances global time while the
    /// SIBLING blocks of the same edge have not run yet — an
    /// `always @(posedge clk) begin #1; end` observer displaced every other
    /// flop's `$time`/NBA commit on that edge by 1ps (same displacement class
    /// as the cv8s blocking-task skew, which the task-call predicate below
    /// already routes). Intra-assignment delays (`q <= #1 d;`) are transport
    /// NBAs and stay on the edge path.
    pub(super) fn stmt_contains_delay_control(stmt: &Statement) -> bool {
        match &stmt.kind {
            StatementKind::TimingControl {
                control,
                stmt: inner,
            } => {
                matches!(control, TimingControl::Delay(_))
                    || Self::stmt_contains_delay_control(inner)
            }
            // §9.4.5 `lhs = #d rhs` suspends like a `#d` statement; the edge
            // path's synchronous executor cannot, so it dropped the delay
            // (issue #160). The nonblocking form schedules without
            // suspending and stays on the edge path.
            StatementKind::BlockingAssign { rvalue, .. } => {
                Self::intra_delay_marker(rvalue).is_some()
            }
            StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
                stmts.iter().any(Self::stmt_contains_delay_control)
            }
            StatementKind::If {
                then_stmt,
                else_stmt,
                ..
            } => {
                Self::stmt_contains_delay_control(then_stmt)
                    || else_stmt
                        .as_ref()
                        .is_some_and(|e| Self::stmt_contains_delay_control(e))
            }
            StatementKind::Case { items, .. } => items
                .iter()
                .any(|it| Self::stmt_contains_delay_control(&it.stmt)),
            StatementKind::For { body, .. }
            | StatementKind::While { body, .. }
            | StatementKind::DoWhile { body, .. }
            | StatementKind::Repeat { body, .. }
            | StatementKind::Forever { body }
            | StatementKind::Foreach { body, .. } => Self::stmt_contains_delay_control(body),
            _ => false,
        }
    }

    /// Does the subtree contain a statement-level `@(...)` EVENT control?
    /// Same displacement class as `stmt_contains_delay_control`, nested-wait
    /// flavor: on the edge path a body `@(posedge clk)` / `repeat (n)
    /// @(posedge clk)` executes as if it were absent (the synchronous
    /// executor cannot suspend), so a credit-paced response block streamed
    /// its beats back-to-back and its `resp_vld` handshake alternated. Such
    /// a body must be a PROCESS: the park machinery suspends at the inner
    /// wait and §9.2.2 miss-edges-while-busy semantics match the reference.
    /// The block's OWN head control was already stripped by
    /// `extract_sensitivity`, so any event control seen here is nested.
    pub(super) fn stmt_contains_event_control(stmt: &Statement) -> bool {
        match &stmt.kind {
            StatementKind::TimingControl {
                control,
                stmt: inner,
            } => {
                matches!(control, TimingControl::Event(_))
                    || Self::stmt_contains_event_control(inner)
            }
            // §9.4.5 `lhs = @(e) rhs` waits like an `@(e)` statement.
            StatementKind::BlockingAssign { rvalue, .. } => matches!(
                &rvalue.kind,
                ExprKind::SystemCall { name, .. }
                    if name == crate::intra_delay::INTRA_EVENT_MARKER
            ),
            StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
                stmts.iter().any(Self::stmt_contains_event_control)
            }
            StatementKind::If {
                then_stmt,
                else_stmt,
                ..
            } => {
                Self::stmt_contains_event_control(then_stmt)
                    || else_stmt
                        .as_ref()
                        .is_some_and(|e| Self::stmt_contains_event_control(e))
            }
            StatementKind::Case { items, .. } => items
                .iter()
                .any(|it| Self::stmt_contains_event_control(&it.stmt)),
            StatementKind::For { body, .. }
            | StatementKind::While { body, .. }
            | StatementKind::DoWhile { body, .. }
            | StatementKind::Repeat { body, .. }
            | StatementKind::Forever { body }
            | StatementKind::Foreach { body, .. } => Self::stmt_contains_event_control(body),
            _ => false,
        }
    }

    /// Shape admitted to the process FSM by default: the plain stimulus
    /// loop of a testbench — blocking/non-blocking assignments, if/case,
    /// counted loops, `#` and `@` timing, system tasks — and nothing the
    /// FSM does not model exactly: fork/join, disable, wait, named events,
    /// named blocks, clocking cycle delays, assertions, force/release,
    /// virtual-interface handles.
    pub(super) fn stmt_is_plain_stimulus(&self, stmt: &Statement) -> bool {
        use crate::ast::stmt::{EventControl, TimingControl};
        match &stmt.kind {
            StatementKind::Null | StatementKind::Break | StatementKind::Continue => true,
            StatementKind::Expr(e) => {
                // Postponed-region observers keep the AST path: its
                // per-statement propagation is what their pinned traces
                // (UDP edge tables with two inputs changing) rely on.
                if let ExprKind::SystemCall { name, .. } = &e.kind {
                    if matches!(
                        name.as_str(),
                        "$monitor" | "$strobe" | "$monitoron" | "$monitoroff"
                    ) {
                        return false;
                    }
                }
                !self.expr_touches_viface(e)
            }
            StatementKind::BlockingAssign { lvalue, rvalue } => {
                !self.expr_touches_viface(lvalue) && !self.expr_touches_viface(rvalue)
            }
            StatementKind::NonblockingAssign {
                lvalue,
                delay,
                rvalue,
            } => {
                delay.is_none()
                    && !self.expr_touches_viface(lvalue)
                    && !self.expr_touches_viface(rvalue)
            }
            StatementKind::If {
                condition,
                then_stmt,
                else_stmt,
                ..
            } => {
                !self.expr_touches_viface(condition)
                    && self.stmt_is_plain_stimulus(then_stmt)
                    && else_stmt
                        .as_ref()
                        .is_none_or(|e| self.stmt_is_plain_stimulus(e))
            }
            StatementKind::Case { expr, items, .. } => {
                !self.expr_touches_viface(expr)
                    && items.iter().all(|it| self.stmt_is_plain_stimulus(&it.stmt))
            }
            StatementKind::For { body, .. }
            | StatementKind::While { body, .. }
            | StatementKind::DoWhile { body, .. }
            | StatementKind::Repeat { body, .. }
            | StatementKind::Forever { body } => self.stmt_is_plain_stimulus(body),
            StatementKind::SeqBlock { name, stmts } => {
                name.is_none() && stmts.iter().all(|st| self.stmt_is_plain_stimulus(st))
            }
            StatementKind::TimingControl { control, stmt } => {
                let ok = match control {
                    TimingControl::Delay(e) => !self.expr_touches_viface(e),
                    TimingControl::Event(EventControl::Identifier(id)) => {
                        !id.name.starts_with("__xz_")
                    }
                    TimingControl::Event(_) => true,
                };
                ok && self.stmt_is_plain_stimulus(stmt)
            }
            StatementKind::VarDecl { .. } => true,
            _ => false,
        }
    }

    /// True when an identifier in `e` is a `virtual <iface>` variable: the
    /// compiled path stores through it as a plain signal, while §25.9 makes
    /// the assignment a name alias the interpreter tracks.
    pub(super) fn expr_touches_viface(&self, e: &Expression) -> bool {
        let mut names: HashSet<String> = HashSet::default();
        Self::collect_expr_reads(e, &self.module, &mut names);
        names.iter().any(|n| {
            let base = n.split('.').next().unwrap_or(n);
            self.module
                .var_decl_types
                .get(base)
                .is_some_and(|dt| self.is_virtual_iface_type(dt))
        })
    }

    pub(super) fn count_timing_controls(stmt: &Statement) -> usize {
        match &stmt.kind {
            StatementKind::TimingControl { stmt: inner, .. } => {
                1 + Self::count_timing_controls(inner)
            }
            StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
                stmts.iter().map(Self::count_timing_controls).sum()
            }
            StatementKind::If {
                then_stmt,
                else_stmt,
                ..
            } => {
                Self::count_timing_controls(then_stmt)
                    + else_stmt
                        .as_ref()
                        .map(|e| Self::count_timing_controls(e))
                        .unwrap_or(0)
            }
            StatementKind::Case { items, .. } => items
                .iter()
                .map(|it| Self::count_timing_controls(&it.stmt))
                .sum(),
            StatementKind::For { body, .. }
            | StatementKind::While { body, .. }
            | StatementKind::DoWhile { body, .. }
            | StatementKind::Repeat { body, .. }
            | StatementKind::Forever { body }
            | StatementKind::Foreach { body, .. } => Self::count_timing_controls(body),
            _ => 0,
        }
    }

    /// Does the subtree contain a statement-level task-enable / void call?
    /// (`pulse(x);` / `drv.step();`) — such a callee may hide waits that
    /// `count_timing_controls` cannot see.
    pub(super) fn stmt_contains_call(stmt: &Statement) -> bool {
        match &stmt.kind {
            StatementKind::Expr(e) => matches!(e.kind, ExprKind::Call { .. }),
            StatementKind::TimingControl { stmt: inner, .. } => Self::stmt_contains_call(inner),
            StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
                stmts.iter().any(Self::stmt_contains_call)
            }
            StatementKind::If {
                then_stmt,
                else_stmt,
                ..
            } => {
                Self::stmt_contains_call(then_stmt)
                    || else_stmt
                        .as_ref()
                        .is_some_and(|e| Self::stmt_contains_call(e))
            }
            StatementKind::Case { items, .. } => {
                items.iter().any(|it| Self::stmt_contains_call(&it.stmt))
            }
            StatementKind::For { body, .. }
            | StatementKind::While { body, .. }
            | StatementKind::DoWhile { body, .. }
            | StatementKind::Repeat { body, .. }
            | StatementKind::Forever { body }
            | StatementKind::Foreach { body, .. } => Self::stmt_contains_call(body),
            _ => false,
        }
    }

    /// Does the subtree contain a statement-level call to a RESOLVABLE task
    /// (or void function) whose body blocks? `stmt_is_blocking` cannot see
    /// through calls, so an `always @(posedge clk) begin q <= d; if (go)
    /// drv.step(); end` classified as an EDGE block — and when the callee
    /// finally blocked, the synchronous executor advanced global time with
    /// the slot's NBAs still queued (a `#1` inside the task committed
    /// `q <= d` picoseconds late; the cv8s BFM reset-release skew). Such a
    /// body must be a PROCESS: parking commits the slot's NBAs on time and
    /// gives the §9.2.2 miss-edges-while-busy semantics the reference
    /// implements. Unresolvable callees (class/virtual methods) keep the
    /// old classification.
    pub(super) fn stmt_calls_blocking_task(
        &self,
        stmt: &Statement,
        scope: &str,
        depth: u8,
    ) -> bool {
        if depth == 0 {
            return false;
        }
        let call_blocks = |raw: &str| -> bool {
            let mut cands: Vec<String> = Vec::with_capacity(3);
            if !scope.is_empty() {
                cands.push(format!("{}.{}", scope, raw));
            }
            cands.push(raw.to_string());
            if let Some((_, leaf)) = raw.rsplit_once('.') {
                if !scope.is_empty() {
                    cands.push(format!("{}.{}", scope, leaf));
                }
            }
            for c in &cands {
                if let Some(td) = self.module.tasks.get(c.as_str()) {
                    return td.items.iter().any(|st| self.stmt_is_blocking(st))
                        || td
                            .items
                            .iter()
                            .any(|st| self.stmt_calls_blocking_task(st, scope, depth - 1));
                }
            }
            false
        };
        match &stmt.kind {
            StatementKind::Expr(e) => match &e.kind {
                ExprKind::Call { func, .. } => match &func.kind {
                    ExprKind::Ident(h) => {
                        let raw: String = h
                            .path
                            .iter()
                            .map(|s| s.name.name.as_str())
                            .collect::<Vec<_>>()
                            .join(".");
                        // A dotted call (`obs.observe()`) can be a CLASS
                        // METHOD on a handle — not in `module.tasks` at all.
                        // Fall back to the leaf-name over-approximation that
                        // resolves method bodies across classes (audit case
                        // c: a delaying method kept its edge block on the
                        // edge path and displaced sibling blocks by 1ps).
                        call_blocks(&raw)
                            || (h.path.len() > 1
                                && h.path.last().is_some_and(|l| {
                                    self.subroutine_name_blocks(l.name.name.as_str())
                                }))
                    }
                    // Class-method receiver (`obs.observe()`): the leaf-name
                    // over-approximation in `callee_transitively_blocks`
                    // resolves method bodies across all classes. Without
                    // this, an edge block calling a delaying class method
                    // stayed on the edge path and its `#1` displaced every
                    // sibling block's $time/NBA commit on that edge (the
                    // class-method variant of the cv8s blocking-task skew;
                    // audit case c).
                    ExprKind::MemberAccess { .. } => self.callee_transitively_blocks(e),
                    _ => false,
                },
                _ => false,
            },
            StatementKind::TimingControl { stmt: inner, .. } => {
                self.stmt_calls_blocking_task(inner, scope, depth)
            }
            StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => stmts
                .iter()
                .any(|st| self.stmt_calls_blocking_task(st, scope, depth)),
            StatementKind::If {
                then_stmt,
                else_stmt,
                ..
            } => {
                self.stmt_calls_blocking_task(then_stmt, scope, depth)
                    || else_stmt
                        .as_ref()
                        .is_some_and(|e| self.stmt_calls_blocking_task(e, scope, depth))
            }
            StatementKind::Case { items, .. } => items
                .iter()
                .any(|it| self.stmt_calls_blocking_task(&it.stmt, scope, depth)),
            StatementKind::For { body, .. }
            | StatementKind::While { body, .. }
            | StatementKind::DoWhile { body, .. }
            | StatementKind::Repeat { body, .. }
            | StatementKind::Forever { body }
            | StatementKind::Foreach { body, .. } => {
                self.stmt_calls_blocking_task(body, scope, depth)
            }
            _ => false,
        }
    }

    pub(super) fn classify_one_always_block(
        &mut self,
        ab: AlwaysBlock,
        lazy: Option<LazyAlways>,
    ) -> Option<AlwaysBlock> {
        // (Body of the original `for ab in blocks.into_iter()` loop, with
        // each `continue` rewritten as `return None` and each `remaining.push`
        // rewritten as `return Some(...)`.)
        // Roadmap 11-12 (opt-in XEZIM_PROC_FSM=1): a MULTI-WAIT always body
        // (an event/delay head plus at least one more wait inside) is served
        // by the edge path only through per-fire AST continuations — compile
        // it into a process FSM instead when the whole tree compiles. Single-
        // wait bodies stay on their (native) edge/clock-gen fast paths.
        {
            use std::sync::OnceLock;
            static FSM_ON: OnceLock<bool> = OnceLock::new();
            let fsm_on =
                *FSM_ON.get_or_init(|| std::env::var("XEZIM_PROC_FSM").as_deref() == Ok("1"));
            let tc = Self::count_timing_controls(&ab.stmt);
            if fsm_on
                && (tc >= 2 || (tc >= 1 && Self::stmt_contains_call(&ab.stmt)))
                && self.try_register_proc_fsm(&ab.stmt, &ab.scope, true, "always block")
            {
                return None;
            }
        }
        // `@*` inference resolves the body's reads; give it this block's own
        // instance scope, or an upward reference from a bound module (e.g.
        // `host_mod.clk` in a monitor bound under a `host_mod` instance)
        // resolves to nothing and the block never re-fires after time 0.
        let saved_hint = self.name_resolve_hint.borrow().clone();
        if !ab.scope.is_empty() {
            *self.name_resolve_hint.borrow_mut() = Some(ab.scope.clone());
        }
        let extracted = self.extract_sensitivity(&ab.stmt);
        // §14.13: a clocking event is delivered after sampling/NBA, rather
        // than as the raw clock edge returned by extract_sensitivity.
        fn leading_event(stmt: &Statement) -> Option<&EventControl> {
            match &stmt.kind {
                StatementKind::TimingControl {
                    control: TimingControl::Event(event),
                    ..
                } => Some(event),
                StatementKind::SeqBlock { stmts, .. } => stmts.first().and_then(leading_event),
                _ => None,
            }
        }
        let clocking_head =
            leading_event(&ab.stmt).is_some_and(|event| self.is_clocking_event(event));
        *self.name_resolve_hint.borrow_mut() = saved_hint;
        if let Some((sens, body)) = extracted {
            if !sens.is_empty() {
                let all_level = sens.iter().all(|s| s.edge == EdgeKind::AnyEdge);
                let top_prefix = format!("{}.", self.module.name);
                // §15.5: `always @(e)` on a named `event` is edge-triggered
                // (`->e` toggles the event signal; the edge wakes the block
                // once). The comb-settle path would instead re-run it every
                // settle iteration, so a non-idempotent body (`i++`) executes
                // up to settle_limit (100) times. Force such blocks to
                // edge_blocks even though their sensitivity is AnyEdge.
                let has_named_event = sens.iter().any(|s| {
                    let n = s.signal_name.as_str();
                    self.module.events.contains(n)
                        || n.strip_prefix(&top_prefix)
                            .is_some_and(|b| self.module.events.contains(b))
                });
                // A body that reads what it writes (`always @(s) cnt++;`) is
                // not idempotent: the comb-settle path re-runs it on every
                // settle iteration, including the ones its own writes caused, so
                // it executed settle_limit (100) times instead of once per
                // change of `s`. Route those through the edge path, which fires
                // exactly on a change of a listed signal.
                let self_ref = Self::body_is_self_referential(&body);
                // A body with a BLOCKING control (`#delay`, inner `@event`,
                // `wait`) cannot go through the comb-settle path — that path
                // strips the `@()` and re-runs the body in a zero-time settle
                // loop, which drops the delay and (for `always @(a) #5 z=a`)
                // spins/starves the rest of the schedule. The edge path DOES
                // honor a delayed body (same as `always @(posedge clk) #1 q=d`),
                // so route a blocking-bodied level block there instead.
                // §9.4.2: an explicit event control fires ONLY on the signals it
                // lists. The comb-settle path DISCARDS the `@()` list (it takes
                // `body`, not `ab.stmt`) and re-derives sensitivity from the
                // body's READ SET, so it is faithful only when the body reads
                // nothing beyond what the list already covers. A vendor
                // timing-check notifier block breaks that assumption:
                //     always @(notifier)
                //        if (se && !bypass) cell.q <= 1'bx;
                //        else if (wclk_seen == 0) corrupt_memory;
                // reads se/bypass/wclk_seen — none of them listed — so the comb
                // path re-fired it on every change of those and corrupted the
                // memory model (a whole register-file testbench read back X).
                // Compare RESOLVED signal ids, the same way the comb path
                // resolves them, and keep the fast path only when the read set
                // adds nothing. Unresolvable reads (parameters, genvars, loop
                // locals) can never trigger anything, so they don't count.
                // PERF: the `comb_sensitivity_is_faithful` test below costs a
                // full `collect_stmt_reads` of the body (two HashSet<String>
                // built from `format!`ed hierarchical names), a scope
                // inference over them, and a per-name id resolution. It is
                // only ever consulted when the four cheap predicates in front
                // of it already admit the comb path, yet it used to be
                // computed EAGERLY for every always block — including every
                // `always @(posedge clk)` flop, where `all_level` is false and
                // the result is thrown away. On c906 that was ~75% of the
                // whole compile phase. Folding it into the `&&` chain as the
                // last operand keeps the identical value and the identical
                // evaluation for the blocks that reach it; the test is pure
                // (`&self` + `&ab` only), so skipping it changes nothing else.
                // A listed memory cell without an element name
                // (`always @(big[i])` on a large memory) has no dependency
                // edge the comb path could wake on; such a block runs as a
                // process below.
                if all_level
                    && !clocking_head
                    && !has_named_event
                    && !self_ref
                    && !self.stmt_is_blocking(&body)
                    && !self.stmt_calls_blocking_task(&body, &ab.scope, 3)
                    && !sens.iter().any(|s| self.sens_unnamed_cell(s))
                    && match Self::event_list_shape(&ab.stmt) {
                        // `@*` and the other implicit forms ARE the read set.
                        EventListShape::Implicit => true,
                        // §9.4.2: an explicit term that selects, concatenates
                        // or computes (`@(a[1])`, `@(mem[i])`, `@(a + b)`)
                        // fires on that VALUE alone — never on whatever else
                        // the body happens to read (a select-listed block
                        // whose body called a task woke on every signal the
                        // body read). The comb path, which wakes on any change
                        // of a whole signal the body reads, is faithful only
                        // when every term is a plain name or constant bits of
                        // one, the body reads those signals only within the
                        // listed bits, and it has nothing but assignments to
                        // repeat: then a wake on a sibling bit recomputes the
                        // very same values. Everything else goes to the edge
                        // path, or to a value waiter below.
                        shape => {
                            let select_masks = match shape {
                                EventListShape::NonPlain => {
                                    self.comb_select_masks(&sens, &ab.scope)
                                }
                                _ => Some(Vec::new()),
                            };
                            select_masks.is_some_and(|select_masks| {
                                let mut b_reads: HashSet<String> = HashSet::default();
                                let mut b_writes: HashSet<String> = HashSet::default();
                                Self::collect_stmt_reads(
                                    &body,
                                    &self.module,
                                    &mut b_reads,
                                    &mut b_writes,
                                );
                                let scope_hint = self.infer_scope_from_rw_sets(&b_writes, &b_reads);
                                let name_to_id = &self.signal_name_to_id;
                                let resolve_ids = |name: &str| -> Vec<usize> {
                                    let mut out: Vec<usize> = Vec::new();
                                    if let Some(scope) = &scope_hint {
                                        if let Some(&id) =
                                            name_to_id.get(format!("{}.{}", scope, name).as_str())
                                        {
                                            out.push(id);
                                        }
                                    }
                                    if let Some(&id) = name_to_id.get(name) {
                                        out.push(id);
                                    } else if let Some(stripped) = name.strip_prefix(&top_prefix) {
                                        if let Some(&id) = name_to_id.get(stripped) {
                                            out.push(id);
                                        }
                                    }
                                    out
                                };
                                let mut sens_ids: Vec<usize> = Vec::new();
                                for s in &sens {
                                    for id in resolve_ids(s.signal_name.as_str()) {
                                        if !sens_ids.contains(&id) {
                                            sens_ids.push(id);
                                        }
                                    }
                                }
                                // Mirror the comb path's `sens_reads = reads - writes`.
                                // The union of the resolved ids is built ONCE and
                                // both directions are answered from it: `for all r,
                                // for all id in resolve_ids(r)` is by construction
                                // the same as `for all id in union` (the union is
                                // exactly that flat_map), so this is the same test
                                // the two separate passes used to run.
                                // A select list used to take this path unchecked, and
                                // generated netlists list every operand but the
                                // parameters (`case (cur_st) IDLE:`). A parameter
                                // never changes, so it adds no wake; for those lists
                                // a read of one does not count.
                                let params = &self.module.parameters;
                                let is_param = |r: &str| {
                                    shape == EventListShape::NonPlain
                                        && (params.contains_key(r)
                                            || scope_hint.as_ref().is_some_and(|sc| {
                                                params
                                                    .contains_key(format!("{}.{}", sc, r).as_str())
                                            }))
                                };
                                let read_id_union: std::collections::HashSet<usize> = b_reads
                                    .difference(&b_writes)
                                    .filter(|r| !is_param(r.as_str()))
                                    .flat_map(|r| resolve_ids(r.as_str()))
                                    .collect();
                                let reads_covered =
                                    read_id_union.iter().all(|id| sens_ids.contains(id));
                                // The reverse must hold too: every LISTED signal has to be
                                // in the derived read set, or the comb path silently drops
                                // it from the sensitivity. `always @(a) t = $time;` reads
                                // no signal at all, so the containment above passes
                                // VACUOUSLY, the entry's read set came out empty, and the
                                // block fired exactly once (at t0) — every later edge of
                                // `a` was missed and the captured time stayed 0. Routing
                                // the not-equal case to the edge path costs a little speed
                                // for an unusual shape and follows §9.4.2 exactly.
                                let sens_covered =
                                    sens_ids.iter().all(|id| read_id_union.contains(id));
                                reads_covered
                                    && sens_covered
                                    && (select_masks.is_empty()
                                        || self.body_reads_within_masks(
                                            &body,
                                            &select_masks,
                                            &ab.scope,
                                            &resolve_ids,
                                        ))
                            })
                        }
                    }
                {
                    return Some(AlwaysBlock {
                        kind: ab.kind,
                        stmt: body,
                        scope: ab.scope,
                    });
                }
                if clocking_head || self.event_needs_value_waiter(&sens, &ab.scope) {
                    if self.try_register_proc_fsm(&ab.stmt, &ab.scope, true, "always block") {
                        return None;
                    }
                    let forever_stmt = Statement::new(
                        StatementKind::Forever {
                            body: Box::new(ab.stmt.clone()),
                        },
                        ab.stmt.span,
                    );
                    let pid = self.next_pid;
                    self.next_pid += 1;
                    if !ab.scope.is_empty() {
                        self.process_scope_hint.insert(pid, ab.scope.clone());
                    }
                    self.process_origin
                        .insert(pid, (ab.stmt.span, "always block"));
                    self.event_queue.schedule(0, pid, vec![forever_stmt].into());
                    return None;
                }
                let mut dropped_terms: Vec<String> = Vec::new();
                let resolved: Vec<SensitivityId> = sens
                    .iter()
                    .filter_map(|s| {
                        if let Some(&id) = self.signal_name_to_id.get(s.signal_name.as_str()) {
                            return Some(SensitivityId {
                                signal_id: id,
                                edge: s.edge,
                                iff: s.iff.clone(),
                                value_of: None,
                                wake_rank: 0,
                                cell: false,
                            });
                        }
                        if let Some(stripped) = s.signal_name.strip_prefix(&top_prefix) {
                            if let Some(&id) = self.signal_name_to_id.get(stripped) {
                                return Some(SensitivityId {
                                    signal_id: id,
                                    edge: s.edge,
                                    iff: s.iff.clone(),
                                    value_of: None,
                                    wake_rank: 0,
                                    cell: false,
                                });
                            }
                        }
                        // The term may still be spelled relative to the
                        // block's INSTANCE scope (an inlined child whose
                        // rewrite left the leaf bare).
                        if !ab.scope.is_empty() {
                            let scoped = format!("{}.{}", ab.scope, s.signal_name);
                            if let Some(&id) = self.signal_name_to_id.get(scoped.as_str()) {
                                return Some(SensitivityId {
                                    signal_id: id,
                                    edge: s.edge,
                                    iff: s.iff.clone(),
                                    value_of: None,
                                    wake_rank: 0,
                                    cell: false,
                                });
                            }
                        }
                        // NOT resolvable. Silently dropping the term changes
                        // semantics invisibly: a lost CLOCK term makes a dead
                        // flop; a lost async-RESET term removes the reset.
                        // Record it for the warning below.
                        dropped_terms.push(format!("{}{}", s.edge.print_str(), s.signal_name));
                        None
                    })
                    .collect();
                if !dropped_terms.is_empty() {
                    eprintln!(
                        "[xezim][warning] edge-sensitive always block{} DROPPED {}                          unresolvable sensitivity term(s): {} — {}",
                        if ab.scope.is_empty() {
                            String::new()
                        } else {
                            format!(" in '{}'", ab.scope)
                        },
                        dropped_terms.len(),
                        dropped_terms.join(", "),
                        if resolved.is_empty() {
                            "NO terms remain; this block will NEVER fire (dead flop)"
                        } else {
                            "the block will not respond to the dropped term(s)"
                        }
                    );
                }
                let block_idx = self.edge_blocks.len();
                // §9.4.2: record constant bit/part-select level terms
                // (`@(v[3])`, `@(v[7:4])`). `event_to_sens` walks past the
                // select and yields the BASE signal, so without this the block
                // would wake on a change to any bit of `v`. `check_edges_inner`
                // narrows the any-change wake to these bits. A signal the block
                // also watches whole (`@(v[3] or v)`) keeps the whole-signal
                // wake; every other select went to a value waiter above.
                let mut narrowed: Vec<(usize, event_bits::EventMask)> = Vec::new();
                let mut whole: Vec<usize> = Vec::new();
                for s in &sens {
                    // Edge-qualified terms use the LSB edge fanout and must
                    // not widen a level-select term on the same signal.
                    if s.edge != EdgeKind::AnyEdge {
                        continue;
                    }
                    let Some(sid) = self.sens_term_sid(&s.signal_name, &ab.scope) else {
                        continue;
                    };
                    let mask = match (&s.value_of, s.edge) {
                        (Some(e), EdgeKind::AnyEdge) => self.narrow_event_mask(e, sid, &ab.scope),
                        _ => None,
                    };
                    match mask {
                        Some(m) => match narrowed.iter_mut().find(|(id, _)| *id == sid) {
                            Some(entry) => entry.1.union(m),
                            None => narrowed.push((sid, m)),
                        },
                        None => whole.push(sid),
                    }
                }
                for (sid, mask) in narrowed {
                    if whole.contains(&sid) {
                        continue;
                    }
                    self.bitsel_edge_sens.insert((sid, block_idx), mask);
                    let (w, b) = (sid >> 6, sid & 63);
                    if w >= self.bitsel_sid_bits.len() {
                        self.bitsel_sid_bits.resize(w + 1, 0);
                    }
                    self.bitsel_sid_bits[w] |= 1u64 << b;
                }
                static DUMP_EDGE_SENS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
                if *DUMP_EDGE_SENS
                    .get_or_init(|| std::env::var_os("XEZIM_DUMP_EDGE_SENS").is_some())
                {
                    let terms: Vec<String> = resolved
                        .iter()
                        .map(|si| {
                            format!("{}{}", si.edge.print_str(), self.name_for_id(si.signal_id))
                        })
                        .collect();
                    eprintln!(
                        "[EDGE-SENS] block#{} kind={:?} scope='{}' sens=[{}]",
                        block_idx,
                        ab.kind,
                        ab.scope,
                        terms.join(", ")
                    );
                }
                // A body that CALLS a blocking task must be a PROCESS: on
                // the edge path the call executes synchronously and its
                // first `#delay` advances global time with this slot's NBAs
                // still queued — `q <= d;` scheduled before the call then
                // commits picoseconds late (the cv8s BFM reset-release
                // skew: rst_l_d1 stamped at t+6ps). As a process, the park
                // machinery suspends the call properly, the slot's NBA
                // region commits on time, and busy-body edges are missed
                // per §9.2.2 exactly as the reference behaves.
                if self.stmt_calls_blocking_task(&body, &ab.scope, 3)
                    // §9.2.2/§9.4: any blocking body needs a process, including
                    // level controls, condition waits and inferred sensitivity.
                    // The process runner derives @* sensitivity from its body.
                    || self.stmt_is_blocking(&body)
                    || Self::stmt_contains_delay_control(&body)
                    // A nested `@(...)` wait is the same class: the edge
                    // path's synchronous executor cannot suspend, so it ran
                    // the inner wait as a no-op (a `repeat (lat) @(posedge
                    // clk)` paced response streamed with zero latency).
                    || Self::stmt_contains_event_control(&body)
                {
                    // §16.9.3: the block's clocking event still clocks the
                    // sampled-value calls in its body; registered with the
                    // edge blocks' (`register_sampled_watches`).
                    if Self::stmt_has_sampled_call(&body) {
                        if let Some((ec, cid)) = resolved.iter().find_map(|s| match s.edge {
                            EdgeKind::Posedge => Some((1u8, s.signal_id)),
                            EdgeKind::Negedge => Some((2u8, s.signal_id)),
                            _ => None,
                        }) {
                            self.proc_sampled_blocks.push((
                                body.clone(),
                                ab.scope.clone(),
                                ec,
                                cid,
                            ));
                        }
                    }
                    // This body already leaves the edge path for the process path
                    // (§9.2.2: edges arriving mid-flight are missed). A compiled process
                    // FSM implements the same semantics without the per-activation AST
                    // walk; use it whenever the body compiles (c906's tb monitors: an
                    // else-chain ending in a rare `#10`, ~9% of the run).
                    if self.try_register_proc_fsm(&ab.stmt, &ab.scope, true, "always block") {
                        return None;
                    }
                    let forever_stmt = Statement::new(
                        StatementKind::Forever {
                            body: Box::new(ab.stmt.clone()),
                        },
                        ab.stmt.span,
                    );
                    let pid = self.next_pid;
                    self.next_pid += 1;
                    if !ab.scope.is_empty() {
                        self.process_scope_hint.insert(pid, ab.scope.clone());
                    }
                    self.process_origin
                        .insert(pid, (ab.stmt.span, "always block"));
                    self.event_queue.schedule(0, pid, vec![forever_stmt].into());
                    return None;
                }
                // `collect` leaves room for four terms; an edge block keeps
                // its list for the whole run and nearly always has one.
                let mut resolved = resolved;
                resolved.shrink_to_fit();
                // A lazy block keeps its source, not the tree: the body is
                // rebuilt (and usually dropped again) when it compiles.
                let (stmt, lazy) = match lazy {
                    Some(l) => (
                        Statement::new(StatementKind::Null, body.span),
                        Some(Box::new(l)),
                    ),
                    None => (body, None),
                };
                Arc::make_mut(&mut self.edge_blocks).push(EdgeSensitiveBlock {
                    resolved_sensitivities: resolved,
                    stmt,
                    lazy,
                    kind: ab.kind,
                    scope: ab.scope,
                });
                return None;
            }
            return Some(AlwaysBlock {
                kind: ab.kind,
                stmt: body,
                scope: ab.scope,
            });
        }
        if ab.kind == AlwaysKind::Always {
            if let StatementKind::TimingControl {
                control: TimingControl::Delay(d),
                stmt: body,
            } = &ab.stmt.kind
            {
                let delay_val = self.eval_delay_ticks(d);
                // A delay that reads a runtime variable (`#(half)` reconfigured
                // at runtime — the PLL refclk/vco reprogramming pattern) must
                // re-evaluate every toggle; never freeze it into a fixed
                // ClockGen (which would keep the ORIGINAL period forever).
                if delay_val > 0 && !self.delay_expr_is_dynamic(d) {
                    let saved_hint = self.name_resolve_hint.borrow().clone();
                    if !ab.scope.is_empty() {
                        *self.name_resolve_hint.borrow_mut() = Some(ab.scope.clone());
                    }
                    let extracted = self.try_extract_clock_gen(body, delay_val);
                    *self.name_resolve_hint.borrow_mut() = saved_hint;
                    if let Some(clock_gen) = extracted {
                        sim_dbg_eprintln!(
                            "[OPT] clock generator: signal {} period {} (always #{} pattern)",
                            self.name_for_id(clock_gen.signal_id),
                            delay_val * 2,
                            delay_val
                        );
                        self.clock_gen_src.push((
                            self.clock_generators.len(),
                            ab.scope.clone(),
                            ab.stmt.span.start,
                        ));
                        self.clock_generators.push(clock_gen);
                        return None;
                    }
                }
                // A variable-period behavioral clock often has a conditional
                // clamp rather than the exact `clk = ~clk` shape, for example
                // `always #(period/2) clk = halt ? 0 : ~clk`. It cannot become
                // a fixed ClockGen because the delay must be re-evaluated after
                // every assignment. Still, a simple blocking assignment needs
                // no general-purpose Forever continuation: retain its AST once
                // and schedule an empty pid marker on each wakeup.
                let simple_retained_assign = match &body.kind {
                    StatementKind::BlockingAssign { lvalue, rvalue } => {
                        matches!(&lvalue.kind, ExprKind::Ident(_))
                            && Self::intra_delay_marker(rvalue).is_none()
                    }
                    _ => false,
                };
                if simple_retained_assign {
                    let pid = self.next_pid;
                    self.next_pid += 1;
                    if !ab.scope.is_empty() {
                        self.process_scope_hint.insert(pid, ab.scope.clone());
                    }
                    self.process_origin
                        .insert(pid, (ab.stmt.span, "always block"));
                    self.fast_delay_always.insert(
                        pid,
                        FastDelayAlways {
                            delay: d.clone(),
                            body: (**body).clone(),
                            compiled_body: None,
                            compiled_delay: None,
                            delay_precision_diff: None,
                            execute_body: false,
                        },
                    );
                    self.event_queue.schedule(0, pid, Vec::new().into());
                    return None;
                }
                // A delay-headed `always` with a compound body (the timed
                // integration step of a real-number model, `always #(TSTEP)
                // begin state = …; end`) ran as an AST `forever` process and
                // paid the identifier-resolution ladder on every step (#159).
                // The process FSM implements the same wait/run cycle from
                // bytecode; use it whenever the body compiles fallback-free.
                if self.try_register_proc_fsm(&ab.stmt, &ab.scope, true, "always block") {
                    return None;
                }
                let forever_stmt = Statement::new(
                    StatementKind::Forever {
                        body: Box::new(ab.stmt.clone()),
                    },
                    ab.stmt.span,
                );
                let pid = self.next_pid;
                self.next_pid += 1;
                if !ab.scope.is_empty() {
                    self.process_scope_hint.insert(pid, ab.scope.clone());
                }
                self.process_origin
                    .insert(pid, (ab.stmt.span, "always block"));
                self.event_queue.schedule(0, pid, vec![forever_stmt].into());
                return None;
            }
            if self.stmt_is_blocking(&ab.stmt)
                || self.stmt_calls_blocking_task(&ab.stmt, &ab.scope, 3)
            {
                // Roadmap 11-12 (opt-in XEZIM_PROC_FSM=1): a blocking always
                // body whose whole statement tree compiles fallback-free
                // becomes a bytecode FSM — resumes re-enter at a saved pc
                // instead of re-walking the AST continuation chain.
                {
                    use std::sync::OnceLock;
                    static ON: OnceLock<bool> = OnceLock::new();
                    let fsm_on =
                        *ON.get_or_init(|| std::env::var("XEZIM_PROC_FSM").as_deref() == Ok("1"));
                    if fsm_on
                        && self.try_register_proc_fsm(&ab.stmt, &ab.scope, true, "always block")
                    {
                        return None;
                    }
                }
                let forever_stmt = Statement::new(
                    StatementKind::Forever {
                        body: Box::new(ab.stmt.clone()),
                    },
                    ab.stmt.span,
                );
                let pid = self.next_pid;
                self.next_pid += 1;
                if !ab.scope.is_empty() {
                    self.process_scope_hint.insert(pid, ab.scope.clone());
                }
                self.process_origin
                    .insert(pid, (ab.stmt.span, "always block"));
                self.event_queue.schedule(0, pid, vec![forever_stmt].into());
                return None;
            }
        }
        Some(ab)
    }

    /// Roadmap 11-12: compile an `always` body (waits included) into a
    /// process FSM. Returns true when the whole tree compiled with no
    /// fallback and at least one wait point; the pid is scheduled with the
    /// empty payload marker that `run_process_payload` routes to
    /// `run_proc_fsm`.
    pub(super) fn try_register_proc_fsm(
        &mut self,
        stmt: &Statement,
        scope: &str,
        is_always: bool,
        origin: &'static str,
    ) -> bool {
        use super::bytecode::{BytecodeCompiler, Insn};
        let scope_hint = if scope.is_empty() {
            None
        } else {
            Some(scope.to_string())
        };
        let mut compiler = BytecodeCompiler::new(
            &self.signal_name_to_id,
            &self.signal_signed,
            &self.signal_widths,
            &self.module.arrays,
            &self.widths,
        );
        compiler.allow_waits = true;
        // System-task statements (`$display`, `$write`, `$finish`, ...)
        // lower to statement fallbacks; the gate below admits only those,
        // and only because the body has no process locals for the
        // interpreter to miss (LoadProcessLocal is rejected there).
        compiler.allow_ast_fallback = true;
        compiler.set_scope_hint(scope_hint);
        compiler.set_tasks(&self.module.tasks);
        compiler.set_functions(&self.module.functions);
        compiler.set_params(&self.module.parameters);
        compiler.set_param_leaf_idx(self.param_leaf_index());
        compiler.set_cast_widths(&self.cast_widths);
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
        // `<top>.<path>` reads (a testbench's `tb.dut.x` macros) resolve as
        // in every other compiled block; without it each one fell back and
        // the whole body stayed on the AST process path.
        compiler.top_module_name = Some(self.module.name.clone());
        let dbg = std::env::var_os("XEZIM_PROC_LOOP_STATS").is_some();
        if !compiler.compile_stmt(stmt) {
            if dbg {
                eprintln!("[PROC-FSM] scope '{}': body did not compile", scope);
            }
            return false;
        }
        let waits: Vec<(
            crate::ast::stmt::EventControl,
            Option<Vec<Sensitivity>>,
            Option<Vec<SensitivityId>>,
        )> = std::mem::take(&mut compiler.wait_specs)
            .into_iter()
            .map(|ev| (ev, None, None))
            .collect();
        let wait_is_clocking: Vec<bool> = if self.clocking_meta.is_empty() {
            vec![false; waits.len()]
        } else {
            let saved = self
                .name_resolve_hint
                .borrow_mut()
                .replace(scope.to_string());
            let v = waits.iter().map(|w| self.is_clocking_event(&w.0)).collect();
            *self.name_resolve_hint.borrow_mut() = saved;
            v
        };
        let mut cb = compiler.finish();
        // Constant-delay fold: an integer bare delay elaborates to
        // `LoadConst(int) ; LoadConst(real scale) ; Mul ; WaitDelayReg`.
        // Rewrite the window to `Nop ; Nop ; LoadConst(real ticks) ; Wait`
        // (indices preserved) so the native generator's adjacency fold —
        // and the bytecode VM identically — see one quantized tick literal.
        {
            use super::bytecode::Insn;
            let n = cb.instructions.len();
            for i in 0..n.saturating_sub(3) {
                let hit = match (
                    &cb.instructions[i],
                    &cb.instructions[i + 1],
                    &cb.instructions[i + 2],
                    &cb.instructions[i + 3],
                ) {
                    (
                        Insn::LoadConst(a, va),
                        Insn::LoadConst(b, vb),
                        Insn::Mul(c, ma, mb),
                        Insn::WaitDelayReg(wr),
                    ) if wr == c
                        && ((ma == a && mb == b) || (ma == b && mb == a))
                        && !va.is_real
                        && vb.is_real =>
                    {
                        let iv = va.to_u64().unwrap_or(0) as f64;
                        let ticks = iv * vb.to_f64();
                        // The elaboration scale factor (unit/tick in f64)
                        // can carry an ulp of error; the product is by
                        // construction a whole tick count — snap it.
                        let snapped = ticks.round();
                        if ticks.is_finite()
                            && snapped >= 0.0
                            && snapped < 9.0e15
                            && (ticks - snapped).abs() <= 1e-6 * snapped.max(1.0)
                        {
                            Some((*c, snapped))
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                if let Some((c, ticks)) = hit {
                    cb.instructions[i] = Insn::Nop;
                    cb.instructions[i + 1] = Insn::Nop;
                    cb.instructions[i + 2] = Insn::LoadConst(c, Box::new(Value::from_f64(ticks)));
                }
            }
        }
        let cb = cb;
        // Gate: fallback-free, at least one wait (else it belongs on the
        // comb/edge paths), and no process-local frame reads (the FSM has
        // no AST call frames).
        let non_systask_fallback = cb.instructions.iter().any(|i| match i {
            Insn::StmtFallback(payload) => !matches!(
                &payload.0.kind,
                StatementKind::Expr(e) if matches!(&e.kind, ExprKind::SystemCall { .. })
            ),
            _ => false,
        });
        if non_systask_fallback
            || cb
                .instructions
                .iter()
                .any(|i| matches!(i, Insn::LoadProcessLocal(..)))
            || !cb
                .instructions
                .iter()
                .any(|i| matches!(i, Insn::WaitDelayReg(..) | Insn::WaitEdge(..)))
        {
            if dbg {
                let ops: std::collections::HashSet<&str> = cb
                    .instructions
                    .iter()
                    .map(super::bytecode::insn_opcode_name)
                    .collect();
                let reasons: Vec<&str> = cb
                    .instructions
                    .iter()
                    .filter_map(|i| match i {
                        Insn::StmtFallback(b) => Some(&*b.1),
                        _ => None,
                    })
                    .collect();
                eprintln!(
                    "[PROC-FSM] scope '{}': fallback reasons {:?}",
                    scope, reasons
                );
                eprintln!(
                    "[PROC-FSM] scope '{}': gated out (fallback={} insns={} ops={:?})",
                    scope,
                    cb.has_fallback,
                    cb.instructions.len(),
                    ops
                );
            }
            return false;
        }
        let pid = self.next_pid;
        self.next_pid += 1;
        if !scope.is_empty() {
            self.process_scope_hint.insert(pid, scope.to_string());
        }
        self.process_origin.insert(pid, (stmt.span, origin));
        self.proc_fsm.insert(
            pid,
            Box::new(ProcFsm {
                writes: Self::compiled_block_write_ids(&cb).map(Arc::from),
                ts: self.lower_proc_fsm_ts(&cb),
                compiled: cb,
                waits,
                wait_is_clocking,
                pc: 0,
                regs: Vec::new(),
                scope: scope.to_string(),
                precision_diff: None,
                is_always,
                #[cfg(feature = "jit")]
                native: None,
                #[cfg(feature = "jit")]
                native_frame: Vec::new(),
            }),
        );
        if dbg || sim_debug_enabled() {
            eprintln!(
                "[PROC-FSM] registered pid {} scope '{}' ({} insns, {} waits)",
                pid,
                scope,
                self.proc_fsm[&pid].compiled.instructions.len(),
                self.proc_fsm[&pid].waits.len()
            );
        }
        self.event_queue.schedule(0, pid, Vec::new().into());
        true
    }

    /// The write-target census of every process body (see
    /// `collect_override_target_leaves` / `collect_procedural_write_names`),
    /// deferred always blocks included. A lazy block is not materialized for
    /// it when its source can be read directly: a written name whose head is
    /// not a substituted port or interface reference is rewritten only by
    /// prefixing a local (`prefix + name`), else kept, so both spellings are
    /// returned (in the second set; a superset of the rewritten census, which
    /// only ever stops an alias). Sources are shared by every instance of a
    /// module, so each is read once. Any other lazy block (an override, or a
    /// write through a port or interface) is materialized for the walk.
    #[allow(clippy::type_complexity)]
    pub(super) fn write_target_census(&self) -> ((HashSet<String>, HashSet<String>), Vec<String>) {
        let mut by_source: HashMap<usize, (HashSet<String>, HashSet<String>)> = HashMap::default();
        let mut extra_writes: Vec<String> = Vec::new();
        let mut walk_lazy: Vec<&LazyAlways> = Vec::new();
        for d in &self.deferred_always {
            let DeferredAlways::Lazy { lazy, .. } = d else {
                continue;
            };
            let src = &lazy.src;
            let (overrides, writes) = by_source
                .entry(std::rc::Rc::as_ptr(&src.source) as usize)
                .or_insert_with(|| super::elaborate::stmt_write_targets(&src.source));
            let ctx = &src.ctx;
            let direct = overrides.is_empty()
                && writes.iter().all(|w| {
                    let head = w.split('.').next().unwrap_or(w);
                    !ctx.port_map.contains_key(head)
                        && !ctx.interface_map.contains_key(head)
                        && !head.starts_with("$unit::")
                });
            if !direct {
                walk_lazy.push(lazy);
                continue;
            }
            for w in writes.iter() {
                let head = w.split('.').next().unwrap_or(w);
                if ctx.local_names.contains(head) {
                    extra_writes.push(format!("{}{}", ctx.prefix, w));
                }
                extra_writes.push(w.clone());
            }
        }
        let deferred = &self.deferred_always;
        let sets = super::elaborate::collect_write_target_sets(
            &self.module,
            &mut |visit: &mut dyn FnMut(&Statement)| {
                for d in deferred {
                    if let DeferredAlways::Eager(ab) = d {
                        visit(&ab.stmt);
                    }
                }
                for lazy in &walk_lazy {
                    visit(&lazy.materialize().stmt);
                }
            },
        );
        (sets, extra_writes)
    }

    /// Materialize every deferred always block onto `module.always_blocks`,
    /// after the module's own (the order classification would take them).
    pub(super) fn flush_deferred_always(&mut self) {
        for d in std::mem::take(&mut self.deferred_always) {
            let ab = match d {
                DeferredAlways::Eager(ab) => ab,
                DeferredAlways::Lazy { lazy, .. } => lazy.materialize(),
            };
            self.module.always_blocks.push(ab);
        }
    }

    pub(super) fn classify_always_blocks(&mut self) {
        // Take ownership — the remaining-only subset is written back at the
        // end. Avoids a full clone of every always-block AST (significant on
        // c910-scale designs with 20K+ blocks).
        //
        // Order: the module's own always blocks, then the instantiated
        // ones `rewrite_edge_select_sensitivities` took out of
        // `module.pending_always`, in their original order. A lazy one is
        // materialized just for its classification; an edge block keeps
        // only its source (see `LazyAlways`), so the trees never all exist
        // at once. Anything still in `pending_always` (none on the normal
        // path) follows, materialized one at a time.
        let blocks = std::mem::take(&mut self.module.always_blocks);
        let deferred = std::mem::take(&mut self.deferred_always);
        let pending = std::mem::take(&mut self.module.pending_always);
        let mut remaining = Vec::new();
        for ab in blocks.into_iter() {
            self.hoist_static_decls(&ab.stmt, &ab.scope);
            if let Some(rest) = self.classify_one_always_block(ab, None) {
                remaining.push(rest);
            }
        }
        for d in deferred {
            let (ab, lazy) = match d {
                DeferredAlways::Eager(ab) => (ab, None),
                DeferredAlways::Lazy { lazy, .. } => (lazy.materialize(), Some(lazy)),
            };
            self.hoist_static_decls(&ab.stmt, &ab.scope);
            if let Some(rest) = self.classify_one_always_block(ab, lazy) {
                remaining.push(rest);
            }
        }
        for p in pending {
            let ab = Self::materialize_pending_always(p);
            self.hoist_static_decls(&ab.stmt, &ab.scope);
            if let Some(rest) = self.classify_one_always_block(ab, None) {
                remaining.push(rest);
            }
        }
        self.module.always_blocks = remaining;
    }

    /// Build pre-computed combinatorial entries with sensitivity sets.
    /// Called once after classify_always_blocks.
    /// Compile edge blocks to bytecode where possible.
    /// JIT-compile COMB (settle) entries. Runs AFTER `build_comb_entries` —
    /// `compile_edge_blocks` (where the JIT module is created) runs before the
    /// comb entries exist. Comb blocks are the same `CompiledBlock` type as
    /// edge blocks, and their writes go through `jit_store_signal*`, which
    /// calls `mark_dirty_id` — so settle's worklist propagation observes JIT
    /// writes exactly as it observes interpreter ones.
    #[cfg(feature = "jit")]
    pub(super) fn jit_compile_comb_entries(&mut self) {
        let enable_jit = std::env::var("XEZIM_JIT")
            .map(|v| v != "0" && !v.is_empty())
            .unwrap_or(false);
        self.comb_jit_fns = vec![None; self.comb_entries.len()];
        #[cfg(feature = "jit")]
        {
            self.comb_tpl_fns = vec![None; self.comb_entries.len()];
        }
        self.comb_jit_strikes = vec![0; self.comb_entries.len()];
        if !enable_jit {
            // `XEZIM_AOT` selects the native BACKEND (rustc-generated Rust
            // instead of cranelift); it does not enable native compilation.
            // Set alone it is a silent no-op — every native path below is
            // gated on `XEZIM_JIT`, and the edge-block arm sits inside the
            // same gate. That silence has already produced a profiling run
            // that looked like "the native backend does not displace the
            // interpreter" and a wrong README recipe, so say so out loud.
            if std::env::var("XEZIM_AOT")
                .map(|v| v == "1")
                .unwrap_or(false)
            {
                eprintln!(
                    "[warn] XEZIM_AOT=1 has no effect without XEZIM_JIT=1 — no blocks \
                     will be natively compiled. Use `XEZIM_JIT=1 XEZIM_AOT=1`."
                );
            }
        }
        if !enable_jit || self.jit_module.is_none() {
            return;
        }
        let blocks: Vec<Option<super::bytecode::CompiledBlock>> = self
            .comb_entries
            .iter()
            .map(|e| match &e.item {
                CombItem::CompiledContAssign { compiled }
                | CombItem::CompiledAlwaysBlock { compiled, .. } => Some(compiled.clone()),
                _ => None,
            })
            .collect();
        // Same >64-bit rule as edge blocks: the bridges move u64 planes and
        // would silently truncate anything wider.
        let safe: Vec<bool> = blocks
            .iter()
            .map(|b| {
                b.as_ref()
                    .map(|cb| self.block_signals_fit_u64(cb))
                    .unwrap_or(false)
            })
            .collect();
        if std::env::var("XEZIM_JIT_BAIL_TRACE").is_ok() {
            for (idx, b) in blocks.iter().enumerate() {
                let Some(cb) = b else { continue };
                if !safe[idx] {
                    let reason = self.block_fit_u64_reason(cb);
                    eprintln!(
                        "[JITGATE] eidx={} insns={} reason={:?}",
                        idx,
                        cb.instructions.len(),
                        reason
                    );
                    if reason
                        .as_deref()
                        .is_some_and(|r| r.starts_with("fill const"))
                    {
                        for (k, insn) in cb.instructions.iter().enumerate() {
                            let hit =
                                matches!(insn, super::bytecode::Insn::LoadConst(_, v) if v.is_fill);
                            if hit {
                                let hi = (k + 3).min(cb.instructions.len());
                                for j in k..hi {
                                    eprintln!("[JITGATE]   insn[{j}] {:?}", cb.instructions[j]);
                                }
                                break;
                            }
                        }
                    }
                }
            }
        }
        let xz_ptr = self.signal_has_xz.as_ptr() as u64;
        let xz_len = self.signal_has_xz.len() as u32;
        let mut n = 0usize;
        // XEZIM_AOT=1: rustc-compiled native block fns instead of cranelift.
        // One generated crate for every eligible block, compiled once,
        // dlopen'd, and installed through the same JitFn slots.
        if std::env::var("XEZIM_AOT")
            .map(|v| v == "1")
            .unwrap_or(false)
        {
            let verbose = std::env::var("XEZIM_JIT_VERBOSE").is_ok();
            let mut names: Vec<(usize, String)> = Vec::new();
            let mut fns: Vec<String> = Vec::new();
            // XEZIM_AOT_TEMPLATE=1: emit ONE body per canonical block shape
            // plus a per-block trampoline carrying that block's signal
            // mapping, instead of one baked body per block. Same ABI, so the
            // loader and dispatch are untouched. Census on C910: 148,106
            // blocks -> 15,778 shapes.
            let templated = std::env::var("XEZIM_AOT_TEMPLATE")
                .map(|v| v == "1")
                .unwrap_or(false);
            if templated {
                let items: Vec<(usize, Vec<super::bytecode::Insn>, u32)> = blocks
                    .iter()
                    .enumerate()
                    .filter_map(|(idx, b)| {
                        let cb = b.as_ref()?;
                        if !safe[idx] {
                            return None;
                        }
                        Some((idx, cb.instructions.clone(), cb.num_regs))
                    })
                    .collect();
                let built = super::aot::gen_templated_blocks(
                    &items,
                    &self.signal_widths,
                    &self.signal_signed,
                );
                if verbose {
                    eprintln!(
                        "[AOT-TEMPLATE] comb blocks={} templates={} ({:.1}x fewer bodies)",
                        built.n_blocks,
                        built.n_templates,
                        built.n_blocks as f64 / built.n_templates.max(1) as f64
                    );
                }
                // The crate exports ONLY the template bodies (2-arg ABI); the
                // per-block mapping arrays live host-side in one flat blob.
                // This branch loads and installs everything itself — the
                // shared JitFn path below stays untouched (`fns` is empty).
                if !built.fns.is_empty() {
                    let src = super::aot::module_source_named(&built.fns, &built.tpl_names);
                    let planes = (
                        self.signal_inline_bits.as_ptr() as u64,
                        self.signal_inline_bits.len() as u32,
                    );
                    if let Some(lib) = super::aot::compile_and_load(&src, verbose, planes) {
                        if let Some(api) = lib.api() {
                            for (idx, tpl_pos, map) in &built.blocks {
                                let p = api(*tpl_pos);
                                if p.is_null() {
                                    continue;
                                }
                                let f: super::aot::AotTplFn = unsafe { std::mem::transmute(p) };
                                let off = self.comb_tpl_map_blob.len() as u32;
                                self.comb_tpl_map_blob.extend_from_slice(map);
                                self.comb_tpl_fns[*idx] = Some((f, off, map.len() as u32));
                                n += 1;
                            }
                        }
                        // Same retention discipline as the baked path: the
                        // library must outlive the Simulator (fn pointers are
                        // stored), so leak it rather than invent a new field.
                        std::mem::forget(lib);
                        if verbose {
                            eprintln!(
                                "[AOT] comb entries compiled {}/{} (templated)",
                                n,
                                blocks.len()
                            );
                        }
                    }
                }
            }
            for (idx, b) in blocks.iter().enumerate() {
                if templated {
                    break;
                }
                let Some(cb) = b else { continue };
                if !safe[idx] {
                    continue;
                }
                let name = format!("xezim_aot_cb_{idx}");
                if let Some(src) = super::aot::gen_block_fn(
                    &name,
                    &cb.instructions,
                    cb.num_regs,
                    &self.signal_widths,
                    &self.signal_signed,
                ) {
                    names.push((idx, name));
                    fns.push(src);
                } else if verbose {
                    // Coverage census: name the first insn the emitter has
                    // no arm for (cranelift compiles these blocks; the gap
                    // is AOT-emitter breadth, not the width gate).
                    let miss = cb
                        .instructions
                        .iter()
                        .map(super::bytecode::insn_opcode_name)
                        .collect::<std::collections::HashSet<_>>();
                    eprintln!("[AOT-BAIL] eidx={} ops={:?}", idx, miss);
                }
            }
            if !fns.is_empty() {
                let ordered: Vec<String> = names.iter().map(|(_, n)| n.clone()).collect();
                let src = super::aot::module_source_named(&fns, &ordered);
                // NativeCtx step 1: hand the generated code the SoA plane
                // base so its loads skip the FFI bridge (empty mirror —
                // XEZIM_INLINE_BITS=0 — passes len 0 and keeps the bridge).
                let planes = (
                    self.signal_inline_bits.as_ptr() as u64,
                    self.signal_inline_bits.len() as u32,
                );
                if let Some(lib) = super::aot::compile_and_load(&src, verbose, planes) {
                    // Step 15: one exported table symbol instead of a
                    // dlsym per block; per-name lookup stays as fallback.
                    if let Some(api) = lib.api() {
                        for (k, (idx, _)) in names.iter().enumerate() {
                            let p = api(k as u32);
                            if !p.is_null() {
                                self.comb_jit_fns[*idx] = Some(unsafe {
                                    std::mem::transmute::<*mut u8, super::jit::JitFn>(p)
                                });
                                n += 1;
                            }
                        }
                    } else {
                        for (idx, name) in &names {
                            if let Some(f) = lib.sym(name) {
                                self.comb_jit_fns[*idx] = Some(f);
                                n += 1;
                            }
                        }
                    }
                    std::mem::forget(lib);
                }
            }
            chatter!(
                "[AOT] comb entries compiled {}/{} (width-safe candidates {})",
                n,
                self.comb_entries.len(),
                safe.iter().filter(|&&x| x).count()
            );
            return;
        }
        if let Some(jm) = self.jit_module.as_mut() {
            for (idx, b) in blocks.iter().enumerate() {
                let Some(cb) = b else { continue };
                if !safe[idx] {
                    continue;
                }
                // Bisection aid, mirroring XEZIM_JIT_ONLY on the edge-block
                // side: `XEZIM_JIT_COMB_RANGE=lo-hi` natively compiles only
                // comb entries in [lo, hi). Halving the range is how a
                // miscompiled entry gets located in ~log2(N) runs.
                {
                    static R: std::sync::OnceLock<Option<(usize, usize)>> =
                        std::sync::OnceLock::new();
                    let r = *R.get_or_init(|| {
                        std::env::var("XEZIM_JIT_COMB_RANGE").ok().and_then(|v| {
                            let (a, b) = v.split_once('-')?;
                            Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
                        })
                    });
                    if let Some((lo, hi)) = r {
                        if idx < lo || idx >= hi {
                            continue;
                        }
                    }
                }
                let compiled_fn = jm.try_compile_with_xz_hint(
                    &cb.instructions,
                    cb.num_regs,
                    xz_ptr,
                    xz_len,
                    self.comb_entries[idx].cold.scope_hint.as_deref(),
                    self.comb_entries[idx]
                        .cold
                        .ts_scope()
                        .as_deref()
                        .unwrap_or(""),
                );
                if compiled_fn.is_none() && std::env::var("XEZIM_JIT_BAIL_TRACE").is_ok() {
                    eprintln!(
                        "[JITSKIP] eidx={} insns={} first_unsupported={:?}",
                        idx,
                        cb.instructions.len(),
                        super::jit::first_unsupported(&cb.instructions)
                    );
                }
                if let Some(f) = compiled_fn {
                    if std::env::var("XEZIM_JIT_DUMP_BLOCKS").is_ok() {
                        eprintln!("[JITCOMB] eidx={} insns:", idx);
                        for i in cb.instructions.iter() {
                            eprintln!("[JITCOMB]   {:?}", i);
                        }
                    }
                    self.comb_jit_fns[idx] = Some(f);
                    n += 1;
                }
            }
        }
        if std::env::var("XEZIM_JIT_VERBOSE").is_ok() {
            eprintln!(
                "[JIT] comb entries compiled {}/{} (width-safe candidates {})",
                n,
                self.comb_entries.len(),
                safe.iter().filter(|b| **b).count()
            );
        }
    }

    /// §23.3.3: a whole-net identity port connection makes formal and actual
    /// ONE net. Substitution already rewrites the child's READS to the
    /// parent name, but the connect assign still re-drove the (mostly
    /// unread) child-port signal on every parent change — on a 200-instance
    /// RAM-macro bench those dead connects were ~85% of the settle work.
    /// Collapse them at the ID level instead: point the child NAME at the
    /// parent's signal id. Every remaining reader of the child name
    /// (no-subst body reads, edge sensitivities, dumps, hierarchical
    /// references) then transparently follows the one collapsed net, and
    /// the connect assign becomes a self-copy that `build_comb_entries`
    /// drops. Runs BEFORE edge-block compilation so bytecode binds the
    /// collapsed ids.
    /// Slot of a comb entry's shape in `prof_entry_hist`.
    pub(super) fn entry_hist_kind(item: &CombItem) -> usize {
        match item {
            CombItem::Noop => 0,
            CombItem::FastDirectCopy { .. } => 1,
            CombItem::DirectCopy { .. } => 2,
            CombItem::FastDirectFanout { .. } => 3,
            CombItem::FusedBufFanout { .. } => 4,
            CombItem::FusedAndFanout { .. } => 5,
            CombItem::FusedGate { .. } | CombItem::CtlGate { .. } => 6,
            CombItem::Udp { .. } => 7,
            CombItem::CompiledContAssign { .. } => 8,
            CombItem::CompiledAlwaysBlock { .. } => 9,
            CombItem::ContAssign { .. } => 10,
            _ => 11,
        }
    }

    /// XEZIM_PROFILE_REPORT=1: ranked profile in the spirit of the
    /// commercial simulators' reports — percentages by DESIGN UNIT (module
    /// definition), by INSTANCE, and by CONSTRUCT — computed from measured
    /// per-comb-entry and per-edge-block nanoseconds. Times outside those
    /// two attributions (NBA apply, testbench processes, scheduling) are
    /// reported as their own lines from the existing phase buckets.
    pub(super) fn print_profile_report(
        &self,
        t_nba: u64,
        t_process: u64,
        t_sched: u64,
        tally: Option<&crate::compiler::prof_sampler::SampleTally>,
    ) {
        struct Agg {
            ns: u64,
            evals: u64,
            instances: HashSet<String>,
        }
        // instance path for a comb entry: its scope hint, else the parent
        // path of its first write target.
        let entry_instance = |eidx: usize| -> String {
            let e = &self.comb_entries[eidx];
            if let Some(sc) = e.cold.scope_hint.as_deref() {
                if !sc.is_empty() {
                    return sc.to_string();
                }
            }
            e.cold
                .write_signal_ids
                .first()
                .and_then(|&id| self.name_opt(id))
                .and_then(|n| n.rsplit_once('.').map(|(p, _)| p.to_string()))
                .unwrap_or_default()
        };
        // instance path -> defining module, walking up for generate scopes.
        let def_of = |path: &str| -> String {
            let mut p = path;
            loop {
                if p.is_empty() {
                    return self.module.name.clone();
                }
                if let Some(inst) = self.instance_by_path(p) {
                    return inst.def_name.clone();
                }
                match p.rsplit_once('.') {
                    Some((up, _)) => p = up,
                    None => return self.module.name.clone(),
                }
            }
        };
        let mut by_def: HashMap<String, Agg> = HashMap::default();
        let mut by_inst: HashMap<String, (u64, u64)> = HashMap::default();
        // Labels are built only for the 20 constructs printed: a comb
        // entry's label joins every written signal name, and building all
        // of them first cost 35 ms of string work on a 24k-entry design.
        let mut constructs: Vec<(u64, u64, Option<usize>, String)> = Vec::new();
        let mut attributed: u64 = 0;
        let (comb_samples, edge_samples, ns_per_sample): (&[u64], &[u64], f64) = match tally {
            Some(t) => (&t.comb, &t.edge, t.ns_per_sample),
            None => (&[], &[], 0.0),
        };
        // Every evaluated entry is aggregated, sampled or not, so the eval
        // and instance columns stay complete; unsampled entries carry 0 ns.
        for eidx in 0..self.comb_entries.len() {
            let n = comb_samples.get(eidx).copied().unwrap_or(0);
            let evals = self.prof_entry_counts.get(eidx).copied().unwrap_or(0);
            if n == 0 && evals == 0 {
                continue;
            }
            let ns = (n as f64 * ns_per_sample) as u64;
            let inst = entry_instance(eidx);
            let def = def_of(&inst);
            attributed += ns;
            let a = by_def.entry(def).or_insert(Agg {
                ns: 0,
                evals: 0,
                instances: HashSet::default(),
            });
            a.ns += ns;
            a.evals += evals;
            a.instances.insert(inst.clone());
            let i = by_inst.entry(inst.clone()).or_insert((0, 0));
            i.0 += ns;
            i.1 += evals;
            constructs.push((ns, evals, Some(eidx), String::new()));
        }
        // Every executed block is aggregated (the per-block timer that the
        // flag also enables says which ran), sampled or not.
        for bi in 0..self.edge_blocks.len() {
            let n = edge_samples.get(bi).copied().unwrap_or(0);
            let ran = self.edge_block_exec_ns.get(bi).copied().unwrap_or(0) > 0;
            if n == 0 && !ran {
                continue;
            }
            let ns = (n as f64 * ns_per_sample) as u64;
            let inst = self.edge_blocks[bi].scope.clone();
            let def = def_of(&inst);
            attributed += ns;
            let a = by_def.entry(def).or_insert(Agg {
                ns: 0,
                evals: 0,
                instances: HashSet::default(),
            });
            a.ns += ns;
            a.instances.insert(inst.clone());
            let i = by_inst.entry(inst.clone()).or_insert((0, 0));
            i.0 += ns;
            constructs.push((
                ns,
                0,
                None,
                format!(
                    "always @(edge) at byte {} ({})",
                    self.edge_blocks[bi].stmt.span.start,
                    if inst.is_empty() {
                        "top"
                    } else {
                        inst.as_str()
                    }
                ),
            ));
        }
        // Samples that landed outside comb entries and edge blocks: AST
        // interpreted processes, waiter wake-ups, scheduling. Printed as its
        // own line so a run dominated by an interpreted testbench loop is
        // not reported as 90% attributed to the little that was sampled.
        let unattributed = tally
            .map(|t| (t.other as f64 * t.ns_per_sample) as u64)
            .unwrap_or(0)
            .saturating_sub(t_nba + t_process + t_sched);
        let proc_fsm = tally
            .map(|t| (t.proc.iter().sum::<u64>() as f64 * t.ns_per_sample) as u64)
            .unwrap_or(0);
        let other = [
            ("nba apply", t_nba),
            ("processes", t_process),
            ("scheduling", t_sched),
            ("compiled process FSMs (sampled)", proc_fsm),
            (
                "interpreted processes, waiters (sampled remainder)",
                unattributed,
            ),
        ];
        let other_total: u64 = other.iter().map(|&(_, n)| n).sum();
        let total = attributed + other_total;
        if total == 0 {
            eprintln!(
                "[xezim-profile] no attributed time (run with XEZIM_PROFILE_REPORT=1 for a full simulation)"
            );
            return;
        }
        let pct = |ns: u64| ns as f64 * 100.0 / total as f64;
        let ms = |ns: u64| ns as f64 / 1e6;
        eprintln!("================ xezim profile report ================");
        if let Some(t) = tally {
            eprintln!(
                "{} samples over {:.1}ms ({:.1} kHz); construct times are sampled estimates",
                t.total,
                t.elapsed_ns as f64 / 1e6,
                t.total as f64 / (t.elapsed_ns as f64 / 1e6)
            );
        }
        eprintln!(
            "attributed {:.1}ms in comb/edge constructs, {:.1}ms elsewhere",
            ms(attributed),
            ms(other_total)
        );
        eprintln!("---- by design unit ----------------------------------");
        eprintln!(
            "{:>7}  {:>10}  {:>10}  {:>5}  unit",
            "%", "time", "evals", "insts"
        );
        let mut defs: Vec<_> = by_def.into_iter().collect();
        defs.sort_by_key(|(_, a)| std::cmp::Reverse(a.ns));
        for (name, a) in defs.into_iter().take(20) {
            eprintln!(
                "{:>6.1}%  {:>8.1}ms  {:>10}  {:>5}  {}",
                pct(a.ns),
                ms(a.ns),
                a.evals,
                a.instances.len(),
                name
            );
        }
        eprintln!("---- by instance (top 20) ----------------------------");
        eprintln!("{:>7}  {:>10}  {:>10}  instance", "%", "time", "evals");
        let mut insts: Vec<_> = by_inst.into_iter().collect();
        insts.sort_by_key(|(_, (ns, _))| std::cmp::Reverse(*ns));
        for (name, (ns, evals)) in insts.into_iter().take(20) {
            eprintln!(
                "{:>6.1}%  {:>8.1}ms  {:>10}  {}",
                pct(ns),
                ms(ns),
                evals,
                if name.is_empty() {
                    "(top)"
                } else {
                    name.as_str()
                }
            );
        }
        eprintln!("---- by construct (top 20) ---------------------------");
        eprintln!("{:>7}  {:>10}  {:>10}  construct", "%", "time", "evals");
        constructs.sort_by_key(|&(ns, _, _, _)| std::cmp::Reverse(ns));
        for (ns, evals, eidx, label) in constructs.into_iter().take(20) {
            let label = match eidx {
                Some(e) => self.comb_entry_trace_label(e, &self.comb_entries[e]),
                None => label,
            };
            eprintln!(
                "{:>6.1}%  {:>8.1}ms  {:>10}  {}",
                pct(ns),
                ms(ns),
                evals,
                label
            );
        }
        eprintln!("---- outside constructs ------------------------------");
        for (name, ns) in other {
            eprintln!("{:>6.1}%  {:>8.1}ms  {}", pct(ns), ms(ns), name);
        }
        eprintln!("======================================================");
    }

    /// P5 two-state fast path for one comb entry. Returns true when the
    /// entry was fully evaluated in 2-state (the caller skips the 4-state
    /// stream); false = run the normal path (not lowerable, X present on a
    /// read, forces active, or warn-x bookkeeping needed).
    /// ARMOR-style template census (XEZIM_TEMPLATE_CENSUS=1): canonicalize
    /// each compiled block's signal/array ids to first-use indices, hash the
    /// canonical form (opcodes, registers, widths, constants, branch
    /// targets), and report how much DYNAMIC execution the repeated
    /// templates cover. This is the go/no-go measurement for generating one
    /// parameterized native body per template instead of per block: worth
    /// building if repeated templates cover >30-40% of native execution.
    /// Wants XEZIM_COMB_PATHS=1 + XEZIM_EDGE_BLOCK_STATS=1 for the weights.
    pub(super) fn canon_block_hash(&self, cb: &super::bytecode::CompiledBlock) -> u64 {
        use super::bytecode::{ArrayOperand, Insn};
        use std::hash::{Hash, Hasher};
        let mut h = std::hash::BuildHasher::build_hasher(
            &xezim_core::hasher::DeterministicState::default(),
        );
        let mut sig_map: HashMap<u32, u32> = HashMap::default();
        let mut arr_map: HashMap<usize, u32> = HashMap::default();
        fn canon(map: &mut HashMap<u32, u32>, id: u32) -> u32 {
            let next = map.len() as u32;
            *map.entry(id).or_insert(next)
        }
        fn canon_arr(
            map: &mut HashMap<usize, u32>,
            _smap: &mut HashMap<u32, u32>,
            a: &ArrayOperand,
        ) -> (u32, i64, i64) {
            match a {
                ArrayOperand::Dense {
                    first_id, lo, hi, ..
                } => {
                    let next = map.len() as u32;
                    (*map.entry(*first_id).or_insert(next), *lo, *hi)
                }
                ArrayOperand::Named(n) => {
                    // Name-keyed arrays canonicalize by first-use order too.
                    let key = n.as_ptr() as usize;
                    let next = map.len() as u32;
                    (*map.entry(key).or_insert(next) | 0x8000_0000, 0, 0)
                }
            }
            .into()
        }
        fn hv(h: &mut impl Hasher, v: &Value) {
            let (b, x) = v.raw_bits();
            (b, x, v.width, v.is_signed, v.is_real, v.is_fill).hash(h);
        }
        for insn in cb.instructions.iter() {
            super::bytecode::insn_opcode_name(insn).hash(&mut h);
            match insn {
                Insn::LoadSignal(d, sig) | Insn::LoadSignalSigned(d, sig) => {
                    (d, canon(&mut sig_map, *sig)).hash(&mut h)
                }
                Insn::LoadSignalBit(d, sig, b) => (d, canon(&mut sig_map, *sig), b).hash(&mut h),
                Insn::LoadSignalRange(d, sig, l, r) => {
                    (d, canon(&mut sig_map, *sig), l, r).hash(&mut h)
                }
                Insn::BranchIfSignalFalse(sig, t, b) => {
                    (canon(&mut sig_map, *sig), t, b).hash(&mut h)
                }
                Insn::BlockingAssign(sig, r, w) | Insn::NbaAssign(sig, r, w) => {
                    (canon(&mut sig_map, *sig), r, w).hash(&mut h)
                }
                Insn::NbaAssignConst(sig, k, w) => {
                    (canon(&mut sig_map, *sig), w).hash(&mut h);
                    hv(&mut h, k);
                }
                Insn::BlockingAssignRange(sig, hi, lo, r)
                | Insn::NbaAssignRange(sig, hi, lo, r) => {
                    (canon(&mut sig_map, *sig), hi, lo, r).hash(&mut h)
                }
                Insn::BlockingAssignRangeDyn(sig, a, b, r)
                | Insn::NbaAssignRangeDyn(sig, a, b, r) => {
                    (canon(&mut sig_map, *sig), a, b, r).hash(&mut h)
                }
                Insn::BlockingAssignBitDyn(sig, i, r) | Insn::NbaAssignBitDyn(sig, i, r) => {
                    (canon(&mut sig_map, *sig), i, r).hash(&mut h)
                }
                Insn::LoadArrayElem(d, arr, i) => {
                    (d, canon_arr(&mut arr_map, &mut sig_map, arr), i).hash(&mut h)
                }
                Insn::BlockingAssignArray(arr, i, r, w) | Insn::NbaAssignArray(arr, i, r, w) => {
                    (canon_arr(&mut arr_map, &mut sig_map, arr), i, r, w).hash(&mut h)
                }
                Insn::NbaAssignArrayRead(dst, arr, idx_sig, w) => (
                    canon(&mut sig_map, *dst),
                    canon_arr(&mut arr_map, &mut sig_map, arr),
                    canon(&mut sig_map, *idx_sig),
                    w,
                )
                    .hash(&mut h),
                Insn::LoadConst(d, v) => {
                    d.hash(&mut h);
                    hv(&mut h, v);
                }
                Insn::BinOpConst(d, sr, k, kind) => {
                    (d, sr, *kind as u8).hash(&mut h);
                    hv(&mut h, k);
                }
                // Register-only / control insns carry no signal identity:
                // their Debug form IS the canonical form.
                other => format!("{:?}", other).hash(&mut h),
            }
        }
        // Per-signal WIDTH and SIGNEDNESS are baked into the generated code
        // (mask constants, resize widths, sign scrubs), so two blocks with an
        // identical opcode/ordinal shape still need DIFFERENT code when their
        // mapped signals differ in width. Fold those in, or the census would
        // over-report how many blocks a single generated template can serve.
        let mut by_ord: Vec<(u32, u32)> = sig_map.iter().map(|(&id, &o)| (o, id)).collect();
        by_ord.sort_unstable();
        for (_, id) in by_ord {
            let w = self.signal_widths.get(id as usize).copied().unwrap_or(0);
            let sg = self
                .signal_signed
                .get(id as usize)
                .copied()
                .unwrap_or(false);
            (w, sg).hash(&mut h);
        }
        h.finish()
    }

    /// Island phase-1 census (`XEZIM_ISLAND_CENSUS=1`): size the
    /// synchronous-island opportunity per clock domain. Reports, for the
    /// dominant clock: edge-block count, the comb cone reachable from the
    /// domain's flop outputs (closure over the dep graph), boundary inputs
    /// (read by the cone, written outside it), internal vs named signals
    /// (named = observed under a waveform dump), and how much of the cone
    /// the native emitter could compile today (width gate). Pure analysis;
    /// the go/no-go for the island wrapper, exactly as the template census
    /// gated dedup.
    /// Cycle-mode phase-1 census (`XEZIM_CYCLE_CENSUS=1`): the comb entries
    /// that can NEVER be evaluated in a single levelized pass — members of a
    /// non-trivial strongly connected component of the entry graph (entry
    /// A writes a signal entry B reads, and B reaches A), and self-looping
    /// level-sensitive blocks (latches: a block that reads what it writes).
    /// Everything else is a DAG and evaluates once per edge in topo order.
    /// See the `clock_tree_by_root` field. Only fused-gate entries qualify
    /// (Buf1 / Bin2 / Mux2 / UdpLut3 with a clock input); the walk from a
    /// derived clock follows the entry's clock-typed input until it reaches
    /// a clock generator, and the per-root list is in evaluation order
    /// (parents before children). Disabled with `XEZIM_CLOCK_TREE=0`.
    pub(super) fn build_clock_tree(&mut self) {
        if !self.cycle_mode || std::env::var("XEZIM_CLOCK_TREE").as_deref() == Ok("0") {
            return;
        }
        let n_sig = self.signal_table.len();
        let mut is_edge_sig = vec![false; n_sig];
        for &sid in &self.edge_signal_ids {
            if sid < n_sig {
                is_edge_sig[sid] = true;
            }
        }
        let mut is_gen = vec![false; n_sig];
        for cg in &self.clock_generators {
            if cg.signal_id < n_sig {
                is_gen[cg.signal_id] = true;
            }
        }
        // single comb writer per signal; multi-writer nets (a clock BUS written
        // per bit by a generate loop) are resolved by bit range below
        let mut writer: HashMap<usize, usize> = HashMap::default();
        let mut multi: std::collections::HashSet<usize> = std::collections::HashSet::new();
        let mut writers_all: HashMap<usize, Vec<usize>> = HashMap::default();
        for (e, ent) in self.comb_entries.iter().enumerate() {
            for &w in &ent.cold.write_signal_ids {
                if writer.insert(w, e).is_some() {
                    multi.insert(w);
                }
                writers_all.entry(w).or_default().push(e);
            }
        }
        let ranges: Vec<(HashMap<usize, (u32, u32)>, Vec<(usize, u32, u32)>)> = self
            .comb_entries
            .iter()
            .map(|e| Self::comb_entry_bit_ranges(e, &self.ctl_gates))
            .collect();
        // A tree net is a generator, or a single-writer net whose writer is a
        // fused gate / direct copy with EXACTLY ONE tree-net input (the rest
        // are enables). Memoized: Some(root) / None (not a tree net).
        let mut memo: HashMap<usize, Option<usize>> = HashMap::default();
        let mut tree_entry_of: HashMap<usize, usize> = HashMap::default(); // net -> writer entry
        let mut clock_input_of: HashMap<usize, usize> = HashMap::default(); // net -> its tree-net input
        fn walk(
            sig: usize,
            hull: (u32, u32),
            n_sig: usize,
            is_gen: &[bool],
            is_edge_sig: &[bool],
            writer: &HashMap<usize, usize>,
            writers_all: &HashMap<usize, Vec<usize>>,
            ranges: &[(HashMap<usize, (u32, u32)>, Vec<(usize, u32, u32)>)],
            multi: &std::collections::HashSet<usize>,
            entries: &[CombEntry],
            memo: &mut HashMap<usize, Option<usize>>,
            tree_entry_of: &mut HashMap<usize, usize>,
            clock_input_of: &mut HashMap<usize, usize>,
            depth: u32,
        ) -> Option<usize> {
            if sig >= n_sig || depth > 64 {
                return None;
            }
            if is_gen[sig] {
                return Some(sig);
            }
            if let Some(&m) = memo.get(&sig) {
                return m;
            }
            let memoize = !multi.contains(&sig);
            if memoize {
                memo.insert(sig, None); // cycle guard
            }
            // the writer: unique, or for a multi-writer bus the one whose static
            // write range covers the reader's hull (a clock bus written per bit)
            let e: usize = if multi.contains(&sig) {
                if hull.0 == 0 && hull.1 == u32::MAX {
                    return None;
                }
                let mut hit: Option<usize> = None;
                for &w in writers_all.get(&sig).map(|v| v.as_slice()).unwrap_or(&[]) {
                    let covers = ranges[w]
                        .1
                        .iter()
                        .any(|&(ws, wlo, whi)| ws == sig && wlo <= hull.0 && hull.1 <= whi);
                    if covers {
                        if hit.is_some() {
                            return None;
                        }
                        hit = Some(w);
                    }
                }
                let Some(w) = hit else { return None };
                w
            } else {
                let Some(&e) = writer.get(&sig) else {
                    return None;
                };
                e
            };
            let ent = &entries[e];
            // Any comb entry kind can sit in a clock tree (buffers, ICG cells,
            // fanout buffers, gate regions, compiled cell bodies); stateful
            // UDPs and no-ops cannot.
            let inputs: Vec<usize> = match &ent.item {
                CombItem::Udp { .. }
                | CombItem::UdpBatch { .. }
                | CombItem::TimingCheck { .. }
                | CombItem::Noop => return None,
                _ => ent.cold.read_signal_ids.clone(),
            };
            // A non-clock output (an ICG enable latch reads the clock too)
            // must not become a tree net: only an edge signal, or a
            // single-input buffer/copy of a tree net, qualifies.
            if !is_edge_sig[sig] && inputs.len() != 1 {
                return None;
            }
            let mut found: Option<(usize, usize)> = None; // (input net, root)
            for &r in &inputs {
                let rh = ranges[e].0.get(&r).copied().unwrap_or((0, u32::MAX));
                if let Some(root) = walk(
                    r,
                    rh,
                    n_sig,
                    is_gen,
                    is_edge_sig,
                    writer,
                    writers_all,
                    ranges,
                    multi,
                    entries,
                    memo,
                    tree_entry_of,
                    clock_input_of,
                    depth + 1,
                ) {
                    if found.is_some() {
                        return None; // two clock inputs (a clock mux)
                    }
                    found = Some((r, root));
                }
            }
            let Some((inp, root)) = found else {
                return None;
            };
            if memoize {
                memo.insert(sig, Some(root));
            }
            tree_entry_of.insert(sig, e);
            clock_input_of.insert(sig, inp);
            Some(root)
        }
        // seed from every edge signal (derived clocks) — intermediates are discovered on the way
        for sid in 0..n_sig {
            if is_edge_sig[sid] && !is_gen[sid] {
                let _ = walk(
                    sid,
                    (0, u32::MAX),
                    n_sig,
                    &is_gen,
                    &is_edge_sig,
                    &writer,
                    &writers_all,
                    &ranges,
                    &multi,
                    &self.comb_entries,
                    &mut memo,
                    &mut tree_entry_of,
                    &mut clock_input_of,
                    0,
                );
            }
        }
        // depth (distance from root) per tree net, for evaluation order
        let mut depth_of: HashMap<usize, usize> = HashMap::default();
        let mut by_root: HashMap<usize, Vec<(usize, usize)>> = HashMap::default();
        for (&net, &e) in &tree_entry_of {
            let Some(Some(root)) = memo.get(&net).copied() else {
                continue;
            };
            let mut d = 0usize;
            let mut cur = net;
            while let Some(&up) = clock_input_of.get(&cur) {
                d += 1;
                cur = up;
                if d > 64 {
                    break;
                }
            }
            depth_of.insert(net, d);
            by_root.entry(root).or_default().push((d, e));
        }
        let n_ent = self.comb_entries.len();
        let mut is_entry = vec![false; n_ent];
        let mut is_sig = vec![false; n_sig];
        let mut tree: HashMap<usize, Vec<usize>> = HashMap::default();
        for (r, mut v) in by_root {
            v.sort_by_key(|&(d, e)| (d, e));
            v.dedup_by_key(|x| x.1);
            for &(_, e) in &v {
                is_entry[e] = true;
            }
            tree.insert(r, v.into_iter().map(|(_, e)| e).collect());
            is_sig[r] = true;
        }
        for (&net, _) in &tree_entry_of {
            is_sig[net] = true;
        }
        let derived_clocks = (0..n_sig)
            .filter(|&sid| is_edge_sig[sid] && tree_entry_of.contains_key(&sid))
            .count();
        if std::env::var_os("XEZIM_CLOCK_TREE_STATS").is_some() || sim_debug_enabled() {
            let total: usize = tree.values().map(|v| v.len()).sum();
            let unconv = (0..n_sig)
                .filter(|&sid| {
                    is_edge_sig[sid] && !is_gen[sid] && !tree_entry_of.contains_key(&sid)
                })
                .count();
            // hop-by-hop trace of named nets (XEZIM_CLOCK_TREE_TRACE=leaf1,leaf2,…) or of the first three unconverted clocks
            let wanted: Vec<String> = std::env::var("XEZIM_CLOCK_TREE_TRACE")
                .ok()
                .map(|v| v.split(',').map(|x| x.to_string()).collect())
                .unwrap_or_default();
            let mut shown = 0;
            for sid in 0..n_sig {
                if shown >= 3 + wanted.len() {
                    break;
                }
                if !wanted.is_empty() {
                    let leaf = self
                        .name_for_id(sid)
                        .rsplit('.')
                        .next()
                        .unwrap_or("")
                        .to_string();
                    if !wanted.iter().any(|w| *w == leaf) {
                        continue;
                    }
                } else {
                    if !is_edge_sig[sid]
                        || is_gen[sid]
                        || tree_entry_of.contains_key(&sid)
                        || !writer.contains_key(&sid)
                    {
                        continue;
                    }
                    if self.name_for_id(sid).contains("__xz_") {
                        continue;
                    }
                }
                shown += 1;
                let mut cur = sid;
                let mut trace: Vec<String> = Vec::new();
                for _ in 0..7 {
                    let leaf = self
                        .name_for_id(cur)
                        .rsplit('.')
                        .next()
                        .unwrap_or("")
                        .to_string();
                    let Some(&e) = writer.get(&cur) else {
                        trace.push(format!("{leaf}: NO WRITER"));
                        break;
                    };
                    if multi.contains(&cur) {
                        trace.push(format!("{leaf}: MULTI-WRITER"));
                        break;
                    }
                    let ent = &self.comb_entries[e];
                    let kind = match &ent.item {
                        CombItem::FusedGate { .. } => "FusedGate",
                        CombItem::CompiledContAssign { .. } => "CompiledCA",
                        CombItem::CompiledAlwaysBlock { .. } => "CompiledAB",
                        CombItem::DirectCopy { .. } | CombItem::FastDirectCopy { .. } => {
                            "DirectCopy"
                        }
                        CombItem::FastDirectFanout { .. }
                        | CombItem::FusedBufFanout { .. }
                        | CombItem::FusedAndFanout { .. } => "Fanout",
                        CombItem::VectorGate { .. } => "VectorGate",
                        CombItem::GateRegion { .. } => "GateRegion",
                        CombItem::Udp { .. } | CombItem::UdpBatch { .. } => "Udp",
                        CombItem::ContAssign { .. } => "ContAssign",
                        CombItem::AlwaysBlock { .. } => "AlwaysBlock",
                        _ => "other",
                    };
                    let ins: Vec<String> = ent
                        .cold
                        .read_signal_ids
                        .iter()
                        .map(|&r| {
                            let t = if r < n_sig && is_gen[r] {
                                "GEN"
                            } else if memo.get(&r).copied().flatten().is_some() {
                                "tree"
                            } else if r < n_sig && is_edge_sig[r] {
                                "edge"
                            } else {
                                "data"
                            };
                            format!(
                                "{}:{}",
                                self.name_for_id(r).rsplit('.').next().unwrap_or(""),
                                t
                            )
                        })
                        .collect();
                    let tn = match memo.get(&cur) {
                        Some(Some(_)) => "TREE",
                        Some(None) => "not-tree",
                        None => "unvisited",
                    };
                    trace.push(format!(
                        "{leaf}({tn}{}{}) <- {kind}[w={}] {:?}",
                        if is_edge_sig[cur] { ",edge" } else { "" },
                        if multi.contains(&cur) { ",MULTI" } else { "" },
                        ent.cold.write_signal_ids.len(),
                        ins
                    ));
                    let next = ent
                        .cold
                        .read_signal_ids
                        .iter()
                        .copied()
                        .find(|&r| {
                            r < n_sig
                                && (is_gen[r]
                                    || is_edge_sig[r]
                                    || memo.get(&r).copied().flatten().is_some())
                        })
                        .or_else(|| {
                            if ent.cold.read_signal_ids.len() == 1 {
                                Some(ent.cold.read_signal_ids[0])
                            } else {
                                None
                            }
                        });
                    match next {
                        Some(n) if n != cur => cur = n,
                        _ => break,
                    }
                    if cur < n_sig && is_gen[cur] {
                        trace.push("GEN".to_string());
                        break;
                    }
                }
                eprintln!("[CLOCK-TREE]   trace {}: {}", shown, trace.join("  ->  "));
            }
            eprintln!(
                "[CLOCK-TREE] roots={} tree entries={} derived clocks converted={} unconverted={} (edge signals {})",
                tree.len(),
                total,
                derived_clocks,
                unconv,
                self.edge_signal_ids.len()
            );
            // coverage by CLOCK ROLE: the first sensitivity of an edge block, weighted by blocks
            let mut blocks_by_clock: HashMap<usize, usize> = HashMap::default();
            for eb in self.edge_blocks.iter() {
                if let Some(s0) = eb.resolved_sensitivities.first() {
                    *blocks_by_clock.entry(s0.signal_id).or_insert(0) += 1;
                }
            }
            let (mut conv_blocks, mut gen_blocks, mut unconv_blocks) = (0usize, 0usize, 0usize);
            let mut unconv_list: Vec<(usize, usize)> = Vec::new();
            let mut conv_list: Vec<(usize, usize)> = Vec::new();
            for (&clk, &nb) in &blocks_by_clock {
                if clk < n_sig && is_gen[clk] {
                    gen_blocks += nb;
                } else if tree_entry_of.contains_key(&clk) {
                    conv_blocks += nb;
                    conv_list.push((nb, clk));
                } else {
                    unconv_blocks += nb;
                    unconv_list.push((nb, clk));
                }
            }
            unconv_list.sort_by(|a, b| b.0.cmp(&a.0));
            conv_list.sort_by(|a, b| b.0.cmp(&a.0));
            let nm = |sid: usize| {
                self.name_for_id(sid)
                    .rsplit('.')
                    .next()
                    .unwrap_or("")
                    .to_string()
            };
            eprintln!(
                "[CLOCK-TREE] edge blocks by clock role: on generators {} | on converted derived clocks {} | on unconverted {} (of {})",
                gen_blocks,
                conv_blocks,
                unconv_blocks,
                self.edge_blocks.len()
            );
            eprintln!(
                "[CLOCK-TREE]   converted top: {:?}",
                conv_list
                    .iter()
                    .take(5)
                    .map(|&(n, c)| format!("{}x{}", n, nm(c)))
                    .collect::<Vec<_>>()
            );
            eprintln!(
                "[CLOCK-TREE]   unconverted top: {:?}",
                unconv_list
                    .iter()
                    .take(8)
                    .map(|&(n, c)| {
                        let w = match writer.get(&c) {
                            None => "no-writer".to_string(),
                            Some(&e) => match &self.comb_entries[e].item {
                                CombItem::FusedGate { .. } => "gate".into(),
                                CombItem::CompiledContAssign { .. } => "compiled".into(),
                                CombItem::CompiledAlwaysBlock { .. } => "compiledAB".into(),
                                CombItem::DirectCopy { .. } | CombItem::FastDirectCopy { .. } => {
                                    "copy".into()
                                }
                                CombItem::Udp { .. } | CombItem::UdpBatch { .. } => "udp".into(),
                                _ => "other".into(),
                            },
                        };
                        format!(
                            "{}x{}[{}{}]",
                            n,
                            nm(c),
                            w,
                            if multi.contains(&c) { ",multi" } else { "" }
                        )
                    })
                    .collect::<Vec<_>>()
            );
        }
        self.clock_tree_by_root = tree;
        self.is_clock_tree_entry = is_entry;
        self.is_clock_tree_signal = is_sig;
    }

    /// Eager clock-tree pass for a root that just toggled (see the field
    /// docs). Evaluates the tree's gate entries in order; a derived clock
    /// that changed is queued for the clocks-only edge pass and marked
    /// dirty for its data readers (the settle skips its tree children).
    pub(super) fn eval_clock_tree(&mut self, root: usize) {
        let Some(list) = self.clock_tree_by_root.get(&root) else {
            return;
        };
        let list: Vec<usize> = list.clone();
        let sdf_any = !self.sdf_delays.is_empty();
        if !self.ctl_rearm_pending.is_empty() {
            for s in std::mem::take(&mut self.ctl_rearm_pending) {
                self.ctl_rearm(s);
            }
        }
        for e in list {
            self.prof_clock_tree_evals += 1;
            if !matches!(
                self.comb_entries[e].item,
                CombItem::FusedGate { .. }
                    | CombItem::CtlGate { .. }
                    | CombItem::DirectCopy { .. }
                    | CombItem::FastDirectCopy { .. }
            ) {
                // generic path: the full single-entry evaluator on a taken entry vector
                let entries = std::mem::take(&mut self.comb_entries);
                let dsts: Vec<usize> = entries[e].cold.write_signal_ids.clone();
                let before: Vec<(u64, u64)> = dsts
                    .iter()
                    .map(|&d| self.signal_table[d].raw_bits())
                    .collect();
                // the settle's fast dispatcher (two-state plan) first; the
                // four-state full evaluator only when it declines
                let handled = match &entries[e].item {
                    CombItem::CompiledContAssign { compiled, .. }
                    | CombItem::CompiledAlwaysBlock { compiled, .. } => {
                        self.dispatch_comb_plan(e, compiled)
                    }
                    _ => false,
                };
                if !handled {
                    self.eval_comb_entry_full(&entries[e]);
                }
                self.comb_entries = entries;
                for (i, &dst) in dsts.iter().enumerate() {
                    if self.signal_table[dst].raw_bits() != before[i] {
                        if let Some(&pos) = self.sig_to_edge_pos.get(dst) {
                            if pos >= 0 {
                                self.toggled_clock_positions.push(pos as usize);
                            }
                        }
                        if !self.dirty_signals[dst] {
                            self.dirty_signals[dst] = true;
                            self.dirty_list.push(dst);
                        }
                        self.dirty_any = true;
                    }
                }
                continue;
            }
            let changed_id: Option<usize> = match &self.comb_entries[e].item {
                CombItem::FusedGate { op } => {
                    let op = *op;
                    let (dst, new_bit) = self.fused_gate_eval(&op);
                    if self.fused_bit_commit(dst, new_bit, sdf_any) {
                        Some(dst.sig_id as usize)
                    } else {
                        None
                    }
                }
                CombItem::CtlGate { slot } => {
                    let slot = *slot;
                    // Masked AND still controlled: the output already holds
                    // the controlled value (its settle evaluation masked it,
                    // and no evaluation or deposit has re-armed it since),
                    // so evaluating would commit nothing. The live test
                    // covers a controlling input written but not yet
                    // settled, which this eager pass must still see.
                    let g = self.ctl_gates[slot as usize];
                    let masked = self
                        .comb_dep_edges
                        .get(g.pos as usize)
                        .is_some_and(|d| d.mask == 0);
                    if masked
                        && self.signal_table[g.other.sig_id as usize]
                            .get_bit_code(g.other.bit as usize)
                            == g.ctl
                    {
                        None
                    } else {
                        self.ctl_gate_exec_self(slot).map(|d| d.sig_id as usize)
                    }
                }
                CombItem::DirectCopy { dst_id, .. } | CombItem::FastDirectCopy { dst_id, .. } => {
                    let dst = *dst_id;
                    let Some(&src) = self.comb_entries[e].cold.read_signal_ids.first() else {
                        continue;
                    };
                    let (sv, sx) = self.signal_table[src].raw_bits();
                    let (dv, dx) = self.signal_table[dst].raw_bits();
                    if self.signal_widths[dst] <= 64
                        && self.signal_widths[src] == self.signal_widths[dst]
                        && !(sv == dv && sx == dx)
                        && self.signal_table[dst].set_inline_bits(sv, sx)
                    {
                        self.sync_mirror(dst);
                        self.table_modified = true;
                        self.after_signal_write_premirrored(dst);
                        Some(dst)
                    } else {
                        None
                    }
                }
                _ => continue,
            };
            if let Some(id) = changed_id {
                if !self.dirty_signals[id] {
                    self.dirty_signals[id] = true;
                    self.dirty_list.push(id);
                }
                self.dirty_any = true;
                if let Some(&pos) = self.sig_to_edge_pos.get(id) {
                    if pos >= 0 {
                        self.toggled_clock_positions.push(pos as usize);
                    }
                }
            }
        }
    }

    /// Bit-range view of one comb entry: per read signal the hull `[lo, hi]`
    /// of what it reads (`(0, u32::MAX)` = whole), and its write ranges.
    /// Exact for fused gates and for compiled bodies (the sixteen `SigId`
    /// instruction forms); everything else is whole-signal.
    pub(super) fn comb_entry_bit_ranges(
        entry: &CombEntry,
        ctl_gates: &[CtlGate],
    ) -> (HashMap<usize, (u32, u32)>, Vec<(usize, u32, u32)>) {
        use super::bytecode::Insn;
        let whole = (0u32, u32::MAX);
        let mut r: HashMap<usize, (u32, u32)> = HashMap::default();
        let mut w: Vec<(usize, u32, u32)> = Vec::new();
        let add = |m: &mut HashMap<usize, (u32, u32)>, sig: usize, lo: u32, hi: u32| {
            let e = m.entry(sig).or_insert((lo, hi));
            if e.0 == 0 && e.1 == u32::MAX {
                return;
            }
            if lo == 0 && hi == u32::MAX {
                *e = whole;
                return;
            }
            e.0 = e.0.min(lo);
            e.1 = e.1.max(hi);
        };
        let mut ranged = true;
        let ctl_item;
        let item = match &entry.item {
            CombItem::CtlGate { slot } => match ctl_gates.get(*slot as usize) {
                Some(g) => {
                    ctl_item = CombItem::FusedGate { op: g.op };
                    &ctl_item
                }
                None => &entry.item,
            },
            item => item,
        };
        match item {
            CombItem::FusedGate { op } => match op {
                FusedGate::Buf1 { dst, src, .. } => {
                    add(&mut r, src.sig_id as usize, src.bit, src.bit);
                    w.push((dst.sig_id as usize, dst.bit, dst.bit));
                }
                FusedGate::Bin2 { dst, a, b, .. } => {
                    add(&mut r, a.sig_id as usize, a.bit, a.bit);
                    add(&mut r, b.sig_id as usize, b.bit, b.bit);
                    w.push((dst.sig_id as usize, dst.bit, dst.bit));
                }
                FusedGate::Mux2 { dst, s, t, e, .. } => {
                    add(&mut r, s.sig_id as usize, s.bit, s.bit);
                    add(&mut r, t.sig_id as usize, t.bit, t.bit);
                    add(&mut r, e.sig_id as usize, e.bit, e.bit);
                    w.push((dst.sig_id as usize, dst.bit, dst.bit));
                }
                FusedGate::UdpLut3 {
                    dst,
                    inputs,
                    input_count,
                    ..
                } => {
                    for i in 0..(*input_count as usize).min(3) {
                        add(
                            &mut r,
                            inputs[i].sig_id as usize,
                            inputs[i].bit,
                            inputs[i].bit,
                        );
                    }
                    w.push((dst.sig_id as usize, dst.bit, dst.bit));
                }
                #[allow(unreachable_patterns)]
                _ => ranged = false,
            },
            CombItem::CompiledContAssign { compiled }
            | CombItem::CompiledAlwaysBlock { compiled, .. } => {
                for insn in &compiled.instructions {
                    match insn {
                        Insn::LoadSignal(_, sg)
                        | Insn::LoadSignalSigned(_, sg)
                        | Insn::BranchIfSignalFalse(sg, _, _) => {
                            add(&mut r, *sg as usize, 0, u32::MAX)
                        }
                        Insn::LoadSignalRange(_, sg, l, rr) => {
                            add(&mut r, *sg as usize, (*l).min(*rr), (*l).max(*rr))
                        }
                        Insn::LoadSignalBit(_, sg, bit) => add(&mut r, *sg as usize, *bit, *bit),
                        Insn::NbaAssignArrayRead(dst, _, src, _) => {
                            add(&mut r, *src as usize, 0, u32::MAX);
                            w.push((*dst as usize, 0, u32::MAX));
                        }
                        Insn::BlockingAssignRange(sg, hi, lo, _)
                        | Insn::NbaAssignRange(sg, hi, lo, _) => {
                            w.push((*sg as usize, (*lo).min(*hi), (*lo).max(*hi)))
                        }
                        Insn::BlockingAssign(sg, _, _)
                        | Insn::NbaAssign(sg, _, _)
                        | Insn::NbaAssignConst(sg, _, _)
                        | Insn::BlockingAssignString(sg, _)
                        | Insn::NbaAssignRangeDyn(sg, _, _, _)
                        | Insn::NbaAssignBitDyn(sg, _, _)
                        | Insn::BlockingAssignRangeDyn(sg, _, _, _)
                        | Insn::BlockingAssignBitDyn(sg, _, _) => {
                            w.push((*sg as usize, 0, u32::MAX))
                        }
                        Insn::StmtFallback(..) | Insn::EvalExprFallback(..) => ranged = false,
                        _ => {}
                    }
                }
                // signals the scan did not see stay whole
                for &sid in &entry.cold.read_signal_ids {
                    r.entry(sid).or_insert(whole);
                }
                for &sid in &entry.cold.write_signal_ids {
                    if !w.iter().any(|x| x.0 == sid) {
                        w.push((sid, 0, u32::MAX));
                    }
                }
            }
            _ => ranged = false,
        }
        if !ranged {
            r.clear();
            w.clear();
            for &sid in &entry.cold.read_signal_ids {
                r.insert(sid, whole);
            }
            for &sid in &entry.cold.write_signal_ids {
                w.push((sid, 0, u32::MAX));
            }
        }
        (r, w)
    }
}
