//! Simulator two state.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

impl Simulator {
    /// Execute a comb entry through its cached dispatch plan. Returns true
    /// when the evaluation ran natively (two-state island or JIT); false
    /// means the caller must interpret this evaluation. Semantics are the
    /// exact unplanned chain: Ts guard-bail falls through to the JIT slot,
    /// JIT rc 1 strikes toward disable, rc >= 2 disables immediately, and a
    /// disabled JIT downgrades the plan so later evals skip the dead slot.
    #[inline]
    pub(super) fn dispatch_comb_plan(
        &mut self,
        eidx: usize,
        compiled: &super::bytecode::CompiledBlock,
    ) -> bool {
        if eidx >= self.comb_plan.len() {
            self.comb_plan.resize(eidx + 1, CombPlan::Unresolved);
        }
        if matches!(self.comb_plan[eidx], CombPlan::Unresolved) {
            let plan = self.resolve_comb_plan(eidx, compiled);
            self.comb_plan[eidx] = plan;
        }
        match &self.comb_plan[eidx] {
            CombPlan::Ts(ts) => {
                // Raw-pointer borrow instead of an Arc clone: the clone's
                // refcount atomics were 6.75% of settle self-time post-PGO
                // (158.9M ts evals/run). SAFETY: the Arc lives in
                // `self.comb_plan[eidx]`, which nothing in
                // `ts_guard_and_exec` touches — the block executes TsInsn
                // stores into signal planes and dirty bookkeeping only; plan
                // slots mutate exclusively in `dispatch_comb_plan` /
                // `run_comb_jit_slot`, which run strictly after this call
                // returns. The pointee is heap-stable under any Vec resize.
                let ts_ptr: *const super::bytecode::TwoStateBlock = std::sync::Arc::as_ptr(ts);
                self.ts_cur_eidx = eidx as u32;
                let ok = self.ts_guard_and_exec(unsafe { &*ts_ptr });
                if !ok && self.ts_last_xbail {
                    self.ts_xbail_note(0, eidx);
                }
                if ok {
                    if self.trace_comb_paths {
                        self.note_comb_path(eidx, 0);
                    }
                    return true;
                }
                if self.ts_exec_aborted {
                    if eidx >= self.comb_plan_abortn.len() {
                        self.comb_plan_abortn.resize(eidx + 1, 0);
                    }
                    self.comb_plan_abortn[eidx] = self.comb_plan_abortn[eidx].saturating_add(1);
                    if self.comb_plan_abortn[eidx] >= TS_ABORT_DEMOTE {
                        self.comb_plan[eidx] = CombPlan::Interp;
                        if let Some(h) = self.ts_hdr.get_mut(eidx) {
                            h.len_kind = 0;
                        }
                    }
                }
                // Dynamic guard bail (force on a target, X input): the
                // unplanned chain tried the JIT next. Rare path — the full
                // slot lookup is fine here.
                self.run_comb_jit_slot(eidx)
            }
            #[cfg(feature = "jit")]
            CombPlan::Jit | CombPlan::Tpl => self.run_comb_jit_slot(eidx),
            CombPlan::Interp | CombPlan::Unresolved => false,
        }
    }

    /// The JIT half of the dispatch: call the slot if still armed, apply
    /// the strike protocol, and downgrade the plan when the slot dies.
    #[allow(unused_variables)]
    pub(super) fn run_comb_jit_slot(&mut self, eidx: usize) -> bool {
        #[cfg(feature = "jit")]
        if let Some(Some((f, off, _len))) = self.comb_tpl_fns.get(eidx).copied() {
            let map = unsafe { self.comb_tpl_map_blob.as_ptr().add(off as usize) };
            let sp: *mut u8 = self as *mut Self as *mut u8;
            match unsafe { f(sp, map) } {
                0 => {
                    self.comb_jit_strikes[eidx] = 0;
                    if self.trace_comb_paths {
                        self.note_comb_path(eidx, 1);
                    }
                    if self.profile_timing {
                        self.prof_comb_jit[0] += 1;
                    }
                    return true;
                }
                1 => {
                    if self.profile_timing {
                        self.prof_comb_jit[1] += 1;
                    }
                    const XZ_STRIKE_LIMIT: u8 = 8;
                    let st = &mut self.comb_jit_strikes[eidx];
                    *st = st.saturating_add(1);
                    if *st >= XZ_STRIKE_LIMIT {
                        self.comb_tpl_fns[eidx] = None;
                        self.downgrade_comb_plan(eidx);
                    }
                    return false;
                }
                _ => {
                    self.comb_tpl_fns[eidx] = None;
                    self.downgrade_comb_plan(eidx);
                    return false;
                }
            }
        }
        #[cfg(feature = "jit")]
        {
            if let Some(Some(f)) = self.comb_jit_fns.get(eidx).copied() {
                let sp: *mut u8 = self as *mut Self as *mut u8;
                match unsafe { f(sp) } {
                    0 => {
                        self.comb_jit_strikes[eidx] = 0;
                        if self.trace_comb_paths {
                            self.note_comb_path(eidx, 1);
                        }
                        if self.profile_timing {
                            self.prof_comb_jit[0] += 1;
                        }
                        return true;
                    }
                    1 => {
                        if self.profile_timing {
                            self.prof_comb_jit[1] += 1;
                        }
                        const XZ_STRIKE_LIMIT: u8 = 8;
                        let st = &mut self.comb_jit_strikes[eidx];
                        *st = st.saturating_add(1);
                        if *st >= XZ_STRIKE_LIMIT {
                            self.comb_jit_fns[eidx] = None;
                            self.downgrade_comb_plan(eidx);
                        }
                    }
                    _ => {
                        self.comb_jit_fns[eidx] = None;
                        self.downgrade_comb_plan(eidx);
                    }
                }
            }
        }
        false
    }

    /// A JIT slot died: re-plan without it (Ts entries keep their island —
    /// only a Jit plan decays to Interp).
    #[cfg(feature = "jit")]
    pub(super) fn downgrade_comb_plan(&mut self, eidx: usize) {
        #[cfg(feature = "jit")]
        if matches!(
            self.comb_plan.get(eidx),
            Some(CombPlan::Jit) | Some(CombPlan::Tpl)
        ) {
            self.comb_plan[eidx] = CombPlan::Interp;
        }
    }

    #[inline]
    pub(super) fn note_comb_path(&mut self, eidx: usize, path: usize) {
        if eidx >= self.comb_path_counts.len() {
            self.comb_path_counts.resize(eidx + 1, [0; 4]);
        }
        if let Some((last, at)) = self.comb_path_last.take() {
            self.comb_path_counts[last][3] += self.prof_insns_executed.saturating_sub(at);
        }
        self.comb_path_counts[eidx][path] += 1;
        if path == 2 {
            self.comb_path_last = Some((eidx, self.prof_insns_executed));
        }
    }

    /// XEZIM_COMB_PATHS=1 end-of-sim report: comb entries ranked by
    /// interpreter-path executions, with the two-state lowering bail
    /// opcode and a per-opcode census aggregated over the hot set —
    /// the work list for widening the u64/native settle coverage.
    pub(super) fn dump_comb_paths(&mut self) {
        if !self.trace_comb_paths {
            return;
        }
        let mut ranked: Vec<(usize, [u64; 4])> = self
            .comb_path_counts
            .iter()
            .enumerate()
            .filter(|(_, c)| c[2] > 0)
            .map(|(i, c)| (i, *c))
            .collect();
        // XEZIM_COMB_PATHS_BY=insns ranks by interpreted VM instructions
        // (the cost proxy) instead of interp-path evaluations.
        let by_insns = std::env::var("XEZIM_COMB_PATHS_BY").as_deref() == Ok("insns");
        ranked.sort_by_key(|(_, c)| std::cmp::Reverse(if by_insns { c[3] } else { c[2] }));
        let tot: [u64; 4] = self.comb_path_counts.iter().fold([0; 4], |mut a, c| {
            a[0] += c[0];
            a[1] += c[1];
            a[2] += c[2];
            a[3] += c[3];
            a
        });
        eprintln!(
            "[COMBPATH] totals: two_state={} jit={} interp={} interp_insns={} ({} entries interp-bound)",
            tot[0],
            tot[1],
            tot[2],
            tot[3],
            ranked.len()
        );
        let mut bail_hist: HashMap<&'static str, u64> = HashMap::default();
        let mut jit_bail_hist: HashMap<&'static str, u64> = HashMap::default();
        let mut op_hist: HashMap<&'static str, u64> = HashMap::default();
        // Depth of the ranked walk. The bail/opcode histograms below are
        // accumulated over exactly this set, so sizing a whole-population
        // opportunity (rather than just the head) needs it raised —
        // `XEZIM_COMB_PATHS_TOP=0` means "every interp-bound entry".
        let top_n: usize = std::env::var("XEZIM_COMB_PATHS_TOP")
            .ok()
            .and_then(|v| v.parse().ok())
            .map(|n: usize| if n == 0 { usize::MAX } else { n })
            .unwrap_or(40);
        for (rank, (eidx, c)) in ranked.iter().take(top_n).enumerate() {
            let Some(entry) = self.comb_entries.get(*eidx) else {
                continue;
            };
            let compiled = match &entry.item {
                CombItem::CompiledContAssign { compiled, .. }
                | CombItem::CompiledAlwaysBlock { compiled, .. } => compiled.clone(),
                other => {
                    eprintln!(
                        "[COMBPATH] #{rank} eidx={eidx} interp={} shape={:?} (non-compiled)",
                        c[2],
                        std::mem::discriminant(other)
                    );
                    continue;
                }
            };
            let lowered = super::bytecode::lower_two_state(
                &compiled,
                &self.signal_widths,
                &self.signal_signed,
                &self.signal_real,
                &self.array_first_id,
            );
            let (bail, bail_i) = if lowered.is_some() {
                // Lowered OK but still interp-bound: X/Z guard failures.
                ("(lowers; xz-guard bails)", usize::MAX)
            } else {
                let (bi, op) = super::bytecode::ts_last_bail();
                (op, bi)
            };
            let dump_n: usize = std::env::var("XEZIM_COMB_PATHS_DUMP")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            if (rank < 6 && bail == "RawHazard") || rank < dump_n {
                for (k, insn) in compiled.instructions.iter().enumerate() {
                    eprintln!("[COMBPATH]     insn[{k}]  {:?}", insn);
                }
            }
            if rank < 12 && bail_i != usize::MAX {
                let lo = bail_i.saturating_sub(2);
                let hi = (bail_i + 2).min(compiled.instructions.len());
                for k in lo..hi {
                    eprintln!(
                        "[COMBPATH]     insn[{k}]{} {:?}",
                        if k == bail_i { "*" } else { " " },
                        compiled.instructions[k]
                    );
                }
            }
            *bail_hist.entry(bail).or_insert(0) += c[2];
            let jit_bail = super::jit::first_unsupported(&compiled.instructions)
                .unwrap_or("(all insns supported; width/other gate)");
            *jit_bail_hist.entry(jit_bail).or_insert(0) += c[2];
            if rank < 20 {
                if let Some(reason) = self.block_fit_u64_reason(&compiled) {
                    eprintln!("[COMBPATH]     gate: {reason}");
                }
            }
            for insn in compiled.instructions.iter() {
                *op_hist
                    .entry(super::bytecode::insn_opcode_name(insn))
                    .or_insert(0) += c[2];
            }
            // Per-block opcode SET. Two-state lowering is all-or-nothing per
            // block, so the first-bail histogram above names only the first
            // blocker and clearing it just exposes the next one in the SAME
            // block (measured: adding a `Select` arm made the `AssignRng`
            // count RISE). Sizing "which set of arms would actually flip
            // blocks" needs the whole opcode closure per block, which this
            // line emits for offline analysis.
            if std::env::var_os("XEZIM_COMB_OPS").is_some() {
                let mut ops: Vec<&'static str> = compiled
                    .instructions
                    .iter()
                    .map(super::bytecode::insn_opcode_name)
                    .collect();
                ops.sort_unstable();
                ops.dedup();
                let why = super::bytecode::ts_last_gate();
                let gate = if bail_i != usize::MAX {
                    compiled
                        .instructions
                        .get(bail_i)
                        .map(|i| self.ts_gate_reason(i))
                        .unwrap_or("?")
                } else {
                    "(lowers; xz-guard)"
                };
                eprintln!(
                    "[COMBOPS] eidx={eidx} interp={} n={} gate={gate} why={why} bail={bail} ops={}",
                    c[2],
                    compiled.instructions.len(),
                    ops.join(",")
                );
            }
            let scope = entry.cold.scope_hint.as_deref().unwrap_or("?");
            eprintln!(
                "[COMBPATH] #{rank} eidx={eidx} interp={} ts={} jit={} insns={} vm_insns={} bail={} scope={}",
                c[2],
                c[0],
                c[1],
                compiled.instructions.len(),
                c[3],
                bail,
                scope
            );
        }
        let mut bh: Vec<_> = bail_hist.into_iter().collect();
        bh.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        eprintln!("[COMBPATH] bail histogram (weighted by interp execs, top-40 set):");
        for (op, n) in bh {
            eprintln!("[COMBPATH]   {op}: {n}");
        }
        let mut oh: Vec<_> = op_hist.into_iter().collect();
        oh.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        let mut jh: Vec<_> = jit_bail_hist.into_iter().collect();
        jh.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        eprintln!("[COMBPATH] JIT first-unsupported histogram (weighted, top-40 set):");
        for (op, n) in jh {
            eprintln!("[COMBPATH]   {op}: {n}");
        }
        eprintln!("[COMBPATH] opcode census over hot interp blocks (weighted):");
        for (op, n) in oh.into_iter().take(25) {
            eprintln!("[COMBPATH]   {op}: {n}");
        }
    }

    pub(super) fn try_two_state_in(
        &mut self,
        eidx: usize,
        compiled: &super::bytecode::CompiledBlock,
        edge: bool,
    ) -> bool {
        use std::sync::OnceLock;
        static ON: OnceLock<bool> = OnceLock::new();
        if !*ON.get_or_init(|| !matches!(std::env::var("XEZIM_TWO_STATE").as_deref(), Ok("0"))) {
            return false;
        }
        let slots = if edge {
            &mut self.ts_edge
        } else {
            &mut self.ts_comb
        };
        if eidx >= slots.len() {
            slots.resize(eidx + 1, TsSlot::Untried);
        }
        if matches!(slots[eidx], TsSlot::Untried) {
            let lowered = super::bytecode::lower_two_state(
                compiled,
                &self.signal_widths,
                &self.signal_signed,
                &self.signal_real,
                &self.array_first_id,
            );
            if lowered.is_none() && std::env::var("XEZIM_TS_DBG").is_ok() {
                let (bi, bop) = super::bytecode::ts_last_bail();
                eprintln!(
                    "[TS-NO] bail at #{} opcode={} why={}",
                    bi,
                    bop,
                    super::bytecode::ts_last_gate()
                );
                eprintln!(
                    "[TS-NO] edge={} idx={} insns={:?}",
                    edge,
                    eidx,
                    compiled
                        .instructions
                        .iter()
                        .map(super::bytecode::insn_opcode_name)
                        .collect::<Vec<_>>()
                );
            }
            let slots = if edge {
                &mut self.ts_edge
            } else {
                &mut self.ts_comb
            };
            slots[eidx] = match lowered {
                Some(ts) => TsSlot::Yes(std::sync::Arc::new(ts)),
                None => TsSlot::No,
            };
            if !edge && matches!(self.ts_comb[eidx], TsSlot::Yes(_)) {
                self.bit_sens_pending.push(eidx as u32);
            }
        }
        let slots = if edge { &self.ts_edge } else { &self.ts_comb };
        let TsSlot::Yes(ts) = &slots[eidx] else {
            return false;
        };
        // Raw view, no refcount traffic per evaluation: the slot is not
        // replaced while its block runs (demotion below happens after).
        let tp: *const super::bytecode::TwoStateBlock = std::sync::Arc::as_ptr(ts);
        self.ts_cur_eidx = if edge { u32::MAX } else { eidx as u32 };
        let ok = self.ts_guard_and_exec(unsafe { &*tp });
        if !ok && self.ts_last_xbail {
            self.ts_xbail_note(if edge { 2 } else { 1 }, eidx);
        }
        if !ok && self.ts_exec_aborted {
            let counts = if edge {
                &mut self.ts_edge_abortn
            } else {
                &mut self.ts_comb_abortn
            };
            if eidx >= counts.len() {
                counts.resize(eidx + 1, 0);
            }
            counts[eidx] = counts[eidx].saturating_add(1);
            if counts[eidx] >= TS_ABORT_DEMOTE {
                let slots = if edge {
                    &mut self.ts_edge
                } else {
                    &mut self.ts_comb
                };
                slots[eidx] = TsSlot::No;
            }
        }
        ok
    }

    /// Per-eval guards + execution: no forces (stores bypass force
    /// filtering), no warn-x (its transition notes live on the 4-state
    /// path), and every read X-free (width ≤ 64 was proven at lower time,
    /// so storage is Inline and raw_bits is exact).
    /// A two-state attempt just bailed on an x read: count it, and put the
    /// block to sleep once the streak is long enough. `kind`: 0 = comb entry
    /// through its plan / arena header, 1 = comb entry through its slot,
    /// 2 = edge block. Only ever called on a bail, so the hot path is
    /// untouched.
    pub(super) fn ts_xbail_note(&mut self, kind: u8, idx: usize) {
        let now = self.prof_ts_evals;
        let v = if kind == 2 {
            &mut self.ts_edge_xb
        } else {
            &mut self.ts_comb_xb
        };
        if idx >= v.len() {
            v.resize(idx + 1, (0, 0, 0));
        }
        let (streak, last, probe_until) = &mut v[idx];
        if *probe_until != 0 {
            // First bail since waking: inside one cadence, the block still
            // reads x and the streak escalates; later, it ran clean for a
            // while and starts over.
            if now > *probe_until {
                *streak = 0;
            }
            *probe_until = 0;
        } else if now.saturating_sub(*last) > TS_XBAIL_FORGET {
            *streak = 0;
        }
        *last = now;
        *streak = streak.saturating_add(1);
        if *streak < TS_XBAIL_STREAK {
            return;
        }
        let k = (*streak - TS_XBAIL_STREAK).min(12) as u32;
        let wake_at = now + (TS_XBAIL_SLEEP_UNIT << k);
        let mut rec = TsSleepRec {
            kind,
            idx: idx as u32,
            len_kind: 0,
            slept_at: now,
            plan: None,
            slot: None,
        };
        {
            let a = if kind == 2 {
                &mut self.ts_asleep_edge
            } else {
                &mut self.ts_asleep_comb
            };
            if idx >= a.len() {
                a.resize(idx + 1, 0);
            }
            a[idx] = 1;
        }
        match kind {
            0 => {
                // An entry runs two-state through its arena header, its
                // plan, or both; swap out whichever it has.
                let mut any = false;
                if let Some(h) = self.ts_hdr.get_mut(idx) {
                    if h.len_kind != 0 {
                        rec.len_kind = h.len_kind;
                        h.len_kind = 0;
                        any = true;
                    }
                }
                if let Some(p) = self.comb_plan.get_mut(idx) {
                    if matches!(p, CombPlan::Ts(_)) {
                        rec.plan = Some(std::mem::replace(p, CombPlan::Interp));
                        any = true;
                    }
                }
                if !any {
                    return;
                }
            }
            1 => {
                let Some(sl) = self.ts_comb.get_mut(idx) else {
                    return;
                };
                rec.slot = Some(std::mem::replace(sl, TsSlot::No));
            }
            _ => {
                let Some(sl) = self.ts_edge.get_mut(idx) else {
                    return;
                };
                rec.slot = Some(std::mem::replace(sl, TsSlot::No));
            }
        }
        // XEZIM_TS_DBG: which streams sleep, by opcode set (census input).
        static SLEEP_DBG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if *SLEEP_DBG.get_or_init(|| std::env::var_os("XEZIM_TS_DBG").is_some()) {
            let stream: Option<&[super::bytecode::TsInsn]> = match (&rec.plan, &rec.slot) {
                (Some(CombPlan::Ts(b)), _) => Some(&b.insns),
                (_, Some(TsSlot::Yes(b))) => Some(&b.insns),
                _ if rec.len_kind != 0 => {
                    let off = self.ts_hdr[idx].off as usize;
                    let len = (rec.len_kind as usize) & TS_HDR_LEN_MAX;
                    Some(&self.ts_arena[off..off + len])
                }
                _ => None,
            };
            let mut ops: Vec<String> = stream
                .map(|s| {
                    s.iter()
                        .map(|i| {
                            let d = format!("{i:?}");
                            d[..d
                                .find(|c: char| c == '(' || c == ' ' || c == '{')
                                .unwrap_or(d.len())]
                                .to_string()
                        })
                        .collect()
                })
                .unwrap_or_default();
            ops.sort_unstable();
            ops.dedup();
            eprintln!(
                "[TS-SLEEP] kind={} idx={} streak={} len={} ops={}",
                kind,
                idx,
                *streak,
                stream.map_or(0, |s| s.len()),
                ops.join(",")
            );
        }
        let key = match self.ts_sleep_free.pop() {
            Some(k) => {
                self.ts_sleep_recs[k as usize] = Some(rec);
                k
            }
            None => {
                self.ts_sleep_recs.push(Some(rec));
                (self.ts_sleep_recs.len() - 1) as u32
            }
        };
        self.ts_sleep.push(std::cmp::Reverse((wake_at, key)));
    }

    /// Restore every sleeping block whose wake time has passed. Called once
    /// per settle call, and only while something is asleep.
    pub(super) fn ts_wake_sleepers(&mut self) {
        let now = self.prof_ts_evals;
        while let Some(&std::cmp::Reverse((t, key))) = self.ts_sleep.peek() {
            if t > now {
                break;
            }
            self.ts_sleep.pop();
            let Some(rec) = self.ts_sleep_recs[key as usize].take() else {
                continue;
            };
            self.ts_sleep_free.push(key);
            let idx = rec.idx as usize;
            // One evaluation cadence, from the interpreted evaluations the
            // block made while asleep; a block that never ran gets a whole
            // sleep's worth, so its next bail counts as immediate.
            let evals = {
                let a = if rec.kind == 2 {
                    &mut self.ts_asleep_edge
                } else {
                    &mut self.ts_asleep_comb
                };
                let e = a.get(idx).copied().unwrap_or(1).saturating_sub(1) as u64;
                if let Some(x) = a.get_mut(idx) {
                    *x = 0;
                }
                e
            };
            let slept = now.saturating_sub(rec.slept_at).max(1);
            let cadence = if evals == 0 {
                slept
            } else {
                (slept / evals).max(1)
            };
            static WAKE_DBG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
            if *WAKE_DBG.get_or_init(|| std::env::var_os("XEZIM_TS_DBG").is_some()) {
                eprintln!("[TS-WAKE] kind={} idx={} evals={}", rec.kind, idx, evals);
            }
            {
                let v = if rec.kind == 2 {
                    &mut self.ts_edge_xb
                } else {
                    &mut self.ts_comb_xb
                };
                if let Some(e) = v.get_mut(idx) {
                    // Four cadences: event-driven blocks evaluate in bursts,
                    // and a persistent bailer must not be mistaken for a
                    // clean one because its next evaluation came late.
                    e.2 = now + cadence * 4;
                }
            }
            match rec.kind {
                0 => {
                    if let (Some(p), Some(plan)) = (self.comb_plan.get_mut(idx), rec.plan) {
                        *p = plan;
                    }
                    if rec.len_kind != 0 {
                        if let Some(h) = self.ts_hdr.get_mut(idx) {
                            h.len_kind = rec.len_kind;
                        }
                    }
                }
                1 => {
                    if let (Some(sl), Some(slot)) = (self.ts_comb.get_mut(idx), rec.slot) {
                        *sl = slot;
                    }
                }
                _ => {
                    if let (Some(sl), Some(slot)) = (self.ts_edge.get_mut(idx), rec.slot) {
                        *sl = slot;
                    }
                }
            }
        }
    }

    pub(super) fn ts_guard_and_exec(&mut self, ts: &super::bytecode::TwoStateBlock) -> bool {
        self.ts_exec_aborted = false;
        self.ts_last_xbail = false;
        if self.warn_x {
            self.prof_ts_bail_warnx += 1;
            return false;
        }
        // §9.3.1: a force only disqualifies a block that WRITES a forced
        // signal (the 4-state `write_sig!` drops such writes; the two-state
        // stores bypass that filter). Bailing on ANY active force made a
        // single `force` anywhere — routine in a real bench — silently
        // disable every island in the design.
        if !self.forced_signals.is_empty() && self.ts_writes_forced(ts) {
            self.prof_ts_bail_forced += 1;
            return false;
        }
        // The x/z check rides on the executor's own loads (every signal
        // read tests the xz plane it fetches alongside the value) instead
        // of a separate pass over the block's read lists: that pre-scan
        // was 11 % of a c906 run while 97 % of evaluations passed it. A
        // store made before an x read used only clean inputs, so the
        // four-state re-run writes the same value back.
        let save_base = self.ts_save_list.len();
        if !self.exec_two_state(ts) {
            self.ts_restore_saved(save_base);
            if std::mem::take(&mut self.ts_xread_bail) {
                // An x read: run the same stream on x-planes instead of
                // handing the block to the four-state VM.
                if ts.tsx && self.tsx_on && self.exec_two_state_x(&ts.insns, ts.num_regs) {
                    self.ts_save_list.truncate(save_base);
                    self.prof_tsx_evals += 1;
                    self.prof_ts_evals += 1;
                    return true;
                }
                self.prof_ts_bail_xread += 1;
                self.ts_last_xbail = true;
                return false;
            }
            self.prof_ts_bail_abort += 1;
            self.ts_exec_aborted = true;
            return false;
        }
        self.ts_save_list.truncate(save_base);
        self.prof_ts_evals += 1;
        true
    }

    /// Does this block write any currently forced signal? Walks the FORCE
    /// map (a handful of entries in practice) against the block's sorted
    /// write set, so the cost stays proportional to the forces in flight.
    /// `ts_writes_forced` for an entry addressed by index.
    #[allow(dead_code)]
    pub(super) fn ts_writes_forced_eidx(&self, eidx: usize) -> bool {
        match self.ts_comb.get(eidx) {
            Some(TsSlot::Yes(ts)) => self.ts_writes_forced(ts),
            _ => false,
        }
    }

    pub(super) fn ts_writes_forced(&self, ts: &super::bytecode::TwoStateBlock) -> bool {
        self.forced_signals.keys().any(|&f| {
            let f = f as u32;
            ts.writes.binary_search(&f).is_ok()
                || ts
                    .writes_span
                    .iter()
                    .any(|&(first, count)| f >= first && f - first < count)
        })
    }

    /// Returns false when the run ABORTED (dynamic element read hit X or an
    /// out-of-range index) — lowering guarantees no side effect has been
    /// committed at that point, so the caller re-runs the 4-state stream.
    pub(super) fn exec_two_state(&mut self, ts: &super::bytecode::TwoStateBlock) -> bool {
        self.exec_two_state_parts(&ts.insns, ts.num_regs, Self::ts_kind_of(ts))
    }

    /// Executor selector for a two-state stream: 4 = wide 512-bit
    /// registers, 3 = wide 128-bit registers, 2 = control flow, 1 =
    /// straight line.
    pub(super) fn ts_kind_of(ts: &super::bytecode::TwoStateBlock) -> u8 {
        if ts.has_wide {
            if ts.wide_words > 2 { 4 } else { 3 }
        } else if ts.has_ctrl {
            2
        } else {
            1
        }
    }

    pub(super) fn exec_two_state_parts(
        &mut self,
        insns: &[super::bytecode::TsInsn],
        num_regs: u32,
        kind: u8,
    ) -> bool {
        match kind {
            4 => self.exec_two_state_wide::<8>(insns, num_regs),
            3 => self.exec_two_state_wide::<2>(insns, num_regs),
            2 => self.exec_two_state_ctrl(insns, num_regs),
            _ => self.exec_two_state_line(insns, num_regs),
        }
    }

    /// `ts_guard_and_exec` for a comb entry whose lowered stream lives in the
    /// arena (`ts_arena` / `ts_hdr`): one dense header load and a contiguous
    /// instruction slice instead of an `Arc` deref plus a per-block heap
    /// `Vec` — the executor's cycles were dominated by those two misses.
    pub(super) fn ts_guard_and_exec_arena(&mut self, eidx: usize) -> bool {
        let h = self.ts_hdr[eidx];
        self.ts_exec_aborted = false;
        self.ts_last_xbail = false;
        if self.warn_x {
            self.prof_ts_bail_warnx += 1;
            return false;
        }
        if !self.forced_signals.is_empty() {
            if let Some(TsSlot::Yes(ts)) = self.ts_comb.get(eidx) {
                let p: *const super::bytecode::TwoStateBlock = std::sync::Arc::as_ptr(ts);
                if self.ts_writes_forced(unsafe { &*p }) {
                    self.prof_ts_bail_forced += 1;
                    return false;
                }
            }
        }
        // SAFETY: the arena only grows at lowering time, never inside an
        // executor, and the executors do not touch `ts_arena`.
        let insns: &[super::bytecode::TsInsn] = unsafe {
            std::slice::from_raw_parts(self.ts_arena.as_ptr().add(h.off as usize), h.len())
        };
        let save_base = self.ts_save_list.len();
        self.ts_cur_eidx = eidx as u32;
        if !self.exec_two_state_parts(insns, h.num_regs as u32, h.kind()) {
            self.ts_restore_saved(save_base);
            if std::mem::take(&mut self.ts_xread_bail) {
                if self.tsx_on
                    && self.ts_hdr_tsx.get(eidx).copied().unwrap_or(0) != 0
                    && self.exec_two_state_x(insns, h.num_regs as u32)
                {
                    self.ts_save_list.truncate(save_base);
                    self.prof_tsx_evals += 1;
                    self.prof_ts_evals += 1;
                    return true;
                }
                self.prof_ts_bail_xread += 1;
                #[cfg(feature = "opcode-census")]
                {
                    let e = self.ts_xbail_by_entry.entry(eidx).or_insert((0, 0));
                    e.0 += 1;
                    e.1 = h.kind();
                }
                return false;
            }
            self.prof_ts_bail_abort += 1;
            self.ts_exec_aborted = true;
            return false;
        }
        self.ts_save_list.truncate(save_base);
        self.prof_ts_evals += 1;
        true
    }

    /// Wide writeback: mirrors ts_store's bookkeeping via Value::set_words.
    /// Splice `w` bits of little-endian words `v` into `t` at `lo`; true
    /// when any bit changed.
    pub(super) fn splice_words(t: &mut Value, lo: u32, v: &[u64], w: u32) -> bool {
        let mut changed = false;
        let mut off = 0u32;
        let mut i = 0usize;
        while off < w {
            let n = (w - off).min(64) as usize;
            changed |= t.splice_bits64(lo as usize + off as usize, v[i], 0, n);
            i += 1;
            off += 64;
        }
        changed
    }

    pub(super) fn ts_store_wide(&mut self, id: usize, v: &[u64]) {
        if self.signal_table[id].set_words(v) {
            if self.ts_direct_writes {
                self.ts_direct_dirty(id, u64::MAX);
            } else if !self.dirty_signals[id] {
                self.dirty_signals[id] = true;
                self.dirty_list.push(id);
                self.dirty_any = true;
            }
            self.table_modified = true;
            self.after_signal_write(id);
        }
    }

    /// `sig[lo+w-1:lo] = v` for a 65..=128-bit source window: two splices,
    /// one round of change bookkeeping.
    pub(super) fn ts_wide_range_store2(&mut self, id: usize, lo: u32, v: &[u64], w: u32) {
        if !Self::splice_words(&mut self.signal_table[id], lo, v, w) {
            return;
        }
        self.sync_mirror(id);
        if self.ts_direct_writes {
            self.ts_direct_dirty(id, u64::MAX);
        } else if !self.dirty_signals[id] {
            self.dirty_signals[id] = true;
            self.dirty_list.push(id);
            self.dirty_any = true;
        }
        self.table_modified = true;
        self.after_signal_write(id);
    }

    /// Non-blocking twin of `ts_wide_range_store2`.
    pub(super) fn ts_wide_range_store2_nba(&mut self, id: usize, lo: u32, v: &[u64], w: u32) {
        self.queue_partial_nba(id, lo, Value::from_words(v, w));
    }

    pub(super) fn ts_store_nba_wide(&mut self, id: usize, v: &[u64], w: u32) {
        let val = Value::from_words(v, w);
        if let Some(i) = self.nba_fast_index.get(id) {
            self.nba_fast[i].value = val;
            self.nba_fast[i].mask = None;
        } else {
            self.nba_fast_index.insert(id, self.nba_fast.len());
            self.nba_fast.push(NbaFast {
                mask: None,
                block_index: 0,
                signal_id: id,
                value: val,
            });
        }
    }

    /// Executor for blocks containing wide (65..=512-bit) ops: full arm set
    /// with a pc loop, monomorphized over the wide register word count `N`
    /// (2 or 8). Kept OUT of the narrow executors so their code size (and
    /// the sbox-class dispatch) is unaffected.
    pub(super) fn exec_two_state_wide<const N: usize>(
        &mut self,
        insns: &[super::bytecode::TsInsn],
        num_regs: u32,
    ) -> bool {
        use super::bytecode::TsInsn;
        // Registers live in `self.ts_regs` and are used in place: no callee
        // below touches that vector, and it never reallocates during the
        // block (sized here), so an unbounded raw view is sound and saves
        // the take/restore pair on every entry and exit.
        if self.ts_regs.len() < num_regs as usize {
            self.ts_regs.resize(num_regs as usize, 0);
        }
        let regs: &mut [u64] = unsafe {
            std::slice::from_raw_parts_mut(self.ts_regs.as_mut_ptr(), self.ts_regs.len())
        };
        // One flat bank per word count, viewed as `[u64; N]` registers.
        let bank: &mut Vec<u64> = if N == 2 {
            &mut self.ts_wregs
        } else {
            &mut self.ts_wregs8
        };
        if bank.len() < num_regs as usize * N {
            bank.resize(num_regs as usize * N, 0);
        }
        let wregs: &mut [[u64; N]] = unsafe {
            std::slice::from_raw_parts_mut(bank.as_mut_ptr() as *mut [u64; N], bank.len() / N)
        };
        let insns_ptr = insns.as_ptr();
        let insns_len = insns.len();
        let mut pc = 0usize;
        // Signal (value, x) planes for a load. The plane mirror exists only
        // under XEZIM_INLINE_BITS / JIT runs; test for it once per block
        // instead of per load (a load through `self` cannot be hoisted past
        // the stores' `&mut self` calls). Signal ids come from the lowered
        // stream, validated against the table at lowering, and the table
        // never shrinks, so the index is unchecked (as in the store helpers).
        let mirror = !self.signal_inline_bits.is_empty();
        macro_rules! planes {
            ($s:expr) => {{
                let __s: usize = $s;
                if mirror {
                    match self.signal_inline_bits.get(__s) {
                        Some(sl) => (sl[0], sl[1]),
                        None => self.signal_table[__s].raw_bits(),
                    }
                } else {
                    debug_assert!(__s < self.signal_table.len());
                    unsafe { self.signal_table.get_unchecked(__s) }.raw_bits()
                }
            }};
        }
        macro_rules! bail {
            () => {{
                return false;
            }};
        }
        macro_rules! xbail {
            () => {{
                self.ts_xread_bail = true;
                #[cfg(feature = "opcode-census")]
                {
                    *self
                        .ts_census
                        .entry(("XBAIL", if N == 2 { "wide128" } else { "wide512" }))
                        .or_insert(0) += 1;
                }
                bail!();
            }};
        }
        #[cfg(feature = "opcode-census")]
        let census_on = self.census_enabled;
        #[cfg(feature = "opcode-census")]
        let mut census_prev: &'static str = "^";
        while pc < insns_len {
            #[cfg(feature = "opcode-census")]
            if census_on {
                let nm = super::bytecode::ts_insn_name(unsafe { &*insns_ptr.add(pc) });
                *self.ts_census.entry((census_prev, nm)).or_insert(0) += 1;
                census_prev = nm;
            }
            match unsafe { &*insns_ptr.add(pc) } {
                TsInsn::LoadSig { d, sig } => {
                    // Plane-direct: one flat load vs the Value discriminant
                    // branch (raw_bits showed at 5% of settle post-PGO).
                    // The mirror is authoritative in JIT runs and absent in
                    // non-JIT runs (fall back).
                    let (v, x) = planes!(*sig as usize);
                    if x != 0 {
                        xbail!();
                    }
                    regs[*d as usize] = v;
                }
                TsInsn::Const { d, v } => regs[*d as usize] = *v,
                TsInsn::SigBit { d, sig, bit } => {
                    let (v, x) = planes!(*sig as usize);
                    if x != 0 {
                        xbail!();
                    }
                    regs[*d as usize] = (v >> bit) & 1;
                }
                TsInsn::SigRange { d, sig, lo, mask } => {
                    let (v, x) = planes!(*sig as usize);
                    if x != 0 {
                        xbail!();
                    }
                    regs[*d as usize] = (v >> lo) & mask;
                }
                TsInsn::SigBitW { d, sig, bit } => {
                    let (v, x) = Self::raw_bits_slice(&self.signal_table[*sig as usize], *bit, 1);
                    if x != 0 {
                        xbail!();
                    }
                    regs[*d as usize] = v;
                }
                TsInsn::SigRangeW {
                    d,
                    sig,
                    lo,
                    w,
                    mask,
                } => {
                    let (v, x) = Self::raw_bits_slice(&self.signal_table[*sig as usize], *lo, *w);
                    if x != 0 {
                        xbail!();
                    }
                    regs[*d as usize] = v & mask;
                }
                TsInsn::Bit { d, s, bit } => {
                    regs[*d as usize] = (regs[*s as usize] >> bit) & 1;
                }
                TsInsn::Range { d, s, lo, mask } => {
                    regs[*d as usize] = (regs[*s as usize] >> lo) & mask;
                }
                TsInsn::Xor { d, a, b } => {
                    regs[*d as usize] = regs[*a as usize] ^ regs[*b as usize];
                }
                TsInsn::And { d, a, b } => {
                    regs[*d as usize] = regs[*a as usize] & regs[*b as usize];
                }
                TsInsn::Or { d, a, b } => {
                    regs[*d as usize] = regs[*a as usize] | regs[*b as usize];
                }
                TsInsn::Sel { d, c, a, b, .. } => {
                    regs[*d as usize] = if regs[*c as usize] != 0 {
                        regs[*a as usize]
                    } else {
                        regs[*b as usize]
                    };
                }
                TsInsn::Not { d, s, mask } => {
                    regs[*d as usize] = !regs[*s as usize] & mask;
                }
                TsInsn::XorC { d, s, k } => {
                    regs[*d as usize] = regs[*s as usize] ^ k;
                }
                TsInsn::MaskEq { d, s, mask, v } => {
                    regs[*d as usize] = ((regs[*s as usize] & mask) == *v) as u64;
                }
                TsInsn::EqC { d, s, k } => {
                    regs[*d as usize] = (regs[*s as usize] == *k) as u64;
                }
                TsInsn::Add { d, a, b, mask } => {
                    regs[*d as usize] = regs[*a as usize].wrapping_add(regs[*b as usize]) & mask;
                }
                TsInsn::AddC { d, s: sr, k, mask } => {
                    regs[*d as usize] = regs[*sr as usize].wrapping_add(*k) & mask;
                }
                TsInsn::Shl { d, a, b, w, mask } => {
                    let amt = regs[*b as usize];
                    regs[*d as usize] = if amt >= *w as u64 {
                        0
                    } else {
                        (regs[*a as usize] << amt) & mask
                    };
                }
                TsInsn::Shr { d, a, b, w } => {
                    let amt = regs[*b as usize];
                    regs[*d as usize] = if amt >= *w as u64 {
                        0
                    } else {
                        regs[*a as usize] >> amt
                    };
                }
                TsInsn::Sub { d, a, b, mask } => {
                    regs[*d as usize] = regs[*a as usize].wrapping_sub(regs[*b as usize]) & mask;
                }
                TsInsn::Eq { d, a, b } => {
                    regs[*d as usize] = (regs[*a as usize] == regs[*b as usize]) as u64;
                }
                TsInsn::Neq { d, a, b } => {
                    regs[*d as usize] = (regs[*a as usize] != regs[*b as usize]) as u64;
                }
                TsInsn::CmpS {
                    d,
                    a,
                    b,
                    kind,
                    sa,
                    sb,
                } => {
                    let x = ((regs[*a as usize] << *sa) as i64) >> *sa;
                    let y = ((regs[*b as usize] << *sb) as i64) >> *sb;
                    regs[*d as usize] = match *kind {
                        0 => x < y,
                        1 => x <= y,
                        2 => x > y,
                        _ => x >= y,
                    } as u64;
                }
                TsInsn::BitStoreNbaDyn { sig, i, s, w } => {
                    let idx = regs[*i as usize];
                    if idx < *w as u64 {
                        let bit = regs[*s as usize] & 1;
                        self.ts_bit_store_nba(*sig as usize, idx as u32, bit);
                    }
                }
                TsInsn::BitDyn { d, s, i, w } => {
                    let idx = regs[*i as usize];
                    if idx >= *w as u64 {
                        return false;
                    }
                    regs[*d as usize] = (regs[*s as usize] >> idx) & 1;
                }
                TsInsn::LoadSigNot { dl, d, sig } => {
                    let (v, x) = planes!(*sig as usize);
                    if x != 0 {
                        xbail!();
                    }
                    regs[*dl as usize] = v;
                    regs[*d as usize] = (v == 0) as u64;
                }
                TsInsn::SigRangeEqC {
                    dr,
                    d,
                    sig,
                    lo,
                    w,
                    k,
                } => {
                    let mask = if *w >= 64 { u64::MAX } else { (1u64 << *w) - 1 };
                    let mask = &mask;
                    let (v, x) = planes!(*sig as usize);
                    if x != 0 {
                        xbail!();
                    }
                    let rv = (v >> lo) & mask;
                    regs[*dr as usize] = rv;
                    regs[*d as usize] = (rv == *k) as u64;
                }
                TsInsn::LoadSigLogAnd { dl, sig, d, a, b } => {
                    let (v, x) = planes!(*sig as usize);
                    if x != 0 {
                        xbail!();
                    }
                    regs[*dl as usize] = v;
                    regs[*d as usize] = (regs[*a as usize] != 0 && regs[*b as usize] != 0) as u64;
                }
                TsInsn::LogNotAnd { dn, s, d, a, b } => {
                    regs[*dn as usize] = (regs[*s as usize] == 0) as u64;
                    regs[*d as usize] = regs[*a as usize] & regs[*b as usize];
                }
                TsInsn::LogNotLogAnd { dn, s, d, a, b } => {
                    regs[*dn as usize] = (regs[*s as usize] == 0) as u64;
                    regs[*d as usize] = (regs[*a as usize] != 0 && regs[*b as usize] != 0) as u64;
                }
                TsInsn::LogAndStore { d, a, b, sig, mask } => {
                    let rv = (regs[*a as usize] != 0 && regs[*b as usize] != 0) as u64;
                    regs[*d as usize] = rv;
                    self.ts_store(*sig as usize, rv & mask, *mask);
                }
                TsInsn::AndRangeStore {
                    d,
                    a,
                    b,
                    sig,
                    hi,
                    lo,
                } => {
                    let mask = if *hi - *lo + 1 >= 64 {
                        u64::MAX
                    } else {
                        (1u64 << (*hi - *lo + 1)) - 1
                    };
                    let mask = &mask;
                    let rv = regs[*a as usize] & regs[*b as usize];
                    regs[*d as usize] = rv;
                    self.ts_range_store(*sig as usize, rv & mask, *lo, *hi);
                }
                TsInsn::SigBitNot { db, d, sig, bit } => {
                    let (v, x) = planes!(*sig as usize);
                    if x != 0 {
                        xbail!();
                    }
                    let bv = (v >> bit) & 1;
                    regs[*db as usize] = bv;
                    regs[*d as usize] = (bv == 0) as u64;
                }
                TsInsn::LoadSig2 { d1, sig1, d2, sig2 } => {
                    let (v, x) = planes!(*sig1 as usize);
                    if x != 0 {
                        xbail!();
                    }
                    regs[*d1 as usize] = v;
                    let (v, x) = planes!(*sig2 as usize);
                    if x != 0 {
                        xbail!();
                    }
                    regs[*d2 as usize] = v;
                }
                TsInsn::SigBit2 {
                    d1,
                    sig1,
                    bit1,
                    d2,
                    sig2,
                    bit2,
                } => {
                    let (v, x) = planes!(*sig1 as usize);
                    if x != 0 {
                        xbail!();
                    }
                    regs[*d1 as usize] = (v >> bit1) & 1;
                    let (v, x) = planes!(*sig2 as usize);
                    if x != 0 {
                        xbail!();
                    }
                    regs[*d2 as usize] = (v >> bit2) & 1;
                }
                TsInsn::LoadSigLogOr { dl, sig, d, a, b } => {
                    let (v, x) = planes!(*sig as usize);
                    if x != 0 {
                        xbail!();
                    }
                    regs[*dl as usize] = v;
                    regs[*d as usize] = (regs[*a as usize] != 0 || regs[*b as usize] != 0) as u64;
                }
                TsInsn::LoadSigAnd { dl, sig, d, a, b } => {
                    let (v, x) = planes!(*sig as usize);
                    if x != 0 {
                        xbail!();
                    }
                    regs[*dl as usize] = v;
                    regs[*d as usize] = regs[*a as usize] & regs[*b as usize];
                }
                TsInsn::LoadSigRepl {
                    dl,
                    sig,
                    d,
                    w,
                    count,
                } => {
                    let (v, x) = planes!(*sig as usize);
                    if x != 0 {
                        xbail!();
                    }
                    regs[*dl as usize] = v;
                    regs[*d as usize] = replicate_narrow(v, *w, *count);
                }
                TsInsn::LoadSigSigRange {
                    dl,
                    sig,
                    d,
                    sig2,
                    lo,
                    mask,
                } => {
                    let (v, x) = planes!(*sig as usize);
                    if x != 0 {
                        xbail!();
                    }
                    regs[*dl as usize] = v;
                    let (v, x) = planes!(*sig2 as usize);
                    if x != 0 {
                        xbail!();
                    }
                    regs[*d as usize] = (v >> lo) & mask;
                }
                TsInsn::SigRangeAnd {
                    dr,
                    sig,
                    lo,
                    mask,
                    d,
                    a,
                    b,
                } => {
                    let (v, x) = planes!(*sig as usize);
                    if x != 0 {
                        xbail!();
                    }
                    regs[*dr as usize] = (v >> lo) & mask;
                    regs[*d as usize] = regs[*a as usize] & regs[*b as usize];
                }
                TsInsn::SigRangeEq {
                    dr,
                    sig,
                    lo,
                    mask,
                    d,
                    a,
                    b,
                } => {
                    let (v, x) = planes!(*sig as usize);
                    if x != 0 {
                        xbail!();
                    }
                    regs[*dr as usize] = (v >> lo) & mask;
                    regs[*d as usize] = (regs[*a as usize] == regs[*b as usize]) as u64;
                }
                TsInsn::ConstEq { dc, v, d, a, b } => {
                    regs[*dc as usize] = *v;
                    regs[*d as usize] = (regs[*a as usize] == regs[*b as usize]) as u64;
                }
                TsInsn::LogOrStore { d, a, b, sig, mask } => {
                    let rv = (regs[*a as usize] != 0 || regs[*b as usize] != 0) as u64;
                    regs[*d as usize] = rv;
                    self.ts_store(*sig as usize, rv & mask, *mask);
                }
                TsInsn::AndOr {
                    d1,
                    a1,
                    b1,
                    d,
                    a,
                    b,
                } => {
                    regs[*d1 as usize] = regs[*a1 as usize] & regs[*b1 as usize];
                    regs[*d as usize] = regs[*a as usize] | regs[*b as usize];
                }
                TsInsn::OrRangeStore {
                    d,
                    a,
                    b,
                    sig,
                    hi,
                    lo,
                } => {
                    let mask = if *hi - *lo + 1 >= 64 {
                        u64::MAX
                    } else {
                        (1u64 << (*hi - *lo + 1)) - 1
                    };
                    let mask = &mask;
                    let rv = regs[*a as usize] | regs[*b as usize];
                    regs[*d as usize] = rv;
                    self.ts_range_store(*sig as usize, rv & mask, *lo, *hi);
                }
                TsInsn::LoadSigBrNz { dl, sig, t } => {
                    let (v, x) = planes!(*sig as usize);
                    if x != 0 {
                        xbail!();
                    }
                    regs[*dl as usize] = v;
                    if v != 0 {
                        pc = *t as usize;
                        continue;
                    }
                }
                TsInsn::BrFalseLoadSig { s, t, dl, sig } => {
                    if regs[*s as usize] == 0 {
                        pc = *t as usize;
                        continue;
                    }
                    let (v, x) = planes!(*sig as usize);
                    if x != 0 {
                        xbail!();
                    }
                    regs[*dl as usize] = v;
                }
                TsInsn::EqBrFalse { d, a, b, t } => {
                    let rv = (regs[*a as usize] == regs[*b as usize]) as u64;
                    regs[*d as usize] = rv;
                    if rv == 0 {
                        pc = *t as usize;
                        continue;
                    }
                }
                TsInsn::EqCBrFalse { d, s, k, t } => {
                    let rv = (regs[*s as usize] == *k) as u64;
                    regs[*d as usize] = rv;
                    if rv == 0 {
                        pc = *t as usize;
                        continue;
                    }
                }
                TsInsn::Lt { d, a, b } => {
                    regs[*d as usize] = ((regs[*a as usize]) < (regs[*b as usize])) as u64;
                }
                TsInsn::Leq { d, a, b } => {
                    regs[*d as usize] = (regs[*a as usize] <= regs[*b as usize]) as u64;
                }
                TsInsn::Gt { d, a, b } => {
                    regs[*d as usize] = (regs[*a as usize] > regs[*b as usize]) as u64;
                }
                TsInsn::Geq { d, a, b } => {
                    regs[*d as usize] = (regs[*a as usize] >= regs[*b as usize]) as u64;
                }
                TsInsn::LogNot { d, s } => {
                    regs[*d as usize] = (regs[*s as usize] == 0) as u64;
                }
                TsInsn::RedOr { d, s } => {
                    regs[*d as usize] = (regs[*s as usize] != 0) as u64;
                }
                TsInsn::RedAnd { d, s, mask } => {
                    regs[*d as usize] = (regs[*s as usize] == *mask) as u64;
                }
                TsInsn::CovHit(c) => self.ts_cov_hit(*c),
                TsInsn::WaitEdge { .. } | TsInsn::WaitDelayRaw { .. } => {
                    // FSM streams run on the control executor only.
                    bail!();
                }
                TsInsn::Fallback(st) => {
                    let st = st.clone();
                    self.prof_fallback_insns += 1;
                    let saved = self.ts_fallback_enter();
                    self.exec_statement(&st);
                    self.ts_fallback_exit(saved);
                    if self.finished {
                        return true;
                    }
                }
                TsInsn::SaveSig { sig } => {
                    let (v, x) = self.signal_table[*sig as usize].raw_bits();
                    self.ts_save_list.push((*sig, v, x));
                }
                TsInsn::WRedOr { d, s } => {
                    let w = wregs[*s as usize];
                    regs[*d as usize] = (w.iter().fold(0u64, |a, &b| a | b) != 0) as u64;
                }
                TsInsn::WRedAnd { d, s, w } => {
                    let x = wregs[*s as usize];
                    let (top, m) = wtop(*w);
                    let mut all = x[top] == m;
                    for i in 0..top {
                        all &= x[i] == u64::MAX;
                    }
                    regs[*d as usize] = all as u64;
                }
                TsInsn::BitStoreDyn { sig, i, s, w } => {
                    // §11.5.1: an out-of-range dynamic bit-select target
                    // drops the write (`Value::set_bit` is a no-op there).
                    let idx = regs[*i as usize];
                    if idx < *w as u64 {
                        let b = regs[*s as usize] & 1;
                        if *w > 64 {
                            self.ts_wide_bit_store(*sig as usize, idx as usize, b);
                        } else {
                            self.ts_range_store(*sig as usize, b, idx as u32, idx as u32);
                        }
                    }
                }
                // Dynamic-offset slice and range stores: one arm, one call,
                // so the executor loop's register allocation stays as it was
                // (three inline arms cost a c906 SoC 0.8%, outlined bodies
                // still 0.5%).
                insn @ (TsInsn::SigRangeDyn { .. }
                | TsInsn::RangeStoreDyn { .. }
                | TsInsn::RangeStoreNbaDyn { .. }
                | TsInsn::SaveSigW { .. }
                | TsInsn::WNbaFromElem(..)
                | TsInsn::RangeFillXW { .. }
                | TsInsn::LoopPoll) => match unsafe { self.ts_exec_dyn(insn, regs.as_mut_ptr()) } {
                    0 => {}
                    1 => return false,
                    _ => xbail!(),
                },
                TsInsn::ConstStoreX { sig, v, x } => {
                    self.ts_store_xz(*sig as usize, *v, *x);
                }
                TsInsn::RangeFillW { sig, hi, lo, bit } => {
                    self.ts_wide_range_fill(*sig as usize, *lo, *hi, *bit);
                }
                TsInsn::RangeFillNbaW { sig, hi, lo, bit } => {
                    self.ts_wide_range_fill_nba(*sig as usize, *lo, *hi, *bit);
                }
                TsInsn::RangeStoreNbaW {
                    sig,
                    hi,
                    lo,
                    s,
                    mask,
                } => {
                    let v = regs[*s as usize] & mask;
                    self.ts_wide_range_store_nba(*sig as usize, *lo, *hi, v);
                }
                TsInsn::RangeStoreW {
                    sig,
                    hi,
                    lo,
                    s,
                    mask,
                } => {
                    let v = regs[*s as usize] & mask;
                    self.ts_wide_range_store(*sig as usize, *lo, *hi, v, 0);
                }
                TsInsn::RangeStoreXW(p) => {
                    let (sig, hi, lo, v, x) = (&p.sig, &p.hi, &p.lo, &p.v, &p.x);
                    self.ts_wide_range_store(*sig as usize, *lo, *hi, *v, *x);
                }
                TsInsn::RangeStoreX(p) => {
                    let (sig, hi, lo, v, x) = (&p.sig, &p.hi, &p.lo, &p.v, &p.x);
                    self.ts_range_store_xz(*sig as usize, *v, *x, *lo, *hi);
                }
                TsInsn::LogAnd { d, a, b } => {
                    regs[*d as usize] = (regs[*a as usize] != 0 && regs[*b as usize] != 0) as u64;
                }
                TsInsn::LogOr { d, a, b } => {
                    regs[*d as usize] = (regs[*a as usize] != 0 || regs[*b as usize] != 0) as u64;
                }
                TsInsn::Concat { d, parts } => {
                    // §11.4.12: a single 64-bit part is already the
                    // result; do not shift an initial zero by a full word.
                    let mut acc = parts.first().map_or(0, |&(r, _)| regs[r as usize]);
                    for &(r, w) in parts.iter().skip(1) {
                        acc = (acc << w) | regs[r as usize];
                    }
                    regs[*d as usize] = acc;
                }
                TsInsn::Concat2 { d, a, wa: _, b, wb } => {
                    regs[*d as usize] = (regs[*a as usize] << *wb) | regs[*b as usize];
                }
                TsInsn::Concat3 {
                    d,
                    a,
                    wa: _,
                    b,
                    wb,
                    c,
                    wc,
                } => {
                    let acc = (regs[*a as usize] << *wb) | regs[*b as usize];
                    regs[*d as usize] = (acc << *wc) | regs[*c as usize];
                }
                TsInsn::Mask { d, mask } => {
                    regs[*d as usize] &= mask;
                }
                TsInsn::Mul { d, a, b, mask } => {
                    regs[*d as usize] = regs[*a as usize].wrapping_mul(regs[*b as usize]) & mask;
                }
                TsInsn::Repl { d, s, w, count } => {
                    let part = regs[*s as usize];
                    regs[*d as usize] = replicate_narrow(part, *w, *count);
                }
                TsInsn::WSigRange { d, sig, lo, w } => {
                    // Any X/Z bit in the slice bails to the 4-state path
                    // (the old 128-bit arm silently dropped the X plane).
                    let mut acc = [0u64; N];
                    let mut xz = 0u64;
                    {
                        let v = &self.signal_table[*sig as usize];
                        let mut off = 0u16;
                        let mut i = 0usize;
                        while off < *w {
                            let cw = (*w - off).min(64);
                            let (b, x) = Self::raw_bits_slice(v, *lo + off, cw);
                            xz |= x;
                            acc[i] = b;
                            i += 1;
                            off += 64;
                        }
                    }
                    if xz != 0 {
                        xbail!();
                    }
                    wregs[*d as usize] = acc;
                }
                TsInsn::WRepl { d, s, w, count } => {
                    let part = regs[*s as usize];
                    wregs[*d as usize] = replicate_wide(part, *w, *count);
                }
                TsInsn::BrSigFalse { sig, bit, t } => {
                    let cond = if *bit == u32::MAX {
                        let (v, x) = self.signal_table[*sig as usize].raw_bits();
                        if x != 0 {
                            xbail!();
                        }
                        v != 0
                    } else {
                        let (v, x) =
                            Self::raw_bits_slice(&self.signal_table[*sig as usize], *bit as u16, 1);
                        if x != 0 {
                            xbail!();
                        }
                        v != 0
                    };
                    if !cond {
                        pc = *t as usize;
                        continue;
                    }
                }
                TsInsn::BrFalse { s, t } => {
                    if regs[*s as usize] == 0 {
                        pc = *t as usize;
                        continue;
                    }
                }
                TsInsn::BrNz { s, t } => {
                    if regs[*s as usize] != 0 {
                        pc = *t as usize;
                        continue;
                    }
                }
                TsInsn::Jmp { t } => {
                    pc = *t as usize;
                    continue;
                }
                TsInsn::JmpPoll { t } => {
                    if loop_stop_requested() {
                        return false;
                    }
                    pc = *t as usize;
                    continue;
                }
                TsInsn::CaseJmp { s, cj } => {
                    // Two-state selector is X-free, so the 4-state x/z ->
                    // default branch is unreachable; out-of-table indices and
                    // holes still take `default`.
                    pc = cj
                        .table
                        .get(regs[*s as usize] as usize)
                        .copied()
                        .unwrap_or(cj.default) as usize;
                    continue;
                }
                TsInsn::CaseMaskJmp { s, mj: cm } => {
                    let (mask, lo, wmask, mj) = (&cm.mask, &cm.lo, &cm.wmask, &cm.mj);
                    // The window is fully defined on an X-free selector, so
                    // `xz_path` (the wildcard compare chain) cannot be taken.
                    let v = regs[*s as usize] & *mask;
                    pc = mj.table[((v >> *lo) & *wmask) as usize] as usize;
                    continue;
                }
                TsInsn::ElemLoad(op) => {
                    let i = regs[op.idx as usize] as i64;
                    if i < op.lo || i > op.hi {
                        bail!();
                    }
                    let eid = op.first as usize + (i - op.lo) as usize;
                    let (v, x) = self.signal_table[eid].raw_bits();
                    if x != 0 {
                        bail!();
                    }
                    regs[op.s as usize] = v;
                }
                TsInsn::WElemLoad(op) => {
                    let i = regs[op.idx as usize] as i64;
                    if i < op.lo || i > op.hi {
                        bail!();
                    }
                    let eid = op.first as usize + (i - op.lo) as usize;
                    let mut wv = [0u64; N];
                    if !self.signal_table[eid].words_if_clean(&mut wv) {
                        xbail!();
                    }
                    wregs[op.s as usize] = wv;
                }
                TsInsn::ElemStoreNbaFromSig(f) => {
                    // Data straight from the source signal's planes; x/z bits
                    // are queued as a 4-state value (see the variant's doc).
                    let op = &f.op;
                    let (sv, sx) = self.signal_table[f.sig as usize].raw_bits();
                    regs[f.dl as usize] = sv;
                    let i = regs[op.idx as usize] as i64;
                    if i >= op.lo && i <= op.hi {
                        let eid = op.first as usize + (i - op.lo) as usize;
                        if sx & op.mask != 0 {
                            let mut v = Value::from_inline(sv & op.mask, sx & op.mask, op.w.max(1));
                            v.is_signed = false;
                            self.ts_store_nba_val(eid, v);
                        } else {
                            self.ts_store_nba(eid, sv & op.mask, op.w);
                        }
                    }
                }
                TsInsn::ElemStoreNba(op) => {
                    let (first, lo, hi, idx, s, w, mask) =
                        (op.first, op.lo, op.hi, op.idx, op.s, op.w, op.mask);
                    let i = regs[idx as usize] as i64;
                    if i >= lo && i <= hi {
                        let eid = first as usize + (i - lo) as usize;
                        let v = regs[s as usize] & mask;
                        self.ts_store_nba(eid, v, w);
                    }
                }
                TsInsn::ElemStore(op) => {
                    let i = regs[op.idx as usize] as i64;
                    if i >= op.lo && i <= op.hi {
                        let eid = op.first as usize + (i - op.lo) as usize;
                        let v = regs[op.s as usize] & op.mask;
                        self.ts_store(eid, v, op.mask);
                    }
                }
                TsInsn::NbaFromElem(op) => {
                    let (iv, ix) = self.signal_table[op.idx_sig as usize].raw_bits();
                    if ix != 0 {
                        // §11.5.1: an x/z index reads an all-x element. Queue
                        // it as a 4-state value (the NBA queue holds Values)
                        // instead of bailing the block: an idle RAM whose
                        // address input holds x fired this bail on every
                        // clock edge (c906: 1.7M x-read bails per 100
                        // iterations, all from the 32 SPSRAM blocks).
                        let mut v = Value::new(op.w.max(1));
                        v.is_signed = false;
                        self.ts_store_nba_val(op.dst as usize, v);
                        pc += 1;
                        continue;
                    }
                    let i = iv as i64;
                    if i < op.lo || i > op.hi {
                        bail!();
                    }
                    let eid = op.first as usize + (i - op.lo) as usize;
                    let (ev, ex) = self.signal_table[eid].raw_bits();
                    let m = if op.w >= 64 {
                        u64::MAX
                    } else {
                        (1u64 << op.w) - 1
                    };
                    if ex != 0 {
                        // X element data: the destination is an NBA QUEUE
                        // entry, which holds a full 4-state Value — queue it
                        // instead of aborting the whole block. On this
                        // design 95% of cells carry X, so the abort was the
                        // common case, and a mid-block abort is what forces
                        // reads to precede every side effect.
                        let mut v = Value::from_inline(ev & m, ex & m, op.w.max(1));
                        v.is_signed = false;
                        self.ts_store_nba_val(op.dst as usize, v);
                    } else {
                        self.ts_store_nba(op.dst as usize, ev & m, op.w);
                    }
                }
                TsInsn::Store { sig, s, mask } => {
                    let v = regs[*s as usize] & mask;
                    self.ts_store(*sig as usize, v, *mask);
                }
                TsInsn::RangeStore {
                    sig,
                    hi,
                    lo,
                    s,
                    mask,
                } => {
                    // §10.4.1 blocking splice: replace only [hi:lo] and keep
                    // the surrounding bits, including any X they already
                    // carry (the eval-site prefilter proves the block's READS
                    // are X-free, not the untouched remainder of a partially
                    // written destination). `compose_inline_range_bits` is
                    // the same merge the NBA arm below uses.
                    self.ts_range_store(*sig as usize, regs[*s as usize] & mask, *lo, *hi);
                }
                TsInsn::StoreNba { sig, s, w, mask } => {
                    let v = regs[*s as usize] & mask;
                    self.ts_store_nba(*sig as usize, v, *w);
                }
                TsInsn::ConstStoreNba { sig, v, w } => {
                    self.ts_store_nba(*sig as usize, *v, *w);
                }
                TsInsn::RangeStoreNba {
                    sig,
                    hi,
                    lo,
                    s,
                    mask,
                } => {
                    self.ts_range_store_nba(*sig as usize, *lo, *hi, regs[*s as usize] & mask);
                }
                // ---- wide bank ----
                TsInsn::WLoadSig { d, sig } => {
                    let mut wv = [0u64; N];
                    if !self.signal_table[*sig as usize].words_if_clean(&mut wv) {
                        xbail!();
                    }
                    wregs[*d as usize] = wv;
                }
                TsInsn::WConst { d, v } => {
                    let mut acc = [0u64; N];
                    for (i, x) in v.iter().enumerate().take(N) {
                        acc[i] = *x;
                    }
                    wregs[*d as usize] = acc;
                }
                TsInsn::WXor { d, a, b } => {
                    let (x, y) = (wregs[*a as usize], wregs[*b as usize]);
                    let mut r = [0u64; N];
                    for i in 0..N {
                        r[i] = x[i] ^ y[i];
                    }
                    wregs[*d as usize] = r;
                }
                TsInsn::WAnd { d, a, b } => {
                    let (x, y) = (wregs[*a as usize], wregs[*b as usize]);
                    let mut r = [0u64; N];
                    for i in 0..N {
                        r[i] = x[i] & y[i];
                    }
                    wregs[*d as usize] = r;
                }
                TsInsn::WSel { d, c, a, b } => {
                    let pick = if regs[*c as usize] != 0 { *a } else { *b };
                    wregs[*d as usize] = wregs[pick as usize];
                }
                TsInsn::WOr { d, a, b } => {
                    let (x, y) = (wregs[*a as usize], wregs[*b as usize]);
                    let mut r = [0u64; N];
                    for i in 0..N {
                        r[i] = x[i] | y[i];
                    }
                    wregs[*d as usize] = r;
                }
                TsInsn::WNot { d, s, w } => {
                    let x = wregs[*s as usize];
                    let mut r = [0u64; N];
                    for i in 0..N {
                        r[i] = !x[i];
                    }
                    wmask_top(&mut r, *w);
                    wregs[*d as usize] = r;
                }
                TsInsn::WRange { d, s, lo, w } => {
                    let mut r = wshr(wregs[*s as usize], *lo as u32);
                    wmask_top(&mut r, *w);
                    wregs[*d as usize] = r;
                }
                TsInsn::RangeFromW { d, s, lo, mask } => {
                    regs[*d as usize] = wget64(&wregs[*s as usize], *lo as u32) & mask;
                }
                TsInsn::BitFromW { d, s, bit } => {
                    let x = wregs[*s as usize];
                    regs[*d as usize] = (x[(*bit >> 6) as usize] >> (*bit & 63)) & 1;
                }
                TsInsn::WConcat { d, parts } => {
                    let mut acc = [0u64; N];
                    for &(r, w, wide) in parts.iter() {
                        acc = wshl(acc, w as u32);
                        if wide {
                            let p = wregs[r as usize];
                            for i in 0..N {
                                acc[i] |= p[i];
                            }
                        } else {
                            acc[0] |= regs[r as usize];
                        }
                    }
                    wregs[*d as usize] = acc;
                }
                TsInsn::WMask { d, w } => {
                    wmask_top(&mut wregs[*d as usize], *w);
                }
                TsInsn::WFromN { r } => {
                    let mut v = [0u64; N];
                    v[0] = regs[*r as usize];
                    wregs[*r as usize] = v;
                }
                TsInsn::NFromW { r, mask } => {
                    regs[*r as usize] = wregs[*r as usize][0] & mask;
                }
                TsInsn::WStore { sig, s } => {
                    let v = wregs[*s as usize];
                    self.ts_store_wide(*sig as usize, &v);
                }
                TsInsn::WStoreNba { sig, s, w } => {
                    let v = wregs[*s as usize];
                    self.ts_store_nba_wide(*sig as usize, &v, *w);
                }
                TsInsn::WRangeStore { sig, lo, s, w } => {
                    let v = wregs[*s as usize];
                    self.ts_wide_range_store2(*sig as usize, *lo, &v, *w);
                }
                TsInsn::WRangeStoreNba { sig, lo, s, w } => {
                    let v = wregs[*s as usize];
                    self.ts_wide_range_store2_nba(*sig as usize, *lo, &v, *w);
                }
            }
            pc += 1;
        }
        true
    }

    /// Straight-line executor — no pc bookkeeping, compact codegen for the
    /// dominant comb shape. Control-flow variants are unreachable here
    /// (`has_ctrl` routed them to the pc-loop twin).
    ///
    /// Kept a function of its own: with a single caller LLVM inlined it into
    /// the dispatcher once the arm set grew, and the tiny core accessors
    /// inside (`Value::raw_bits`) then fell out of the inlining budget and
    /// became calls — a C910 SoC ran 1% more instructions with nothing new
    /// executing.
    #[inline(never)]
    pub(super) fn exec_two_state_line(
        &mut self,
        insns: &[super::bytecode::TsInsn],
        num_regs: u32,
    ) -> bool {
        use super::bytecode::TsInsn;
        // Registers live in `self.ts_regs` and are used in place: no callee
        // below touches that vector, and it never reallocates during the
        // block (sized here), so an unbounded raw view is sound and saves
        // the take/restore pair on every entry and exit.
        if self.ts_regs.len() < num_regs as usize {
            self.ts_regs.resize(num_regs as usize, 0);
        }
        let regs: &mut [u64] = unsafe {
            std::slice::from_raw_parts_mut(self.ts_regs.as_mut_ptr(), self.ts_regs.len())
        };
        // Register indices are assigned by the block compiler below
        // `num_regs`, and `regs` was just sized to it: the bounds checks on
        // every operand access are provably redundant, so go through a raw
        // pointer (this loop is a quarter of a c906 run).
        let rp: *mut u64 = regs.as_mut_ptr();
        // Signal (value, x) planes for a load. The plane mirror exists only
        // under XEZIM_INLINE_BITS / JIT runs; test for it once per block
        // instead of per load (a load through `self` cannot be hoisted past
        // the stores' `&mut self` calls). Signal ids come from the lowered
        // stream, validated against the table at lowering, and the table
        // never shrinks, so the index is unchecked (as in the store helpers).
        let mirror = !self.signal_inline_bits.is_empty();
        macro_rules! planes {
            ($s:expr) => {{
                let __s: usize = $s;
                if mirror {
                    match self.signal_inline_bits.get(__s) {
                        Some(sl) => (sl[0], sl[1]),
                        None => self.signal_table[__s].raw_bits(),
                    }
                } else {
                    debug_assert!(__s < self.signal_table.len());
                    self.signal_table.get_unchecked(__s).raw_bits()
                }
            }};
        }
        macro_rules! r {
            ($i:expr) => {
                (*rp.add($i as usize))
            };
        }
        macro_rules! xbail {
            () => {{
                self.ts_xread_bail = true;
                #[cfg(feature = "opcode-census")]
                {
                    *self.ts_census.entry(("XBAIL", "line")).or_insert(0) += 1;
                }
                return false;
            }};
        }
        #[cfg(feature = "opcode-census")]
        let census_on = self.census_enabled;
        #[cfg(feature = "opcode-census")]
        let mut census_prev: &'static str = "^";
        for insn in insns {
            #[cfg(feature = "opcode-census")]
            if census_on {
                let nm = super::bytecode::ts_insn_name(insn);
                *self.ts_census.entry((census_prev, nm)).or_insert(0) += 1;
                census_prev = nm;
            }
            unsafe {
                match insn {
                    TsInsn::LoadSig { d, sig } => {
                        // Plane-direct: one flat load vs the Value discriminant
                        // branch (raw_bits showed at 5% of settle post-PGO).
                        // The mirror is authoritative in JIT runs and absent in
                        // non-JIT runs (fall back).
                        let (v, x) = planes!(*sig as usize);
                        if x != 0 {
                            xbail!();
                        }
                        r!(*d) = v;
                    }
                    TsInsn::Const { d, v } => r!(*d) = *v,
                    TsInsn::SigBit { d, sig, bit } => {
                        let (v, x) = planes!(*sig as usize);
                        if x != 0 {
                            xbail!();
                        }
                        r!(*d) = (v >> bit) & 1;
                    }
                    TsInsn::SigRange { d, sig, lo, mask } => {
                        let (v, x) = planes!(*sig as usize);
                        if x != 0 {
                            xbail!();
                        }
                        r!(*d) = (v >> lo) & mask;
                    }
                    TsInsn::SigBitW { d, sig, bit } => {
                        let (v, x) =
                            Self::raw_bits_slice(&self.signal_table[*sig as usize], *bit, 1);
                        if x != 0 {
                            xbail!();
                        }
                        r!(*d) = v;
                    }
                    TsInsn::SigRangeW {
                        d,
                        sig,
                        lo,
                        w,
                        mask,
                    } => {
                        let (v, x) =
                            Self::raw_bits_slice(&self.signal_table[*sig as usize], *lo, *w);
                        if x != 0 {
                            xbail!();
                        }
                        r!(*d) = v & mask;
                    }
                    TsInsn::Bit { d, s, bit } => {
                        r!(*d) = (r!(*s) >> bit) & 1;
                    }
                    TsInsn::Range { d, s, lo, mask } => {
                        r!(*d) = (r!(*s) >> lo) & mask;
                    }
                    TsInsn::Xor { d, a, b } => {
                        r!(*d) = r!(*a) ^ r!(*b);
                    }
                    TsInsn::And { d, a, b } => {
                        r!(*d) = r!(*a) & r!(*b);
                    }
                    TsInsn::Or { d, a, b } => {
                        r!(*d) = r!(*a) | r!(*b);
                    }
                    TsInsn::Sel { d, c, a, b, .. } => {
                        r!(*d) = if r!(*c) != 0 { r!(*a) } else { r!(*b) };
                    }
                    TsInsn::Not { d, s, mask } => {
                        r!(*d) = !r!(*s) & mask;
                    }
                    TsInsn::XorC { d, s, k } => {
                        r!(*d) = r!(*s) ^ k;
                    }
                    TsInsn::MaskEq { d, s, mask, v } => {
                        r!(*d) = ((r!(*s) & mask) == *v) as u64;
                    }
                    TsInsn::EqC { d, s, k } => {
                        r!(*d) = (r!(*s) == *k) as u64;
                    }
                    TsInsn::Add { d, a, b, mask } => {
                        r!(*d) = r!(*a).wrapping_add(r!(*b)) & mask;
                    }
                    TsInsn::AddC { d, s: sr, k, mask } => {
                        r!(*d) = r!(*sr).wrapping_add(*k) & mask;
                    }
                    TsInsn::Shl { d, a, b, w, mask } => {
                        let amt = r!(*b);
                        r!(*d) = if amt >= *w as u64 {
                            0
                        } else {
                            (r!(*a) << amt) & mask
                        };
                    }
                    TsInsn::Shr { d, a, b, w } => {
                        let amt = r!(*b);
                        r!(*d) = if amt >= *w as u64 { 0 } else { r!(*a) >> amt };
                    }
                    TsInsn::Sub { d, a, b, mask } => {
                        r!(*d) = r!(*a).wrapping_sub(r!(*b)) & mask;
                    }
                    TsInsn::Eq { d, a, b } => {
                        r!(*d) = (r!(*a) == r!(*b)) as u64;
                    }
                    TsInsn::Neq { d, a, b } => {
                        r!(*d) = (r!(*a) != r!(*b)) as u64;
                    }
                    TsInsn::CmpS {
                        d,
                        a,
                        b,
                        kind,
                        sa,
                        sb,
                    } => {
                        let x = ((r!(*a) << *sa) as i64) >> *sa;
                        let y = ((r!(*b) << *sb) as i64) >> *sb;
                        r!(*d) = match *kind {
                            0 => x < y,
                            1 => x <= y,
                            2 => x > y,
                            _ => x >= y,
                        } as u64;
                    }
                    TsInsn::BitStoreNbaDyn { sig, i, s, w } => {
                        let idx = r!(*i);
                        if idx < *w as u64 {
                            let bit = r!(*s) & 1;
                            self.ts_bit_store_nba(*sig as usize, idx as u32, bit);
                        }
                    }
                    TsInsn::BitDyn { d, s, i, w } => {
                        let idx = r!(*i);
                        if idx >= *w as u64 {
                            return false;
                        }
                        r!(*d) = (r!(*s) >> idx) & 1;
                    }
                    TsInsn::LoadSigNot { dl, d, sig } => {
                        let (v, x) = planes!(*sig as usize);
                        if x != 0 {
                            xbail!();
                        }
                        r!(*dl) = v;
                        r!(*d) = (v == 0) as u64;
                    }
                    TsInsn::SigRangeEqC {
                        dr,
                        d,
                        sig,
                        lo,
                        w,
                        k,
                    } => {
                        let mask = if *w >= 64 { u64::MAX } else { (1u64 << *w) - 1 };
                        let mask = &mask;
                        let (v, x) = planes!(*sig as usize);
                        if x != 0 {
                            xbail!();
                        }
                        let rv = (v >> lo) & mask;
                        r!(*dr) = rv;
                        r!(*d) = (rv == *k) as u64;
                    }
                    TsInsn::LoadSigLogAnd { dl, sig, d, a, b } => {
                        let (v, x) = planes!(*sig as usize);
                        if x != 0 {
                            xbail!();
                        }
                        r!(*dl) = v;
                        r!(*d) = (r!(*a) != 0 && r!(*b) != 0) as u64;
                    }
                    TsInsn::LogNotAnd { dn, s, d, a, b } => {
                        r!(*dn) = (r!(*s) == 0) as u64;
                        r!(*d) = r!(*a) & r!(*b);
                    }
                    TsInsn::LogNotLogAnd { dn, s, d, a, b } => {
                        r!(*dn) = (r!(*s) == 0) as u64;
                        r!(*d) = (r!(*a) != 0 && r!(*b) != 0) as u64;
                    }
                    TsInsn::LogAndStore { d, a, b, sig, mask } => {
                        let rv = (r!(*a) != 0 && r!(*b) != 0) as u64;
                        r!(*d) = rv;
                        self.ts_store(*sig as usize, rv & mask, *mask);
                    }
                    TsInsn::AndRangeStore {
                        d,
                        a,
                        b,
                        sig,
                        hi,
                        lo,
                    } => {
                        let mask = if *hi - *lo + 1 >= 64 {
                            u64::MAX
                        } else {
                            (1u64 << (*hi - *lo + 1)) - 1
                        };
                        let mask = &mask;
                        let rv = r!(*a) & r!(*b);
                        r!(*d) = rv;
                        self.ts_range_store(*sig as usize, rv & mask, *lo, *hi);
                    }
                    TsInsn::SigBitNot { db, d, sig, bit } => {
                        let (v, x) = planes!(*sig as usize);
                        if x != 0 {
                            xbail!();
                        }
                        let bv = (v >> bit) & 1;
                        r!(*db) = bv;
                        r!(*d) = (bv == 0) as u64;
                    }
                    TsInsn::LoadSig2 { d1, sig1, d2, sig2 } => {
                        let (v, x) = planes!(*sig1 as usize);
                        if x != 0 {
                            xbail!();
                        }
                        r!(*d1) = v;
                        let (v, x) = planes!(*sig2 as usize);
                        if x != 0 {
                            xbail!();
                        }
                        r!(*d2) = v;
                    }
                    TsInsn::SigBit2 {
                        d1,
                        sig1,
                        bit1,
                        d2,
                        sig2,
                        bit2,
                    } => {
                        let (v, x) = planes!(*sig1 as usize);
                        if x != 0 {
                            xbail!();
                        }
                        r!(*d1) = (v >> bit1) & 1;
                        let (v, x) = planes!(*sig2 as usize);
                        if x != 0 {
                            xbail!();
                        }
                        r!(*d2) = (v >> bit2) & 1;
                    }
                    TsInsn::LoadSigLogOr { dl, sig, d, a, b } => {
                        let (v, x) = planes!(*sig as usize);
                        if x != 0 {
                            xbail!();
                        }
                        r!(*dl) = v;
                        r!(*d) = (r!(*a) != 0 || r!(*b) != 0) as u64;
                    }
                    TsInsn::LoadSigAnd { dl, sig, d, a, b } => {
                        let (v, x) = planes!(*sig as usize);
                        if x != 0 {
                            xbail!();
                        }
                        r!(*dl) = v;
                        r!(*d) = r!(*a) & r!(*b);
                    }
                    TsInsn::LoadSigRepl {
                        dl,
                        sig,
                        d,
                        w,
                        count,
                    } => {
                        let (v, x) = planes!(*sig as usize);
                        if x != 0 {
                            xbail!();
                        }
                        r!(*dl) = v;
                        r!(*d) = replicate_narrow(v, *w, *count);
                    }
                    TsInsn::LoadSigSigRange {
                        dl,
                        sig,
                        d,
                        sig2,
                        lo,
                        mask,
                    } => {
                        let (v, x) = planes!(*sig as usize);
                        if x != 0 {
                            xbail!();
                        }
                        r!(*dl) = v;
                        let (v, x) = planes!(*sig2 as usize);
                        if x != 0 {
                            xbail!();
                        }
                        r!(*d) = (v >> lo) & mask;
                    }
                    TsInsn::SigRangeAnd {
                        dr,
                        sig,
                        lo,
                        mask,
                        d,
                        a,
                        b,
                    } => {
                        let (v, x) = planes!(*sig as usize);
                        if x != 0 {
                            xbail!();
                        }
                        r!(*dr) = (v >> lo) & mask;
                        r!(*d) = r!(*a) & r!(*b);
                    }
                    TsInsn::SigRangeEq {
                        dr,
                        sig,
                        lo,
                        mask,
                        d,
                        a,
                        b,
                    } => {
                        let (v, x) = planes!(*sig as usize);
                        if x != 0 {
                            xbail!();
                        }
                        r!(*dr) = (v >> lo) & mask;
                        r!(*d) = (r!(*a) == r!(*b)) as u64;
                    }
                    TsInsn::ConstEq { dc, v, d, a, b } => {
                        r!(*dc) = *v;
                        r!(*d) = (r!(*a) == r!(*b)) as u64;
                    }
                    TsInsn::LogOrStore { d, a, b, sig, mask } => {
                        let rv = (r!(*a) != 0 || r!(*b) != 0) as u64;
                        r!(*d) = rv;
                        self.ts_store(*sig as usize, rv & mask, *mask);
                    }
                    TsInsn::AndOr {
                        d1,
                        a1,
                        b1,
                        d,
                        a,
                        b,
                    } => {
                        r!(*d1) = r!(*a1) & r!(*b1);
                        r!(*d) = r!(*a) | r!(*b);
                    }
                    TsInsn::OrRangeStore {
                        d,
                        a,
                        b,
                        sig,
                        hi,
                        lo,
                    } => {
                        let mask = if *hi - *lo + 1 >= 64 {
                            u64::MAX
                        } else {
                            (1u64 << (*hi - *lo + 1)) - 1
                        };
                        let mask = &mask;
                        let rv = r!(*a) | r!(*b);
                        r!(*d) = rv;
                        self.ts_range_store(*sig as usize, rv & mask, *lo, *hi);
                    }
                    TsInsn::Lt { d, a, b } => {
                        r!(*d) = (r!(*a) < r!(*b)) as u64;
                    }
                    TsInsn::Leq { d, a, b } => {
                        r!(*d) = (r!(*a) <= r!(*b)) as u64;
                    }
                    TsInsn::Gt { d, a, b } => {
                        r!(*d) = (r!(*a) > r!(*b)) as u64;
                    }
                    TsInsn::Geq { d, a, b } => {
                        r!(*d) = (r!(*a) >= r!(*b)) as u64;
                    }
                    TsInsn::LogNot { d, s } => {
                        r!(*d) = (r!(*s) == 0) as u64;
                    }
                    TsInsn::RedOr { d, s } => {
                        r!(*d) = (r!(*s) != 0) as u64;
                    }
                    TsInsn::RedAnd { d, s, mask } => {
                        r!(*d) = (r!(*s) == *mask) as u64;
                    }
                    TsInsn::CovHit(c) => self.ts_cov_hit(*c),
                    TsInsn::WaitEdge { .. } | TsInsn::WaitDelayRaw { .. } => {
                        // FSM streams run on the control executor only.
                        return false;
                    }
                    TsInsn::Fallback(st) => {
                        let st = st.clone();
                        self.prof_fallback_insns += 1;
                        let saved = self.ts_fallback_enter();
                        self.exec_statement(&st);
                        self.ts_fallback_exit(saved);
                        if self.finished {
                            return true;
                        }
                    }
                    TsInsn::SaveSig { sig } => {
                        let (v, x) = self.signal_table[*sig as usize].raw_bits();
                        self.ts_save_list.push((*sig, v, x));
                    }
                    TsInsn::WRedOr { .. } | TsInsn::WRedAnd { .. } => {
                        unreachable!("wide insn outside the wide executor")
                    }
                    TsInsn::BitStoreDyn { sig, i, s, w } => {
                        // §11.5.1: an out-of-range dynamic bit-select target
                        // drops the write (`Value::set_bit` is a no-op there).
                        let idx = r!(*i);
                        if idx < *w as u64 {
                            let b = r!(*s) & 1;
                            if *w > 64 {
                                self.ts_wide_bit_store(*sig as usize, idx as usize, b);
                            } else {
                                self.ts_range_store(*sig as usize, b, idx as u32, idx as u32);
                            }
                        }
                    }
                    // Dynamic-offset slice and range stores: one arm, one call,
                    // so the executor loop's register allocation stays as it was
                    // (three inline arms cost a c906 SoC 0.8%, outlined bodies
                    // still 0.5%).
                    TsInsn::SigRangeDyn { .. }
                    | TsInsn::RangeStoreDyn { .. }
                    | TsInsn::RangeStoreNbaDyn { .. }
                    | TsInsn::SaveSigW { .. }
                    | TsInsn::WNbaFromElem(..)
                    | TsInsn::RangeFillXW { .. }
                    | TsInsn::LoopPoll => match self.ts_exec_dyn(insn, rp) {
                        0 => {}
                        1 => return false,
                        _ => xbail!(),
                    },
                    TsInsn::ConstStoreX { sig, v, x } => {
                        self.ts_store_xz(*sig as usize, *v, *x);
                    }
                    TsInsn::RangeFillW { sig, hi, lo, bit } => {
                        self.ts_wide_range_fill(*sig as usize, *lo, *hi, *bit);
                    }
                    TsInsn::RangeFillNbaW { sig, hi, lo, bit } => {
                        self.ts_wide_range_fill_nba(*sig as usize, *lo, *hi, *bit);
                    }
                    TsInsn::RangeStoreNbaW {
                        sig,
                        hi,
                        lo,
                        s,
                        mask,
                    } => {
                        let v = r!(*s) & mask;
                        self.ts_wide_range_store_nba(*sig as usize, *lo, *hi, v);
                    }
                    TsInsn::RangeStoreW {
                        sig,
                        hi,
                        lo,
                        s,
                        mask,
                    } => {
                        let v = r!(*s) & mask;
                        self.ts_wide_range_store(*sig as usize, *lo, *hi, v, 0);
                    }
                    TsInsn::RangeStoreXW(p) => {
                        let (sig, hi, lo, v, x) = (&p.sig, &p.hi, &p.lo, &p.v, &p.x);
                        self.ts_wide_range_store(*sig as usize, *lo, *hi, *v, *x);
                    }
                    TsInsn::RangeStoreX(p) => {
                        let (sig, hi, lo, v, x) = (&p.sig, &p.hi, &p.lo, &p.v, &p.x);
                        self.ts_range_store_xz(*sig as usize, *v, *x, *lo, *hi);
                    }
                    TsInsn::LogAnd { d, a, b } => {
                        r!(*d) = (r!(*a) != 0 && r!(*b) != 0) as u64;
                    }
                    TsInsn::LogOr { d, a, b } => {
                        r!(*d) = (r!(*a) != 0 || r!(*b) != 0) as u64;
                    }
                    TsInsn::Concat { d, parts } => {
                        let mut acc = parts.first().map_or(0, |&(r, _)| r!(r));
                        for &(r, w) in parts.iter().skip(1) {
                            acc = (acc << w) | r!(r);
                        }
                        r!(*d) = acc;
                    }
                    TsInsn::Concat2 { d, a, wa: _, b, wb } => {
                        r!(*d) = (r!(*a) << *wb) | r!(*b);
                    }
                    TsInsn::Concat3 {
                        d,
                        a,
                        wa: _,
                        b,
                        wb,
                        c,
                        wc,
                    } => {
                        let acc = (r!(*a) << *wb) | r!(*b);
                        r!(*d) = (acc << *wc) | r!(*c);
                    }
                    TsInsn::Mask { d, mask } => {
                        r!(*d) &= mask;
                    }
                    TsInsn::Mul { d, a, b, mask } => {
                        r!(*d) = r!(*a).wrapping_mul(r!(*b)) & mask;
                    }
                    TsInsn::Repl { d, s, w, count } => {
                        let part = r!(*s);
                        r!(*d) = replicate_narrow(part, *w, *count);
                    }
                    TsInsn::WSigRange { .. } => {
                        unreachable!("wide insn outside the wide executor")
                    }
                    TsInsn::WRepl { .. } => {
                        unreachable!("wide insn outside the wide executor")
                    }
                    TsInsn::ElemLoad(op) => {
                        let i = r!(op.idx) as i64;
                        if i < op.lo || i > op.hi {
                            return false;
                        }
                        let eid = op.first as usize + (i - op.lo) as usize;
                        let (v, x) = self.signal_table[eid].raw_bits();
                        if x != 0 {
                            return false;
                        }
                        r!(op.s) = v;
                    }
                    TsInsn::ElemStoreNbaFromSig(f) => {
                        // Data straight from the source signal's planes; x/z bits
                        // are queued as a 4-state value (see the variant's doc).
                        let op = &f.op;
                        let (sv, sx) = self.signal_table[f.sig as usize].raw_bits();
                        r!(f.dl) = sv;
                        let i = r!(op.idx) as i64;
                        if i >= op.lo && i <= op.hi {
                            let eid = op.first as usize + (i - op.lo) as usize;
                            if sx & op.mask != 0 {
                                let mut v =
                                    Value::from_inline(sv & op.mask, sx & op.mask, op.w.max(1));
                                v.is_signed = false;
                                self.ts_store_nba_val(eid, v);
                            } else {
                                self.ts_store_nba(eid, sv & op.mask, op.w);
                            }
                        }
                    }
                    TsInsn::ElemStoreNba(op) => {
                        let i = r!(op.idx) as i64;
                        if i >= op.lo && i <= op.hi {
                            let eid = op.first as usize + (i - op.lo) as usize;
                            let v = r!(op.s) & op.mask;
                            self.ts_store_nba(eid, v, op.w);
                        }
                        // Out-of-range: silent drop, as in 4-state.
                    }
                    TsInsn::ElemStore(op) => {
                        let i = r!(op.idx) as i64;
                        if i >= op.lo && i <= op.hi {
                            let eid = op.first as usize + (i - op.lo) as usize;
                            let v = r!(op.s) & op.mask;
                            self.ts_store(eid, v, op.mask);
                        }
                    }
                    TsInsn::NbaFromElem(op) => {
                        let (iv, ix) = self.signal_table[op.idx_sig as usize].raw_bits();
                        if ix != 0 {
                            // §11.5.1: an x/z index reads an all-x element (see
                            // the straight-line executor's arm).
                            let mut v = Value::new(op.w.max(1));
                            v.is_signed = false;
                            self.ts_store_nba_val(op.dst as usize, v);
                            continue;
                        }
                        let i = iv as i64;
                        if i < op.lo || i > op.hi {
                            return false;
                        }
                        let eid = op.first as usize + (i - op.lo) as usize;
                        let (ev, ex) = self.signal_table[eid].raw_bits();
                        if ex != 0 {
                            return false;
                        }
                        let m = if op.w >= 64 {
                            u64::MAX
                        } else {
                            (1u64 << op.w) - 1
                        };
                        self.ts_store_nba(op.dst as usize, ev & m, op.w);
                    }
                    TsInsn::Store { sig, s, mask } => {
                        let v = r!(*s) & mask;
                        self.ts_store(*sig as usize, v, *mask);
                    }
                    TsInsn::RangeStore {
                        sig,
                        hi,
                        lo,
                        s,
                        mask,
                    } => {
                        // §10.4.1 blocking splice: replace only [hi:lo] and keep
                        // the surrounding bits, including any X they already
                        // carry (the eval-site prefilter proves the block's READS
                        // are X-free, not the untouched remainder of a partially
                        // written destination). `compose_inline_range_bits` is
                        // the same merge the NBA arm below uses.
                        self.ts_range_store(*sig as usize, r!(*s) & mask, *lo, *hi);
                    }
                    TsInsn::StoreNba { sig, s, w, mask } => {
                        let v = r!(*s) & mask;
                        self.ts_store_nba(*sig as usize, v, *w);
                    }
                    TsInsn::ConstStoreNba { sig, v, w } => {
                        self.ts_store_nba(*sig as usize, *v, *w);
                    }
                    TsInsn::RangeStoreNba {
                        sig,
                        hi,
                        lo,
                        s,
                        mask,
                    } => {
                        self.ts_range_store_nba(*sig as usize, *lo, *hi, r!(*s) & mask);
                    }
                    TsInsn::WLoadSig { .. }
                    | TsInsn::WRangeStore { .. }
                    | TsInsn::WRangeStoreNba { .. }
                    | TsInsn::WConst { .. }
                    | TsInsn::WXor { .. }
                    | TsInsn::WAnd { .. }
                    | TsInsn::WOr { .. }
                    | TsInsn::WSel { .. }
                    | TsInsn::WElemLoad(..)
                    | TsInsn::WNot { .. }
                    | TsInsn::WRange { .. }
                    | TsInsn::RangeFromW { .. }
                    | TsInsn::BitFromW { .. }
                    | TsInsn::WConcat { .. }
                    | TsInsn::WMask { .. }
                    | TsInsn::WFromN { .. }
                    | TsInsn::NFromW { .. }
                    | TsInsn::WStore { .. }
                    | TsInsn::WStoreNba { .. } => {
                        unreachable!("wide insn outside the wide executor")
                    }
                    TsInsn::BrSigFalse { .. }
                    | TsInsn::BrFalse { .. }
                    | TsInsn::BrNz { .. }
                    | TsInsn::LoadSigBrNz { .. }
                    | TsInsn::BrFalseLoadSig { .. }
                    | TsInsn::EqBrFalse { .. }
                    | TsInsn::EqCBrFalse { .. }
                    | TsInsn::Jmp { .. }
                    | TsInsn::JmpPoll { .. }
                    | TsInsn::CaseJmp { .. }
                    | TsInsn::CaseMaskJmp { .. } => {
                        unreachable!("ctrl insn in straight-line block")
                    }
                }
            }
        }
        true
    }

    /// Writeback shared by both executors; mirrors the 4-state
    /// BlockingAssign fast path's bookkeeping exactly (dirty +
    /// after_signal_write, which also records §9.2 edge-exec writes).
    #[inline]
    /// Blocking partial-range store for two-state blocks: splice `v` into
    /// `signal[hi:lo]`, leaving every other bit (value AND x/z planes)
    /// untouched, then run the ordinary change/dirty/post-write bookkeeping.
    pub(crate) fn ts_range_store(&mut self, id: usize, v: u64, lo: u32, hi: u32) {
        self.ts_range_store_xz(id, v, 0, lo, hi)
    }

    /// Two-state dynamic bit store into a >64-bit signal: set the one bit in
    /// the wide planes (value known, x cleared) and run the same write
    /// bookkeeping as the inline stores. Everything else in the destination,
    /// X included, is untouched.
    pub(super) fn ts_wide_bit_store(&mut self, id: usize, idx: usize, bit: u64) {
        self.ts_wide_range_store(id, idx as u32, idx as u32, bit & 1, 0);
    }

    /// Two-state range store into a >64-bit signal: word-level splice of the
    /// ≤64-bit planes `(v, x)` over bits [hi:lo], then the same write
    /// bookkeeping as the inline stores. Untouched bits keep their value
    /// and X.
    /// §6.11.1/§10.7: a two-state destination drops x/z on every write
    /// (x/z bits read 0), as the four-state VM's stores do.
    #[inline(always)]
    pub(crate) fn ts_scrub_two_state(&self, id: usize, v: u64, x: u64) -> (u64, u64) {
        if x != 0 && self.signal_two_state.get(id).copied().unwrap_or(false) {
            (v & !x, 0)
        } else {
            (v, x)
        }
    }

    pub(super) fn ts_wide_range_store(&mut self, id: usize, lo: u32, hi: u32, v: u64, x: u64) {
        let (v, x) = self.ts_scrub_two_state(id, v, x);
        let n = (hi - lo + 1) as usize;
        if !self.signal_table[id].splice_bits64(lo as usize, v, x, n) {
            return;
        }
        self.sync_mirror(id);
        if self.ts_direct_writes {
            // Chunked by the declared width, as the readers' masks are.
            let m = super::bytecode::sig_span_mask(self.signal_widths[id], lo, n as u32);
            self.ts_direct_dirty(id, m);
        } else if !self.dirty_signals[id] {
            self.dirty_signals[id] = true;
            self.dirty_list.push(id);
            self.dirty_any = true;
        }
        self.table_modified = true;
        self.after_signal_write(id);
    }

    /// `ts_range_store` with an explicit x/z plane, for a folded 4-state
    /// constant source (`y[hi:lo] = 'x;`).
    pub(crate) fn ts_range_store_xz(&mut self, id: usize, v: u64, x: u64, lo: u32, hi: u32) {
        // Ids come from lowered streams, validated against the table when
        // the block was lowered: one unchecked entry reference replaces the
        // three bounds checks this helper paid per store.
        debug_assert!(id < self.signal_table.len());
        let (v, x) = self.ts_scrub_two_state(id, v, x);
        let entry: &mut Value = unsafe { self.signal_table.get_unchecked_mut(id) };
        let (base_v, base_x) = entry.raw_bits();
        let (new_v, new_x) = Self::compose_inline_range_bits(base_v, base_x, v, x, lo, hi);
        if new_v == base_v && new_x == base_x {
            return;
        }
        // An inline (<= 64-bit) destination changed exactly these bits.
        let mut chg = (new_v ^ base_v) | (new_x ^ base_x);
        if !entry.set_inline_bits(new_v, new_x) {
            let mut val = Value::from_inline(new_v, new_x, self.signal_widths[id]);
            val.is_signed = self.signal_signed[id];
            self.signal_table[id] = val;
            chg = u64::MAX;
        }
        self.sync_mirror(id);
        if self.ts_direct_writes {
            self.ts_direct_dirty(id, chg);
        } else if !self.dirty_signals[id] {
            self.dirty_signals[id] = true;
            self.dirty_list.push(id);
            self.dirty_any = true;
        }
        self.table_modified = true;
        self.after_signal_write(id);
    }

    /// Whole-signal blocking store of a folded 4-state constant. Mirrors
    /// `ts_store`'s bookkeeping but carries an x/z plane.
    pub(crate) fn ts_store_xz(&mut self, id: usize, v: u64, x: u64) {
        let (v, x) = self.ts_scrub_two_state(id, v, x);
        let (dv, dx) = self.signal_table[id].raw_bits();
        if v == dv && x == dx {
            return;
        }
        let mut chg = (v ^ dv) | (x ^ dx);
        if !self.signal_table[id].set_inline_bits(v, x) {
            let mut val = Value::from_inline(v, x, self.signal_widths[id]);
            val.is_signed = self.signal_signed[id];
            self.signal_table[id] = val;
            chg = u64::MAX;
        }
        self.sync_mirror(id);
        if self.ts_direct_writes {
            self.ts_direct_dirty(id, chg);
        } else if !self.dirty_signals[id] {
            self.dirty_signals[id] = true;
            self.dirty_list.push(id);
            self.dirty_any = true;
        }
        self.table_modified = true;
        self.after_signal_write(id);
    }

    // The in-place (`set_inline_bits`) stores above and below keep the
    // table Value's `is_signed` as it was: every write path stamps it from
    // `signal_signed` (write_sig!), so re-stamping here only cost a random
    // load from a 35M-entry table per store.
    /// Undo the two-state stores to the block's read-after-write signals
    /// (`TsInsn::SaveSig`), oldest value last, so the four-state re-run after
    /// a bail starts from what the block saw.
    /// `sig[hi:lo] = {N{1'bx}}` / `{N{1'bz}}` on a wide signal: both planes
    /// filled, 64 bits at a time.
    pub(super) fn ts_wide_range_fill_xz(&mut self, id: usize, lo: u32, hi: u32, vbit: u8) {
        if self.signal_two_state.get(id).copied().unwrap_or(false) {
            return self.ts_wide_range_fill(id, lo, hi, 0);
        }
        let fill_v = if vbit != 0 { u64::MAX } else { 0 };
        let mut pos = lo as usize;
        let end = hi as usize + 1;
        let mut changed = false;
        while pos < end {
            let n = (end - pos).min(64);
            let m = if n >= 64 { u64::MAX } else { (1u64 << n) - 1 };
            if self.signal_table[id].splice_bits64(pos, fill_v & m, m, n) {
                changed = true;
            }
            pos += n;
        }
        if !changed {
            return;
        }
        self.sync_mirror(id);
        if self.ts_direct_writes {
            self.ts_direct_dirty(id, u64::MAX);
        } else if !self.dirty_signals[id] {
            self.dirty_signals[id] = true;
            self.dirty_list.push(id);
            self.dirty_any = true;
        }
        self.table_modified = true;
        self.after_signal_write(id);
    }

    pub(super) fn ts_restore_saved(&mut self, base: usize) {
        while self.ts_save_list.len() > base {
            let Some((sig, v, x)) = self.ts_save_list.pop() else {
                break;
            };
            if sig & (TS_SAVE_WIDE | TS_SAVE_COV) == TS_SAVE_COV {
                if let Some(h) = self.code_cov_hits.get_mut((sig & !TS_SAVE_COV) as usize) {
                    *h = h.saturating_sub(1);
                }
                continue;
            }
            if sig & TS_SAVE_WIDE != 0 {
                // A wide save: `v` words follow, highest first.
                let id = (sig & !TS_SAVE_WIDE) as usize;
                let nw = v as usize;
                let w = self.signal_widths[id] as usize;
                let mut changed = false;
                for i in (0..nw).rev() {
                    let Some((_, wv, wx)) = self.ts_save_list.pop() else {
                        break;
                    };
                    let n = (w - i * 64).min(64);
                    if self.signal_table[id].splice_bits64(i * 64, wv, wx, n) {
                        changed = true;
                    }
                }
                if changed {
                    self.sync_mirror(id);
                    if self.ts_direct_writes {
                        self.ts_direct_dirty(id, u64::MAX);
                    } else if !self.dirty_signals[id] {
                        self.dirty_signals[id] = true;
                        self.dirty_list.push(id);
                        self.dirty_any = true;
                    }
                    self.table_modified = true;
                    self.after_signal_write(id);
                }
                continue;
            }
            let id = sig as usize;
            let entry: &mut Value = &mut self.signal_table[id];
            if entry.raw_bits() == (v, x) {
                continue;
            }
            if !entry.set_inline_bits(v, x) {
                let mut val = Value::from_inline(v, x, self.signal_widths[id].max(1));
                val.is_signed = self.signal_signed[id];
                self.signal_table[id] = val;
            }
            self.sync_mirror(id);
            if self.ts_direct_writes {
                self.ts_direct_dirty(id, u64::MAX);
            } else if !self.dirty_signals[id] {
                self.dirty_signals[id] = true;
                self.dirty_list.push(id);
                self.dirty_any = true;
            }
            self.table_modified = true;
            self.after_signal_write(id);
        }
    }

    /// Changed-bit mask (`bytecode::sig_span_mask`) of a one-bit write.
    #[inline(always)]
    pub(super) fn bit_chg_mask(&self, sig: u32, bit: u32) -> u64 {
        let w = self
            .signal_widths
            .get(sig as usize)
            .copied()
            .unwrap_or(u32::MAX);
        super::bytecode::sig_span_mask(w, bit, 1)
    }

    /// Changed-bit mask of a `width`-bit write at bit `lo`.
    #[inline(always)]
    pub(super) fn span_chg_mask(&self, sig: u32, lo: u32, width: u32) -> u64 {
        let w = self
            .signal_widths
            .get(sig as usize)
            .copied()
            .unwrap_or(u32::MAX);
        super::bytecode::sig_span_mask(w, lo, width)
    }

    /// Dirty record for a store made in direct mode (a two-state entry in
    /// the settle loop), with the store's changed bits `m` for bit-granular
    /// sensitivity (`u64::MAX` when unknown). Consecutive stores to one
    /// signal share a record; the list only ever holds the current entry's
    /// records (the settle loop drains it after each evaluation).
    #[inline(always)]
    pub(super) fn ts_direct_dirty(&mut self, id: usize, m: u64) {
        let n = self.ts_rec_n;
        if n == 0 {
            self.ts_rec0 = (id, m);
            self.ts_rec_n = 1;
            return;
        }
        let last = if n == 1 {
            &mut self.ts_rec0
        } else {
            match self.ts_recs.last_mut() {
                Some(l) => l,
                None => &mut self.ts_rec0,
            }
        };
        if last.0 == id {
            last.1 |= m;
        } else {
            self.ts_recs.push((id, m));
            self.ts_rec_n = n + 1;
        }
    }

    pub(crate) fn ts_store(&mut self, id: usize, v: u64, mask: u64) {
        debug_assert!(id < self.signal_table.len());
        let entry: &mut Value = unsafe { self.signal_table.get_unchecked_mut(id) };
        let (dv, dx) = entry.raw_bits();
        if v != (dv & mask) || (dx & mask) != 0 {
            let mut chg = ((v ^ dv) | dx) & mask;
            if entry.set_inline_bits(v, 0) {
                self.sync_mirror(id);
            } else {
                let mut val = Value::from_u64(v, self.signal_widths[id]);
                val.is_signed = self.signal_signed[id];
                self.signal_table[id] = val;
                self.sync_mirror(id);
                chg = u64::MAX;
            }
            if self.ts_direct_writes {
                self.ts_direct_dirty(id, chg);
            } else if !self.dirty_signals[id] {
                self.dirty_signals[id] = true;
                self.dirty_list.push(id);
                self.dirty_any = true;
            }
            self.table_modified = true;
            self.after_signal_write(id);
        }
    }

    /// NBA writeback: exact mirror of the `NbaAssign` exec arm — §10.4.2
    /// last-write-wins via nba_fast_index, then equality at NBA commit.
    #[inline]
    /// `ts_store_nba` for an already-built (possibly 4-STATE) Value — the
    /// NBA queue holds full Values, so an X element read by a two-state
    /// block can be queued without the block aborting.
    pub(super) fn ts_store_nba_val(&mut self, id: usize, val: Value) {
        if let Some(i) = self.nba_fast_index.get(id) {
            self.nba_fast[i].value = val;
            self.nba_fast[i].mask = None;
        } else {
            self.nba_fast_index.insert(id, self.nba_fast.len());
            self.nba_fast.push(NbaFast {
                mask: None,
                block_index: 0,
                signal_id: id,
                value: val,
            });
        }
    }

    /// `sig[idx] <= bit`: merge into the pending entry or retain a masked
    /// intent, even when the selected bit matches its current value.
    pub(super) fn ts_bit_store_nba(&mut self, id: usize, idx: u32, bit: u64) {
        self.queue_partial_nba(id, idx, Value::from_u64(bit, 1));
    }

    /// `sig[hi:lo] <= v` for a wide destination: merge a masked intent
    /// without discarding equal selected bits.
    pub(super) fn ts_wide_range_store_nba(&mut self, id: usize, lo: u32, hi: u32, v: u64) {
        self.queue_partial_nba(id, lo, Value::from_u64(v, hi - lo + 1));
    }

    /// `sig[hi:lo] = {N{bit}}` for a destination of any width: fill the
    /// window 64 bits at a time through `splice_bits64`, then the ordinary
    /// change bookkeeping when any bit moved.
    pub(super) fn ts_wide_range_fill(&mut self, id: usize, lo: u32, hi: u32, bit: u8) {
        let fill = if bit != 0 { u64::MAX } else { 0 };
        let mut pos = lo as usize;
        let end = hi as usize + 1;
        let mut changed = false;
        while pos < end {
            let n = (end - pos).min(64);
            if self.signal_table[id].splice_bits64(pos, fill, 0, n) {
                changed = true;
            }
            pos += n;
        }
        if !changed {
            return;
        }
        self.sync_mirror(id);
        if self.ts_direct_writes {
            self.ts_direct_dirty(id, u64::MAX);
        } else if !self.dirty_signals[id] {
            self.dirty_signals[id] = true;
            self.dirty_list.push(id);
            self.dirty_any = true;
        }
        self.table_modified = true;
        self.after_signal_write(id);
    }

    /// Non-blocking twin of `ts_wide_range_fill`: fill the window in the
    /// pending NBA value, retaining the selected-bit mask until apply.
    pub(super) fn ts_wide_range_fill_nba(&mut self, id: usize, lo: u32, hi: u32, bit: u8) {
        let part = if bit != 0 {
            Value::ones(hi - lo + 1)
        } else {
            Value::zero(hi - lo + 1)
        };
        self.queue_partial_nba(id, lo, part);
    }

    /// `sig[hi:lo] <= v` (narrow destination): merge into the pending NBA
    /// entry when one exists, otherwise queue a masked intent — the
    /// `RangeStoreNba` executor arm as a callable.
    /// The dynamic-offset two-state instructions, executed out of line on
    /// a raw view of the register file: 0 = done, 1 = abort (slice past
    /// the signal), 2 = x read. See the executors' arm comment.
    ///
    /// SAFETY: `regs` is the executor's register file, sized for every
    /// register index the lowered stream names.
    #[inline(never)]
    pub(super) unsafe fn ts_exec_dyn(
        &mut self,
        insn: &super::bytecode::TsInsn,
        regs: *mut u64,
    ) -> u8 {
        unsafe {
            use super::bytecode::TsInsn;
            match insn {
                // 1 = bail: the four-state re-run takes the interrupt at the
                // same back-edge (`Insn::Jump`'s poll in `exec_insns`).
                TsInsn::LoopPoll => u8::from(loop_stop_requested()),
                TsInsn::SigRangeDyn {
                    d,
                    sig,
                    i,
                    w,
                    sw,
                    mask,
                } => match self.ts_sig_range_dyn(*sig, *regs.add(*i as usize), *w, *sw) {
                    TsDyn::Ok(v) => {
                        *regs.add(*d as usize) = v & mask;
                        0
                    }
                    TsDyn::Oob => 1,
                    TsDyn::X => 2,
                },
                TsInsn::RangeStoreDyn {
                    sig,
                    i,
                    s,
                    w,
                    sw,
                    mask,
                } => {
                    let (lo, v) = (*regs.add(*i as usize), *regs.add(*s as usize) & mask);
                    (!self.ts_range_store_dyn(*sig, lo, v, *w, *sw, false)) as u8
                }
                TsInsn::RangeStoreNbaDyn {
                    sig,
                    i,
                    s,
                    w,
                    sw,
                    mask,
                } => {
                    let (lo, v) = (*regs.add(*i as usize), *regs.add(*s as usize) & mask);
                    (!self.ts_range_store_dyn(*sig, lo, v, *w, *sw, true)) as u8
                }
                TsInsn::WNbaFromElem(op) => {
                    let (iv, ix) = self.signal_table[op.idx_sig as usize].raw_bits();
                    if ix != 0 {
                        // §11.5.1: an x/z index reads an all-x element.
                        let mut v = Value::new(op.w.max(1));
                        v.is_signed = false;
                        self.ts_store_nba_val(op.dst as usize, v);
                        return 0;
                    }
                    let i = iv as i64;
                    if i < op.lo || i > op.hi {
                        return 1;
                    }
                    let eid = op.first as usize + (i - op.lo) as usize;
                    let mut v = self.signal_table[eid].clone();
                    v.is_signed = false;
                    self.ts_store_nba_val(op.dst as usize, v);
                    0
                }
                TsInsn::SaveSigW { sig } => {
                    // One entry per 64-bit word, then a marker (`TS_SAVE_WIDE`
                    // in the id, word count in `v`, `u64::MAX` in `x`) that
                    // `ts_restore_saved` meets first. Rides the narrow save list
                    // so a hazard on a wide signal adds nothing per attempt.
                    let id = *sig as usize;
                    let w = self.signal_widths[id] as usize;
                    let nw = w.div_ceil(64);
                    for i in 0..nw {
                        let n = (w - i * 64).min(64);
                        let (wv, wx) =
                            Self::raw_bits_slice(&self.signal_table[id], (i * 64) as u16, n as u16);
                        self.ts_save_list.push((*sig | TS_SAVE_WIDE, wv, wx));
                    }
                    self.ts_save_list
                        .push((*sig | TS_SAVE_WIDE, nw as u64, u64::MAX));
                    0
                }
                TsInsn::RangeFillXW { sig, hi, lo, v } => {
                    self.ts_wide_range_fill_xz(*sig as usize, *lo, *hi, *v);
                    0
                }
                _ => 0,
            }
        }
    }

    /// `sig[lo +: w]` at a run-time offset, for the two-state executors.
    #[inline(never)]
    pub(super) fn ts_sig_range_dyn(&self, sig: u32, lo: u64, w: u16, sw: u32) -> TsDyn {
        if lo + w as u64 > sw as u64 {
            return TsDyn::Oob;
        }
        let (v, x) = Self::raw_bits_slice(&self.signal_table[sig as usize], lo as u16, w);
        if x != 0 {
            return TsDyn::X;
        }
        TsDyn::Ok(v)
    }

    /// `sig[lo +: w] = v` (or `<=` with `nba`) at a run-time offset; false
    /// when the slice runs past the signal, which the four-state re-run
    /// handles.
    #[inline(never)]
    pub(super) fn ts_range_store_dyn(
        &mut self,
        sig: u32,
        lo: u64,
        v: u64,
        w: u16,
        sw: u32,
        nba: bool,
    ) -> bool {
        if lo + w as u64 > sw as u64 {
            return false;
        }
        let (id, lo, hi) = (sig as usize, lo as u32, lo as u32 + w as u32 - 1);
        match (nba, sw > 64) {
            (false, true) => self.ts_wide_range_store(id, lo, hi, v, 0),
            (false, false) => self.ts_range_store(id, v, lo, hi),
            (true, true) => self.ts_wide_range_store_nba(id, lo, hi, v),
            (true, false) => self.ts_range_store_nba(id, lo, hi, v),
        }
        true
    }

    pub(crate) fn ts_range_store_nba(&mut self, id: usize, lo: u32, hi: u32, v: u64) {
        self.queue_partial_nba(id, lo, Value::from_u64(v, hi - lo + 1));
    }

    pub(crate) fn ts_store_nba(&mut self, id: usize, v: u64, w: u32) {
        let val = Value::from_u64(v, w);
        if let Some(i) = self.nba_fast_index.get(id) {
            self.nba_fast[i].value = val;
            self.nba_fast[i].mask = None;
        } else {
            self.nba_fast_index.insert(id, self.nba_fast.len());
            self.nba_fast.push(NbaFast {
                mask: None,
                block_index: 0,
                signal_id: id,
                value: val,
            });
        }
    }

    pub(super) fn exec_two_state_ctrl(
        &mut self,
        insns: &[super::bytecode::TsInsn],
        num_regs: u32,
    ) -> bool {
        self.exec_two_state_ctrl_from(insns, num_regs, 0)
    }

    /// Control executor entered at stream index `start` (process FSMs resume
    /// mid-stream); a wait opcode records the suspend and returns true.
    /// Standalone for the same reason as `exec_two_state_line`.
    #[inline(never)]
    pub(super) fn exec_two_state_ctrl_from(
        &mut self,
        insns: &[super::bytecode::TsInsn],
        num_regs: u32,
        start: usize,
    ) -> bool {
        use super::bytecode::TsInsn;
        // Registers live in `self.ts_regs` and are used in place: no callee
        // below touches that vector, and it never reallocates during the
        // block (sized here), so an unbounded raw view is sound and saves
        // the take/restore pair on every entry and exit.
        if self.ts_regs.len() < num_regs as usize {
            self.ts_regs.resize(num_regs as usize, 0);
        }
        let regs: &mut [u64] = unsafe {
            std::slice::from_raw_parts_mut(self.ts_regs.as_mut_ptr(), self.ts_regs.len())
        };
        // Register indices are assigned by the block compiler below
        // `num_regs` and the file was just sized to it: every operand access
        // goes through a raw pointer, as in the straight-line executor.
        let rp: *mut u64 = regs.as_mut_ptr();
        // Signal (value, x) planes for a load. The plane mirror exists only
        // under XEZIM_INLINE_BITS / JIT runs; test for it once per block
        // instead of per load (a load through `self` cannot be hoisted past
        // the stores' `&mut self` calls). Signal ids come from the lowered
        // stream, validated against the table at lowering, and the table
        // never shrinks, so the index is unchecked (as in the store helpers).
        let mirror = !self.signal_inline_bits.is_empty();
        macro_rules! planes {
            ($s:expr) => {{
                let __s: usize = $s;
                if mirror {
                    match self.signal_inline_bits.get(__s) {
                        Some(sl) => (sl[0], sl[1]),
                        None => self.signal_table[__s].raw_bits(),
                    }
                } else {
                    debug_assert!(__s < self.signal_table.len());
                    self.signal_table.get_unchecked(__s).raw_bits()
                }
            }};
        }
        macro_rules! xbail {
            () => {{
                self.ts_xread_bail = true;
                #[cfg(feature = "opcode-census")]
                {
                    *self.ts_census.entry(("XBAIL", "ctrl")).or_insert(0) += 1;
                }
                return false;
            }};
        }
        // Raw-ptr walk mirrors exec_insns: the indexed form's bounds check
        // per instruction cost ~14% on a comb-dense fabric. Targets were
        // range-checked against the lowered stream at fixup time.
        let insns_ptr = insns.as_ptr();
        let insns_len = insns.len();
        let mut pc = start;
        #[cfg(feature = "opcode-census")]
        let census_on = self.census_enabled;
        #[cfg(feature = "opcode-census")]
        let mut census_prev: &'static str = "^";
        while pc < insns_len {
            #[cfg(feature = "opcode-census")]
            if census_on {
                let nm = super::bytecode::ts_insn_name(unsafe { &*insns_ptr.add(pc) });
                *self.ts_census.entry((census_prev, nm)).or_insert(0) += 1;
                census_prev = nm;
            }
            unsafe {
                match &*insns_ptr.add(pc) {
                    TsInsn::LoadSig { d, sig } => {
                        // Plane-direct: one flat load vs the Value discriminant
                        // branch (raw_bits showed at 5% of settle post-PGO).
                        // The mirror is authoritative in JIT runs and absent in
                        // non-JIT runs (fall back).
                        let (v, x) = planes!(*sig as usize);
                        if x != 0 {
                            xbail!();
                        }
                        (*rp.add(*d as usize)) = v;
                    }
                    TsInsn::Const { d, v } => (*rp.add(*d as usize)) = *v,
                    TsInsn::SigBit { d, sig, bit } => {
                        let (v, x) = planes!(*sig as usize);
                        if x != 0 {
                            xbail!();
                        }
                        (*rp.add(*d as usize)) = (v >> bit) & 1;
                    }
                    TsInsn::SigRange { d, sig, lo, mask } => {
                        let (v, x) = planes!(*sig as usize);
                        if x != 0 {
                            xbail!();
                        }
                        (*rp.add(*d as usize)) = (v >> lo) & mask;
                    }
                    TsInsn::SigBitW { d, sig, bit } => {
                        let (v, x) =
                            Self::raw_bits_slice(&self.signal_table[*sig as usize], *bit, 1);
                        if x != 0 {
                            xbail!();
                        }
                        (*rp.add(*d as usize)) = v;
                    }
                    TsInsn::SigRangeW {
                        d,
                        sig,
                        lo,
                        w,
                        mask,
                    } => {
                        let (v, x) =
                            Self::raw_bits_slice(&self.signal_table[*sig as usize], *lo, *w);
                        if x != 0 {
                            xbail!();
                        }
                        (*rp.add(*d as usize)) = v & mask;
                    }
                    TsInsn::Bit { d, s, bit } => {
                        (*rp.add(*d as usize)) = ((*rp.add(*s as usize)) >> bit) & 1;
                    }
                    TsInsn::Range { d, s, lo, mask } => {
                        (*rp.add(*d as usize)) = ((*rp.add(*s as usize)) >> lo) & mask;
                    }
                    TsInsn::Xor { d, a, b } => {
                        (*rp.add(*d as usize)) = (*rp.add(*a as usize)) ^ (*rp.add(*b as usize));
                    }
                    TsInsn::And { d, a, b } => {
                        (*rp.add(*d as usize)) = (*rp.add(*a as usize)) & (*rp.add(*b as usize));
                    }
                    TsInsn::Or { d, a, b } => {
                        (*rp.add(*d as usize)) = (*rp.add(*a as usize)) | (*rp.add(*b as usize));
                    }
                    TsInsn::Sel { d, c, a, b, .. } => {
                        (*rp.add(*d as usize)) = if (*rp.add(*c as usize)) != 0 {
                            *rp.add(*a as usize)
                        } else {
                            *rp.add(*b as usize)
                        };
                    }
                    TsInsn::Not { d, s, mask } => {
                        (*rp.add(*d as usize)) = !(*rp.add(*s as usize)) & mask;
                    }
                    TsInsn::XorC { d, s, k } => {
                        (*rp.add(*d as usize)) = (*rp.add(*s as usize)) ^ k;
                    }
                    TsInsn::MaskEq { d, s, mask, v } => {
                        (*rp.add(*d as usize)) = (((*rp.add(*s as usize)) & mask) == *v) as u64;
                    }
                    TsInsn::EqC { d, s, k } => {
                        (*rp.add(*d as usize)) = ((*rp.add(*s as usize)) == *k) as u64;
                    }
                    TsInsn::Add { d, a, b, mask } => {
                        (*rp.add(*d as usize)) =
                            (*rp.add(*a as usize)).wrapping_add(*rp.add(*b as usize)) & mask;
                    }
                    TsInsn::AddC { d, s: sr, k, mask } => {
                        (*rp.add(*d as usize)) = (*rp.add(*sr as usize)).wrapping_add(*k) & mask;
                    }
                    TsInsn::Shl { d, a, b, w, mask } => {
                        let amt = *rp.add(*b as usize);
                        (*rp.add(*d as usize)) = if amt >= *w as u64 {
                            0
                        } else {
                            ((*rp.add(*a as usize)) << amt) & mask
                        };
                    }
                    TsInsn::Shr { d, a, b, w } => {
                        let amt = *rp.add(*b as usize);
                        (*rp.add(*d as usize)) = if amt >= *w as u64 {
                            0
                        } else {
                            (*rp.add(*a as usize)) >> amt
                        };
                    }
                    TsInsn::Sub { d, a, b, mask } => {
                        (*rp.add(*d as usize)) =
                            (*rp.add(*a as usize)).wrapping_sub(*rp.add(*b as usize)) & mask;
                    }
                    TsInsn::Eq { d, a, b } => {
                        (*rp.add(*d as usize)) =
                            ((*rp.add(*a as usize)) == (*rp.add(*b as usize))) as u64;
                    }
                    TsInsn::Neq { d, a, b } => {
                        (*rp.add(*d as usize)) =
                            ((*rp.add(*a as usize)) != (*rp.add(*b as usize))) as u64;
                    }
                    TsInsn::CmpS {
                        d,
                        a,
                        b,
                        kind,
                        sa,
                        sb,
                    } => {
                        let x = (((*rp.add(*a as usize)) << *sa) as i64) >> *sa;
                        let y = (((*rp.add(*b as usize)) << *sb) as i64) >> *sb;
                        (*rp.add(*d as usize)) = match *kind {
                            0 => x < y,
                            1 => x <= y,
                            2 => x > y,
                            _ => x >= y,
                        } as u64;
                    }
                    TsInsn::BitStoreNbaDyn { sig, i, s, w } => {
                        let idx = *rp.add(*i as usize);
                        if idx < *w as u64 {
                            let bit = (*rp.add(*s as usize)) & 1;
                            self.ts_bit_store_nba(*sig as usize, idx as u32, bit);
                        }
                    }
                    TsInsn::BitDyn { d, s, i, w } => {
                        let idx = *rp.add(*i as usize);
                        if idx >= *w as u64 {
                            return false;
                        }
                        (*rp.add(*d as usize)) = ((*rp.add(*s as usize)) >> idx) & 1;
                    }
                    TsInsn::LoadSigNot { dl, d, sig } => {
                        let (v, x) = planes!(*sig as usize);
                        if x != 0 {
                            xbail!();
                        }
                        (*rp.add(*dl as usize)) = v;
                        (*rp.add(*d as usize)) = (v == 0) as u64;
                    }
                    TsInsn::SigRangeEqC {
                        dr,
                        d,
                        sig,
                        lo,
                        w,
                        k,
                    } => {
                        let mask = if *w >= 64 { u64::MAX } else { (1u64 << *w) - 1 };
                        let mask = &mask;
                        let (v, x) = planes!(*sig as usize);
                        if x != 0 {
                            xbail!();
                        }
                        let rv = (v >> lo) & mask;
                        (*rp.add(*dr as usize)) = rv;
                        (*rp.add(*d as usize)) = (rv == *k) as u64;
                    }
                    TsInsn::LoadSigLogAnd { dl, sig, d, a, b } => {
                        let (v, x) = planes!(*sig as usize);
                        if x != 0 {
                            xbail!();
                        }
                        (*rp.add(*dl as usize)) = v;
                        (*rp.add(*d as usize)) =
                            ((*rp.add(*a as usize)) != 0 && (*rp.add(*b as usize)) != 0) as u64;
                    }
                    TsInsn::LogNotAnd { dn, s, d, a, b } => {
                        (*rp.add(*dn as usize)) = ((*rp.add(*s as usize)) == 0) as u64;
                        (*rp.add(*d as usize)) = (*rp.add(*a as usize)) & (*rp.add(*b as usize));
                    }
                    TsInsn::LogNotLogAnd { dn, s, d, a, b } => {
                        (*rp.add(*dn as usize)) = ((*rp.add(*s as usize)) == 0) as u64;
                        (*rp.add(*d as usize)) =
                            ((*rp.add(*a as usize)) != 0 && (*rp.add(*b as usize)) != 0) as u64;
                    }
                    TsInsn::LogAndStore { d, a, b, sig, mask } => {
                        let rv =
                            ((*rp.add(*a as usize)) != 0 && (*rp.add(*b as usize)) != 0) as u64;
                        (*rp.add(*d as usize)) = rv;
                        self.ts_store(*sig as usize, rv & mask, *mask);
                    }
                    TsInsn::AndRangeStore {
                        d,
                        a,
                        b,
                        sig,
                        hi,
                        lo,
                    } => {
                        let mask = if *hi - *lo + 1 >= 64 {
                            u64::MAX
                        } else {
                            (1u64 << (*hi - *lo + 1)) - 1
                        };
                        let mask = &mask;
                        let rv = (*rp.add(*a as usize)) & (*rp.add(*b as usize));
                        (*rp.add(*d as usize)) = rv;
                        self.ts_range_store(*sig as usize, rv & mask, *lo, *hi);
                    }
                    TsInsn::SigBitNot { db, d, sig, bit } => {
                        let (v, x) = planes!(*sig as usize);
                        if x != 0 {
                            xbail!();
                        }
                        let bv = (v >> bit) & 1;
                        (*rp.add(*db as usize)) = bv;
                        (*rp.add(*d as usize)) = (bv == 0) as u64;
                    }
                    TsInsn::LoadSig2 { d1, sig1, d2, sig2 } => {
                        let (v, x) = planes!(*sig1 as usize);
                        if x != 0 {
                            xbail!();
                        }
                        (*rp.add(*d1 as usize)) = v;
                        let (v, x) = planes!(*sig2 as usize);
                        if x != 0 {
                            xbail!();
                        }
                        (*rp.add(*d2 as usize)) = v;
                    }
                    TsInsn::SigBit2 {
                        d1,
                        sig1,
                        bit1,
                        d2,
                        sig2,
                        bit2,
                    } => {
                        let (v, x) = planes!(*sig1 as usize);
                        if x != 0 {
                            xbail!();
                        }
                        (*rp.add(*d1 as usize)) = (v >> bit1) & 1;
                        let (v, x) = planes!(*sig2 as usize);
                        if x != 0 {
                            xbail!();
                        }
                        (*rp.add(*d2 as usize)) = (v >> bit2) & 1;
                    }
                    TsInsn::LoadSigLogOr { dl, sig, d, a, b } => {
                        let (v, x) = planes!(*sig as usize);
                        if x != 0 {
                            xbail!();
                        }
                        (*rp.add(*dl as usize)) = v;
                        (*rp.add(*d as usize)) =
                            ((*rp.add(*a as usize)) != 0 || (*rp.add(*b as usize)) != 0) as u64;
                    }
                    TsInsn::LoadSigAnd { dl, sig, d, a, b } => {
                        let (v, x) = planes!(*sig as usize);
                        if x != 0 {
                            xbail!();
                        }
                        (*rp.add(*dl as usize)) = v;
                        (*rp.add(*d as usize)) = (*rp.add(*a as usize)) & (*rp.add(*b as usize));
                    }
                    TsInsn::LoadSigRepl {
                        dl,
                        sig,
                        d,
                        w,
                        count,
                    } => {
                        let (v, x) = planes!(*sig as usize);
                        if x != 0 {
                            xbail!();
                        }
                        (*rp.add(*dl as usize)) = v;
                        (*rp.add(*d as usize)) = replicate_narrow(v, *w, *count);
                    }
                    TsInsn::LoadSigSigRange {
                        dl,
                        sig,
                        d,
                        sig2,
                        lo,
                        mask,
                    } => {
                        let (v, x) = planes!(*sig as usize);
                        if x != 0 {
                            xbail!();
                        }
                        (*rp.add(*dl as usize)) = v;
                        let (v, x) = planes!(*sig2 as usize);
                        if x != 0 {
                            xbail!();
                        }
                        (*rp.add(*d as usize)) = (v >> lo) & mask;
                    }
                    TsInsn::SigRangeAnd {
                        dr,
                        sig,
                        lo,
                        mask,
                        d,
                        a,
                        b,
                    } => {
                        let (v, x) = planes!(*sig as usize);
                        if x != 0 {
                            xbail!();
                        }
                        (*rp.add(*dr as usize)) = (v >> lo) & mask;
                        (*rp.add(*d as usize)) = (*rp.add(*a as usize)) & (*rp.add(*b as usize));
                    }
                    TsInsn::SigRangeEq {
                        dr,
                        sig,
                        lo,
                        mask,
                        d,
                        a,
                        b,
                    } => {
                        let (v, x) = planes!(*sig as usize);
                        if x != 0 {
                            xbail!();
                        }
                        (*rp.add(*dr as usize)) = (v >> lo) & mask;
                        (*rp.add(*d as usize)) =
                            ((*rp.add(*a as usize)) == (*rp.add(*b as usize))) as u64;
                    }
                    TsInsn::ConstEq { dc, v, d, a, b } => {
                        (*rp.add(*dc as usize)) = *v;
                        (*rp.add(*d as usize)) =
                            ((*rp.add(*a as usize)) == (*rp.add(*b as usize))) as u64;
                    }
                    TsInsn::LogOrStore { d, a, b, sig, mask } => {
                        let rv =
                            ((*rp.add(*a as usize)) != 0 || (*rp.add(*b as usize)) != 0) as u64;
                        (*rp.add(*d as usize)) = rv;
                        self.ts_store(*sig as usize, rv & mask, *mask);
                    }
                    TsInsn::AndOr {
                        d1,
                        a1,
                        b1,
                        d,
                        a,
                        b,
                    } => {
                        (*rp.add(*d1 as usize)) = (*rp.add(*a1 as usize)) & (*rp.add(*b1 as usize));
                        (*rp.add(*d as usize)) = (*rp.add(*a as usize)) | (*rp.add(*b as usize));
                    }
                    TsInsn::OrRangeStore {
                        d,
                        a,
                        b,
                        sig,
                        hi,
                        lo,
                    } => {
                        let mask = if *hi - *lo + 1 >= 64 {
                            u64::MAX
                        } else {
                            (1u64 << (*hi - *lo + 1)) - 1
                        };
                        let mask = &mask;
                        let rv = (*rp.add(*a as usize)) | (*rp.add(*b as usize));
                        (*rp.add(*d as usize)) = rv;
                        self.ts_range_store(*sig as usize, rv & mask, *lo, *hi);
                    }
                    TsInsn::LoadSigBrNz { dl, sig, t } => {
                        let (v, x) = planes!(*sig as usize);
                        if x != 0 {
                            xbail!();
                        }
                        (*rp.add(*dl as usize)) = v;
                        if v != 0 {
                            pc = *t as usize;
                            continue;
                        }
                    }
                    TsInsn::BrFalseLoadSig { s, t, dl, sig } => {
                        if (*rp.add(*s as usize)) == 0 {
                            pc = *t as usize;
                            continue;
                        }
                        let (v, x) = planes!(*sig as usize);
                        if x != 0 {
                            xbail!();
                        }
                        (*rp.add(*dl as usize)) = v;
                    }
                    TsInsn::EqBrFalse { d, a, b, t } => {
                        let rv = ((*rp.add(*a as usize)) == (*rp.add(*b as usize))) as u64;
                        (*rp.add(*d as usize)) = rv;
                        if rv == 0 {
                            pc = *t as usize;
                            continue;
                        }
                    }
                    TsInsn::EqCBrFalse { d, s, k, t } => {
                        let rv = ((*rp.add(*s as usize)) == *k) as u64;
                        (*rp.add(*d as usize)) = rv;
                        if rv == 0 {
                            pc = *t as usize;
                            continue;
                        }
                    }
                    TsInsn::Lt { d, a, b } => {
                        (*rp.add(*d as usize)) =
                            ((*rp.add(*a as usize)) < (*rp.add(*b as usize))) as u64;
                    }
                    TsInsn::Leq { d, a, b } => {
                        (*rp.add(*d as usize)) =
                            ((*rp.add(*a as usize)) <= (*rp.add(*b as usize))) as u64;
                    }
                    TsInsn::Gt { d, a, b } => {
                        (*rp.add(*d as usize)) =
                            ((*rp.add(*a as usize)) > (*rp.add(*b as usize))) as u64;
                    }
                    TsInsn::Geq { d, a, b } => {
                        (*rp.add(*d as usize)) =
                            ((*rp.add(*a as usize)) >= (*rp.add(*b as usize))) as u64;
                    }
                    TsInsn::LogNot { d, s } => {
                        (*rp.add(*d as usize)) = ((*rp.add(*s as usize)) == 0) as u64;
                    }
                    TsInsn::RedOr { d, s } => {
                        (*rp.add(*d as usize)) = ((*rp.add(*s as usize)) != 0) as u64;
                    }
                    TsInsn::RedAnd { d, s, mask } => {
                        (*rp.add(*d as usize)) = ((*rp.add(*s as usize)) == *mask) as u64;
                    }
                    TsInsn::CovHit(c) => self.ts_cov_hit(*c),
                    TsInsn::WaitEdge { ix, resume } => {
                        self.fsm_suspend = Some((*resume, FsmWait::Edge(*ix)));
                        return true;
                    }
                    TsInsn::WaitDelayRaw { raw, resume } => {
                        let v = Value::from_f64(f64::from_bits(*raw));
                        let t = self.fsm_delay_ticks(&v, self.fsm_precision_diff);
                        self.fsm_suspend = Some((*resume, FsmWait::Delay(t)));
                        return true;
                    }
                    TsInsn::Fallback(st) => {
                        let st = st.clone();
                        self.prof_fallback_insns += 1;
                        let saved = self.ts_fallback_enter();
                        self.exec_statement(&st);
                        self.ts_fallback_exit(saved);
                        if self.finished {
                            return true;
                        }
                    }
                    TsInsn::SaveSig { sig } => {
                        let (v, x) = self.signal_table[*sig as usize].raw_bits();
                        self.ts_save_list.push((*sig, v, x));
                    }
                    TsInsn::WRedOr { .. } | TsInsn::WRedAnd { .. } => {
                        unreachable!("wide insn outside the wide executor")
                    }
                    TsInsn::BitStoreDyn { sig, i, s, w } => {
                        // §11.5.1: an out-of-range dynamic bit-select target
                        // drops the write (`Value::set_bit` is a no-op there).
                        let idx = *rp.add(*i as usize);
                        if idx < *w as u64 {
                            let b = (*rp.add(*s as usize)) & 1;
                            if *w > 64 {
                                self.ts_wide_bit_store(*sig as usize, idx as usize, b);
                            } else {
                                self.ts_range_store(*sig as usize, b, idx as u32, idx as u32);
                            }
                        }
                    }
                    // Dynamic-offset slice and range stores: one arm, one call,
                    // so the executor loop's register allocation stays as it was
                    // (three inline arms cost a c906 SoC 0.8%, outlined bodies
                    // still 0.5%).
                    insn @ (TsInsn::SigRangeDyn { .. }
                    | TsInsn::RangeStoreDyn { .. }
                    | TsInsn::RangeStoreNbaDyn { .. }
                    | TsInsn::SaveSigW { .. }
                    | TsInsn::WNbaFromElem(..)
                    | TsInsn::RangeFillXW { .. }
                    | TsInsn::LoopPoll) => match self.ts_exec_dyn(insn, rp) {
                        0 => {}
                        1 => return false,
                        _ => xbail!(),
                    },
                    TsInsn::ConstStoreX { sig, v, x } => {
                        self.ts_store_xz(*sig as usize, *v, *x);
                    }
                    TsInsn::RangeFillW { sig, hi, lo, bit } => {
                        self.ts_wide_range_fill(*sig as usize, *lo, *hi, *bit);
                    }
                    TsInsn::RangeFillNbaW { sig, hi, lo, bit } => {
                        self.ts_wide_range_fill_nba(*sig as usize, *lo, *hi, *bit);
                    }
                    TsInsn::RangeStoreNbaW {
                        sig,
                        hi,
                        lo,
                        s,
                        mask,
                    } => {
                        let v = (*rp.add(*s as usize)) & mask;
                        self.ts_wide_range_store_nba(*sig as usize, *lo, *hi, v);
                    }
                    TsInsn::RangeStoreW {
                        sig,
                        hi,
                        lo,
                        s,
                        mask,
                    } => {
                        let v = (*rp.add(*s as usize)) & mask;
                        self.ts_wide_range_store(*sig as usize, *lo, *hi, v, 0);
                    }
                    TsInsn::RangeStoreXW(p) => {
                        let (sig, hi, lo, v, x) = (&p.sig, &p.hi, &p.lo, &p.v, &p.x);
                        self.ts_wide_range_store(*sig as usize, *lo, *hi, *v, *x);
                    }
                    TsInsn::RangeStoreX(p) => {
                        let (sig, hi, lo, v, x) = (&p.sig, &p.hi, &p.lo, &p.v, &p.x);
                        self.ts_range_store_xz(*sig as usize, *v, *x, *lo, *hi);
                    }
                    TsInsn::LogAnd { d, a, b } => {
                        (*rp.add(*d as usize)) =
                            ((*rp.add(*a as usize)) != 0 && (*rp.add(*b as usize)) != 0) as u64;
                    }
                    TsInsn::LogOr { d, a, b } => {
                        (*rp.add(*d as usize)) =
                            ((*rp.add(*a as usize)) != 0 || (*rp.add(*b as usize)) != 0) as u64;
                    }
                    TsInsn::Concat { d, parts } => {
                        let mut acc = parts.first().map_or(0, |&(r, _)| *rp.add(r as usize));
                        for &(r, w) in parts.iter().skip(1) {
                            acc = (acc << w) | (*rp.add(r as usize));
                        }
                        (*rp.add(*d as usize)) = acc;
                    }
                    TsInsn::Concat2 { d, a, wa: _, b, wb } => {
                        (*rp.add(*d as usize)) =
                            ((*rp.add(*a as usize)) << *wb) | (*rp.add(*b as usize));
                    }
                    TsInsn::Concat3 {
                        d,
                        a,
                        wa: _,
                        b,
                        wb,
                        c,
                        wc,
                    } => {
                        let acc = ((*rp.add(*a as usize)) << *wb) | (*rp.add(*b as usize));
                        (*rp.add(*d as usize)) = (acc << *wc) | (*rp.add(*c as usize));
                    }
                    TsInsn::Mask { d, mask } => {
                        (*rp.add(*d as usize)) &= mask;
                    }
                    TsInsn::Mul { d, a, b, mask } => {
                        (*rp.add(*d as usize)) =
                            (*rp.add(*a as usize)).wrapping_mul(*rp.add(*b as usize)) & mask;
                    }
                    TsInsn::Repl { d, s, w, count } => {
                        let part = *rp.add(*s as usize);
                        (*rp.add(*d as usize)) = replicate_narrow(part, *w, *count);
                    }
                    TsInsn::WSigRange { .. } => {
                        unreachable!("wide insn outside the wide executor")
                    }
                    TsInsn::WRepl { .. } => {
                        unreachable!("wide insn outside the wide executor")
                    }
                    TsInsn::BrSigFalse { sig, bit, t } => {
                        let cond = if *bit == u32::MAX {
                            // Whole-value form was lowered only for <=64-bit sigs.
                            let (v, x) = self.signal_table[*sig as usize].raw_bits();
                            if x != 0 {
                                xbail!();
                            }
                            v != 0
                        } else {
                            let (v, x) = Self::raw_bits_slice(
                                &self.signal_table[*sig as usize],
                                *bit as u16,
                                1,
                            );
                            if x != 0 {
                                xbail!();
                            }
                            v != 0
                        };
                        if !cond {
                            pc = *t as usize;
                            continue;
                        }
                    }
                    TsInsn::BrFalse { s, t } => {
                        if (*rp.add(*s as usize)) == 0 {
                            pc = *t as usize;
                            continue;
                        }
                    }
                    TsInsn::BrNz { s, t } => {
                        if (*rp.add(*s as usize)) != 0 {
                            pc = *t as usize;
                            continue;
                        }
                    }
                    TsInsn::Jmp { t } => {
                        pc = *t as usize;
                        continue;
                    }
                    // Back-edge: bail on Ctrl-C (see `TsInsn::JmpPoll`).
                    TsInsn::JmpPoll { t } => {
                        if loop_stop_requested() {
                            return false;
                        }
                        pc = *t as usize;
                        continue;
                    }
                    TsInsn::CaseJmp { s, cj } => {
                        // Two-state selector is X-free, so the 4-state x/z ->
                        // default branch is unreachable; out-of-table indices and
                        // holes still take `default`.
                        pc = cj
                            .table
                            .get((*rp.add(*s as usize)) as usize)
                            .copied()
                            .unwrap_or(cj.default) as usize;
                        continue;
                    }
                    TsInsn::CaseMaskJmp { s, mj: cm } => {
                        let (mask, lo, wmask, mj) = (&cm.mask, &cm.lo, &cm.wmask, &cm.mj);
                        // The window is fully defined on an X-free selector, so
                        // `xz_path` (the wildcard compare chain) cannot be taken.
                        let v = (*rp.add(*s as usize)) & *mask;
                        pc = mj.table[((v >> *lo) & *wmask) as usize] as usize;
                        continue;
                    }
                    TsInsn::ElemLoad(op) => {
                        let i = (*rp.add(op.idx as usize)) as i64;
                        if i < op.lo || i > op.hi {
                            return false;
                        }
                        let eid = op.first as usize + (i - op.lo) as usize;
                        let (v, x) = self.signal_table[eid].raw_bits();
                        if x != 0 {
                            return false;
                        }
                        (*rp.add(op.s as usize)) = v;
                    }
                    TsInsn::ElemStoreNbaFromSig(f) => {
                        // Data straight from the source signal's planes; x/z bits
                        // are queued as a 4-state value (see the variant's doc).
                        let op = &f.op;
                        let (sv, sx) = self.signal_table[f.sig as usize].raw_bits();
                        (*rp.add(f.dl as usize)) = sv;
                        let i = (*rp.add(op.idx as usize)) as i64;
                        if i >= op.lo && i <= op.hi {
                            let eid = op.first as usize + (i - op.lo) as usize;
                            if sx & op.mask != 0 {
                                let mut v =
                                    Value::from_inline(sv & op.mask, sx & op.mask, op.w.max(1));
                                v.is_signed = false;
                                self.ts_store_nba_val(eid, v);
                            } else {
                                self.ts_store_nba(eid, sv & op.mask, op.w);
                            }
                        }
                    }
                    TsInsn::ElemStoreNba(op) => {
                        let i = (*rp.add(op.idx as usize)) as i64;
                        if i >= op.lo && i <= op.hi {
                            let eid = op.first as usize + (i - op.lo) as usize;
                            let v = (*rp.add(op.s as usize)) & op.mask;
                            self.ts_store_nba(eid, v, op.w);
                        }
                        // Out-of-range: silent drop, as in 4-state.
                    }
                    TsInsn::ElemStore(op) => {
                        let i = (*rp.add(op.idx as usize)) as i64;
                        if i >= op.lo && i <= op.hi {
                            let eid = op.first as usize + (i - op.lo) as usize;
                            let v = (*rp.add(op.s as usize)) & op.mask;
                            self.ts_store(eid, v, op.mask);
                        }
                    }
                    TsInsn::NbaFromElem(op) => {
                        let (iv, ix) = self.signal_table[op.idx_sig as usize].raw_bits();
                        if ix != 0 {
                            // §11.5.1: an x/z index reads an all-x element (see
                            // the straight-line executor's arm).
                            let mut v = Value::new(op.w.max(1));
                            v.is_signed = false;
                            self.ts_store_nba_val(op.dst as usize, v);
                            pc += 1;
                            continue;
                        }
                        let i = iv as i64;
                        if i < op.lo || i > op.hi {
                            return false;
                        }
                        let eid = op.first as usize + (i - op.lo) as usize;
                        let (ev, ex) = self.signal_table[eid].raw_bits();
                        if ex != 0 {
                            return false;
                        }
                        let m = if op.w >= 64 {
                            u64::MAX
                        } else {
                            (1u64 << op.w) - 1
                        };
                        self.ts_store_nba(op.dst as usize, ev & m, op.w);
                    }
                    TsInsn::Store { sig, s, mask } => {
                        let v = (*rp.add(*s as usize)) & mask;
                        self.ts_store(*sig as usize, v, *mask);
                    }
                    TsInsn::RangeStore {
                        sig,
                        hi,
                        lo,
                        s,
                        mask,
                    } => {
                        // §10.4.1 blocking splice: replace only [hi:lo] and keep
                        // the surrounding bits, including any X they already
                        // carry (the eval-site prefilter proves the block's READS
                        // are X-free, not the untouched remainder of a partially
                        // written destination). `compose_inline_range_bits` is
                        // the same merge the NBA arm below uses.
                        self.ts_range_store(*sig as usize, (*rp.add(*s as usize)) & mask, *lo, *hi);
                    }
                    TsInsn::StoreNba { sig, s, w, mask } => {
                        let v = (*rp.add(*s as usize)) & mask;
                        self.ts_store_nba(*sig as usize, v, *w);
                    }
                    TsInsn::ConstStoreNba { sig, v, w } => {
                        self.ts_store_nba(*sig as usize, *v, *w);
                    }
                    TsInsn::RangeStoreNba {
                        sig,
                        hi,
                        lo,
                        s,
                        mask,
                    } => {
                        self.ts_range_store_nba(
                            *sig as usize,
                            *lo,
                            *hi,
                            (*rp.add(*s as usize)) & mask,
                        );
                    }
                    TsInsn::WLoadSig { .. }
                    | TsInsn::WRangeStore { .. }
                    | TsInsn::WRangeStoreNba { .. }
                    | TsInsn::WConst { .. }
                    | TsInsn::WXor { .. }
                    | TsInsn::WAnd { .. }
                    | TsInsn::WOr { .. }
                    | TsInsn::WSel { .. }
                    | TsInsn::WElemLoad(..)
                    | TsInsn::WNot { .. }
                    | TsInsn::WRange { .. }
                    | TsInsn::RangeFromW { .. }
                    | TsInsn::BitFromW { .. }
                    | TsInsn::WConcat { .. }
                    | TsInsn::WMask { .. }
                    | TsInsn::WFromN { .. }
                    | TsInsn::NFromW { .. }
                    | TsInsn::WStore { .. }
                    | TsInsn::WStoreNba { .. } => {
                        unreachable!("wide insn outside the wide executor")
                    }
                }
            }
            pc += 1;
        }
        true
    }

    pub(super) fn collapse_identity_port_nets(&mut self) {
        use std::borrow::Cow;
        // The flat name `resolve_hier_name_static` gives, borrowed for the
        // overwhelmingly common single segment (no allocation per assign).
        fn flat_name<'a>(h: &'a HierarchicalIdentifier, module: &ElaboratedModule) -> Cow<'a, str> {
            let skip_root = usize::from(h.path.len() > 1 && h.path[0].name.name == "$root");
            if h.path.len() == skip_root + 1 {
                Cow::Borrowed(h.path[skip_root].name.name.as_str())
            } else {
                Cow::Owned(Simulator::resolve_hier_name_static(h, module))
            }
        }
        let buf_collapse_on = !matches!(std::env::var("XEZIM_BUF_COLLAPSE").as_deref(), Ok("0"))
            && self.sdf_delays.is_empty()
            && self.module.dpi_imports.is_empty()
            && configured_vpi_libs().is_empty()
            && configured_dpi_libs().is_empty();
        // The pass runs again at compile time. When the first run settled and
        // nothing it reads has changed since (no name added or re-pointed, the
        // same assign list and port aliases; widths and the two-state and
        // gate-driven tables are fixed at construction), the second run would
        // re-derive the same links and re-point nothing.
        if let Some((generation, ca_len, ca_ptr, alias_len, buf_ran)) = self.collapse_settled {
            if generation == self.signal_name_to_id.generation()
                && ca_len == self.module.continuous_assigns.len()
                && ca_ptr == self.module.continuous_assigns.as_ptr() as usize
                && alias_len == self.module.port_aliases.len()
                && (buf_ran || !buf_collapse_on)
                && std::env::var_os("XEZIM_BUF_CENSUS").is_none()
            {
                return;
            }
        }
        let arrays_settled = self.collapse_array_port_nets();
        let mut collapsed = 0usize;
        // A net the connect CA drives may have OTHER continuous drivers (a
        // hierarchical TB assign, a shorted second output). Dropping the
        // connect then loses multi-driver resolution, so only nets with
        // exactly one continuous driver — the connect itself — collapse.
        let mut ca_driver_counts: HashMap<Cow<'_, str>, u32> = HashMap::default();
        for ca in &self.module.continuous_assigns {
            let base = match &ca.lhs.kind {
                ExprKind::Ident(h) => Some(h),
                ExprKind::Index { expr, .. } => match &expr.kind {
                    ExprKind::Ident(h) => Some(h),
                    _ => None,
                },
                _ => None,
            };
            let Some(h) = base else { continue };
            *ca_driver_counts
                .entry(flat_name(h, &self.module))
                .or_insert(0) += 1;
        }
        // INPUT identity connects only (`u1.din = src`, rhs in parent
        // scope): the child name re-points at the parent's id. Output
        // connects (`snk = u1.dout`) must KEEP their propagation step:
        // a waiter parked on the same edge reads the parent net before
        // a blocking write in the child reaches it (reference-verified
        // in scheduling::waiter_edge_ordering), so collapsing them is
        // observably wrong, not just unprofitable. These shape tests and the
        // resolved rhs name do not change between passes, so they are taken
        // once, in assign order.
        let port_candidates: Vec<(&str, Cow<'_, str>)> = self
            .module
            .continuous_assigns
            .iter()
            .filter_map(|ca| {
                if ca.delay != 0 {
                    return None;
                }
                let (ExprKind::Ident(lh), ExprKind::Ident(rh)) = (&ca.lhs.kind, &ca.rhs.kind)
                else {
                    return None;
                };
                if lh.path.len() != 1
                    || !lh.path[0].selects.is_empty()
                    || !rh.path.iter().all(|s| s.selects.is_empty())
                    || !ca.rhs_parent_scoped
                {
                    return None;
                }
                Some((lh.path[0].name.name.as_str(), flat_name(rh, &self.module)))
            })
            .collect();
        // The passes below re-point names in `signal_name_to_id` and read
        // back what earlier links wrote. They run on a local mirror of just
        // the names involved (one hash probe per name instead of several per
        // link per pass); the re-pointed names are written back in the order
        // they first changed, which leaves both tables exactly as re-pointing
        // them in place would.
        // A key type of its own, so these compile-time maps share no
        // monomorphized code with run-time ones.
        #[derive(Clone, Copy, PartialEq, Eq, Hash)]
        struct NameKey<'a>(&'a str);
        struct Mirror<'a> {
            slot: HashMap<NameKey<'a>, usize>,
            names: Vec<&'a str>,
            ids: Vec<Option<usize>>,
            changed: Vec<bool>,
            order: Vec<usize>,
        }
        impl<'a> Mirror<'a> {
            fn new() -> Self {
                Mirror {
                    slot: HashMap::default(),
                    names: Vec::new(),
                    ids: Vec::new(),
                    changed: Vec::new(),
                    order: Vec::new(),
                }
            }
            fn slot(&mut self, name: &'a str, map: &NameMap) -> usize {
                if let Some(&s) = self.slot.get(&NameKey(name)) {
                    return s;
                }
                let s = self.names.len();
                self.slot.insert(NameKey(name), s);
                self.names.push(name);
                self.ids.push(map.get(name).copied());
                self.changed.push(false);
                s
            }
            fn repoint(&mut self, s: usize, id: usize) {
                self.ids[s] = Some(id);
                if !self.changed[s] {
                    self.changed[s] = true;
                    self.order.push(s);
                }
            }
        }
        let mut mirror = Mirror::new();
        // Only connections the elaborator recorded as WHOLE-NET identity
        // (the same predicate the dump-side fold trusts), driven by the
        // connect alone (the net it drives is its CA lhs), between non-event
        // names.
        let mut port_links: Vec<(usize, usize)> = Vec::new();
        for (lhs_name, rhs_name) in &port_candidates {
            let (child, parent): (&str, &str) = (lhs_name, rhs_name);
            if self.module.port_aliases.get(child).map(String::as_str) != Some(parent)
                || ca_driver_counts.get(child).copied().unwrap_or(0) != 1
                || self.module.events.contains(child)
                || self.module.events.contains(parent)
            {
                continue;
            }
            let c = mirror.slot(child, &self.signal_name_to_id);
            let p = mirror.slot(parent, &self.signal_name_to_id);
            port_links.push((c, p));
        }
        // Chains (leaf -> mid -> top) resolve over multiple passes; they are
        // short, and each pass only ever re-points names, so a small bound
        // is plenty.
        let mut port_settled = false;
        for _ in 0..8 {
            let mut changed = false;
            for &(c, p) in &port_links {
                let (Some(cid), Some(pid)) = (mirror.ids[c], mirror.ids[p]) else {
                    continue;
                };
                if cid == pid
                    || self.signal_widths[cid] != self.signal_widths[pid]
                    || self.signal_real[cid] != self.signal_real[pid]
                {
                    continue;
                }
                mirror.repoint(c, pid);
                collapsed += 1;
                changed = true;
            }
            if !changed {
                port_settled = true;
                break;
            }
        }
        for &s in &mirror.order {
            let name = mirror.names[s];
            self.signal_name_to_id
                .insert(name.into(), mirror.ids[s].unwrap());
            // The lhs is the name the dropped connect was driving; it
            // stays wire-typed in dumps by NAME (the shared id can't say).
            self.collapsed_port_children.insert(name.into());
        }
        if collapsed > 0 && std::env::var("XEZIM_DEBUG").is_ok() {
            eprintln!("[NET-COLLAPSE] {} identity port nets collapsed", collapsed);
        }
        // Collapse GENERAL whole-net identity continuous assigns
        // (`assign y = x;` — the std-cell BUF fabric on gate-level SoCs;
        // 5,939 of 25,850 CAs on the C906). Same mechanism as the port
        // collapse above: re-point the lhs NAME at the rhs id before
        // anything bakes ids, and the connect drops as a self-copy in
        // build_comb_entries. Beyond deleting the entry evals, clock-buffer
        // chains merge onto their root, shrinking edge sensitivity fan-out.
        // It removes the one-delta propagation step the buffer otherwise
        // adds (the same transform the reference's optimizer applies), which
        // is why it was opt-in at first; the cases that makes observable are
        // now excluded per net by the conditions below instead, so the pass
        // runs by default. Skipped entirely under SDF (a collapsed net would
        // lose its annotated delay).
        // Whole-net identity buffers (`assign y = x`) alias onto their source
        // by default; `XEZIM_BUF_COLLAPSE=0` turns the pass off. A net keeps
        // its own storage when aliasing would be observable: it is a force /
        // release / procedural-assign target anywhere in the design, its
        // two-state-ness differs from the source (a 2-state copy must drop
        // X/Z), the design carries SDF delays, or a DPI/VPI backdoor may look
        // the folded name up by itself.
        let port_mirror = mirror;
        // Settled when every pass that ran reached its fixed point and the
        // buffer pass re-pointed no name a port link reads (which would give
        // the port pass new work).
        let mut settled = port_settled && arrays_settled;
        if buf_collapse_on {
            // Same rule as the port pass above: never alias onto a source a
            // process WRITES. A reader parked on the same edge that writes
            // the source relies on the copy's delta step (reference-verified
            // in tests/scheduling/waiter_edge_ordering.rs); gate primitives
            // keep their z→x mapping on the net they drive. This pass runs
            // again after the always blocks have been drained into compiled
            // form, so the two name sets are collected on the FIRST call,
            // while every process body is still on the module, and kept.
            if self.buf_collapse_write_sets.is_none() {
                let (sets, lazy_writes) = self.write_target_census();
                let mut sets = sets;
                sets.1.extend(lazy_writes);
                self.buf_collapse_write_sets = Some(sets);
            }
            let (override_leaves, proc_writes) = self
                .buf_collapse_write_sets
                .as_ref()
                .expect("collected above");
            fn leaf_of(n: &str) -> &str {
                n.rsplit('.').next().unwrap_or(n)
            }
            let mut buf_collapsed = 0usize;
            // Whole-net identity assigns and their resolved names: fixed
            // across passes, taken once in assign order.
            let buf_candidates: Vec<(Cow<'_, str>, Cow<'_, str>)> = self
                .module
                .continuous_assigns
                .iter()
                .filter_map(|ca| {
                    if ca.delay != 0 {
                        return None;
                    }
                    let (ExprKind::Ident(lh), ExprKind::Ident(rh)) = (&ca.lhs.kind, &ca.rhs.kind)
                    else {
                        return None;
                    };
                    if !lh.path.iter().all(|s| s.selects.is_empty())
                        || !rh.path.iter().all(|s| s.selects.is_empty())
                    {
                        return None;
                    }
                    Some((flat_name(lh, &self.module), flat_name(rh, &self.module)))
                })
                .collect();
            // The name-only conditions do not change between passes.
            let mut mirror = Mirror::new();
            let mut buf_links: Vec<(usize, usize)> = Vec::new();
            for (ln, rn) in &buf_candidates {
                let (ln, rn): (&str, &str) = (ln, rn);
                if self.module.events.contains(ln)
                    || self.module.events.contains(rn)
                    || ca_driver_counts.get(ln).copied().unwrap_or(0) != 1
                    || override_leaves.contains(leaf_of(ln))
                    || override_leaves.contains(leaf_of(rn))
                    || proc_writes.contains(rn)
                {
                    continue;
                }
                let l = mirror.slot(ln, &self.signal_name_to_id);
                let r = mirror.slot(rn, &self.signal_name_to_id);
                buf_links.push((l, r));
            }
            let mut buf_settled = false;
            for _ in 0..8 {
                let mut changed = false;
                for &(l, r) in &buf_links {
                    let (Some(lid), Some(rid)) = (mirror.ids[l], mirror.ids[r]) else {
                        continue;
                    };
                    if lid == rid
                        || self.signal_widths[lid] != self.signal_widths[rid]
                        || self.signal_real[lid] != self.signal_real[rid]
                        || self.signal_two_state[lid] != self.signal_two_state[rid]
                        || self.signal_gate_driven.get(lid).copied().unwrap_or(false)
                    {
                        continue;
                    }
                    mirror.repoint(l, rid);
                    buf_collapsed += 1;
                    changed = true;
                }
                if !changed {
                    buf_settled = true;
                    break;
                }
            }
            settled &= buf_settled;
            for &s in &mirror.order {
                let name = mirror.names[s];
                settled &= !port_mirror.slot.contains_key(&NameKey(name));
                self.signal_name_to_id
                    .insert(name.into(), mirror.ids[s].unwrap());
                self.collapsed_port_children.insert(name.to_string());
            }
            if buf_collapsed > 0 && std::env::var("XEZIM_DEBUG").is_ok() {
                eprintln!(
                    "[BUF-COLLAPSE] {} identity buffer nets collapsed",
                    buf_collapsed
                );
            }
        }
        self.collapse_settled = settled.then(|| {
            (
                self.signal_name_to_id.generation(),
                self.module.continuous_assigns.len(),
                self.module.continuous_assigns.as_ptr() as usize,
                self.module.port_aliases.len(),
                buf_collapse_on,
            )
        });
        // Census for the buffer-net collapse experiment: how many WHOLE-NET
        // identity continuous assigns (`assign y = x;`, equal widths, single
        // driver) remain after the port collapse above — the std-cell BUF
        // fabric class on gate-level SoCs.
        if std::env::var("XEZIM_BUF_CENSUS").is_ok() {
            let mut total_ca = 0usize;
            let mut identity = 0usize;
            let mut identity_single = 0usize;
            for ca in &self.module.continuous_assigns {
                total_ca += 1;
                if ca.delay != 0 {
                    continue;
                }
                let (ExprKind::Ident(lh), ExprKind::Ident(rh)) = (&ca.lhs.kind, &ca.rhs.kind)
                else {
                    continue;
                };
                if !lh.path.iter().all(|s| s.selects.is_empty())
                    || !rh.path.iter().all(|s| s.selects.is_empty())
                {
                    continue;
                }
                let ln = Self::resolve_hier_name_static(lh, &self.module);
                let rn = Self::resolve_hier_name_static(rh, &self.module);
                let (Some(&lid), Some(&rid)) = (
                    self.signal_name_to_id.get(ln.as_str()),
                    self.signal_name_to_id.get(rn.as_str()),
                ) else {
                    continue;
                };
                if lid == rid || self.signal_widths[lid] != self.signal_widths[rid] {
                    continue;
                }
                identity += 1;
                if ca_driver_counts.get(ln.as_str()).copied().unwrap_or(0) == 1 {
                    identity_single += 1;
                }
            }
            eprintln!(
                "[BUF-CENSUS] cont_assigns={} identity={} identity_single_driver={}",
                total_ca, identity, identity_single
            );
        }
    }

    /// §23.3.3.7: an unpacked-array port connected to a whole array is one
    /// object with it, as `collapse_identity_port_nets` makes a scalar port and
    /// its actual one net. The elaborator connects such a port element by
    /// element (`u.a[i] = a[i]`), so the port kept elements of its own: a value
    /// a VPI client wrote to `u.a[i]` reached the instance but not `a[i]`, nor
    /// the port of an outer instance that `a` passes through, and one written
    /// to `a[i]` reached `u.a[i]` only through the connect assign. Here every
    /// element name of the port, and its `array_first_id` entry, is pointed at
    /// the actual's cells, and the connects become self-copies that
    /// `build_comb_entries` drops. A port collapses whole or not at all: every
    /// element driven by its connect alone, onto cells of the same width,
    /// realness and two-state-ness laid out in the same order. A chain through
    /// several port levels resolves over a few passes. Returns whether the
    /// passes reached a fixed point.
    pub(super) fn collapse_array_port_nets(&mut self) -> bool {
        // The array an assignment target selects from.
        fn base_of<'a>(
            mut e: &'a Expression,
            module: &ElaboratedModule,
        ) -> Option<std::borrow::Cow<'a, str>> {
            while let ExprKind::Index { expr, .. } | ExprKind::RangeSelect { expr, .. } = &e.kind {
                e = expr;
            }
            match &e.kind {
                ExprKind::Ident(h) => Some(Simulator::flat_ident_name(h, module)),
                _ => None,
            }
        }
        // Element connects (port base, port indices, actual base, actual
        // indices), grouped by port array.
        let mut conns: Vec<(String, Vec<i64>, String, Vec<i64>)> = self
            .module
            .continuous_assigns
            .iter()
            .filter(|ca| {
                ca.rhs_parent_scoped
                    && ca.delay == 0
                    && ca.delay_fall.is_none()
                    && ca.delay_off.is_none()
                    && matches!(ca.lhs.kind, ExprKind::Index { .. })
            })
            .filter_map(|ca| {
                let (port, pi) = Self::const_elem_ref(&ca.lhs, &self.module)?;
                let (actual, ai) = Self::const_elem_ref(&ca.rhs, &self.module)?;
                // §23.3.3.7 merges nets. A variable formal or actual keeps
                // its implicit continuous assignment (§23.3.3.3): aliasing
                // it would expose a blocking write before propagation.
                if !self.module.nets.contains(&port) || !self.module.nets.contains(&actual) {
                    return None;
                }
                Some((port, pi, actual, ai))
            })
            .collect();
        if conns.is_empty() {
            return true;
        }
        conns.sort_by(|a, b| a.0.cmp(&b.0));
        let ports: Vec<(&str, &[(String, Vec<i64>, String, Vec<i64>)])> = conns
            .chunk_by(|a, b| a.0 == b.0)
            .map(|g| (g[0].0.as_str(), g))
            .collect();
        // Continuous drivers of each port array, its connects included.
        let mut drivers = vec![0usize; ports.len()];
        for ca in &self.module.continuous_assigns {
            if let Some(b) = base_of(&ca.lhs, &self.module) {
                if let Ok(k) = ports.binary_search_by(|(p, _)| (*p).cmp(b.as_ref())) {
                    drivers[k] += 1;
                }
            }
        }
        let len = |(l, r): (i64, i64)| (r - l).unsigned_abs() as usize + 1;
        let m = &self.module;
        let ports: Vec<_> = ports
            .into_iter()
            .zip(drivers)
            .filter(|((port, elems), drivers)| {
                let count = if let Some(&(lo, hi, _)) = m.arrays.get(*port) {
                    len((lo, hi))
                } else if let Some(&(a, b, _)) = m.arrays_2d.get(*port) {
                    len(a) * len(b)
                } else if let Some((shape, _)) = m.arrays_nd.get(*port) {
                    shape.iter().map(|&d| len(d)).product()
                } else {
                    return false;
                };
                elems.len() == count
                    && *drivers == count
                    && !m.events.contains(*port)
                    && !elems
                        .iter()
                        .any(|(_, _, a, _)| m.events.contains(a.as_str()))
            })
            .map(|(p, _)| p)
            .collect();
        let mut settled = false;
        let mut collapsed = vec![false; ports.len()];
        for _ in 0..8 {
            let mut changed = false;
            for (k, &(port, elems)) in ports.iter().enumerate() {
                // The actual's cells sit at one fixed offset from the port's
                // own, which are consecutive.
                let mut delta = None;
                let mut cells = Vec::with_capacity(elems.len());
                for (_, pi, actual, ai) in elems {
                    let (Some(c), Some(a)) = (self.elem_cell(port, pi), self.elem_cell(actual, ai))
                    else {
                        break;
                    };
                    let d = a.wrapping_sub(c);
                    if *delta.get_or_insert(d) != d
                        || self.cell_storage(c).is_none()
                        || self.cell_storage(c) != self.cell_storage(a)
                    {
                        break;
                    }
                    cells.push(c);
                }
                let Some(delta) = delta.filter(|&d| d != 0 && cells.len() == elems.len()) else {
                    continue;
                };
                cells.sort_unstable();
                cells.dedup();
                if cells.len() != elems.len()
                    || cells[cells.len() - 1] - cells[0] + 1 != cells.len()
                {
                    continue;
                }
                for (_, pi, _, _) in elems {
                    let name = Self::elem_name(port, pi);
                    if let Some(&c) = self.signal_name_to_id.get(name.as_str()) {
                        self.signal_name_to_id
                            .insert(name.into(), c.wrapping_add(delta));
                    }
                }
                if let Some(e) = self.array_first_id.get_mut(port) {
                    e.0 = e.0.wrapping_add(delta);
                }
                // Typed as a wire in a dump, as a collapsed scalar port is.
                self.collapsed_port_children.insert(port.to_string());
                collapsed[k] = true;
                changed = true;
            }
            if !changed {
                settled = true;
                break;
            }
        }
        let n = collapsed.iter().filter(|&&c| c).count();
        if n > 0 && std::env::var_os("XEZIM_DEBUG").is_some() {
            eprintln!("[ARRAY-PORT-COLLAPSE] {} unpacked-array ports collapsed", n);
        }
        settled
    }

    /// The flat base and constant indices of an element connect operand
    /// `base[i]..[k]`, as the elaborator spells an element of an
    /// unpacked-array port and of its actual.
    pub(super) fn const_elem_ref(
        e: &Expression,
        module: &ElaboratedModule,
    ) -> Option<(String, Vec<i64>)> {
        let mut idx = Vec::new();
        let mut cur = e;
        while let ExprKind::Index { expr, index } = &cur.kind {
            idx.push(super::elaborate::const_eval_i64_with_params(index, None)?);
            cur = expr;
        }
        let ExprKind::Ident(h) = &cur.kind else {
            return None;
        };
        if idx.is_empty() || !h.path.iter().all(|s| s.selects.is_empty()) {
            return None;
        }
        idx.reverse();
        Some((Self::flat_ident_name(h, module).into_owned(), idx))
    }

    /// The flat name `resolve_hier_name_static` gives, borrowed for a single
    /// segment.
    pub(super) fn flat_ident_name<'a>(
        h: &'a HierarchicalIdentifier,
        module: &ElaboratedModule,
    ) -> std::borrow::Cow<'a, str> {
        let skip_root = usize::from(h.path.len() > 1 && h.path[0].name.name == "$root");
        if h.path.len() == skip_root + 1 {
            std::borrow::Cow::Borrowed(h.path[skip_root].name.name.as_str())
        } else {
            std::borrow::Cow::Owned(Self::resolve_hier_name_static(h, module))
        }
    }

    /// `base[i]..[k]`, written the way array element names are registered.
    pub(super) fn elem_name(base: &str, idx: &[i64]) -> String {
        let mut name = String::with_capacity(base.len() + 8 * idx.len());
        name.push_str(base);
        for &i in idx {
            name.push('[');
            Self::push_i64(&mut name, i);
            name.push(']');
        }
        name
    }

    /// The cell of element `base[i]..[k]`: by name, else through
    /// `array_first_id` for a 1-D array without element names.
    pub(super) fn elem_cell(&self, base: &str, idx: &[i64]) -> Option<usize> {
        let name = Self::elem_name(base, idx);
        self.signal_name_to_id
            .get(name.as_str())
            .copied()
            .or_else(|| resolve_array_elem_id(&name, &self.array_first_id))
    }

    /// Width, realness and two-state-ness of a cell, in the signal table or
    /// the packed arena.
    pub(super) fn cell_storage(&self, id: usize) -> Option<(u32, bool, bool)> {
        if is_packed_id(id) {
            let (seg, _) = self.packed.locate(id)?;
            return Some((seg.width as u32, false, seg.xmask == 0));
        }
        Some((
            *self.signal_widths.get(id)?,
            self.signal_real.get(id).copied().unwrap_or(false),
            self.signal_two_state.get(id).copied().unwrap_or(false),
        ))
    }

    /// Does a compiled edge block still need its source statement after
    /// compilation? Three kinds of later reader use it: the AST interpreter
    /// (a block that did not compile), the writer census at simulation start
    /// (a block whose bytecode holds fallback or whole-array writes) and the
    /// §16.9.3 sampled-value registration (a `$rose`/`$fell`/`$stable`/
    /// `$changed`/`$past` call). `--warn-x` and `XEZIM_TRACE_ALWAYS` derive
    /// driver write sets from every statement at run time.
    pub(super) fn edge_body_needed(
        &self,
        cb: Option<&super::bytecode::CompiledBlock>,
        body: &Statement,
    ) -> bool {
        use super::bytecode::Insn;
        if self.warn_x || self.trace_always.is_some() {
            return true;
        }
        let Some(cb) = cb else {
            return true;
        };
        let census_reads_ast = cb.instructions.iter().any(|insn| {
            matches!(
                insn,
                Insn::StmtFallback(..)
                    | Insn::EvalExprFallback(..)
                    | Insn::NbaAssignArray(..)
                    | Insn::BlockingAssignArray(..)
                    | Insn::NbaAssignArrayRange(..)
                    | Insn::BlockingAssignArrayRange(..)
            )
        });
        census_reads_ast || Self::stmt_has_sampled_call(body)
    }

    pub(super) fn expr_has_sampled_call(e: &Expression) -> bool {
        match &e.kind {
            ExprKind::SystemCall { name, args } => {
                matches!(
                    name.as_str(),
                    "$rose" | "$fell" | "$stable" | "$changed" | "$past"
                ) || args.iter().any(Self::expr_has_sampled_call)
            }
            ExprKind::Call { args, .. } => args.iter().any(Self::expr_has_sampled_call),
            ExprKind::Binary { left, right, .. } => {
                Self::expr_has_sampled_call(left) || Self::expr_has_sampled_call(right)
            }
            ExprKind::Unary { operand, .. } => Self::expr_has_sampled_call(operand),
            ExprKind::Paren(inner) => Self::expr_has_sampled_call(inner),
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                Self::expr_has_sampled_call(condition)
                    || Self::expr_has_sampled_call(then_expr)
                    || Self::expr_has_sampled_call(else_expr)
            }
            ExprKind::Concatenation(items) => items.iter().any(Self::expr_has_sampled_call),
            ExprKind::Index { expr, index } => {
                Self::expr_has_sampled_call(expr) || Self::expr_has_sampled_call(index)
            }
            _ => false,
        }
    }

    /// A sampled-value call anywhere `scan_sampled_stmt` and
    /// `collect_sampled_sites_stmt` look (a superset of both).
    pub(super) fn stmt_has_sampled_call(st: &Statement) -> bool {
        use crate::ast::stmt::StatementKind as SK;
        match &st.kind {
            SK::SeqBlock { stmts, .. } | SK::ParBlock { stmts, .. } => {
                stmts.iter().any(Self::stmt_has_sampled_call)
            }
            SK::If {
                condition,
                then_stmt,
                else_stmt,
                ..
            } => {
                Self::expr_has_sampled_call(condition)
                    || Self::stmt_has_sampled_call(then_stmt)
                    || else_stmt
                        .as_deref()
                        .is_some_and(Self::stmt_has_sampled_call)
            }
            SK::TimingControl { stmt, .. } => Self::stmt_has_sampled_call(stmt),
            SK::For { body, .. }
            | SK::While { body, .. }
            | SK::DoWhile { body, .. }
            | SK::Repeat { body, .. }
            | SK::Forever { body }
            | SK::Foreach { body, .. } => Self::stmt_has_sampled_call(body),
            SK::Case { expr, items, .. } => {
                Self::expr_has_sampled_call(expr)
                    || items.iter().any(|it| Self::stmt_has_sampled_call(&it.stmt))
            }
            SK::BlockingAssign { rvalue, .. } | SK::NonblockingAssign { rvalue, .. } => {
                Self::expr_has_sampled_call(rvalue)
            }
            SK::Expr(e) => Self::expr_has_sampled_call(e),
            _ => false,
        }
    }

    /// Free the source statement of every edge block whose bytecode now
    /// stands in for it everywhere (see `edge_body_needed`); a block folded
    /// into a merged one never runs at all. Such a block keeps an empty
    /// statement with the same span (diagnostics report the span). A lazy
    /// block's body was already dropped, or kept, as it compiled.
    pub(super) fn drop_compiled_edge_block_asts(&mut self) {
        let nb = self.edge_blocks.len();
        let droppable: Vec<bool> = (0..nb)
            .map(|bi| {
                let retired = self.edge_block_retired.get(bi).copied().unwrap_or(false);
                let cb = self.compiled_edge_blocks.get(bi).and_then(|c| c.as_ref());
                let body = &self.edge_blocks[bi].stmt;
                !matches!(body.kind, StatementKind::Null)
                    && !(self.warn_x || self.trace_always.is_some())
                    && (retired || !self.edge_body_needed(cb, body))
            })
            .collect();
        if !droppable.iter().any(|&d| d) {
            return;
        }
        // Shared (a worker holds the list): leave it rather than copy it.
        let Some(blocks) = Arc::get_mut(&mut self.edge_blocks) else {
            return;
        };
        for (b, drop) in blocks.iter_mut().zip(droppable) {
            if drop {
                let span = b.stmt.span;
                b.stmt = Statement::new(StatementKind::Null, span);
            }
        }
    }

    pub(super) fn compile_edge_blocks(&mut self) {
        use super::bytecode::BytecodeCompiler;
        let mut compiled = Vec::with_capacity(self.edge_blocks.len());
        let mut bc_count = 0;
        let mut max_regs: u32 = 0;
        let bc_dump = std::env::var("XEZIM_BC_DUMP").ok();
        // Bodies rebuilt for lazy blocks that must keep them (see
        // `edge_body_needed`); the others are dropped right after compiling,
        // so the bodies never all exist at once.
        let mut kept_bodies: Vec<(usize, Statement)> = Vec::new();
        let mut any_lazy = false;
        for (bi, block) in self.edge_blocks.iter().enumerate() {
            let lazy_body = block.lazy.as_ref().map(|l| l.materialize_body());
            any_lazy |= lazy_body.is_some();
            let body: &Statement = lazy_body.as_ref().unwrap_or(&block.stmt);
            // Scope hint for unqualified idents inside the block: the
            // block's own inlining scope when it has one, else derived from
            // the first sensitivity signal. (Sensitivity-only derivation
            // fails when a port-connected clock collapses to the parent's
            // signal — e.g. `always_ff @(posedge clk)` in a submodule whose
            // clk is the top clock.)
            let scope_hint = if !block.scope.is_empty() {
                Some(block.scope.clone())
            } else {
                // An EMPTY scope means the block belongs to the TOP module —
                // its bare names are top names. Deriving a scope from the
                // first sensitivity signal mis-scoped a top-level block to a
                // SUB-instance whenever the clock resolved to an instance's
                // same-named port copy ("clk" → "d.clk"): every bare NBA
                // target then wrote the instance's copy and the top signal
                // was never written at all (a TB grant strobe stayed x
                // forever and its whole datapath wedged). Derive only when
                // the derived scope can also account for the block's OWN
                // first write target; a top-resolvable target keeps None.
                let derived = block
                    .resolved_sensitivities
                    .first()
                    .and_then(|sid| self.name_opt(sid.signal_id))
                    .and_then(|full| full.rsplit_once('.').map(|(p, _)| p.to_string()));
                match derived {
                    Some(d) => {
                        let mut reads: HashSet<String> = HashSet::default();
                        let mut writes: HashSet<String> = HashSet::default();
                        Self::collect_stmt_reads(body, &self.module, &mut reads, &mut writes);
                        let top_resolvable = writes.iter().any(|w| {
                            !w.contains('.') && self.signal_name_to_id.contains_key(w.as_str())
                        });
                        if top_resolvable { None } else { Some(d) }
                    }
                    None => None,
                }
            };
            let mut compiler = BytecodeCompiler::new(
                &self.signal_name_to_id,
                &self.signal_signed,
                &self.signal_widths,
                &self.module.arrays,
                &self.widths,
            );
            compiler.set_ast_fallback(true);
            // Edge blocks only: sensitivity is the explicit clock list, so a
            // read hidden inside an interpreted expression can't break wake-up
            // analysis here (unlike comb entries / cont-assigns).
            compiler.set_expr_fallback(true);
            compiler.set_pkg_enum_consts(&self.pkg_enum_consts);
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
            let ok = compiler.compile_stmt(body);
            if !ok && std::env::var_os("XEZIM_EDGE_BLOCK_STATS").is_some() {
                eprintln!(
                    "[EDGE-BAIL] scope='{}' reason={} span={}..{}",
                    block.scope,
                    compiler.bail_reason.unwrap_or("unknown"),
                    body.span.start,
                    body.span.end
                );
            }
            if ok {
                let cb = compiler.finish();
                if let Some(limit) = &bc_dump {
                    let limit: usize = limit.parse().unwrap_or(0);
                    if compiled.len() < limit {
                        eprintln!(
                            "[BC-DUMP] block#{} scope='{}' insns={:?}",
                            compiled.len(),
                            block.scope,
                            if std::env::var_os("XEZIM_BC_DUMP_FULL").is_some() {
                                cb.instructions
                                    .iter()
                                    .map(|i| format!("{:?}", i))
                                    .collect::<Vec<_>>()
                            } else {
                                cb.instructions
                                    .iter()
                                    .map(|i| super::bytecode::insn_opcode_name(i).to_string())
                                    .collect::<Vec<_>>()
                            }
                        );
                    }
                }
                if cb.num_regs > max_regs {
                    max_regs = cb.num_regs;
                }
                bc_count += 1;
                compiled.push(Some(cb));
            } else {
                compiled.push(None);
            }
            if let Some(body) = lazy_body {
                let cb = compiled.last().and_then(|c| c.as_ref());
                if self.edge_body_needed(cb, &body) {
                    kept_bodies.push((bi, body));
                }
            }
        }
        if any_lazy {
            let blocks = Arc::make_mut(&mut self.edge_blocks);
            for b in blocks.iter_mut() {
                b.lazy = None;
            }
            for (bi, body) in kept_bodies {
                blocks[bi].stmt = body;
            }
        }
        self.compiled_edge_blocks = compiled;
        self.ts_edge.clear();

        // Process merging:
        // merge edge blocks with IDENTICAL plain sensitivities into one
        // compiled block. Same-signature members always fire together (100%
        // co-activation BY CONSTRUCTION — the recompute-waste defect that
        // sank comb-region fusion cannot occur here), and concatenation in
        // block-index order preserves relative order among the members.
        //
        // Guards, each paid for by an earlier measurement:
        //  * members must be individually native-admissible (fit_u64 AND
        //    two-state-lowerable) and the MERGED block must be too — comb
        //    fusion without this demoted 4,563 c906 entries from native to
        //    interpreted and cost 24% wall;
        //  * NBA-only bodies (no blocking stores): merging changes a member's
        //    position relative to NON-member blocks sharing the same trigger
        //    signal, which is observable for blocking writes but not for
        //    queued NBAs;
        //  * no `iff` / non-trivial event terms; no fallback insns.
        // The measured default cap is eight. `=0` disables the pass,
        // `=census` sizes the population, and `=N` selects another cap.
        {
            use super::bytecode::Insn as EI;
            let em = std::env::var("XEZIM_EDGE_MERGE").unwrap_or_else(|_| "8".to_string());
            let census_only = em == "census";
            let cap: usize = if census_only {
                usize::MAX
            } else {
                em.parse().unwrap_or(0)
            };
            if census_only || cap >= 2 {
                let native_ok = |this: &Self, cb: &super::bytecode::CompiledBlock| -> bool {
                    this.block_signals_fit_u64(cb)
                        && super::bytecode::lower_two_state(
                            cb,
                            &this.signal_widths,
                            &this.signal_signed,
                            &this.signal_real,
                            &this.array_first_id,
                        )
                        .is_some()
                };
                let nba_only = |cb: &super::bytecode::CompiledBlock| -> bool {
                    !cb.instructions.iter().any(|ins| {
                        matches!(
                            ins,
                            EI::BlockingAssign(..)
                                | EI::BlockingAssignRange(..)
                                | EI::BlockingAssignRangeDyn(..)
                                | EI::BlockingAssignBitDyn(..)
                                | EI::BlockingAssignArray(..)
                                | EI::BlockingAssignArrayRange(..)
                                | EI::BlockingAssignString(..)
                        )
                    })
                };
                // §10.4.2 last-write-wins hazard: merging moves a member's
                // NBA pushes relative to NON-member blocks that share a
                // trigger signal, so if two edge blocks drive the SAME target
                // on one edge, the surviving value could flip. Admit only
                // single-driver targets — the same discipline the identity-net
                // collapse uses. (Conservative: two members of one chunk
                // driving one target keep their order by construction, but
                // proving that per-chunk is not worth the complexity.)
                let mut nba_target_writers: HashMap<u64, u32> = HashMap::default();
                let block_nba_targets = |cb: &super::bytecode::CompiledBlock| -> Vec<u64> {
                    let mut out: Vec<u64> = Vec::new();
                    for ins in cb.instructions.iter() {
                        match ins {
                            EI::NbaAssign(sig, ..)
                            | EI::NbaAssignConst(sig, ..)
                            | EI::NbaAssignRange(sig, ..)
                            | EI::NbaAssignRangeDyn(sig, ..)
                            | EI::NbaAssignBitDyn(sig, ..)
                            | EI::NbaAssignArrayRead(sig, ..) => out.push(*sig as u64),
                            EI::NbaAssignArray(arr, ..) | EI::NbaAssignArrayRange(arr, ..) => {
                                // Array targets: key on the dense base id (or
                                // a name hash for Named arrays), tagged above
                                // the signal-id space.
                                let key = match arr.as_ref() {
                                    super::bytecode::ArrayOperand::Dense { first_id, .. } => {
                                        *first_id as u64
                                    }
                                    super::bytecode::ArrayOperand::Named(n) => {
                                        let mut h: u64 = 1469598103934665603;
                                        for b in n.as_bytes() {
                                            h = (h ^ *b as u64).wrapping_mul(1099511628211);
                                        }
                                        h
                                    }
                                };
                                out.push(key | (1u64 << 63));
                            }
                            _ => {}
                        }
                    }
                    out.sort_unstable();
                    out.dedup();
                    out
                };
                for cb in self.compiled_edge_blocks.iter().flatten() {
                    for t in block_nba_targets(cb) {
                        *nba_target_writers.entry(t).or_insert(0) += 1;
                    }
                }
                let mut groups: HashMap<Vec<(usize, u8)>, Vec<usize>> = HashMap::default();
                let mut n_eligible = 0usize;
                let mut r_multidrv = 0usize;
                let (
                    mut r_nocb,
                    mut r_fallback,
                    mut r_blocking,
                    mut r_offset,
                    mut r_native,
                    mut r_sens,
                ) = (0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
                for (i, b) in self.edge_blocks.iter().enumerate() {
                    let Some(cb) = self.compiled_edge_blocks[i].as_ref() else {
                        r_nocb += 1;
                        continue;
                    };
                    if cb.has_fallback {
                        r_fallback += 1;
                        continue;
                    }
                    if !nba_only(cb) {
                        r_blocking += 1;
                        continue;
                    }
                    if !cb.instructions.iter().all(|ins| {
                        let mut probe = ins.clone();
                        probe.offset_regs_and_targets(0, 0)
                    }) {
                        r_offset += 1;
                        continue;
                    }
                    if !native_ok(self, cb) {
                        r_native += 1;
                        continue;
                    }
                    if block_nba_targets(cb)
                        .iter()
                        .any(|t| nba_target_writers.get(t).copied().unwrap_or(0) != 1)
                    {
                        r_multidrv += 1;
                        continue;
                    }
                    let mut sig: Vec<(usize, u8)> = Vec::new();
                    let mut ok = true;
                    for t in &b.resolved_sensitivities {
                        if t.iff.is_some() || t.value_of.is_some() {
                            ok = false;
                            break;
                        }
                        let tag = match t.edge {
                            EdgeKind::Posedge => 0u8,
                            EdgeKind::Negedge => 1u8,
                            EdgeKind::AnyEdge => 2u8,
                            _ => {
                                ok = false;
                                break;
                            }
                        };
                        sig.push((t.signal_id, tag));
                    }
                    if !ok || sig.is_empty() {
                        r_sens += 1;
                        continue;
                    }
                    sig.sort_unstable();
                    sig.dedup();
                    n_eligible += 1;
                    groups.entry(sig).or_default().push(i);
                }
                if census_only {
                    let mergeable: usize = groups
                        .values()
                        .filter(|v| v.len() >= 2)
                        .map(|v| v.len())
                        .sum();
                    let ngroups = groups.values().filter(|v| v.len() >= 2).count();
                    let mut sizes: Vec<usize> = groups
                        .values()
                        .map(|v| v.len())
                        .filter(|&n| n >= 2)
                        .collect();
                    sizes.sort_unstable_by(|a, b| b.cmp(a));
                    eprintln!(
                        "[EDGE-MERGE] reject: no_cb={} fallback={} blocking={} offset={} native={} multidrv={} sens={}",
                        r_nocb, r_fallback, r_blocking, r_offset, r_native, r_multidrv, r_sens
                    );
                    eprintln!(
                        "[EDGE-MERGE] census: eligible={} of {} blocks; mergeable={} in {} groups; top sizes {:?}",
                        n_eligible,
                        self.edge_blocks.len(),
                        mergeable,
                        ngroups,
                        &sizes[..sizes.len().min(10)]
                    );
                } else {
                    let mut merged_blocks = 0usize;
                    let mut merged_members = 0usize;
                    let mut retired: Vec<usize> = Vec::new();
                    let mut group_list: Vec<Vec<usize>> =
                        groups.into_values().filter(|v| v.len() >= 2).collect();
                    // Deterministic order (HashMap iteration is not).
                    group_list.sort_unstable();
                    for mut idxs in group_list {
                        idxs.sort_unstable();
                        for chunk in idxs.chunks(cap) {
                            if chunk.len() < 2 {
                                continue;
                            }
                            let total_regs: u64 = chunk
                                .iter()
                                .map(|&m| {
                                    self.compiled_edge_blocks[m]
                                        .as_ref()
                                        .map(|c| c.num_regs as u64)
                                        .unwrap_or(0)
                                })
                                .sum();
                            if total_regs > u16::MAX as u64 {
                                continue;
                            }
                            let mut merged = super::bytecode::CompiledBlock {
                                instructions: Vec::new(),
                                num_regs: 0,
                                has_fallback: false,
                                nba_dup_targets: true,
                                foreach_slots: 0,
                                static_locals: Box::new([]),
                            };
                            let mut ok = true;
                            for &m in chunk {
                                let mb = self.compiled_edge_blocks[m].as_ref().unwrap();
                                let off = merged.instructions.len() as u32;
                                let reg_base = merged.num_regs as u16;
                                for ins in mb.instructions.iter() {
                                    let mut i2 = ins.clone();
                                    if !i2.offset_regs_and_targets(reg_base, off) {
                                        ok = false;
                                        break;
                                    }
                                    merged.instructions.push(i2);
                                }
                                if !ok {
                                    break;
                                }
                                merged.num_regs += mb.num_regs;
                            }
                            // The merged block must keep the native tier its
                            // members individually had, or the merge is a
                            // demotion (see the guard note above).
                            if !ok || !native_ok(self, &merged) {
                                continue;
                            }
                            let head = chunk[0];
                            self.compiled_edge_blocks[head] = Some(merged);
                            for &m in &chunk[1..] {
                                self.compiled_edge_blocks[m] = None;
                                retired.push(m);
                            }
                            merged_blocks += 1;
                            merged_members += chunk.len();
                        }
                    }
                    if !retired.is_empty() {
                        // Clearing the sensitivity list takes a retired member
                        // out of every trigger path: the edge fanout index and
                        // the scan structures are built from these lists AFTER
                        // this function returns.
                        self.edge_block_retired
                            .resize(self.compiled_edge_blocks.len(), false);
                        for &m in &retired {
                            self.edge_block_retired[m] = true;
                        }
                        let blocks = Arc::make_mut(&mut self.edge_blocks);
                        for m in retired {
                            blocks[m].resolved_sensitivities.clear();
                        }
                    }
                    if merged_members > 0 {
                        chatter!(
                            "[EDGE-MERGE] merged {} blocks into {} (cap {})",
                            merged_members,
                            merged_blocks,
                            cap
                        );
                    }
                }
            }
        }

        // Retained dynamic-delay processes are also hot procedural code. In
        // PLL models this is commonly `always #(period/2) clk = ...`, which
        // runs millions of times. Resolve that assignment once instead of
        // repeating AST name lookup and temporary allocation at every edge.
        let retained_pids: Vec<usize> = self.fast_delay_always.keys().copied().collect();
        let mut retained_bc_count = 0usize;
        let mut retained_delay_bc_count = 0usize;
        for pid in retained_pids {
            let (compiled_body, compiled_delay) = {
                let Some(fast) = self.fast_delay_always.get(&pid) else {
                    continue;
                };
                let scope_hint = self.process_scope_hint.get(&pid).cloned();
                let mut compiler = BytecodeCompiler::new(
                    &self.signal_name_to_id,
                    &self.signal_signed,
                    &self.signal_widths,
                    &self.module.arrays,
                    &self.widths,
                );
                compiler.set_ast_fallback(false);
                compiler.set_pkg_enum_consts(&self.pkg_enum_consts);
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
                compiler
                    .set_collection_denies(&self.module.dynamic_arrays, &self.module.queue_vars);
                compiler.set_array_first_id(&self.array_first_id);
                compiler.set_string_signals(&self.module.string_signals);
                compiler.set_signal_real(&self.signal_real);
                compiler.top_module_name = Some(self.module.name.clone());
                let body = if compiler.compile_stmt(&fast.body) {
                    Some(compiler.finish())
                } else {
                    None
                };

                let mut delay_compiler = BytecodeCompiler::new(
                    &self.signal_name_to_id,
                    &self.signal_signed,
                    &self.signal_widths,
                    &self.module.arrays,
                    &self.widths,
                );
                delay_compiler.set_ast_fallback(false);
                delay_compiler.set_scope_hint(self.process_scope_hint.get(&pid).cloned());
                delay_compiler.set_pkg_enum_consts(&self.pkg_enum_consts);
                delay_compiler.set_tasks(&self.module.tasks);
                delay_compiler.set_functions(&self.module.functions);
                delay_compiler.set_params(&self.module.parameters);
                delay_compiler.set_param_leaf_idx(self.param_leaf_index());
                delay_compiler.set_cast_widths(&self.cast_widths);
                delay_compiler.set_packed_struct_fields(
                    self.bc_packed_fields
                        .as_ref()
                        .unwrap_or(&self.module.packed_struct_fields),
                );
                delay_compiler.set_packed_elem_widths(&self.module.packed_signal_elem_widths);
                delay_compiler
                    .set_typedefs(&self.module.typedefs, &self.module.typedef_elem_widths);
                delay_compiler.set_typedef_types(&self.module.typedef_types);
                delay_compiler.set_assoc_elem_widths(&self.module.assoc_elem_widths);
                delay_compiler.set_assoc_arrays(&self.module.associative_arrays);
                delay_compiler.set_packed_full_dims(&self.module.packed_full_dims);
                delay_compiler.set_multi_dim_arrays(&self.multi_dim_array_names);
                delay_compiler.set_arrays_2d(&self.module.arrays_2d);
                delay_compiler
                    .set_collection_denies(&self.module.dynamic_arrays, &self.module.queue_vars);
                delay_compiler.set_array_first_id(&self.array_first_id);
                delay_compiler.set_string_signals(&self.module.string_signals);
                delay_compiler.set_signal_real(&self.signal_real);
                delay_compiler.top_module_name = Some(self.module.name.clone());
                let delay = delay_compiler
                    .compile_root_expr(&fast.delay)
                    .map(|result_reg| (delay_compiler.finish(), result_reg));
                (body, delay)
            };
            if let Some(cb) = &compiled_body {
                max_regs = max_regs.max(cb.num_regs);
                retained_bc_count += 1;
            }
            if let Some((cb, _)) = &compiled_delay {
                max_regs = max_regs.max(cb.num_regs);
                retained_delay_bc_count += 1;
            }
            if let Some(fast) = self.fast_delay_always.get_mut(&pid) {
                fast.compiled_body = compiled_body;
                fast.compiled_delay = compiled_delay;
            }
        }

        // Pre-allocate register file for the largest compiled block
        self.vm_regs = vec![Value::zero(1); max_regs as usize];
        sim_dbg_eprintln!(
            "[OPT] bytecode compiled: {}/{} edge blocks",
            bc_count,
            self.edge_blocks.len()
        );
        sim_dbg_eprintln!(
            "[OPT] bytecode compiled: {}/{} retained dynamic-delay bodies",
            retained_bc_count,
            self.fast_delay_always.len()
        );
        sim_dbg_eprintln!(
            "[OPT] bytecode compiled: {}/{} retained dynamic-delay expressions",
            retained_delay_bc_count,
            self.fast_delay_always.len()
        );
        self.edge_block_exec_counts = vec![0; self.compiled_edge_blocks.len()];
        if self.edge_block_stats_enabled {
            self.edge_block_exec_ns = vec![0u64; self.compiled_edge_blocks.len()];
            self.signal_toggle_count = vec![0u64; self.signal_table.len()];
            // nba_count_per_block is the COMPILE-TIME count of
            // NbaAssign* instructions per block. Multiply by
            // edge_block_exec_counts[bi] to get the runtime cost.
            let mut nba_count = vec![0u64; self.compiled_edge_blocks.len()];
            for (bi, cb_opt) in self.compiled_edge_blocks.iter().enumerate() {
                if let Some(cb) = cb_opt {
                    let n = cb
                        .instructions
                        .iter()
                        .filter(|i| {
                            matches!(
                                i,
                                super::bytecode::Insn::NbaAssign(..)
                                    | super::bytecode::Insn::NbaAssignConst(..)
                                    | super::bytecode::Insn::NbaAssignRange(..)
                                    | super::bytecode::Insn::NbaAssignRangeDyn(..)
                                    | super::bytecode::Insn::NbaAssignBitDyn(..)
                                    | super::bytecode::Insn::NbaAssignArray(..)
                                    | super::bytecode::Insn::NbaAssignArrayRange(..)
                                    | super::bytecode::Insn::NbaAssignArrayRead(..)
                            )
                        })
                        .count() as u64;
                    nba_count[bi] = n;
                }
            }
            self.nba_count_per_block = nba_count;
            self.sensitivity_trigger_count = vec![0u64; self.signal_table.len()];
            self.cross_block_wakeup_count = vec![0u64; self.compiled_edge_blocks.len()];
            // boundary_packet_count + local_quiescence_iterations are
            // sized later when a partition is loaded — they're
            // partition-indexed, not block-indexed.
        }

        // JIT pass: attempt native codegen for each compiled block.
        // Unsupported Insns inside a block → None (interpreter runs it).
        // When feature=jit is off, `JitModule::new` returns None and
        // every `try_compile` also returns None — zero-cost fallback.
        let enable_jit = std::env::var("XEZIM_JIT")
            .map(|v| v != "0" && !v.is_empty())
            .unwrap_or(false);
        self.jit_fns = vec![None; self.compiled_edge_blocks.len()];
        // Pre-compute which compiled blocks touch any signal with width
        // > 64. JIT bridges (jit_load_signal / jit_load_array_elem) read
        // signals as u64 and silently truncate wider values; refuse to
        // JIT those blocks so the interpreter (which preserves full
        // Value width) handles them.
        let block_jit_safe: Vec<bool> = if enable_jit {
            self.compiled_edge_blocks
                .iter()
                .map(|cb_opt| match cb_opt {
                    Some(cb) => self.block_signals_fit_u64(cb),
                    None => false,
                })
                .collect()
        } else {
            Vec::new()
        };
        if enable_jit && std::env::var("XEZIM_JIT_BAIL_TRACE").is_ok() {
            for (idx, cb_opt) in self.compiled_edge_blocks.iter().enumerate() {
                if let Some(cb) = cb_opt {
                    if !block_jit_safe[idx] {
                        eprintln!(
                            "[EDGEGATE] idx={} insns={} reason={:?}",
                            idx,
                            cb.instructions.len(),
                            self.block_fit_u64_reason(cb)
                        );
                    }
                }
            }
        }
        if enable_jit {
            // Single backend: cranelift. (An LLVM backend once existed
            // behind XEZIM_JIT_BACKEND=llvm; it was removed — warn if
            // someone still asks for it.)
            let backend =
                std::env::var("XEZIM_JIT_BACKEND").unwrap_or_else(|_| "cranelift".to_string());
            let xz_ptr = self.signal_has_xz.as_ptr() as u64;
            let xz_len = self.signal_has_xz.len() as u32;
            let jit_compile_start = std::time::Instant::now();
            let mut jit_count = 0usize;
            if backend == "llvm" {
                eprintln!(
                    "[JIT] the LLVM backend was removed; using cranelift \
                     (unset XEZIM_JIT_BACKEND)"
                );
            }
            {
                {
                    if self.jit_module.is_none() {
                        self.jit_module = super::jit::JitModule::new();
                    }
                    // JIT Stage 2: hand the JIT module the
                    // signal_inline_bits base pointer so LoadSignal
                    // codegen can read values without crossing FFI.
                    // Only effective when XEZIM_INLINE_BITS=1 has
                    // populated signal_inline_bits.
                    let inline_ptr = self.signal_inline_bits.as_ptr() as u64;
                    let inline_len = self.signal_inline_bits.len() as u32;
                    // Fallback statements inside a natively compiled block
                    // run under the block's scope (%m, bare names, %t), as
                    // on the bytecode path.
                    let edge_scopes: Vec<Option<String>> = (0..self.compiled_edge_blocks.len())
                        .map(|i| self.edge_block_scope_for(i))
                        .collect();
                    if let Some(jm) = self.jit_module.as_mut() {
                        if inline_len > 0 {
                            jm.set_inline_bits_storage(inline_ptr, inline_len);
                        }
                        // JIT Stage 4 Tier A: hand the JIT module a
                        // snapshot of signal widths so the NbaAssign
                        // codegen can pick between the slow / fast
                        // bridges at compile time.
                        jm.set_signal_widths(self.signal_widths.clone());
                        jm.set_signal_signed(self.signal_signed.clone());
                        // JIT Stage 4 Tier C Path C1: hand the JIT
                        // module the side-queue base pointer + length
                        // pointer so NbaAssign codegen can emit a
                        // direct inline write (no FFI).  Only
                        // populated when XEZIM_INLINE_BITS=1.
                        if !self.jit_nba_side_queue.is_empty() {
                            let base = self.jit_nba_side_queue.as_ptr() as u64;
                            let len_p = (&self.jit_nba_side_len as *const u32) as u64;
                            jm.set_nba_side_queue(
                                base,
                                len_p,
                                self.jit_nba_side_queue.len() as u32,
                            );
                        }
                        // Bisection aid: XEZIM_JIT_ONLY="3,17" compiles just
                        // those block indices; XEZIM_JIT_SKIP_IDX skips listed.
                        let jit_only: Option<Vec<usize>> = std::env::var("XEZIM_JIT_ONLY")
                            .ok()
                            .map(|v| v.split(',').filter_map(|t| t.trim().parse().ok()).collect());
                        let jit_skip: Vec<usize> = std::env::var("XEZIM_JIT_SKIP_IDX")
                            .ok()
                            .map(|v| v.split(',').filter_map(|t| t.trim().parse().ok()).collect())
                            .unwrap_or_default();
                        let trace_bails = std::env::var("XEZIM_JIT_BAIL_TRACE").is_ok();
                        // XEZIM_AOT=1 (roadmap step 10): rustc-native edge
                        // blocks through the same crate/bridge machinery as
                        // comb entries — a second dylib, installed into the
                        // same jit_fns slots. NBAs stay bridged. Cranelift
                        // below fills whatever the AOT emitter couldn't.
                        // (The `aot` module only exists in jit builds.)
                        #[cfg(feature = "jit")]
                        if std::env::var("XEZIM_AOT")
                            .map(|v| v == "1")
                            .unwrap_or(false)
                        {
                            let verbose = std::env::var("XEZIM_JIT_VERBOSE").is_ok();
                            let templated = std::env::var("XEZIM_AOT_TEMPLATE")
                                .map(|v| v == "1")
                                .unwrap_or(false);
                            self.edge_tpl_fns = vec![None; self.compiled_edge_blocks.len()];
                            if templated {
                                // Mirror of the comb templated branch: one
                                // body per canonical shape, per-block signal
                                // maps host-side. Edge blocks were the last
                                // per-block-baked crate (37MB / ~308s rustc
                                // on the C910 SoC).
                                let items: Vec<(usize, Vec<super::bytecode::Insn>, u32)> = self
                                    .compiled_edge_blocks
                                    .iter()
                                    .enumerate()
                                    .filter_map(|(idx, b)| {
                                        let cb = b.as_ref()?;
                                        if !block_jit_safe[idx] {
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
                                        "[AOT-TEMPLATE] edge blocks={} templates={} ({:.1}x fewer bodies)",
                                        built.n_blocks,
                                        built.n_templates,
                                        built.n_blocks as f64 / built.n_templates.max(1) as f64
                                    );
                                }
                                if !built.fns.is_empty() {
                                    let src = super::aot::module_source_named(
                                        &built.fns,
                                        &built.tpl_names,
                                    );
                                    let planes = (
                                        self.signal_inline_bits.as_ptr() as u64,
                                        self.signal_inline_bits.len() as u32,
                                    );
                                    if let Some(lib) =
                                        super::aot::compile_and_load(&src, verbose, planes)
                                    {
                                        let mut n = 0usize;
                                        if let Some(api) = lib.api() {
                                            for (idx, tpl_pos, map) in &built.blocks {
                                                let ptr = api(*tpl_pos);
                                                if ptr.is_null() {
                                                    continue;
                                                }
                                                let f: super::aot::AotTplFn =
                                                    unsafe { std::mem::transmute(ptr) };
                                                let off = self.edge_tpl_map_blob.len() as u32;
                                                self.edge_tpl_map_blob.extend_from_slice(map);
                                                self.edge_tpl_fns[*idx] =
                                                    Some((f, off, map.len() as u32));
                                                n += 1;
                                            }
                                        }
                                        std::mem::forget(lib);
                                        if verbose {
                                            eprintln!(
                                                "[AOT] edge blocks compiled {}/{} (templated)",
                                                n,
                                                self.compiled_edge_blocks.len()
                                            );
                                        }
                                    }
                                }
                            }
                            let mut names: Vec<(usize, String)> = Vec::new();
                            let mut fns_src: Vec<String> = Vec::new();
                            for (idx, cb_opt) in self.compiled_edge_blocks.iter().enumerate() {
                                if templated {
                                    break;
                                }
                                let Some(cb) = cb_opt else { continue };
                                if !block_jit_safe[idx] {
                                    continue;
                                }
                                let name = format!("xezim_aot_eb_{idx}");
                                if let Some(src) = super::aot::gen_block_fn(
                                    &name,
                                    &cb.instructions,
                                    cb.num_regs,
                                    &self.signal_widths,
                                    &self.signal_signed,
                                ) {
                                    names.push((idx, name));
                                    fns_src.push(src);
                                } else if verbose {
                                    let miss = cb
                                        .instructions
                                        .iter()
                                        .map(super::bytecode::insn_opcode_name)
                                        .collect::<std::collections::HashSet<_>>();
                                    eprintln!("[AOT-BAIL-EDGE] idx={} ops={:?}", idx, miss);
                                }
                            }
                            if !fns_src.is_empty() {
                                let ordered: Vec<String> =
                                    names.iter().map(|(_, n)| n.clone()).collect();
                                let src = super::aot::module_source_named(&fns_src, &ordered);
                                let planes = (
                                    self.signal_inline_bits.as_ptr() as u64,
                                    self.signal_inline_bits.len() as u32,
                                );
                                if let Some(lib) =
                                    super::aot::compile_and_load(&src, verbose, planes)
                                {
                                    let mut n = 0usize;
                                    if let Some(api) = lib.api() {
                                        for (k, (idx, _)) in names.iter().enumerate() {
                                            let p = api(k as u32);
                                            if !p.is_null() {
                                                self.jit_fns[*idx] = Some(unsafe {
                                                    std::mem::transmute::<*mut u8, super::jit::JitFn>(
                                                        p,
                                                    )
                                                });
                                                n += 1;
                                            }
                                        }
                                    } else {
                                        for (idx, name) in &names {
                                            if let Some(f) = lib.sym(name) {
                                                self.jit_fns[*idx] = Some(f);
                                                n += 1;
                                            }
                                        }
                                    }
                                    std::mem::forget(lib);
                                    chatter!(
                                        "[AOT] edge blocks compiled {}/{}",
                                        n,
                                        self.compiled_edge_blocks.len()
                                    );
                                }
                            }
                        }
                        for (idx, cb_opt) in self.compiled_edge_blocks.iter().enumerate() {
                            if let Some(cb) = cb_opt {
                                if !block_jit_safe[idx] {
                                    continue;
                                }
                                // Slot already filled by the AOT branch above.
                                #[cfg(feature = "jit")]
                                let tpl_filled =
                                    self.edge_tpl_fns.get(idx).is_some_and(|t| t.is_some());
                                #[cfg(not(feature = "jit"))]
                                let tpl_filled = false;
                                if self.jit_fns[idx].is_some() || tpl_filled {
                                    jit_count += 1;
                                    continue;
                                }
                                if let Some(only) = &jit_only {
                                    if !only.contains(&idx) {
                                        continue;
                                    }
                                }
                                if jit_skip.contains(&idx) {
                                    continue;
                                }
                                if std::env::var("XEZIM_JIT_DUMP_BLOCKS").is_ok() {
                                    eprintln!("[JITBLK] idx={} insns:", idx);
                                    for i in cb.instructions.iter() {
                                        eprintln!("[JITBLK]   {:?}", i);
                                    }
                                }
                                let compiled_fn = jm.try_compile_with_xz_hint(
                                    &cb.instructions,
                                    cb.num_regs,
                                    xz_ptr,
                                    xz_len,
                                    edge_scopes.get(idx).and_then(|s| s.as_deref()),
                                );
                                if compiled_fn.is_none() && trace_bails {
                                    eprintln!(
                                        "[EDGESKIP] idx={} insns={} first_unsupported={:?}",
                                        idx,
                                        cb.instructions.len(),
                                        super::jit::first_unsupported(&cb.instructions)
                                    );
                                }
                                if let Some(f) = compiled_fn {
                                    self.jit_fns[idx] = Some(f);
                                    jit_count += 1;
                                }
                            }
                        }
                        // Diagnostic only. It used to print unconditionally,
                        // which leaked into tests that compare whole-run
                        // output (`dump_merged_sv_top_pruning`) the moment the
                        // JIT was enabled.
                        if std::env::var("XEZIM_JIT_VERBOSE").is_ok() {
                            eprintln!(
                                "[JIT] backend=cranelift compiled {}/{} edge blocks in {:.1}s (inline_bits={})",
                                jit_count,
                                self.compiled_edge_blocks.len(),
                                jit_compile_start.elapsed().as_secs_f64(),
                                if inline_len > 0 { "on" } else { "off" },
                            );
                        }
                    } else {
                        eprintln!("[JIT] cranelift init failed; interpreter only");
                    }
                }
            }
        }
        // Classify blocks for parallel execution: blocks with StmtFallback or
        // BlockingAssign/BlockingAssignRange/BlockingAssignBitDyn must run
        // sequentially — fallbacks need &mut self, blocking assigns mutate
        // signal_table which would race with parallel reads.
        let mut pure_count = 0;
        self.edge_block_parallel.clear();
        self.edge_block_scope.clear();
        self.edge_block_needs_hint.clear();
        self.edge_block_light.clear();
        self.edge_block_insn_len.clear();

        // Single-writer NBA map: count how many edge blocks NBA-write each
        // signal. A static-range/bit NBA block (NbaAssignRange/NbaAssignBitDyn)
        // is normally serialized because the parallel merge is last-writer-
        // wins on the full register, which loses bits when TWO blocks write
        // different sub-ranges of the SAME register. But if a signal is
        // written by exactly ONE block, its single NbaFast entry is correct
        // as-is — so such blocks are safe to parallelize without any merge
        // change. Set XEZIM_NO_PARALLEL_RANGE=1 to disable (revert to the
        // conservative all-range-sequential behavior).
        let unlock_range = std::env::var_os("XEZIM_NO_PARALLEL_RANGE").is_none();
        // Built unconditionally: the sub-range arm below consults it only when
        // `unlock_range`, but the WHOLE-SIGNAL arm needs it always. (Before,
        // this map was skipped entirely under XEZIM_NO_PARALLEL_RANGE, which
        // would have made every whole-signal NBA look single-writer.)
        let mut nba_writer_count: HashMap<usize, u32> = HashMap::default();
        {
            for cb in self.compiled_edge_blocks.iter().flatten() {
                let mut seen: Vec<usize> = Vec::new();
                for insn in &cb.instructions {
                    let id = match insn {
                        super::bytecode::Insn::NbaAssign(id, _, _)
                        | super::bytecode::Insn::NbaAssignConst(id, _, _)
                        | super::bytecode::Insn::NbaAssignRange(id, _, _, _)
                        | super::bytecode::Insn::NbaAssignBitDyn(id, _, _)
                        | super::bytecode::Insn::NbaAssignArrayRead(id, _, _, _)
                        | super::bytecode::Insn::NbaAssignRangeDyn(id, _, _, _) => *id,
                        _ => continue,
                    };
                    let id = id as usize;
                    if !seen.contains(&id) {
                        seen.push(id);
                        *nba_writer_count.entry(id).or_insert(0) += 1;
                    }
                }
            }
        }

        for (idx, cb) in self.compiled_edge_blocks.iter().enumerate() {
            let has_fallback = cb.as_ref().is_some_and(|cb| {
                cb.instructions.iter().any(|insn| {
                    matches!(
                        insn,
                        super::bytecode::Insn::StmtFallback(..)
                            | super::bytecode::Insn::CovHit(..)
                            | super::bytecode::Insn::EvalExprFallback(..)
                    )
                })
            });
            // NbaAssignBitDyn and NbaAssignRange read signal_table.clone() in
            // the parallel path and produce a full-register NbaFast entry with
            // only the addressed sub-range modified. When multiple parallel
            // blocks each write different sub-ranges of the same register
            // (e.g. yosys-synthesized per-bit FFs: `cpu_state[0] <= _00014_;`
            // in one always block, `cpu_state[1] <= _00015_;` in another),
            // merging their entries preserves only the last one — bits
            // written by earlier blocks revert to the snapshot value.
            // The sequential path merges correctly via nba_fast.rposition,
            // so keep these on the sequential path.
            // A block is parallel-safe ("pure") when it contains none of the
            // hard disqualifiers below. Static sub-range NBA writes
            // (NbaAssignRange/NbaAssignBitDyn) are conditionally allowed: they
            // disqualify only when their target signal is multi-writer (see
            // nba_writer_count above) — single-writer sub-range writes merge
            // correctly because their NbaFast entry is the sole one for that
            // signal. NbaAssignRangeDyn and all array NBA writes remain
            // sequential (dynamic-range exec is unsupported in the isolated
            // path; array writes hit the cross-thread nba_fast_index issue
            // that stalls c910 cmark).
            use super::bytecode::Insn as BcInsn;
            let is_pure = cb.as_ref().is_some_and(|cb| {
                for insn in &cb.instructions {
                    match insn {
                        BcInsn::StmtFallback(..)
                        | BcInsn::CovHit(..)
                        | BcInsn::EvalExprFallback(..)
                        | BcInsn::BlockingAssign(..)
                        | BcInsn::BlockingAssignRange(..)
                        | BcInsn::BlockingAssignRangeDyn(..)
                        | BcInsn::BlockingAssignBitDyn(..)
                        | BcInsn::NbaAssignRangeDyn(..)
                        | BcInsn::BlockingAssignArray(..)
                        | BcInsn::NbaAssignArray(..)
                        | BcInsn::NbaAssignArrayRange(..)
                        | BcInsn::BlockingAssignArrayRange(..) => return false,
                        BcInsn::NbaAssignRange(id, _, _, _) | BcInsn::NbaAssignBitDyn(id, _, _)
                            if (!unlock_range
                                || nba_writer_count.get(&(*id as usize)).copied().unwrap_or(0)
                                    > 1) =>
                        {
                            return false;
                        }
                        // WHOLE-SIGNAL NBA to a multi-writer target. The
                        // sub-range case above was already excluded because a
                        // last-writer-wins merge loses bits; the whole-signal
                        // case loses the whole VALUE the same way, and which
                        // block wins is decided by position in `nba_fast`
                        // (`Insn::NbaAssign` stamps `block_index: 0` on the
                        // sequential path and `nba_fast_index` overwrites in
                        // place). So two blocks NBA-writing one signal are
                        // resolved by DISPATCH ORDER while being labelled
                        // order-independent — which is what makes reordering
                        // `parallel_blocks` change results.
                        //
                        // Static and therefore conservative: two blocks writing
                        // the same signal under mutually exclusive conditions
                        // never collide at run time, but are demoted anyway.
                        // Measured cost of that conservatism is in the
                        // `[PURITY]` line below.
                        BcInsn::NbaAssign(id, _, _)
                        | BcInsn::NbaAssignConst(id, _, _)
                        | BcInsn::NbaAssignArrayRead(id, _, _, _)
                            if nba_writer_count.get(&(*id as usize)).copied().unwrap_or(0) > 1 =>
                        {
                            return false;
                        }
                        _ => {}
                    }
                }
                true
            });
            if is_pure {
                pure_count += 1;
            }
            self.edge_block_parallel.push(is_pure);
            // Non-compiled blocks (AST-only) always need hint.
            self.edge_block_needs_hint
                .push(has_fallback || cb.is_none());
            let has_format = cb.as_ref().is_some_and(|cb| {
                cb.instructions
                    .iter()
                    .any(|insn| matches!(insn, super::bytecode::Insn::Format(..)))
            });
            self.edge_block_light
                .push(!has_fallback && !has_format && cb.is_some());
            self.edge_block_insn_len.push(
                cb.as_ref()
                    .map(|cb| cb.instructions.len() as u32)
                    .unwrap_or(0),
            );
            // Prefer the block's own inlining scope (see compile_edge_blocks).
            let scope = self.edge_block_scope_for(idx);
            self.edge_block_scope.push(scope);
        }
        sim_dbg_eprintln!(
            "[OPT] parallel-eligible edge blocks: {}/{}",
            pure_count,
            self.compiled_edge_blocks.len()
        );
        // XEZIM_PURITY_STATS=1: break down WHY blocks are non-pure, weighted by
        // instruction count (a proxy for exec work), so we can see which merge
        // limitation to fix to widen the parallel fraction.
        if std::env::var("XEZIM_PURITY_STATS").is_ok() {
            use super::bytecode::Insn;
            // (category, blocks-containing-it, total-insns-of-those-blocks)
            let mut cats: Vec<(&str, usize, usize)> = vec![
                ("StmtFallback", 0, 0),
                ("BlockingAssign(scalar)", 0, 0),
                ("BlockingAssign(range/bit-dyn)", 0, 0),
                ("BlockingAssign(array)", 0, 0),
                ("Nba(range/bit-dyn)", 0, 0),
                ("Nba(array)", 0, 0),
                ("Nba(whole-sig, multi-writer)", 0, 0),
            ];
            let mut nonpure_blocks = 0usize;
            let mut nonpure_insns = 0usize;
            let mut pure_insns = 0usize;
            for (idx, cb) in self.compiled_edge_blocks.iter().enumerate() {
                let cb = match cb.as_ref() {
                    Some(c) => c,
                    None => continue,
                };
                let n = cb.instructions.len();
                if self.edge_block_parallel.get(idx).copied().unwrap_or(false) {
                    pure_insns += n;
                    continue;
                }
                nonpure_blocks += 1;
                nonpure_insns += n;
                let mut seen = [false; 7];
                for insn in &cb.instructions {
                    let c = match insn {
                        Insn::StmtFallback(..) => 0,
                        Insn::EvalExprFallback(..) => 0,
                        Insn::BlockingAssign(..) => 1,
                        Insn::BlockingAssignRange(..)
                        | Insn::BlockingAssignRangeDyn(..)
                        | Insn::BlockingAssignBitDyn(..) => 2,
                        Insn::BlockingAssignArray(..) | Insn::BlockingAssignArrayRange(..) => 3,
                        Insn::NbaAssignBitDyn(..)
                        | Insn::NbaAssignRange(..)
                        | Insn::NbaAssignRangeDyn(..) => 4,
                        Insn::NbaAssignArray(..) | Insn::NbaAssignArrayRange(..) => 5,
                        Insn::NbaAssign(id, _, _)
                        | Insn::NbaAssignConst(id, _, _)
                        | Insn::NbaAssignArrayRead(id, _, _, _)
                            if nba_writer_count.get(&(*id as usize)).copied().unwrap_or(0) > 1 =>
                        {
                            6
                        }
                        _ => continue,
                    };
                    seen[c] = true;
                }
                for (c, &s) in seen.iter().enumerate() {
                    if s {
                        cats[c].1 += 1;
                        cats[c].2 += n;
                    }
                }
            }
            eprintln!(
                "[PURITY] pure={} blocks ({} insns), non-pure={} blocks ({} insns)",
                pure_count, pure_insns, nonpure_blocks, nonpure_insns
            );
            for (name, blocks, insns) in &cats {
                if *blocks > 0 {
                    eprintln!(
                        "[PURITY]   {:<30} {} blocks, {} insns ({:.0}% of non-pure insns)",
                        name,
                        blocks,
                        insns,
                        100.0 * *insns as f64 / nonpure_insns.max(1) as f64
                    );
                }
            }
            // Single-writer analysis: how many range/bit-dyn-only blocks write
            // ONLY signals that no other block writes (safe to parallelize with
            // NO merge change — the lone NbaFast entry per signal is correct).
            let mut writer_count: HashMap<usize, u32> = HashMap::default();
            let block_nba_sigs = |cb: &super::bytecode::CompiledBlock| -> Vec<usize> {
                let mut v = Vec::new();
                for insn in &cb.instructions {
                    let id = match insn {
                        Insn::NbaAssign(id, _, _)
                        | Insn::NbaAssignConst(id, _, _)
                        | Insn::NbaAssignRange(id, _, _, _)
                        | Insn::NbaAssignBitDyn(id, _, _)
                        | Insn::NbaAssignArrayRead(id, _, _, _)
                        | Insn::NbaAssignRangeDyn(id, _, _, _) => *id,
                        _ => continue,
                    };
                    let id = id as usize;
                    if !v.contains(&id) {
                        v.push(id);
                    }
                }
                v
            };
            for cb in self.compiled_edge_blocks.iter().flatten() {
                for id in block_nba_sigs(cb) {
                    *writer_count.entry(id).or_insert(0) += 1;
                }
            }
            let mut safe_blocks = 0usize;
            let mut safe_insns = 0usize;
            for (idx, cb) in self.compiled_edge_blocks.iter().enumerate() {
                let cb = match cb.as_ref() {
                    Some(c) => c,
                    None => continue,
                };
                if self.edge_block_parallel.get(idx).copied().unwrap_or(false) {
                    continue; // already pure
                }
                // Eligible only if its sole disqualifier is range/bit-dyn NBA
                // (no fallback, blocking, or array writes).
                let only_range = !cb.instructions.iter().any(|i| {
                    matches!(
                        i,
                        Insn::StmtFallback(..)
                            | Insn::CovHit(..)
                            | Insn::EvalExprFallback(..)
                            | Insn::BlockingAssign(..)
                            | Insn::BlockingAssignRange(..)
                            | Insn::BlockingAssignRangeDyn(..)
                            | Insn::BlockingAssignBitDyn(..)
                            | Insn::BlockingAssignArray(..)
                            | Insn::BlockingAssignArrayRange(..)
                            | Insn::NbaAssignArray(..)
                            | Insn::NbaAssignArrayRange(..)
                            | Insn::NbaAssignRangeDyn(..)
                    )
                });
                if !only_range {
                    continue;
                }
                let all_single = block_nba_sigs(cb)
                    .iter()
                    .all(|id| writer_count.get(id).copied().unwrap_or(0) <= 1);
                if all_single {
                    safe_blocks += 1;
                    safe_insns += cb.instructions.len();
                }
            }
            eprintln!(
                "[PURITY] SAFE-to-unlock (range/bit-dyn, single-writer): {} blocks, {} insns \
                 -> would raise pure from {} to {} insns ({:.0}% of all edge insns)",
                safe_blocks,
                safe_insns,
                pure_insns,
                pure_insns + safe_insns,
                100.0 * (pure_insns + safe_insns) as f64
                    / (pure_insns + nonpure_insns).max(1) as f64
            );
        }
    }
}
