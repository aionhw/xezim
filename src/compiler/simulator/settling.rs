//! Simulator settling.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

impl Simulator {
    pub(super) fn settle_combinatorial(&mut self) {
        if self.finished {
            return;
        }
        if self.event_measure {
            self.event_phase += 1; // comb SETTLE phase (distinct from sample)
        }
        // §10.6.2: expression-RHS forces track their operands (rare —
        // guarded to a single is_empty check on the hot path).
        if !self.active_force_exprs.is_empty() {
            self.refresh_active_forces();
        }
        // §29: sequential UDPs mutate per-instance state during evaluation,
        // which the parallel/BSP isolated eval (`&self`, view-based) cannot do.
        // Force the serial fixpoint path whenever the design contains any UDP.
        self.settle_dispatch();
        // A `release` executed inside the pass (by a level-sensitive block
        // evaluated as a comb entry) could not re-drive its net because the
        // entry table was taken out: do it now and settle again until quiet.
        let mut rounds = 0;
        while !self.pending_release_ids.is_empty() && rounds < 8 {
            rounds += 1;
            let ids = std::mem::take(&mut self.pending_release_ids);
            for id in ids {
                self.mark_release_sources_dirty(id);
            }
            if self.dirty_any {
                self.settle_dispatch();
            }
        }
        if !self.active_force_exprs.is_empty() {
            self.refresh_active_forces();
        }
        // §38.36.1: continuous assignments settle without `write_sig!`, so
        // watched values are compared here; releases are reported once the
        // released net has been re-driven.
        if self.vpi_cb_mask != 0 {
            self.vpi_checkpoint();
        }
    }

    pub(super) fn settle_dispatch(&mut self) {
        if self.has_udp
            || self.has_timing_checks
            || self.proc_settle_defer
            || !self.deferred_proc_entries.is_empty()
        {
            self.settle_combinatorial_inner();
        } else if self.bsp_shadow {
            self.settle_combinatorial_bsp_shadow();
        } else if self.bsp_settle {
            self.settle_combinatorial_bsp();
        } else {
            self.settle_combinatorial_inner();
        }
        // §10.6.2 again, AFTER the settle body: a tracked operand that is
        // itself comb-driven (a bound module forcing from `host.wire`, where
        // the wire copies a child port) only bumps its epoch DURING the
        // settle above — the entry refresh at the top of this call ran too
        // early to see it, and if no later settle comes the override went
        // stale. The forced targets are override-registered, so this write
        // cannot re-trigger the very entries that fed it (no oscillation);
        // a target with comb readers marks them dirty and the caller's
        // next settle picks that up as usual.
    }

    /// `release` of a net: its continuous drivers must re-evaluate. Outside a
    /// settle the source signals of every entry writing the net are marked
    /// dirty right away; inside one (entry table taken out, dirty marks
    /// dropped with the pass) the id is queued for `settle_combinatorial`.
    pub(super) fn redrive_released_signal(&mut self, id: usize) {
        self.packed_array_copy_values
            .retain(|&(dst, _), _| dst != id);
        if self.settling || self.comb_entries.is_empty() {
            self.pending_release_ids.push(id);
            self.dirty_any = true;
            return;
        }
        self.mark_release_sources_dirty(id);
    }

    pub(super) fn mark_release_sources_dirty(&mut self, id: usize) {
        let mut sources: Vec<usize> = Vec::new();
        for entry in self.comb_entries.iter() {
            if entry.cold.write_signal_ids.contains(&id) {
                sources.extend(entry.cold.read_signal_ids.iter().copied());
            }
        }
        for src_id in sources {
            if src_id < self.dirty_signals.len() && !self.dirty_signals[src_id] {
                self.dirty_signals[src_id] = true;
                self.dirty_list.push(src_id);
                self.dirty_any = true;
            }
        }
    }

    /// Level-BSP combinational settle (XEZIM_BSP_SETTLE=1). Reaches the SAME
    /// fixpoint as `settle_combinatorial_inner`, but drains triggered entries
    /// in a level-ordered wavefront: entries sharing a static level are
    /// mutually independent (no intra-level read-after-write), which is the
    /// structure a parallel intra-level eval (Stage 2) plugs into. Serial here.
    /// Isolated entries (copy/bytecode/gate) eval via `eval_comb_entry_isolated`
    /// against the signal table (taken into `view`); the rare AST entries via
    /// `eval_ast_comb_entry` (restores the table around the call). Per-write
    /// bookkeeping mirrors `write_sig!`. Falls back to canonical settle if
    /// levels are missing.
    /// Per-write bookkeeping the level-BSP drain must mirror from `write_sig!`
    /// beyond the has_xz / inline-bit mirrors it already keeps: the dirty-edge
    /// position (without it a comb-driven clock never fires its edge blocks
    /// under the dirty-edge scan), the VCD dirty mark and the force epochs.
    /// Value-trace and DPI value-change callbacks are not replayed here.
    #[inline(always)]
    pub(super) fn bsp_after_write(&mut self, id: usize) {
        self.note_edge_write(id);
        vcd_mark!(self, id);
        if !self.active_force_exprs.is_empty() && id < self.active_force_signal_epochs.len() {
            self.active_force_signal_epochs[id] =
                self.active_force_signal_epochs[id].wrapping_add(1);
            self.force_epoch_gen = self.force_epoch_gen.wrapping_add(1);
        }
    }

    pub(super) fn settle_combinatorial_bsp(&mut self) {
        if self.settling || !self.dirty_any {
            return;
        }
        let num_entries = self.comb_entries.len();
        if self.comb_level.len() != num_entries || num_entries == 0 {
            self.settle_combinatorial_inner();
            return;
        }
        self.settling = true;
        self.settle_calls += 1;
        if self.settle_triggered.len() < num_entries {
            self.settle_triggered.resize(num_entries, false);
        }
        let limit = self.settle_limit as u64;

        // --- seed (identical to settle_combinatorial_inner) ---
        for &eidx in &self.settle_triggered_list {
            self.settle_triggered[eidx] = false;
        }
        self.settle_triggered_list.clear();
        let skip_deferred_at_t0 = self.time == 0 && !self.comb_time0_fired;
        let seed_dirty_len = self.dirty_list.len();
        for seed_idx in 0..seed_dirty_len {
            let id = self.dirty_list[seed_idx];
            if self.dirty_signals[id] {
                self.dirty_signals[id] = false;
                if id + 1 < self.comb_dep_offsets.len() {
                    let lo = self.comb_dep_offsets[id] as usize;
                    let hi = self.comb_dep_offsets[id + 1] as usize;
                    for k in lo..hi {
                        let eidx = self.comb_dep_entries[k] as usize;
                        if skip_deferred_at_t0 && self.comb_entries[eidx].defer_at_time0 {
                            continue;
                        }
                        if !self.settle_triggered[eidx] {
                            self.settle_triggered[eidx] = true;
                            self.settle_triggered_list.push(eidx);
                        }
                    }
                }
            }
        }
        self.dirty_list.clear();
        self.dirty_any = false;
        for ui in 0..self.comb_unresolved_idx.len() {
            let eidx = self.comb_unresolved_idx[ui];
            if eidx < num_entries && !self.settle_triggered[eidx] {
                self.settle_triggered[eidx] = true;
                self.settle_triggered_list.push(eidx);
            }
        }
        // See `settle_combinatorial_inner`: the one-shot must not retire on a
        // static-init settle that runs before the comb graph is built.
        if self.time == 0 && !self.comb_time0_fired && !self.comb_entries.is_empty() {
            for ti in 0..self.comb_time0_idx.len() {
                let eidx = self.comb_time0_idx[ti];
                if eidx < num_entries && !self.settle_triggered[eidx] {
                    self.settle_triggered[eidx] = true;
                    self.settle_triggered_list.push(eidx);
                }
            }
            self.comb_time0_fired = true;
        }
        if self.settle_triggered_list.is_empty() {
            self.settling = false;
            return;
        }

        // --- wavefront drain ---
        let num_levels = self.comb_level.iter().copied().max().unwrap_or(0) as usize + 1;
        // Reuse persistent buckets (retain capacity across calls); grow once.
        let mut buckets = std::mem::take(&mut self.bsp_buckets);
        if buckets.len() < num_levels {
            buckets.resize_with(num_levels, Vec::new);
        }
        for b in buckets.iter_mut() {
            b.clear();
        }
        let mut seed_list = std::mem::take(&mut self.settle_triggered_list);
        let mut next_seed = std::mem::take(&mut self.bsp_next_seed);
        next_seed.clear();
        let mut view = std::mem::take(&mut self.signal_table);
        let mut vm_regs = std::mem::take(&mut self.vm_regs);
        let mut dirtied: Vec<u32> = Vec::new();
        let mut par_scratch: Vec<usize> = Vec::new();
        let mut sweeps = 0u64;

        loop {
            sweeps += 1;
            if sweeps > limit {
                break;
            }
            for &e in &seed_list {
                if self.settle_triggered[e] {
                    buckets[self.comb_level[e] as usize].push(e);
                }
            }
            seed_list.clear();
            for l in 0..num_levels {
                // Width diagnostic: count entries actually evaluated at this
                // level this sweep (steady-state only) to gauge intra-level
                // parallelism. Cheap pre-scan of still-triggered entries.
                if self.bsp_widths_on && self.time > 0 && !buckets[l].is_empty() {
                    let mut w = 0usize;
                    for &e in &buckets[l] {
                        if self.settle_triggered[e] {
                            w += 1;
                        }
                    }
                    if w > 0 {
                        let b = if w == 1 {
                            0
                        } else if w < 16 {
                            1
                        } else if w < 64 {
                            2
                        } else if w < 256 {
                            3
                        } else if w < 1024 {
                            4
                        } else {
                            5
                        };
                        self.bsp_width_buckets[b] += w as u64;
                    }
                }
                // Parallel intra-level batch (Stage 2): when enabled and the
                // level is wide, evaluate its par_safe entries across worker
                // threads. Within a level every entry reads only lower,
                // already-finalized levels, and par_safe guarantees single-writer
                // outputs, so concurrent eval against the shared `view` is
                // race-free (disjoint writes). Non-par entries fall through to
                // the serial drain below.
                if self.bsp_par && buckets[l].len() >= self.bsp_par_threshold {
                    par_scratch.clear();
                    for &eidx in &buckets[l] {
                        if self.settle_triggered[eidx] && self.comb_par_safe[eidx] {
                            self.settle_triggered[eidx] = false;
                            par_scratch.push(eidx);
                        }
                    }
                    if par_scratch.len() >= self.bsp_par_threshold {
                        self.entry_evals += par_scratch.len() as u64;
                        dirtied.clear();
                        let nthreads = std::thread::available_parallelism()
                            .map(|x| x.get().min(8))
                            .unwrap_or(2)
                            .min(par_scratch.len())
                            .max(1);
                        let chunk = par_scratch.len().div_ceil(nthreads);
                        let view_ptr = view.as_mut_ptr() as usize;
                        let view_len = view.len();
                        let widths: &[u32] = &self.signal_widths;
                        let signed: &[bool] = &self.signal_signed;
                        let two_state: &[bool] = &self.signal_two_state;
                        let gate_driven: &[bool] = &self.signal_gate_driven;
                        let n2id = &self.signal_name_to_id;
                        let afi = &self.array_first_id;
                        // comb_entries' AST variants hold Cell/OnceCell (not
                        // Sync), so the slice can't cross threads by ref. Pass a
                        // raw ptr: SAFE because threads only READ par_safe items
                        // (Compiled/copy/gate — no Cell), and comb_entries is not
                        // mutated during the scope.
                        let entries_ptr = self.comb_entries.as_ptr() as usize;
                        let entries_len = self.comb_entries.len();
                        let scratch: &[usize] = &par_scratch;
                        let results: Vec<Vec<u32>> = std::thread::scope(|s| {
                            let mut handles = Vec::new();
                            let mut start = 0usize;
                            while start < scratch.len() {
                                let end = (start + chunk).min(scratch.len());
                                let part = &scratch[start..end];
                                start = end;
                                handles.push(s.spawn(move || {
                                    // SAFETY: par_safe → single-writer, so the
                                    // dst signals written by this thread's
                                    // entries are disjoint from every other
                                    // thread's. Reads hit lower, finalized
                                    // levels. `view` outlives the scope.
                                    let v = unsafe {
                                        std::slice::from_raw_parts_mut(
                                            view_ptr as *mut Value,
                                            view_len,
                                        )
                                    };
                                    let entries = unsafe {
                                        std::slice::from_raw_parts(
                                            entries_ptr as *const CombEntry,
                                            entries_len,
                                        )
                                    };
                                    let mut vm: Vec<Value> = Vec::new();
                                    let mut d: Vec<u32> = Vec::new();
                                    for &eidx in part {
                                        Self::eval_comb_item_isolated(
                                            &entries[eidx].item,
                                            widths,
                                            signed,
                                            two_state,
                                            gate_driven,
                                            n2id,
                                            afi,
                                            v,
                                            &mut vm,
                                            &mut d,
                                        );
                                    }
                                    d
                                }));
                            }
                            handles.into_iter().map(|h| h.join().unwrap()).collect()
                        });
                        for d in &results {
                            dirtied.extend_from_slice(d);
                        }
                        // Bookkeeping for changed signals (isolated writes only
                        // touched `view`).
                        for &id_u in dirtied.iter() {
                            let id = id_u as usize;
                            let (vv, xx) = view[id].raw_bits();
                            if id < self.signal_has_xz.len() {
                                self.signal_has_xz[id] = if xx != 0 { 1 } else { 0 };
                            }
                            if id < self.signal_inline_bits.len() {
                                self.signal_inline_bits[id] = [vv, xx];
                            }
                            if self.event_measure
                                && !self.armed_edge
                                && id < self.sig_last_change.len()
                            {
                                self.sig_last_change[id] = self.event_phase;
                            }
                            self.note_armed_write(id);
                            self.bsp_after_write(id);
                        }
                        self.table_modified = true;
                        // Propagate to dependents (serial, after the barrier).
                        for &id_u in dirtied.iter() {
                            let id = id_u as usize;
                            if id + 1 < self.comb_dep_offsets.len() {
                                let lo = self.comb_dep_offsets[id] as usize;
                                let hi = self.comb_dep_offsets[id + 1] as usize;
                                for k in lo..hi {
                                    let dep = self.comb_dep_entries[k] as usize;
                                    if !self.settle_triggered[dep] {
                                        self.settle_triggered[dep] = true;
                                        let dl = self.comb_level[dep] as usize;
                                        if dl > l {
                                            buckets[dl].push(dep);
                                        } else {
                                            next_seed.push(dep);
                                        }
                                    }
                                }
                            }
                        }
                    } else {
                        // Too few par entries after filtering — re-arm so the
                        // serial drain below handles them.
                        for &eidx in &par_scratch {
                            self.settle_triggered[eidx] = true;
                        }
                    }
                }
                let mut bi = 0;
                while bi < buckets[l].len() {
                    let eidx = buckets[l][bi];
                    bi += 1;
                    if !self.settle_triggered[eidx] {
                        continue;
                    }
                    self.settle_triggered[eidx] = false;
                    self.entry_evals += 1;
                    dirtied.clear();
                    let is_ast = matches!(
                        self.comb_entries[eidx].item,
                        CombItem::ContAssign { .. } | CombItem::AlwaysBlock { .. }
                    );
                    // Isolated eval for non-AST entries; if it reports the block
                    // uses an insn it can't handle, discard the partial result
                    // and fall through to the full &mut self path below.
                    // NBA-bearing blocks MUST use the full path so their
                    // deferred NBAs are scheduled (isolated eval drops them).
                    let mut needs_full = is_ast || self.comb_has_nba[eidx];
                    if !needs_full {
                        let supported = self.eval_comb_entry_isolated(
                            eidx,
                            &mut view,
                            &mut vm_regs,
                            &mut dirtied,
                        );
                        if supported {
                            // Replicate write_sig! bookkeeping for changed
                            // signals (eval_comb_entry_isolated only mutates view).
                            for &id_u in dirtied.iter() {
                                let id = id_u as usize;
                                let (v, x) = view[id].raw_bits();
                                if id < self.signal_has_xz.len() {
                                    self.signal_has_xz[id] = if x != 0 { 1 } else { 0 };
                                }
                                if id < self.signal_inline_bits.len() {
                                    self.signal_inline_bits[id] = [v, x];
                                }
                                if self.event_measure
                                    && !self.armed_edge
                                    && id < self.sig_last_change.len()
                                {
                                    self.sig_last_change[id] = self.event_phase;
                                }
                                self.note_armed_write(id);
                                self.bsp_after_write(id);
                            }
                            self.table_modified = true;
                        } else {
                            dirtied.clear();
                            needs_full = true;
                        }
                    }
                    if needs_full {
                        // Full path needs the table + vm_regs back on self.
                        self.signal_table = std::mem::take(&mut view);
                        self.vm_regs = std::mem::take(&mut vm_regs);
                        // SAFETY: eval_comb_entry_full never mutates
                        // self.comb_entries — the canonical settle runs the same
                        // eval with comb_entries taken OUT (empty), proving the
                        // eval path neither reads nor writes it. A raw shared
                        // ref avoids cloning the whole CombEntry (compiled
                        // bytecode / AST stmt) on every NBA/AST entry, which was
                        // the dominant `process`-time cost on c910.
                        let entry: &CombEntry =
                            unsafe { &*(&self.comb_entries[eidx] as *const CombEntry) };
                        self.eval_comb_entry_full(entry);
                        view = std::mem::take(&mut self.signal_table);
                        vm_regs = std::mem::take(&mut self.vm_regs);
                        for i in 0..self.dirty_list.len() {
                            let id = self.dirty_list[i];
                            if self.dirty_signals[id] {
                                self.dirty_signals[id] = false;
                                dirtied.push(id as u32);
                            }
                        }
                        self.dirty_list.clear();
                        self.dirty_any = false;
                    }
                    // Propagate changed signals to their comb dependents.
                    for &id_u in dirtied.iter() {
                        let id = id_u as usize;
                        if id + 1 < self.comb_dep_offsets.len() {
                            let lo = self.comb_dep_offsets[id] as usize;
                            let hi = self.comb_dep_offsets[id + 1] as usize;
                            for k in lo..hi {
                                let dep = self.comb_dep_entries[k] as usize;
                                if !self.settle_triggered[dep] {
                                    self.settle_triggered[dep] = true;
                                    let dl = self.comb_level[dep] as usize;
                                    if dl > l {
                                        buckets[dl].push(dep);
                                    } else {
                                        next_seed.push(dep);
                                    }
                                }
                            }
                        }
                    }
                }
                buckets[l].clear();
            }
            if next_seed.is_empty() {
                break;
            }
            std::mem::swap(&mut seed_list, &mut next_seed);
            next_seed.clear();
        }

        self.signal_table = view;
        self.vm_regs = vm_regs;
        // Restore reusable buffers (retain capacity for the next call).
        seed_list.clear();
        self.settle_triggered_list = seed_list;
        next_seed.clear();
        self.bsp_next_seed = next_seed;
        self.bsp_buckets = buckets;
        self.dirty_list.clear();
        self.dirty_any = false;
        self.settling = false;
    }

    /// BSP-settle shadow comparator (XEZIM_BSP_SHADOW=1). Per settle: snapshot
    /// pre-state, run canonical (truth), restore pre-state, run BSP, diff, and
    /// abort at the first divergent signal with its name. The sim evolves on the
    /// CANONICAL result (always completes).
    pub(super) fn settle_combinatorial_bsp_shadow(&mut self) {
        if self.settling || !self.dirty_any {
            return;
        }
        if self.time == 0 {
            // time-0 init settle is enormous; run BSP only, no compare.
            self.settle_combinatorial_bsp();
            return;
        }
        if self.dirty_list.len() > 256 {
            self.settle_combinatorial_inner();
            return;
        }
        let pre_dirty_list = self.dirty_list.clone();
        let pre_dirty_any = self.dirty_any;
        let pre_time0 = self.comb_time0_fired;
        let pre_table = self.signal_table.clone();

        self.settle_combinatorial_inner(); // canonical = truth

        let canon = std::mem::replace(&mut self.signal_table, pre_table);
        self.dirty_any = pre_dirty_any;
        for &id in &pre_dirty_list {
            self.dirty_signals[id] = true;
        }
        self.dirty_list = pre_dirty_list;
        self.comb_time0_fired = pre_time0;
        self.settle_combinatorial_bsp();

        let n = canon.len().min(self.signal_table.len());
        let mut diffs = 0u64;
        let mut shown = 0;
        for sid in 0..n {
            let (cv, cx) = canon[sid].raw_bits();
            let (pv, px) = self.signal_table[sid].raw_bits();
            if cv == pv && cx == px {
                continue;
            }
            diffs += 1;
            if shown < 16 {
                shown += 1;
                eprintln!(
                    "[BSP-SHADOW] time={} diff sid={} '{}' canon=(v{:#x},x{:#x}) bsp=(v{:#x},x{:#x})",
                    self.time,
                    sid,
                    self.name_for_id(sid),
                    cv,
                    cx,
                    pv,
                    px,
                );
            }
        }
        self.signal_table = canon; // sim continues on canonical
        if diffs > 0 {
            eprintln!(
                "[BSP-SHADOW] time={} FIRST divergence: {} signals differ — aborting",
                self.time, diffs
            );
            self.abort_run();
            return;
        }
        self.bsp_shadow_checks += 1;
        if self.bsp_shadow_checks % 5000 == 0 {
            eprintln!(
                "[BSP-SHADOW] checked {} time>0 settles, no divergence (now time={})",
                self.bsp_shadow_checks, self.time
            );
        }
    }

    /// AST comb-entry eval (`CombItem::ContAssign` / `AlwaysBlock`). These need
    /// the full `&mut self` interpreter (`eval_expr_ctx` / `exec_statement`), so
    /// they can't go through `eval_comb_entry_isolated`. Factored out of the
    /// `settle_combinatorial_inner` match so the canonical settle and a future
    /// level-BSP settle driver share one implementation. `entry` borrows the
    /// caller's (taken) comb-entry list, not `self`. Behavior is identical to
    /// the former inline arms (signal_table writes + dirty bookkeeping).
    pub(super) fn eval_ast_comb_entry(&mut self, entry: &CombEntry) {
        match &entry.item {
            CombItem::ContAssign {
                lhs,
                rhs,
                delay: explicit_delay,
            } => {
                if let Some(c) = entry.cold.cov {
                    self.cov_count(c);
                }
                let scope_hint = entry.cold.scope_hint.clone();
                let t_entry = std::time::Instant::now();
                let saved_hint = self.name_resolve_hint.borrow().clone();
                if let Some(hint) = &scope_hint {
                    *self.name_resolve_hint.borrow_mut() = Some(hint.clone());
                }
                let mut lhs_id = self.get_lhs_signal_id(lhs);
                if scope_hint.is_none() {
                    // The bare-lhs resolve above can itself need the hint
                    // (an instance-scoped CA whose lhs is a bare name) — the
                    // entry's pre-resolved write id breaks that cycle. Without
                    // it, a submodule pattern CA (`assign r = '{hi: p.lo,…}`)
                    // read its operands UNSCOPED and produced x.
                    let hint_id = lhs_id.or_else(|| entry.cold.write_signal_ids.first().copied());
                    if let Some(id) = hint_id {
                        if let Some(full) = self.name_opt(id) {
                            if let Some((parent, _)) = full.rsplit_once('.') {
                                *self.name_resolve_hint.borrow_mut() = Some(parent.to_string());
                            }
                        }
                    }
                    if lhs_id.is_none() {
                        lhs_id = self.get_lhs_signal_id(lhs);
                    }
                }
                // A HIERARCHICAL lhs writes into a foreign scope, but the RHS
                // is lexical to the WRITING scope. The lhs resolution above
                // ratchets name_resolve_hint to the TARGET's parent, under
                // which a bare RHS name binds to a same-named net inside the
                // target — a TB `assign dut.core.rst_in = grst_l` read the
                // DUT's own dead `grst_l` and forwarded x forever (its clock
                // sibling worked only because that name had no deep twin).
                // Restore the entry's own hint before evaluating the RHS.
                // Bare-lhs entries keep the write-derived hint set above —
                // their operands ARE target-scoped (submodule pattern CAs).
                fn lhs_is_hier(e: &Expression) -> bool {
                    match &e.kind {
                        ExprKind::MemberAccess { expr, .. }
                        | ExprKind::Index { expr, .. }
                        | ExprKind::RangeSelect { expr, .. } => lhs_is_hier(expr),
                        ExprKind::Ident(h) => {
                            h.path.len() > 1
                                || h.path.first().is_some_and(|s| s.name.name.contains('.'))
                        }
                        _ => false,
                    }
                }
                if lhs_is_hier(lhs) {
                    *self.name_resolve_hint.borrow_mut() = scope_hint.clone();
                }
                let w = self.infer_lhs_width(lhs);
                // §7.4.2/§10.9: `assign x = '{...}` (or `assign x[i] = '{...}`
                // on an array of packed vectors) converts each item to the
                // ELEMENT type. The generic eval concatenates items at their
                // own widths, which scrambles any item that is not exactly
                // element-sized.
                //
                let val = if let ExprKind::AssignmentPattern(items) = &rhs.kind {
                    self.packed_pattern_for_lhs(lhs, items)
                        .or_else(|| self.eval_packed_struct_pattern(lhs, items))
                        .unwrap_or_else(|| self.eval_expr_ctx(rhs, w))
                        .resize(w)
                } else {
                    self.eval_expr_ctx(rhs, w).resize(w)
                };
                let delay = if *explicit_delay > 0 {
                    *explicit_delay
                } else {
                    lhs_id
                        .and_then(|id| self.sdf_delays.get(id).copied())
                        .unwrap_or(0)
                };
                // A packed-array read is polled on every settle because its
                // cell has no dependency epoch. An implicit array-port
                // connection still drives only when its source changes;
                // otherwise polling erases a deposited net value (§38.34).
                let copy_ids = if delay == 0 && matches!(rhs.kind, ExprKind::Index { .. }) {
                    let cell = |e: &Expression| {
                        let (base, idx) = Self::const_elem_ref(e, &self.module)?;
                        self.elem_cell(&base, &idx)
                    };
                    cell(lhs)
                        .zip(cell(rhs))
                        .filter(|&(_, src)| is_packed_id(src))
                } else {
                    None
                };
                if let Some(ids) = copy_ids {
                    if self.packed_array_copy_values.get(&ids) == Some(&val) {
                        *self.name_resolve_hint.borrow_mut() = saved_hint;
                        self.prof_settle_ca_ns += t_entry.elapsed().as_nanos() as u64;
                        self.prof_settle_ca_count += 1;
                        return;
                    }
                    self.packed_array_copy_values.insert(ids, val.clone());
                }
                if delay > 0 {
                    if let Some(id) = lhs_id {
                        if self.signal_table[id] != val {
                            if !self.delayed_update_pending_is(id, &val) {
                                self.schedule_delayed_with_delay(id, val, delay);
                            }
                        } else {
                            self.cancel_delayed(id);
                        }
                    } else if let Some((base_id, lo, sel_w)) = self.delayed_lhs_select_range(lhs) {
                        // Select-LHS delayed assign — same fix as the other
                        // ContAssign eval site (see delayed_lhs_select_range):
                        // this used to DROP the write silently. Composes
                        // against the PENDING delayed value (issue #143).
                        let sel_val = val.resize(sel_w);
                        self.schedule_delayed_slice(base_id, lo, sel_w, &sel_val, delay);
                    }
                } else if let Some(id) = lhs_id {
                    let width = self.signal_widths[id];
                    let mut resized = if self.signal_real[id] {
                        if val.is_real {
                            val.clone()
                        } else {
                            Value::from_f64(val.to_f64())
                        }
                    } else if val.is_real {
                        Self::real_to_int(val.to_f64(), width)
                    } else {
                        val.resize(width)
                    };
                    resized.is_signed = self.signal_signed[id];
                    if self.signal_table[id] != resized {
                        self.mark_dirty_id(id);
                        write_sig!(self, id, resized);
                        self.table_modified = true;
                    }
                } else {
                    self.assign_value(lhs, &val);
                }
                *self.name_resolve_hint.borrow_mut() = saved_hint;
                self.prof_settle_ca_ns += t_entry.elapsed().as_nanos() as u64;
                self.prof_settle_ca_count += 1;
            }
            CombItem::AlwaysBlock { stmt, .. } => {
                let scope_hint = entry.cold.scope_hint.clone();
                let t_entry = std::time::Instant::now();
                let saved_hint = self.name_resolve_hint.borrow().clone();
                if let Some(hint) = &scope_hint {
                    *self.name_resolve_hint.borrow_mut() = Some(hint.clone());
                }
                self.settle_prev_values.clear();
                for &id in &entry.cold.write_signal_ids {
                    self.settle_prev_values
                        .push((id, self.signal_table[id].clone()));
                }
                // $time/%t in the body scales to the BLOCK's module timescale.
                let saved_ts = self.timescale_scope_override.take();
                self.timescale_scope_override = scope_hint.clone();
                self.exec_statement(stmt);
                self.timescale_scope_override = saved_ts;
                *self.name_resolve_hint.borrow_mut() = saved_hint;
                for i in 0..self.settle_prev_values.len() {
                    let (id, ref old_val) = self.settle_prev_values[i];
                    if self.signal_table[id] != *old_val {
                        if !self.dirty_signals[id] {
                            self.dirty_signals[id] = true;
                            self.dirty_list.push(id);
                        }
                        self.dirty_any = true;
                    }
                }
                self.prof_settle_ab_ns += t_entry.elapsed().as_nanos() as u64;
                self.prof_settle_ab_count += 1;
            }
            _ => {}
        }
    }

    /// Full `&mut self` eval of one comb entry against `signal_table` (used by
    /// the BSP driver as the fallback for entries the isolated evaluator can't
    /// handle — exotic compiled insns — and for AST entries). Mirrors the
    /// canonical `settle_combinatorial_inner` per-entry arms; marks dirty via
    /// the normal write path. `entry` borrows the caller's clone, not `self`.
    pub(super) fn eval_comb_entry_full(&mut self, entry: &CombEntry) {
        self.set_m_block_scope(entry.cold.scope_hint.as_deref());
        match &entry.item {
            CombItem::ContAssign { .. } | CombItem::AlwaysBlock { .. } => {
                self.eval_ast_comb_entry(entry);
            }
            CombItem::CompiledContAssign { compiled, .. }
            | CombItem::CompiledAlwaysBlock { compiled, .. } => {
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
                self.exec_insns(insns);
            }
            CombItem::FusedGate { op } => {
                self.exec_fused_gate(op);
            }
            CombItem::CtlGate { slot } => {
                if let Some(dst) = self.ctl_gate_exec_self(*slot) {
                    self.mark_dirty_id(dst.sig_id as usize);
                }
            }
            CombItem::VectorGate { op } => {
                self.exec_vector_gate(op);
            }
            CombItem::ScatterGate { op } => {
                self.exec_scatter_gate(op);
            }
            CombItem::GateRegion { gates } => {
                for op in gates.iter() {
                    self.exec_fused_gate(op);
                }
            }
            CombItem::FusedBufFanout { src, dsts, invert } => {
                self.exec_fused_buf_fanout(*src, dsts, *invert);
            }
            CombItem::FusedAndFanout {
                common,
                branches,
                invert,
            } => {
                self.exec_fused_and_fanout(*common, branches, *invert);
            }
            CombItem::Udp { idx } => {
                let idx = *idx;
                self.eval_udp(idx);
            }
            CombItem::UdpBatch { event_ref, indices } => {
                self.eval_udp_batch(*event_ref, indices);
            }
            CombItem::TimingCheck { idx } => {
                let idx = *idx;
                self.eval_timing_watch(idx);
            }
            // Copies are always handled by the isolated path; reaching here for
            // them would be a logic error, but eval them correctly regardless.
            CombItem::FastDirectCopy { dst_id, src_id } => {
                // §10.7: fit to the destination net type (2-state X/Z drop,
                // real<->int, signedness) — same as DirectCopy.
                let v = self.fit_value_to_signal(*dst_id, &self.signal_table[*src_id].clone());
                if self.signal_table[*dst_id] != v {
                    self.mark_dirty_id(*dst_id);
                    write_sig!(self, *dst_id, v);
                }
            }
            CombItem::FastDirectFanout { src_id, dst_ids } => {
                let src = self.signal_table[*src_id].clone();
                for &dst_id in dst_ids.iter() {
                    let v = self.fit_value_to_signal(dst_id, &src);
                    if self.signal_table[dst_id] != v {
                        self.mark_dirty_id(dst_id);
                        write_sig!(self, dst_id, v);
                    }
                }
            }
            CombItem::DirectCopy { dst_id, src_id, .. } => {
                let v = self.fit_value_to_signal(*dst_id, &self.signal_table[*src_id].clone());
                if self.signal_table[*dst_id] != v {
                    self.mark_dirty_id(*dst_id);
                    write_sig!(self, *dst_id, v);
                }
            }
            CombItem::Noop => {}
        }
    }

    /// Classify the fused single-bit gates whose destination has no comb
    /// reader (its only consumers are edge blocks / processes): those never
    /// feed the worklist, so evaluation order inside a pass is irrelevant
    /// and they can run in a batch. Rebuilt whenever the dependency tables
    /// change. Clock-tree entries stay on the generic path.
    pub(super) fn build_quiet_bits(&mut self) {
        let n = self.signal_table.len();
        self.quiet_bits.clear();
        self.quiet_bits.resize(n.div_ceil(64), 0);
        for id in 0..n {
            let armed = self.armed_input_set(id);
            let edge = self.sig_to_edge_pos.get(id).is_some_and(|&p| p >= 0);
            if !armed && !edge {
                self.quiet_bits[id >> 6] |= 1u64 << (id & 63);
            }
        }
        self.quiet_valid = true;
    }

    /// Operand graph census (`XEZIM_COMB_GRAPH=<path>`): one line per comb
    /// entry, `E <eidx> R <read ids> W <write ids>`, then one line per edge
    /// block, `B <bidx> W <write ids>` (`A<first>` for a dense array
    /// target), so a placement study can tell which operands are comb
    /// outputs, flop outputs or primary inputs, and how far apart an entry's
    /// operands sit in id space. Runs after the entries exist on BOTH paths
    /// (built, or restored from the prepared-comb cache).
    pub(super) fn dump_comb_graph(&self) {
        use super::bytecode::Insn as EI;
        use std::io::Write;
        let Ok(path) = std::env::var("XEZIM_COMB_GRAPH") else {
            return;
        };
        let Ok(mut f) = std::fs::File::create(&path) else {
            return;
        };
        for (i, e) in self.comb_entries.iter().enumerate() {
            let _ = write!(f, "E {} R", i);
            for r in &e.cold.read_signal_ids {
                let _ = write!(f, " {}", r);
            }
            let _ = write!(f, " W");
            for w in &e.cold.write_signal_ids {
                let _ = write!(f, " {}", w);
            }
            let _ = writeln!(f);
        }
        let mut n_blocks = 0usize;
        for (b, cb) in self.compiled_edge_blocks.iter().enumerate() {
            let Some(cb) = cb else { continue };
            n_blocks += 1;
            let _ = write!(f, "B {} W", b);
            for ins in cb.instructions.iter() {
                match ins {
                    EI::NbaAssign(sig, ..)
                    | EI::NbaAssignConst(sig, ..)
                    | EI::NbaAssignRange(sig, ..)
                    | EI::NbaAssignRangeDyn(sig, ..)
                    | EI::NbaAssignBitDyn(sig, ..)
                    | EI::NbaAssignArrayRead(sig, ..) => {
                        let _ = write!(f, " {}", sig);
                    }
                    EI::NbaAssignArray(arr, ..) | EI::NbaAssignArrayRange(arr, ..) => {
                        if let super::bytecode::ArrayOperand::Dense { first_id, .. } = arr.as_ref()
                        {
                            let _ = write!(f, " A{}", first_id);
                        }
                    }
                    _ => {}
                }
            }
            let _ = writeln!(f);
        }
        eprintln!(
            "[COMB-GRAPH] wrote {} entries, {} edge blocks to {}",
            self.comb_entries.len(),
            n_blocks,
            path
        );
    }

    /// Comb-graph signal placement: see the call site in `new`. Pure name
    /// reordering; `names` is sorted and deduplicated on entry and the
    /// result is a permutation of it.
    pub(super) fn place_signal_names(module: &ElaboratedModule, names: Vec<String>) -> Vec<String> {
        use crate::ast::expr::{ExprKind, Expression};
        use crate::ast::stmt::{Statement, StatementKind};
        fn expr_names(e: &Expression, module: &ElaboratedModule, out: &mut Vec<String>) {
            match &e.kind {
                ExprKind::Ident(h) => out.push(Simulator::resolve_hier_name_static(h, module)),
                ExprKind::Unary { operand, .. } => expr_names(operand, module, out),
                ExprKind::Binary { left, right, .. } => {
                    expr_names(left, module, out);
                    expr_names(right, module, out);
                }
                ExprKind::Conditional {
                    condition,
                    then_expr,
                    else_expr,
                } => {
                    expr_names(condition, module, out);
                    expr_names(then_expr, module, out);
                    expr_names(else_expr, module, out);
                }
                ExprKind::Concatenation(v) => v.iter().for_each(|x| expr_names(x, module, out)),
                ExprKind::Replication { count, exprs } => {
                    expr_names(count, module, out);
                    exprs.iter().for_each(|x| expr_names(x, module, out));
                }
                ExprKind::Call { func, args } => {
                    expr_names(func, module, out);
                    args.iter().for_each(|x| expr_names(x, module, out));
                }
                ExprKind::SystemCall { args, .. } => {
                    args.iter().for_each(|x| expr_names(x, module, out))
                }
                ExprKind::Inside { expr, ranges } => {
                    expr_names(expr, module, out);
                    ranges.iter().for_each(|x| expr_names(x, module, out));
                }
                ExprKind::MemberAccess { expr, .. } => expr_names(expr, module, out),
                ExprKind::Index { expr, index } => {
                    expr_names(expr, module, out);
                    expr_names(index, module, out);
                }
                ExprKind::RangeSelect {
                    expr, left, right, ..
                } => {
                    expr_names(expr, module, out);
                    expr_names(left, module, out);
                    expr_names(right, module, out);
                }
                ExprKind::Range(a, b) => {
                    expr_names(a, module, out);
                    expr_names(b, module, out);
                }
                ExprKind::Paren(x) => expr_names(x, module, out),
                _ => {}
            }
        }
        fn stmt_names(
            st: &Statement,
            module: &ElaboratedModule,
            reads: &mut Vec<String>,
            writes: &mut Vec<String>,
        ) {
            match &st.kind {
                StatementKind::BlockingAssign { lvalue, rvalue }
                | StatementKind::NonblockingAssign { lvalue, rvalue, .. } => {
                    expr_names(rvalue, module, reads);
                    // Index/select operands of the target are reads too.
                    let mut lhs = Vec::new();
                    expr_names(lvalue, module, &mut lhs);
                    if let Some(first) = lhs.first() {
                        writes.push(first.clone());
                    }
                    reads.extend(lhs.into_iter().skip(1));
                }
                StatementKind::If {
                    condition,
                    then_stmt,
                    else_stmt,
                    ..
                } => {
                    expr_names(condition, module, reads);
                    stmt_names(then_stmt, module, reads, writes);
                    if let Some(e) = else_stmt {
                        stmt_names(e, module, reads, writes);
                    }
                }
                StatementKind::Case { expr, items, .. } => {
                    expr_names(expr, module, reads);
                    for it in items {
                        it.patterns
                            .iter()
                            .for_each(|x| expr_names(x, module, reads));
                        stmt_names(&it.stmt, module, reads, writes);
                    }
                }
                StatementKind::For {
                    condition,
                    step,
                    body,
                    ..
                } => {
                    if let Some(c) = condition {
                        expr_names(c, module, reads);
                    }
                    step.iter().for_each(|x| expr_names(x, module, reads));
                    stmt_names(body, module, reads, writes);
                }
                StatementKind::Foreach { array, body, .. } => {
                    expr_names(array, module, reads);
                    stmt_names(body, module, reads, writes);
                }
                StatementKind::While { condition, body }
                | StatementKind::DoWhile { body, condition } => {
                    expr_names(condition, module, reads);
                    stmt_names(body, module, reads, writes);
                }
                StatementKind::Repeat { count, body } => {
                    expr_names(count, module, reads);
                    stmt_names(body, module, reads, writes);
                }
                StatementKind::Forever { body } | StatementKind::ForeverTail { body } => {
                    stmt_names(body, module, reads, writes)
                }
                StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
                    stmts
                        .iter()
                        .for_each(|x| stmt_names(x, module, reads, writes))
                }
                StatementKind::TimingControl { stmt, .. } => {
                    stmt_names(stmt, module, reads, writes)
                }
                _ => {}
            }
        }
        let mut pos: HashMap<&str, usize> = HashMap::default();
        for (i, n) in names.iter().enumerate() {
            pos.insert(n.as_str(), i);
        }
        let n = names.len();
        // Undirected adjacency between the names one assign/block relates:
        // every operand is linked to every target of that statement. A
        // breadth-first walk over this graph, starting from the lowest
        // unplaced name and expanding a degree-sorted frontier (Cuthill–
        // McKee), keeps each entry's operands and outputs within a few ids
        // of each other while leaving declaration-adjacent bus bits, which
        // share writers, together. The earlier source-order greedy walk
        // scattered those bits and measured negative.
        let mut adj: Vec<Vec<u32>> = vec![Vec::new(); n];
        let mut reads: Vec<String> = Vec::new();
        let mut writes: Vec<String> = Vec::new();
        let link = |reads: &[String], writes: &[String], adj: &mut Vec<Vec<u32>>| {
            let rs: Vec<usize> = reads
                .iter()
                .filter_map(|r| pos.get(r.as_str()).copied())
                .collect();
            let ws: Vec<usize> = writes
                .iter()
                .filter_map(|w| pos.get(w.as_str()).copied())
                .collect();
            for &w in &ws {
                for &r in &rs {
                    if r != w {
                        adj[w].push(r as u32);
                        adj[r].push(w as u32);
                    }
                }
            }
        };
        for ca in &module.continuous_assigns {
            reads.clear();
            writes.clear();
            expr_names(&ca.rhs, module, &mut reads);
            let mut lhs = Vec::new();
            expr_names(&ca.lhs, module, &mut lhs);
            if let Some(first) = lhs.first() {
                writes.push(first.clone());
            }
            reads.extend(lhs.into_iter().skip(1));
            link(&reads, &writes, &mut adj);
        }
        for ab in &module.always_blocks {
            reads.clear();
            writes.clear();
            stmt_names(&ab.stmt, module, &mut reads, &mut writes);
            link(&reads, &writes, &mut adj);
        }
        for pa in &module.pending_always {
            reads.clear();
            writes.clear();
            stmt_names(&pa.source, module, &mut reads, &mut writes);
            link(&reads, &writes, &mut adj);
        }
        for a in adj.iter_mut() {
            a.sort_unstable();
            a.dedup();
        }
        if std::env::var("XEZIM_PLACE_DBG").is_ok() {
            let linked = adj.iter().filter(|a| !a.is_empty()).count();
            let edges: usize = adj.iter().map(|a| a.len()).sum::<usize>() / 2;
            eprintln!(
                "[PLACE] names={} linked={} edges={} cont_assigns={} always={} pending={}",
                n,
                linked,
                edges,
                module.continuous_assigns.len(),
                module.always_blocks.len(),
                module.pending_always.len()
            );
        }
        let mut placed = vec![false; n];
        let mut order: Vec<usize> = Vec::with_capacity(n);
        let mut queue: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
        let mut frontier: Vec<usize> = Vec::new();
        for seed in 0..n {
            if placed[seed] {
                continue;
            }
            placed[seed] = true;
            queue.push_back(seed);
            while let Some(v) = queue.pop_front() {
                order.push(v);
                frontier.clear();
                for &u in &adj[v] {
                    let u = u as usize;
                    if !placed[u] {
                        placed[u] = true;
                        frontier.push(u);
                    }
                }
                // Low-degree neighbours first (Cuthill–McKee), ties by name.
                frontier.sort_by_key(|&u| (adj[u].len(), u));
                queue.extend(frontier.iter().copied());
            }
        }
        let mut names = names;
        let mut out: Vec<String> = Vec::with_capacity(n);
        for i in order {
            out.push(std::mem::take(&mut names[i]));
        }
        out
    }

    pub(super) fn build_gate_lane(&mut self) {
        let n = self.comb_entries.len();
        self.gate_lane.clear();
        self.gate_lane.resize(n, u32::MAX);
        self.gate_ops.clear();
        self.gate_obs.clear();
        for eidx in 0..n {
            let CombItem::FusedGate { op } = &self.comb_entries[eidx].item else {
                continue;
            };
            if self.is_clock_tree_entry.get(eidx).copied().unwrap_or(false) {
                continue;
            }
            let dst = match op {
                FusedGate::Buf1 { dst, .. }
                | FusedGate::Bin2 { dst, .. }
                | FusedGate::Mux2 { dst, .. }
                | FusedGate::UdpLut3 { dst, .. } => *dst,
            };
            let id = dst.sig_id as usize;
            let readers = if id + 1 < self.comb_dep_offsets.len() {
                self.comb_dep_offsets[id + 1] - self.comb_dep_offsets[id]
            } else {
                0
            };
            if readers != 0 {
                continue;
            }
            self.gate_lane[eidx] = self.gate_ops.len() as u32;
            self.gate_ops.push(*op);
            let armed = self.armed_input_set(id);
            let edge = self.sig_to_edge_pos.get(id).is_some_and(|&p| p >= 0);
            let wide = self.signal_widths.get(id).copied().unwrap_or(65) > 64;
            self.gate_obs
                .push((armed as u8) | ((edge as u8) << 1) | ((wide as u8) << 2));
        }
        self.gate_queued.clear();
        self.gate_queued.resize(self.gate_ops.len(), false);
        self.gate_queue.clear();
        self.gate_lane_valid = true;
    }

    pub(super) fn settle_combinatorial_inner(&mut self) {
        if !self.ts_sleep.is_empty() {
            self.ts_wake_sleepers();
        }

        if self.settling {
            return;
        }
        // #35: deferred cont-assign entries make a full settle necessary even
        // when nothing is freshly dirty — the deferral consumed the source
        // dirt when it recorded the entries.
        if !self.dirty_any && (self.proc_settle_defer || self.deferred_proc_entries.is_empty()) {
            return;
        }
        self.settling = true;
        self.settle_calls += 1;

        // Edges first: picking the controlling-value gates changes which
        // entries the gate lane may take.
        if self.comb_dep_edges.len() != self.comb_dep_entries.len()
            || !self.bit_sens_pending.is_empty()
        {
            self.bit_sens_refresh();
        }
        if !self.ctl_rearm_pending.is_empty() {
            for s in std::mem::take(&mut self.ctl_rearm_pending) {
                self.ctl_rearm(s);
            }
        }
        if !self.gate_lane_valid || self.gate_lane.len() != self.comb_entries.len() {
            self.build_gate_lane();
        }
        if !self.quiet_valid || self.quiet_bits.len() != self.signal_table.len().div_ceil(64) {
            self.build_quiet_bits();
        }
        #[cfg(feature = "jit")]
        if let Some(m) = self.ts_jit.as_mut() {
            // Batch finalization: a whole group of streams lands in one
            // contiguous code segment (per-function finalize cost a page
            // per function). The threshold shrinks once lowering settles
            // so late stragglers still get installed.
            let thr = if self.settle_calls < 50_000 { 64 } else { 1 };
            if m.pending_len() >= thr {
                for (eidx, f) in m.flush() {
                    if self.ts_jit_fns.len() <= eidx {
                        self.ts_jit_fns.resize(eidx + 1, None);
                    }
                    self.ts_jit_fns[eidx] = Some(f);
                }
            }
        }
        let entries = std::mem::take(&mut self.comb_entries);
        self.settle_entries_view = (entries.as_ptr(), entries.len());
        let dep_offsets = std::mem::take(&mut self.comb_dep_offsets);
        let dep_entries = std::mem::take(&mut self.comb_dep_entries);
        let mut dep_edges = std::mem::take(&mut self.comb_dep_edges);
        debug_assert_eq!(dep_edges.len(), dep_entries.len());
        let tree_sig = std::mem::take(&mut self.is_clock_tree_signal);
        let tree_entry = std::mem::take(&mut self.is_clock_tree_entry);
        let num_entries = entries.len();

        // Resize persistent buffers if needed (only happens once)
        if self.settle_triggered.len() < num_entries {
            self.settle_triggered.resize(num_entries, false);
        }
        // The trigger bitmap and the next-pass worklist move into locals for
        // the duration of the settle, exactly like `entries`/`dep_*` above.
        // Both are indexed by every propagation step, and as `self` fields
        // LLVM has to reload their base pointer after each `&mut self` call in
        // the evaluation arms. Re-entry can't observe the empty fields: both
        // settle drivers bail on `self.settling`, which is already set, and
        // nothing else touches them. Restored on every exit path below.
        let mut triggered = std::mem::take(&mut self.settle_triggered);
        let mut next_list = std::mem::take(&mut self.settle_triggered_list);
        // The lane bypasses the per-entry hooks (activity monitor, trace,
        // per-entry profile, path census), so it is off while any is armed.
        let lane_on = !self.activity_mon
            && self.trace_always.is_none()
            && !self.profile_report
            && !self.trace_comb_paths
            && self.proc_depth == 0;
        let gate_lane = std::mem::take(&mut self.gate_lane);
        let mut gate_queue = std::mem::take(&mut self.gate_queue);
        let mut gate_queued = std::mem::take(&mut self.gate_queued);

        let mut total_iters = 0u64;
        let limit = self.settle_limit as u64;

        // One-time seed: consume the initial dirty set, mark dependents.
        // settle_triggered acts as persistent "needs evaluation" across passes.
        for &eidx in &next_list {
            triggered[eidx] = false;
        }
        next_list.clear();
        // #35: a FULL settle first re-dirties the source signals of the
        // entries a deferring (post-proc-write) settle skipped, so they
        // re-trigger through the ordinary dependency walk below.
        if !self.proc_settle_defer && !self.deferred_proc_entries.is_empty() {
            let deferred = std::mem::take(&mut self.deferred_proc_entries);
            for sid in deferred {
                let sid = sid as usize;
                if sid < self.dirty_signals.len() && !self.dirty_signals[sid] {
                    self.dirty_signals[sid] = true;
                    self.dirty_list.push(sid);
                }
            }
            self.dirty_any = true;
        }
        // First time-0 settle pass: don't fire `always @(list)` blocks that
        // can reach `$finish`/`$stop` — they should suspend at their event
        // control until a sensitivity input actually transitions, not run
        // with uninitialised inputs (see CombEntry::defer_at_time0).
        let skip_deferred_at_t0 = self.time == 0 && !self.comb_time0_fired;
        let seed_dirty_len = self.dirty_list.len();
        // `$ctl`: whether a seeded signal's edges may carry a `CtlGate`
        // mask (then a zero mask skips the dependent). Two instances, so a
        // design without controlled gates runs the original loop.
        macro_rules! seed_dirty {
            ($ctl:expr) => {
                for seed_idx in 0..seed_dirty_len {
                    let id = self.dirty_list[seed_idx];
                    if self.dirty_signals[id] {
                        self.dirty_signals[id] = false;
                        if id + 1 < dep_offsets.len() {
                            let lo = dep_offsets[id] as usize;
                            let hi = dep_offsets[id + 1] as usize;
                            let tree_clk = tree_sig.get(id).copied().unwrap_or(false);
                            let masked: bool = $ctl(id);
                            for (j, &eidx_u32) in dep_entries[lo..hi].iter().enumerate() {
                                let eidx = eidx_u32 as usize;
                                if skip_deferred_at_t0 && entries[eidx].defer_at_time0 {
                                    continue;
                                }
                                // A controlled `CtlGate` masked this edge
                                // (static masks are never zero on such a
                                // signal).
                                if masked && dep_edges[lo + j].mask == 0 {
                                    continue;
                                }
                                // the eager clock-tree pass already evaluated these
                                if tree_clk && tree_entry.get(eidx).copied().unwrap_or(false) {
                                    continue;
                                }
                                if !triggered[eidx] {
                                    triggered[eidx] = true;
                                    next_list.push(eidx);
                                }
                            }
                        }
                    }
                }
            };
        }
        if self.ctl_sig.is_empty() {
            seed_dirty!(|_id: usize| false);
        } else {
            let ctl_sig = std::mem::take(&mut self.ctl_sig);
            seed_dirty!(|id: usize| ctl_sig.get(id).copied().unwrap_or(false));
            self.ctl_sig = ctl_sig;
        }
        self.dirty_list.clear();
        self.dirty_any = false;

        // Unresolved entries always re-eval. Iterate the precomputed
        // index list instead of scanning `0..num_entries` (can be ~500K
        // and is called on every BlockingAssign). A t0-deferred entry (a
        // write-free `always @(list)` observer, unresolved because it reads
        // $time) is held back on the first t0 settle — same rule the
        // dirty-seed loop above applies (§9.2.2.1).
        for &eidx in self.comb_unresolved_idx.iter() {
            if eidx < num_entries && !triggered[eidx] {
                if skip_deferred_at_t0 && entries[eidx].defer_at_time0 {
                    continue;
                }
                triggered[eidx] = true;
                next_list.push(eidx);
            }
        }
        // Time-0 / empty-read-set fire unconditionally — but only the FIRST
        // settle call at time=0 needs to seed them. Subsequent settle calls
        // at the same (or later) time rely on dirty propagation through the
        // worklist. The prior O(num_entries) scan per call was ~500μs on
        // c910 and dominated per-assign cost during memory-init loops.
        //
        // `num_entries > 0` keeps the one-shot from being CONSUMED before the
        // comb graph exists. Class static initializers run during elaboration
        // (the `registry spec static init` phase), long before
        // `build combinational entries`; a blocking write in one of them
        // reaches `settle_after_proc_write`, whose settle finds an empty
        // entry list. Latching there seeded nothing yet retired the flag, so
        // the real time-0 settle skipped `comb_time0_idx` entirely and every
        // entry with an EMPTY READ SET — a constant-RHS continuous assign
        // such as `assign a[i][v][j] = '0;` — never evaluated at all, leaving
        // its net x for the whole run. UVM's parameterized-class registry
        // makes that early static-init settle happen in any UVM testbench.
        if self.time == 0 && !self.comb_time0_fired && num_entries > 0 {
            for &eidx in self.comb_time0_idx.iter() {
                if eidx < num_entries && !triggered[eidx] {
                    triggered[eidx] = true;
                    next_list.push(eidx);
                }
            }
            self.comb_time0_fired = true;
        }

        if next_list.is_empty() {
            self.settle_entries_view = (std::ptr::null(), 0);
            self.comb_entries = entries;
            self.comb_dep_offsets = dep_offsets;
            self.comb_dep_entries = dep_entries;
            self.comb_dep_edges = dep_edges;
            self.is_clock_tree_signal = tree_sig;
            self.is_clock_tree_entry = tree_entry;
            self.settle_triggered = triggered;
            self.settle_triggered_list = next_list;
            self.gate_lane = gate_lane;
            if self.profile_report {
                self.prof_cur.store(
                    crate::compiler::prof_sampler::KIND_OTHER,
                    std::sync::atomic::Ordering::Relaxed,
                );
            }
            self.gate_queue = gate_queue;
            self.gate_queued = gate_queued;
            self.settling = false;
            return;
        }

        // Chaotic-iteration loop: drain a sorted worklist of triggered
        // entries each pass. New triggers added during a pass (via worklist
        // propagation below) land in `settle_triggered_list` for the next
        // pass. Sorting per pass keeps entries processed in topo order
        // (small k since only triggered entries appear), which is dramatically
        // faster than a 0..num_entries linear scan when num_entries is large
        // (e.g. ~467K on c910) but the triggered set is small.
        let mut cur_list = std::mem::take(&mut self.settle_dirty_ids);
        cur_list.clear();
        // Settle-limit attribution: when the loop is about to exhaust its
        // budget, record which signals are still being written — and by
        // which entry — so the warning below can NAME the non-converging
        // ring instead of just saying "signals may not have converged".
        // Costs one compare per pass until the final CHURN_TAIL passes.
        const CHURN_TAIL: u64 = 3;
        let mut churn: Vec<(usize, usize)> = Vec::new(); // (signal_id, entry_idx)
        let mut converged = false;

        // Diagnostic modes are latched for the whole settle: every one of
        // these is a construction-time / CLI flag, and `self.time` cannot
        // advance while `settling` is set — so re-reading them from `self` on
        // each of the ~679 entry evaluations per settle call is pure overhead.
        let prof_on = self.profile_timing;
        // Profile bookkeeping sized once per settle, not per evaluation.
        if prof_on && self.prof_entry_counts.len() != num_entries {
            self.prof_entry_counts.resize(num_entries, 0);
        }
        if self.profile_report && self.prof_sampler.is_none() {
            let s = crate::compiler::prof_sampler::ProfSampler::start();
            self.prof_cur = s.cur.clone();
            self.prof_sampler = Some(s);
        }
        // Settle-worklist prefetch distance (entries ahead of the cursor).
        // Latched once per settle: it is a process-wide env knob.
        let prefetch_dist: usize = {
            static D: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
            *D.get_or_init(|| {
                std::env::var("XEZIM_PREFETCH_DIST")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(8)
            })
        };
        static PREFETCH_MODE: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
        let prefetch_mode = *PREFETCH_MODE.get_or_init(|| {
            std::env::var("XEZIM_PREFETCH_MODE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0)
        });
        static INJECT_PREFETCH: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let inject_prefetch = *INJECT_PREFETCH
            .get_or_init(|| std::env::var("XEZIM_INJECT_PREFETCH").ok().as_deref() != Some("0"));
        let mon_on = self.activity_mon;
        let trace_on = self.trace_always.is_some();
        // Any per-evaluation diagnostic hook armed. All are construction-time
        // or CLI modes, fixed for the settle, so a normal run tests one bool
        // per evaluation instead of six.
        let diag_on = self.trace_comb_paths
            || self.coact_enabled
            || prof_on
            || mon_on
            || trace_on
            || self.profile_report;
        let warn_x_on = self.warn_x && self.time > 0;
        // Profiling counters accumulate in registers and fold back into `self`
        // once, instead of a read-modify-write per evaluation.
        // Dependents injected into the running pass, as a bitset plus the
        // index of the smallest pending one. A binary heap here cost a
        // push/pop per dependency edge (~780M per CoreMark run); the bitset
        // is one OR per injection and a forward word scan per take.
        let mut inject_bits: Vec<u64> = std::mem::take(&mut self.settle_inject_bits);
        let words = num_entries.div_ceil(64).max(1);
        if inject_bits.len() < words {
            inject_bits.resize(words, 0);
        }
        let mut next_inject: usize = usize::MAX;
        // Summary level: bit `w` set iff `inject_bits[w] != 0`, so the
        // forward scan for the next injected entry skips empty words in
        // one step instead of walking the whole (30k-entry / 470-word on
        // c906) bitset each time — that walk was 6% of the settle loop.
        let mut inject_sum: Vec<u64> = std::mem::take(&mut self.settle_inject_sum);
        let sum_words = words.div_ceil(64).max(1);
        if inject_sum.len() < sum_words {
            inject_sum.resize(sum_words, 0);
        }
        fn next_set_bit(bits: &[u64], sum: &[u64], from: usize) -> usize {
            let w = from >> 6;
            if w >= bits.len() {
                return usize::MAX;
            }
            let word = bits[w] & (u64::MAX << (from & 63));
            if word != 0 {
                return (w << 6) + word.trailing_zeros() as usize;
            }
            // Next non-empty word after `w`, via the summary.
            let mut sw = (w + 1) >> 6;
            if sw >= sum.len() {
                return usize::MAX;
            }
            let mut sword = sum[sw] & (u64::MAX << ((w + 1) & 63));
            loop {
                if sword != 0 {
                    let nw = (sw << 6) + sword.trailing_zeros() as usize;
                    if nw >= bits.len() {
                        return usize::MAX;
                    }
                    let word = bits[nw];
                    debug_assert!(word != 0);
                    return (nw << 6) + word.trailing_zeros() as usize;
                }
                sw += 1;
                if sw >= sum.len() {
                    return usize::MAX;
                }
                sword = sum[sw];
            }
        }
        let mut n_evals = 0u64;
        let mut n_dc = 0u64;
        let mut n_ab = 0u64;
        let mut n_repass = 0u64;
        // Settle census (folded back once, like the counters above).
        let mut n_writes = 0u64;
        let mut n_dep_edges = 0u64;

        // Trigger every comb entry that depends on signal `$id`, written by
        // entry `$eidx`. This is exactly what `mark_dirty_id($id)` plus the
        // loop's post-entry `dirty_list` rescan used to do, minus the round
        // trip: the push onto `dirty_list`, the `dirty_signals` set/clear
        // pair, the `dirty_any` store and a second `comb_dep_offsets` bounds
        // test. `[XZP] dirty_prop` measured 119.1M such round trips per c906
        // run, 93% of them from the fused arms below. Expanded inline (rather
        // than a method) so it can borrow the settle-local `dep_*`/worklist
        // vectors while the arms still hold `&mut self`.
        macro_rules! trigger_deps {
            ($id:expr, $eidx:expr, $chg:expr) => {{
                let __tid: usize = $id;
                let __chg: u64 = $chg;
                n_writes += 1;
                if __tid + 1 < dep_offsets.len() {
                    // SAFETY: `__tid + 1 < dep_offsets.len()` was just tested;
                    // offsets are a CSR prefix sum into `dep_entries` and every
                    // entry index is below `num_entries` (`triggered.len()`),
                    // built together in `build_comb_entries` and validated
                    // when loaded from the prepared-comb cache.
                    let __lo = unsafe { *dep_offsets.get_unchecked(__tid) } as usize;
                    let __hi = unsafe { *dep_offsets.get_unchecked(__tid + 1) } as usize;
                    debug_assert!(__lo <= __hi && __hi <= dep_entries.len());
                    n_dep_edges += (__hi - __lo) as u64;
                    let __tree_clk = tree_sig.get(__tid).copied().unwrap_or(false);
                    // SAFETY: `dep_edges` parallels `dep_entries`
                    // (`bit_sens_refresh` sizes it before every settle).
                    for __e in unsafe { dep_edges.get_unchecked(__lo..__hi) } {
                        // Bit-granular sensitivity: this dependent reads
                        // none of the bits that changed.
                        if __e.mask & __chg == 0 {
                            continue;
                        }
                        let __dep = __e.entry as usize;
                        if __tree_clk && tree_entry.get(__dep).copied().unwrap_or(false) {
                            continue;
                        }
                        if lane_on {
                            let __g = unsafe { *gate_lane.get_unchecked(__dep) };
                            if __g != u32::MAX {
                                let __g = __g as usize;
                                if !gate_queued[__g] {
                                    gate_queued[__g] = true;
                                    gate_queue.push(__g as u32);
                                }
                                continue;
                            }
                        }
                        debug_assert!(__dep < triggered.len());
                        if !unsafe { *triggered.get_unchecked(__dep) } {
                            unsafe { *triggered.get_unchecked_mut(__dep) = true };
                            if __dep > $eidx {
                                // Keep the pass in entry (topological)
                                // order: an injected dependent runs BEFORE
                                // the entries that read it, so they are not
                                // re-triggered into a second pass. Appending
                                // it to the pass list ran it last and cost
                                // 15% more entry evaluations on c906.
                                inject_bits[__dep >> 6] |= 1u64 << (__dep & 63);
                                inject_sum[__dep >> 12] |= 1u64 << ((__dep >> 6) & 63);
                                if __dep < next_inject {
                                    next_inject = __dep;
                                }
                            } else {
                                next_list.push(__dep);
                            }
                        }
                    }
                }
            }};
        }
        // `mark_dirty_id`'s activity-monitor half, for the write paths that
        // now propagate directly instead of going through the dirty list.
        macro_rules! note_toggle {
            ($id:expr) => {{
                if mon_on {
                    if self.signal_toggle_counts.len() != self.signal_table.len() {
                        self.signal_toggle_counts.resize(self.signal_table.len(), 0);
                    }
                    self.signal_toggle_counts[$id] += 1;
                }
            }};
        }

        for _iteration in 0..limit {
            total_iters += 1;
            let mut evaluated_any = false;
            let capture_churn = _iteration + CHURN_TAIL >= limit;

            // Take the current worklist and sort by eidx (topo order).
            std::mem::swap(&mut cur_list, &mut next_list);
            if cur_list.len() > 1 && !cur_list.windows(2).all(|w| w[0] <= w[1]) {
                cur_list.sort_unstable();
            }

            // Hot loop: >90% of iterations hit DC / CompiledContAssign /
            // CompiledAlwaysBlock — avoid scope_hint.clone() and Instant::now()
            // on those paths. Only the AST fallback arms pay that cost.
            //
            // NOTE: A previous version added a single-stage `_mm_prefetch`
            // on entries[cur_list[cur_pos+8]] which gave c906 cmark a
            // 31% wall-time win. Reverted because c910 t=1 k=0 starts
            // hanging at iters=200040 with the prefetch active despite
            // the prefetch being a semantic no-op — likely a downstream
            // cache-state interaction we haven't traced yet. Don't
            // re-enable without a c910 t=1 k=0 + t=4 k=4 cmark sanity.
            let mut cur_pos = 0usize;
            while cur_pos < cur_list.len() || next_inject != usize::MAX {
                let eidx = if next_inject != usize::MAX
                    && (cur_pos >= cur_list.len() || next_inject < cur_list[cur_pos])
                {
                    let e = next_inject;
                    inject_bits[e >> 6] &= !(1u64 << (e & 63));
                    if inject_bits[e >> 6] == 0 {
                        inject_sum[e >> 12] &= !(1u64 << ((e >> 6) & 63));
                    }
                    next_inject = next_set_bit(&inject_bits, &inject_sum, e + 1);
                    // One-ahead prefetch of the NEXT injected entry's packed
                    // two-state header. The distance-8 prefetch below walks
                    // `cur_list` only; entries injected during the pass (the
                    // majority on the C906/C910 fabrics) are popped from the
                    // bitset and their header load — the loop's dominant
                    // stall — was never prefetched. The next index is known
                    // here, and this entry's evaluation (~500 host
                    // instructions) covers an L3 round trip. Bounds-checked
                    // index; the prefetch itself is inert.
                    #[cfg(target_arch = "x86_64")]
                    if inject_prefetch && next_inject < self.ts_hdr.len() {
                        unsafe {
                            core::arch::x86_64::_mm_prefetch(
                                self.ts_hdr.as_ptr().add(next_inject) as *const i8,
                                core::arch::x86_64::_MM_HINT_T0,
                            );
                        }
                    }
                    e
                } else {
                    cur_pos += 1;
                    cur_list[cur_pos - 1]
                };
                // The entry header load is the loop's dominant stall (~19%
                // of settle self-time: 35k 64-byte entries visited in
                // data-dependent order). The worklist names the upcoming
                // entries, so prefetch AHEAD of the cursor while this one
                // evaluates. One-ahead only hides the latency of a
                // dependent L1/L2 hit; the entry array is ~2 MB on the C906
                // SoC, so the misses that matter are L3 (and the 64-byte
                // entry is one line). `XEZIM_PREFETCH_DIST` tunes the
                // distance; 8 measured best, 0 disables.
                //
                // An earlier revision of this prefetch was reverted after a
                // C910 run hung with it enabled "despite the prefetch being
                // a semantic no-op". A prefetch instruction is indeed inert
                // — but the ADDRESS computation was not: it indexed
                // `cur_list[cur_pos + D]` without checking the worklist
                // length, so past the end it read a garbage index and formed
                // `entries.as_ptr().add(garbage)`, which is out-of-bounds
                // pointer arithmetic (UB) that LLVM is free to miscompile
                // around. BOTH indices are bounds-checked here, which keeps
                // the address inside the allocation on every path.
                #[cfg(target_arch = "x86_64")]
                {
                    let d = cur_pos + prefetch_dist;
                    if prefetch_mode != 0 {
                        // Two-state entries never touch `entries[]` any more:
                        // their first loads are the packed header and the
                        // arena stream. Header at full distance; the stream
                        // of the entry whose header arrived half a distance
                        // ago.
                        if d < cur_list.len() {
                            let nxt = cur_list[d];
                            if nxt < self.ts_hdr.len() {
                                unsafe {
                                    core::arch::x86_64::_mm_prefetch(
                                        self.ts_hdr.as_ptr().add(nxt) as *const i8,
                                        core::arch::x86_64::_MM_HINT_T0,
                                    );
                                }
                            }
                        }
                        if prefetch_mode >= 2 {
                            let d2 = cur_pos + prefetch_dist / 2;
                            if d2 < cur_list.len() {
                                let nxt = cur_list[d2];
                                if let Some(h) = self.ts_hdr.get(nxt) {
                                    let off = h.off as usize;
                                    if h.kind() != 0 && off < self.ts_arena.len() {
                                        unsafe {
                                            core::arch::x86_64::_mm_prefetch(
                                                self.ts_arena.as_ptr().add(off) as *const i8,
                                                core::arch::x86_64::_MM_HINT_T0,
                                            );
                                        }
                                    }
                                }
                            }
                        }
                        // Mode 3: the signal VALUES the entry loads first —
                        // the random 32-byte `Value` fetch at the head of
                        // nearly every two-state block is the executor's
                        // dominant stall. The stream was prefetched half a
                        // distance ago; read its first two instructions and
                        // touch the values they will load.
                        if prefetch_mode >= 3 {
                            let d3 = cur_pos + prefetch_dist / 4;
                            if d3 < cur_list.len() {
                                let nxt = cur_list[d3];
                                if let Some(h) = self.ts_hdr.get(nxt) {
                                    let off = h.off as usize;
                                    if h.kind() != 0 {
                                        let n = h.len().min(3);
                                        for k in 0..n {
                                            let Some(insn) = self.ts_arena.get(off + k) else {
                                                break;
                                            };
                                            use super::bytecode::TsInsn as T;
                                            let sig = match insn {
                                                T::LoadSig { sig, .. }
                                                | T::SigRange { sig, .. }
                                                | T::SigBit { sig, .. }
                                                | T::LoadSigNot { sig, .. }
                                                | T::SigRangeEqC { sig, .. }
                                                | T::LoadSigLogAnd { sig, .. }
                                                | T::SigBitNot { sig, .. }
                                                | T::LoadSigLogOr { sig, .. }
                                                | T::LoadSigAnd { sig, .. }
                                                | T::LoadSigRepl { sig, .. }
                                                | T::LoadSigSigRange { sig, .. }
                                                | T::SigRangeAnd { sig, .. }
                                                | T::SigRangeEq { sig, .. }
                                                | T::LoadSigBrNz { sig, .. }
                                                | T::BrSigFalse { sig, .. }
                                                | T::LoadSig2 { sig1: sig, .. }
                                                | T::SigBit2 { sig1: sig, .. } => *sig as usize,
                                                _ => continue,
                                            };
                                            if sig < self.signal_table.len() {
                                                unsafe {
                                                    core::arch::x86_64::_mm_prefetch(
                                                        self.signal_table.as_ptr().add(sig)
                                                            as *const i8,
                                                        core::arch::x86_64::_MM_HINT_T0,
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    } else if d < cur_list.len() {
                        let nxt = cur_list[d];
                        if nxt < entries.len() {
                            unsafe {
                                core::arch::x86_64::_mm_prefetch(
                                    entries.as_ptr().add(nxt) as *const i8,
                                    core::arch::x86_64::_MM_HINT_T0,
                                );
                            }
                        }
                    } else if cur_pos < cur_list.len() {
                        // Tail of a short worklist: fall back to one-ahead so
                        // small settle passes keep the original behaviour.
                        let nxt = cur_list[cur_pos];
                        if nxt < entries.len() {
                            unsafe {
                                core::arch::x86_64::_mm_prefetch(
                                    entries.as_ptr().add(nxt) as *const i8,
                                    core::arch::x86_64::_MM_HINT_T0,
                                );
                            }
                        }
                    }
                }
                if !triggered[eidx] {
                    continue;
                }
                // Consume the trigger now. If any signal is written AGAIN
                // later this pass (or next), the entry is re-triggered by
                // the worklist propagation below.
                triggered[eidx] = false;
                // #35 §5.5/§10.3: while a PROCESS's post-write settle runs,
                // cont-assign re-evaluation is a SEPARATE active-region event
                // the process must not observe before yielding — defer these
                // entries to the next full settle. UDP/gate/always entries
                // (a synthesized netlist's cells) stay eager, which is the
                // reference-measured split (c1b lazy vs dep eager).
                if self.proc_settle_defer
                    && matches!(
                        entries[eidx].item,
                        CombItem::ContAssign { .. }
                            | CombItem::CompiledContAssign { .. }
                            | CombItem::DirectCopy { .. }
                            | CombItem::FastDirectCopy { .. }
                            | CombItem::FastDirectFanout { .. }
                            | CombItem::FusedBufFanout { .. }
                            | CombItem::FusedAndFanout { .. }
                    )
                {
                    self.deferred_proc_entries.extend(
                        entries[eidx]
                            .cold
                            .read_signal_ids
                            .iter()
                            .map(|&id| id as u32),
                    );
                    // An entry with no resolvable read (one reading only
                    // memory cells without element names) has no id to
                    // re-dirty: an out-of-range marker still makes the next
                    // full settle run, which seeds every unresolved entry.
                    if entries[eidx].has_unresolved_reads
                        && entries[eidx].cold.read_signal_ids.is_empty()
                    {
                        self.deferred_proc_entries.push(u32::MAX);
                    }
                    continue;
                }
                evaluated_any = true;
                let dirty_before = self.dirty_list.len();

                n_evals += 1;
                if _iteration > 0 {
                    n_repass += 1;
                }
                if diag_on {
                    if self.trace_comb_paths {
                        if eidx >= self.comb_eval_counts.len() {
                            self.comb_eval_counts.resize(eidx + 1, [0; 2]);
                            self.comb_eval_last_time.resize(eidx + 1, u64::MAX);
                        }
                        self.comb_eval_counts[eidx][0] += 1;
                        if self.comb_eval_last_time[eidx] == self.time {
                            self.comb_eval_counts[eidx][1] += 1;
                        }
                        self.comb_eval_last_time[eidx] = self.time;
                    }
                    // Co-activation stamp: count, per predecessor edge, how often
                    // this entry evaluates in the same TIME SLOT as that
                    // predecessor. Opt-in; ~4 loads per eval when enabled.
                    if self.coact_enabled {
                        let now = self.time;
                        let lo = self.coact_pred_off[eidx] as usize;
                        let hi = self.coact_pred_off[eidx + 1] as usize;
                        for k in lo..hi {
                            let p = self.coact_pred_ent[k] as usize;
                            if self.coact_epoch[p] == now {
                                self.coact_edge_count[k] += 1;
                            }
                        }
                        self.coact_epoch[eidx] = now;
                    }
                    // Which CombItem shapes actually dominate evaluation — the
                    // number that says whether to attack the interpreter or the
                    // worklist. Gated: `profile_timing` is already an opt-in slow
                    // mode (it adds Instant::now() on these paths), so a normal
                    // run does not pay for this match.
                    if prof_on {
                        // The per-shape histogram is derived from these counts
                        // and each entry's static kind at report time.
                        self.prof_entry_counts[eidx] += 1;
                    }
                    if mon_on {
                        if let Some(slot) = self.activity_counts.get_mut(eidx) {
                            *slot += 1;
                        }
                    }
                    if trace_on {
                        let label = self.comb_entry_trace_label(eidx, &entries[eidx]);
                        self.trace_always_fire(&label);
                    }
                    // Profile report: two clock reads per evaluation made the
                    // settle 35% slower on gate-level designs; the sampler
                    // thread times what this one store publishes.
                    if self.profile_report {
                        self.prof_cur.store(
                            crate::compiler::prof_sampler::KIND_COMB | eidx as u64,
                            std::sync::atomic::Ordering::Relaxed,
                        );
                    }
                }
                // Plan-first fast path: an entry with a resolved two-state
                // plan runs straight from the plan table — no `CombEntry`
                // load (a cache miss on this design's 30k-entry table), no
                // item match, no `dispatch_comb_plan` round trip. A guard
                // bail falls through to the ordinary path below, which
                // re-dispatches (and demotes) exactly as before.
                let mut ts_fast = false;
                #[cfg(feature = "jit")]
                let mut ts_native_bailed = false;
                #[cfg(not(feature = "jit"))]
                let ts_native_bailed = false;
                // A native x-read bail keeps its write records: the stores it
                // made before the bail are already in the table, and the
                // executor's re-run below finds them unchanged.
                #[cfg(feature = "jit")]
                let mut ts_keep_recs = false;
                #[cfg(not(feature = "jit"))]
                let ts_keep_recs = false;
                #[cfg(feature = "jit")]
                if self.proc_depth == 0 && self.ts_jit_on {
                    if let Some(Some(f)) = self.ts_jit_fns.get(eidx) {
                        let f = *f;
                        self.ts_exec_aborted = false;
                        if !self.warn_x
                            && (self.forced_signals.is_empty() || !self.ts_writes_forced_eidx(eidx))
                        {
                            let sp: *mut u8 = self as *mut Self as *mut u8;
                            let tp: *const u8 = self.signal_table.as_ptr() as *const u8;
                            self.ts_rec_n = 0;
                            self.ts_direct_writes = true;
                            let rc = unsafe { f(sp, tp) };
                            self.ts_direct_writes = false;
                            match rc {
                                0 => {
                                    ts_fast = true;
                                    n_dc += 1;
                                    self.prof_ts_evals += 1;
                                    if self.trace_comb_paths {
                                        self.note_comb_path(eidx, 0);
                                    }
                                }
                                1 => {
                                    // An x read: the executor below re-runs
                                    // the stream and hands it to the x-plane
                                    // executor, as for a stream with no native
                                    // code, instead of the four-state VM. The
                                    // native code has no `SaveSig`, so nothing
                                    // needs restoring first.
                                    ts_keep_recs = true;
                                }
                                _ => {
                                    self.prof_ts_bail_abort += 1;
                                    self.ts_exec_aborted = true;
                                    ts_native_bailed = true;
                                }
                            }
                        }
                    }
                }
                if !ts_fast && !ts_native_bailed && self.proc_depth == 0 {
                    let arena_kind = self.ts_hdr.get(eidx).map_or(0, |h| h.kind());
                    if arena_kind != 0 {
                        if !ts_keep_recs {
                            self.ts_rec_n = 0;
                        }
                        self.ts_direct_writes = true;
                        let ok = self.ts_guard_and_exec_arena(eidx);
                        self.ts_direct_writes = false;
                        if !ok && self.ts_last_xbail {
                            self.ts_xbail_note(0, eidx);
                        }
                        #[cfg(feature = "jit")]
                        if ok && self.ts_jit_on {
                            if self.ts_jit_evals.len() <= eidx {
                                self.ts_jit_evals.resize(eidx + 1, 0);
                            }
                            let c = self.ts_jit_evals[eidx].saturating_add(1);
                            self.ts_jit_evals[eidx] = c;
                            if c == self.ts_jit_hot {
                                if self.ts_jit.is_none() {
                                    self.ts_jit =
                                        super::ts_jit::enabled::TsJitModule::new().map(Box::new);
                                    if self.ts_jit.is_none() {
                                        self.ts_jit_on = false;
                                    }
                                }
                                if let (Some(TsSlot::Yes(ts)), Some(m)) =
                                    (self.ts_comb.get(eidx), self.ts_jit.as_mut())
                                {
                                    if !ts.has_ctrl && !ts.has_wide {
                                        m.compile(eidx, &ts.insns, ts.num_regs);
                                    }
                                }
                            }
                        }
                        if ok {
                            ts_fast = true;
                            n_dc += 1;
                            if self.trace_comb_paths {
                                self.note_comb_path(eidx, 0);
                            }
                        }
                    } else if let Some(CombPlan::Ts(ts)) = self.comb_plan.get(eidx) {
                        let tp: *const super::bytecode::TwoStateBlock = std::sync::Arc::as_ptr(ts);
                        if !ts_keep_recs {
                            self.ts_rec_n = 0;
                        }
                        self.ts_direct_writes = true;
                        self.ts_cur_eidx = eidx as u32;
                        let ok = self.ts_guard_and_exec(unsafe { &*tp });
                        self.ts_direct_writes = false;
                        if !ok && self.ts_last_xbail {
                            self.ts_xbail_note(0, eidx);
                        }
                        if ok {
                            ts_fast = true;
                            n_dc += 1;
                            if self.trace_comb_paths {
                                self.note_comb_path(eidx, 0);
                            }
                        }
                    }
                }
                if !ts_fast {
                    if !self.ts_sleep.is_empty() {
                        if let Some(c) = self.ts_asleep_comb.get_mut(eidx) {
                            if *c != 0 {
                                *c = c.saturating_add(1);
                            }
                        }
                    }
                    match &entries[eidx].item {
                        CombItem::Noop => {}
                        CombItem::FastDirectCopy { dst_id, src_id } => {
                            // LRM §9.3.1: skip if the destination is forced.
                            // The is_empty() short-circuit avoids hashing dst_id on
                            // every eval — this arm fires ~130M times on c910 and
                            // forced_signals is empty in the common (no-force) case.
                            if self.forced_signals.is_empty()
                                || !self.forced_signals.contains_key(dst_id)
                            {
                                let (mut sv, mut sx) = self.signal_table[*src_id].raw_bits();
                                // §6.11.1/§10.7: a 2-state destination drops X/Z
                                // (X/Z -> 0) on every write; the raw-bit copy would
                                // otherwise keep it (e.g. an X-at-time-0 4-state driver).
                                if sx != 0
                                    && self.signal_two_state.get(*dst_id).copied().unwrap_or(false)
                                {
                                    sv &= !sx;
                                    sx = 0;
                                }
                                let (dv, dx) = self.signal_table[*dst_id].raw_bits();
                                if warn_x_on {
                                    self.warn_x_note_bits(*dst_id, dv, dx, sv, sx);
                                }
                                if (sv != dv || sx != dx)
                                    && self.signal_table[*dst_id].set_inline_bits(sv, sx)
                                {
                                    self.table_modified = true;
                                    self.after_signal_write(*dst_id);
                                    if capture_churn {
                                        churn.push((*dst_id, eidx));
                                    }
                                    note_toggle!(*dst_id);
                                    trigger_deps!(*dst_id, eidx, (sv ^ dv) | (sx ^ dx));
                                }
                            }
                            n_dc += 1;
                        }
                        CombItem::FastDirectFanout { src_id, dst_ids } => {
                            let (src_v, src_x) = self.signal_table[*src_id].raw_bits();
                            // Hoist the empty-check: skip per-dst hashing entirely
                            // when nothing is forced (the common case).
                            let any_forced = !self.forced_signals.is_empty();
                            for &dst_id in dst_ids.iter() {
                                if any_forced && self.forced_signals.contains_key(&dst_id) {
                                    continue;
                                }
                                let (mut sv, mut sx) = (src_v, src_x);
                                if sx != 0
                                    && self.signal_two_state.get(dst_id).copied().unwrap_or(false)
                                {
                                    sv &= !sx;
                                    sx = 0;
                                }
                                let (dv, dx) = self.signal_table[dst_id].raw_bits();
                                if warn_x_on {
                                    self.warn_x_note_bits(dst_id, dv, dx, sv, sx);
                                }
                                if (sv == dv && sx == dx)
                                    || !self.signal_table[dst_id].set_inline_bits(sv, sx)
                                {
                                    continue;
                                }
                                self.sync_mirror(dst_id);
                                self.table_modified = true;
                                self.after_signal_write(dst_id);
                                if capture_churn {
                                    churn.push((dst_id, eidx));
                                }
                                note_toggle!(dst_id);
                                trigger_deps!(dst_id, eidx, (sv ^ dv) | (sx ^ dx));
                            }
                            n_dc += dst_ids.len() as u64;
                        }
                        CombItem::DirectCopy {
                            dst_id,
                            src_id,
                            width,
                        } => {
                            // Inline fast path. On gate-level netlists (e.g. yosys
                            // synth.v output) DirectCopy fires hundreds of millions
                            // of times — picorv32 test_synth measured 1.17B
                            // entry_evals dominated by DirectCopy. The slow path
                            // below clones a `Value`, runs PartialEq, and assigns —
                            // ~50ns/call. The fast path reads raw u64 bits via the
                            // inlined `raw_bits()` and mutates the destination in
                            // place via `set_inline_bits`, skipping the clone +
                            // enum-match comparison entirely. Falls back to the
                            // slow path for SDF-delayed targets, width mismatches
                            // (need a true resize), or Wide values.
                            let dst_w = self.signal_widths[*dst_id];
                            let mut handled = false;
                            // 2-state / real destinations take the slow (fit) path so
                            // X/Z is dropped and real<->int converts (§10.7); the
                            // raw-bit fast path would copy them verbatim.
                            if *width <= 64
                                && dst_w == *width
                                && self.signal_widths[*src_id] == *width
                                && !self.signal_real[*dst_id]
                                && !self.signal_two_state.get(*dst_id).copied().unwrap_or(false)
                                && (self.sdf_delays.is_empty()
                                    || self.sdf_delays.get(*dst_id).copied().unwrap_or(0) == 0)
                            {
                                let (sv, sx) = self.signal_table[*src_id].raw_bits();
                                let (dv, dx) = self.signal_table[*dst_id].raw_bits();
                                if warn_x_on {
                                    self.warn_x_note_bits(*dst_id, dv, dx, sv, sx);
                                }
                                if sv == dv && sx == dx {
                                    // Already equal; no work, no dirty mark.
                                    handled = true;
                                } else if self.signal_table[*dst_id].set_inline_bits(sv, sx) {
                                    self.table_modified = true;
                                    self.after_signal_write(*dst_id);
                                    if capture_churn {
                                        churn.push((*dst_id, eidx));
                                    }
                                    note_toggle!(*dst_id);
                                    trigger_deps!(*dst_id, eidx, (sv ^ dv) | (sx ^ dx));
                                    handled = true;
                                }
                                // If set_inline_bits returned false (Wide storage),
                                // fall through to the slow path.
                            }
                            if !handled {
                                let src_val = self.signal_table[*src_id].clone();
                                // §10.7 fit to the destination net type (2-state X/Z
                                // drop, real<->int, sign/X-extend, net signedness).
                                let resized = self.fit_value_to_signal(*dst_id, &src_val);
                                if self.signal_table[*dst_id] != resized {
                                    let delay = self.sdf_delays.get(*dst_id).copied().unwrap_or(0);
                                    if delay > 0 && self.time > 0 {
                                        self.schedule_delayed(*dst_id, resized);
                                    } else {
                                        write_sig!(self, *dst_id, resized);
                                        self.table_modified = true;
                                        if capture_churn {
                                            churn.push((*dst_id, eidx));
                                        }
                                        note_toggle!(*dst_id);
                                        trigger_deps!(*dst_id, eidx, u64::MAX);
                                    }
                                }
                            }
                            n_dc += 1;
                        }
                        CombItem::CompiledContAssign { compiled, .. } => {
                            // P5: X-free evaluations run on u64 words. NO
                            // `continue` — the worklist propagation after this
                            // match must still see the stores' dirtied signals.
                            if self.dispatch_comb_plan(eidx, compiled) {
                                n_dc += 1;
                            } else {
                                {
                                    if self.trace_comb_paths {
                                        self.note_comb_path(eidx, 2);
                                    }
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
                                    // A fallback in an `assign` (an expression the
                                    // compiler declined) re-enters the interpreter:
                                    // `%m`, bare names and `$time` must see THIS
                                    // entry's instance, as the always-block arm below
                                    // arranges. Pure-bytecode entries pay one bool.
                                    let saved = if compiled.has_fallback {
                                        let prev_hint = self.name_resolve_hint.borrow().clone();
                                        let prev_ts = self.timescale_scope_override.take();
                                        let sc = entries[eidx].cold.scope_hint.clone();
                                        self.set_m_block_scope(sc.as_deref());
                                        if sc.is_some() {
                                            *self.name_resolve_hint.borrow_mut() = sc.clone();
                                        }
                                        self.timescale_scope_override = sc;
                                        Some((prev_hint, prev_ts))
                                    } else {
                                        None
                                    };
                                    self.exec_insns(insns);
                                    if let Some((prev_hint, prev_ts)) = saved {
                                        *self.name_resolve_hint.borrow_mut() = prev_hint;
                                        self.timescale_scope_override = prev_ts;
                                    }
                                }
                                // Counted in a register and folded into
                                // `prof_settle_dc_count` after the loop, so the
                                // per-evaluation cost is not a read-modify-write
                                // through `self` (which also forces the entry tables'
                                // base pointers to be re-derived on every iteration).
                                n_dc += 1;
                            }
                        }
                        CombItem::ContAssign { .. } | CombItem::AlwaysBlock { .. } => {
                            // Factored AST eval (shared with the BSP settle driver).
                            self.set_m_block_scope(entries[eidx].cold.scope_hint.as_deref());
                            // `name_resolve_hint` is a ratchet every resolution
                            // advances; without a reset here this entry resolves
                            // bare names under whatever scope the PREVIOUS entry
                            // left behind — and the shared-node cache then pins
                            // the wrong binding forever. A TB's hierarchical
                            // `assign dut.deep.rst_in = grst_l` evaluated after a
                            // DUT-internal entry read the DUT's own dead `grst_l`
                            // (the clock sibling worked only because its source
                            // name had no deep twin). Same disease as the
                            // edge-block fix in exec_bytecode.
                            *self.name_resolve_hint.borrow_mut() =
                                entries[eidx].cold.scope_hint.clone();
                            self.eval_ast_comb_entry(&entries[eidx]);
                            if self.proc_depth > 0
                                && matches!(entries[eidx].item, CombItem::AlwaysBlock { .. })
                            {
                                self.note_comb_ran_in_process(eidx, &entries[eidx]);
                            }
                        }
                        CombItem::CompiledAlwaysBlock { compiled, .. } => {
                            // P5: X-free evaluations run on u64 words (a lowered
                            // block has no fallbacks, so none of the scope /
                            // timescale bookkeeping below applies to it).
                            if self.dispatch_comb_plan(eidx, compiled) {
                                n_ab += 1;
                                if self.proc_depth > 0 {
                                    self.note_comb_ran_in_process(eidx, &entries[eidx]);
                                }
                            } else {
                                {
                                    if self.trace_comb_paths {
                                        self.note_comb_path(eidx, 2);
                                        #[cfg(feature = "jit")]
                                        if std::env::var("XEZIM_JIT_BAIL_TRACE").is_ok() {
                                            use std::cell::RefCell;
                                            thread_local! {
                                                static SEEN: RefCell<std::collections::HashSet<usize>> =
                                                    RefCell::new(std::collections::HashSet::new());
                                            }
                                            SEEN.with(|m| {
                                    if m.borrow_mut().insert(eidx) {
                                        eprintln!(
                                            "[JITNORUN] eidx={} slot_none={:?} fns_len={} strikes={:?}",
                                            eidx,
                                            self.comb_jit_fns.get(eidx).map(|f| f.is_none()),
                                            self.comb_jit_fns.len(),
                                            self.comb_jit_strikes.get(eidx)
                                        );
                                    }
                                });
                                        }
                                    }
                                    // Bytecode path: BlockingAssign/NbaAssign insns mark
                                    // dirty automatically; no pre/post value snapshot needed.
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
                                    // $time/%t in a fallback stmt scales to the BLOCK's
                                    // module timescale (mirrors the edge-block dispatch).
                                    let saved_ts = self.timescale_scope_override.take();
                                    self.timescale_scope_override =
                                        entries[eidx].cold.scope_hint.clone();
                                    // …and `%m` in that fallback names the BLOCK's instance.
                                    self.set_m_block_scope(
                                        entries[eidx].cold.scope_hint.as_deref(),
                                    );
                                    // A StmtFallback insn re-enters the AST interpreter,
                                    // which resolves bare names through the hint — without
                                    // it a `foreach (m[i]) m[i] = v;` in a SUBMODULE
                                    // resolved the bare array name, found nothing, and
                                    // dropped every write. Pure-bytecode blocks pre-resolve
                                    // their ids, so this costs them one bool test.
                                    let saved_hint = if compiled.has_fallback {
                                        let prev = self.name_resolve_hint.borrow().clone();
                                        if let Some(sc) = entries[eidx].cold.scope_hint.as_ref() {
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
                                    n_ab += 1;
                                    if self.proc_depth > 0 {
                                        self.note_comb_ran_in_process(eidx, &entries[eidx]);
                                    }
                                }
                            }
                        }
                        // The three fused arms are the settle loop's highest-volume
                        // shapes on RTL netlists (c906: 57.8% / 5.8% / 0.02% of all
                        // entry evaluations). They are expanded here rather than
                        // calling `exec_fused_*` so a changed destination triggers
                        // its dependents through `trigger_deps!` directly, skipping
                        // the `mark_dirty_id` -> `dirty_list` -> post-entry rescan
                        // round trip. Same reads, same writes, same trigger set.
                        CombItem::GateRegion { gates } => {
                            // One worklist dispatch executes the whole coact
                            // cluster in topo member order via the SAME per-gate
                            // evaluator; internal links are dropped from the
                            // region's read set at formation, so their writes
                            // trigger nothing spurious.
                            let sdf_any = !self.sdf_delays.is_empty();
                            for op in gates.iter() {
                                let (dst, new_bit) = self.fused_gate_eval(op);
                                if self.fused_bit_commit(dst, new_bit, sdf_any) {
                                    let id = dst.sig_id as usize;
                                    if capture_churn {
                                        churn.push((id, eidx));
                                    }
                                    note_toggle!(id);
                                    trigger_deps!(id, eidx, self.bit_chg_mask(dst.sig_id, dst.bit));
                                }
                            }
                        }
                        CombItem::FusedGate { op } => {
                            let (dst, new_bit) = self.fused_gate_eval(op);
                            let sdf_any = !self.sdf_delays.is_empty();
                            if self.fused_bit_commit(dst, new_bit, sdf_any) {
                                let id = dst.sig_id as usize;
                                if capture_churn {
                                    churn.push((id, eidx));
                                }
                                note_toggle!(id);
                                trigger_deps!(id, eidx, self.bit_chg_mask(dst.sig_id, dst.bit));
                            }
                            n_dc += 1;
                        }
                        // Out of line; a changed bit is recorded like a
                        // two-state store and propagated by the shared
                        // post-entry walk below (keeps this arm small).
                        CombItem::CtlGate { slot } => {
                            if let Some(dst) = self.ctl_gate_exec(*slot, &mut dep_edges, true) {
                                note_toggle!(dst.sig_id as usize);
                            }
                            n_dc += 1;
                        }
                        CombItem::VectorGate { op } => {
                            let (dst, width, val, xz) = self.vector_gate_eval(op);
                            let sdf_any = !self.sdf_delays.is_empty();
                            if self.fused_vector_commit(dst, width, val, xz, sdf_any) {
                                let id = dst.sig_id as usize;
                                if capture_churn {
                                    churn.push((id, eidx));
                                }
                                note_toggle!(id);
                                trigger_deps!(
                                    id,
                                    eidx,
                                    self.span_chg_mask(dst.sig_id, dst.lo, width as u32)
                                );
                            }
                            n_dc += width as u64;
                        }
                        CombItem::ScatterGate { op } => {
                            let (val, xz) = self.scatter_gate_eval(op);
                            let dsts = match op {
                                ScatterGate::Buf { dsts, .. } | ScatterGate::Bin { dsts, .. } => {
                                    dsts
                                }
                            };
                            let sdf_any = !self.sdf_delays.is_empty();
                            if let Some((edge_active, armed_active)) =
                                self.fused_batch_commit_state()
                            {
                                for (lane, &id) in dsts.iter().enumerate() {
                                    let bit =
                                        (((val >> lane) & 1) | (((xz >> lane) & 1) << 1)) as u8;
                                    if self.fused_scalar_commit_planned(
                                        id,
                                        bit,
                                        sdf_any,
                                        edge_active,
                                        armed_active,
                                    ) {
                                        let id = id as usize;
                                        if capture_churn {
                                            churn.push((id, eidx));
                                        }
                                        note_toggle!(id);
                                        trigger_deps!(id, eidx, self.bit_chg_mask(id as u32, 0));
                                    }
                                }
                            } else {
                                for (lane, &id) in dsts.iter().enumerate() {
                                    let bit =
                                        (((val >> lane) & 1) | (((xz >> lane) & 1) << 1)) as u8;
                                    if self.fused_bit_commit(
                                        BitRef { sig_id: id, bit: 0 },
                                        bit,
                                        sdf_any,
                                    ) {
                                        let id = id as usize;
                                        if capture_churn {
                                            churn.push((id, eidx));
                                        }
                                        note_toggle!(id);
                                        trigger_deps!(id, eidx, self.bit_chg_mask(id as u32, 0));
                                    }
                                }
                            }
                            n_dc += dsts.len() as u64;
                        }
                        CombItem::FusedBufFanout { src, dsts, invert } => {
                            let new_bit = self.fused_buf_src_bit(*src, *invert);
                            let sdf_any = !self.sdf_delays.is_empty();
                            // The §28.4 z->x rule can only bite when the (single,
                            // loop-invariant) source bit is z on a non-inverting
                            // group — hoist that test so the common group pays
                            // nothing per destination.
                            let z_check = !*invert && new_bit == 3;
                            for &dst in dsts.iter() {
                                let bit = if z_check {
                                    self.fused_buf_dst_bit(dst, new_bit, *invert)
                                } else {
                                    new_bit
                                };
                                if self.fused_bit_commit(dst, bit, sdf_any) {
                                    let id = dst.sig_id as usize;
                                    if capture_churn {
                                        churn.push((id, eidx));
                                    }
                                    note_toggle!(id);
                                    trigger_deps!(id, eidx, self.bit_chg_mask(dst.sig_id, dst.bit));
                                }
                            }
                            n_dc += dsts.len() as u64;
                        }
                        CombItem::FusedAndFanout {
                            common,
                            branches,
                            invert,
                        } => {
                            let common_code = self.signal_table[common.sig_id as usize]
                                .get_bit_code(common.bit as usize);
                            let sdf_any = !self.sdf_delays.is_empty();
                            for branch in branches.iter() {
                                let bit = self.fused_and_branch_bit(common_code, branch, *invert);
                                if self.fused_bit_commit(branch.dst, bit, sdf_any) {
                                    let id = branch.dst.sig_id as usize;
                                    if capture_churn {
                                        churn.push((id, eidx));
                                    }
                                    note_toggle!(id);
                                    trigger_deps!(
                                        id,
                                        eidx,
                                        self.bit_chg_mask(branch.dst.sig_id, branch.dst.bit)
                                    );
                                }
                            }
                            n_dc += branches.len() as u64;
                        }
                        CombItem::Udp { idx } => {
                            let idx = *idx;
                            self.eval_udp(idx);
                            n_dc += 1;
                        }
                        CombItem::UdpBatch { event_ref, indices } => {
                            self.eval_udp_batch(*event_ref, indices);
                            n_dc += indices.len() as u64;
                        }
                        CombItem::TimingCheck { idx } => {
                            let idx = *idx;
                            self.eval_timing_watch(idx);
                            n_dc += 1;
                        }
                    }
                }

                // Worklist propagation: any signals newly dirtied by this
                // entry's evaluation trigger their dependents, which are
                // appended to `settle_triggered_list` if not already fired.
                // Topo-ordered entries → most dependents sit at higher tidx
                // and will be reached before the outer iter boundary. A
                // two-state entry's stores come first, each record with its
                // changed bits (`ts_direct_dirty`); every other write claims
                // the whole signal.
                let n_recs = self.ts_rec_n;
                if n_recs != 0 {
                    let (sig_id, chg) = self.ts_rec0;
                    if capture_churn {
                        churn.push((sig_id, eidx));
                    }
                    trigger_deps!(sig_id, eidx, chg);
                    if n_recs > 1 {
                        for ri in 0..self.ts_recs.len() {
                            let (sig_id, chg) = self.ts_recs[ri];
                            if capture_churn {
                                churn.push((sig_id, eidx));
                            }
                            trigger_deps!(sig_id, eidx, chg);
                        }
                        self.ts_recs.clear();
                    }
                    self.ts_rec_n = 0;
                }
                let dirty_after = self.dirty_list.len();
                if dirty_after > dirty_before {
                    if capture_churn {
                        for di in dirty_before..dirty_after {
                            churn.push((self.dirty_list[di], eidx));
                        }
                    }
                    for di in dirty_before..dirty_after {
                        let sig_id = self.dirty_list[di];
                        // Consume the dirty flag — we're propagating it now.
                        // If the signal gets dirtied again later, it'll be
                        // re-pushed to dirty_list with a fresh flag. A
                        // two-state entry never set the flag (direct
                        // writes), so it skips the clear.
                        if !ts_fast {
                            self.dirty_signals[sig_id] = false;
                        }
                        trigger_deps!(sig_id, eidx, u64::MAX);
                    }
                }
            }

            // Gate lane: the fused gates queued during this pass, in one
            // tight loop. None of them feeds a comb entry, so nothing here
            // can trigger further work in this pass.
            if !gate_queue.is_empty() {
                let slow = self.gate_lane_slow();
                for qi in 0..gate_queue.len() {
                    let g = gate_queue[qi] as usize;
                    gate_queued[g] = false;
                    let (dst, new_bit) = self.fused_gate_eval(&self.gate_ops[g]);
                    let obs = self.gate_obs[g];
                    if self.gate_lane_commit(dst, new_bit, obs, slow) {
                        n_writes += 1;
                    }
                }
                n_evals += gate_queue.len() as u64;
                n_dc += gate_queue.len() as u64;
                gate_queue.clear();
            }
            // After one full topo scan: if no entry was evaluated, fixpoint.
            if !evaluated_any {
                converged = true;
                break;
            }
            // Reset dirty bookkeeping for the next scan. (We already consumed
            // the per-signal flags via the propagation loop; clear the list.)
            self.dirty_list.clear();
            self.dirty_any = false;
            // Prepare cur_list for next pass reuse (it'll be swapped in).
            cur_list.clear();
            if next_list.is_empty() {
                converged = true;
                break;
            }
        }

        self.entry_evals += n_evals;
        self.prof_settle_dc_count += n_dc;
        self.prof_settle_ab_count += n_ab;
        self.prof_settle_repass_evals += n_repass;
        self.prof_settle_writes += n_writes;
        self.prof_settle_dep_edges += n_dep_edges;
        self.settle_entries_view = (std::ptr::null(), 0);
        self.comb_entries = entries;
        self.comb_dep_offsets = dep_offsets;
        self.comb_dep_entries = dep_entries;
        self.comb_dep_edges = dep_edges;
        self.is_clock_tree_signal = tree_sig;
        self.is_clock_tree_entry = tree_entry;
        self.settle_inject_bits = inject_bits;
        self.settle_inject_sum = inject_sum;
        self.settle_triggered = triggered;
        self.settle_triggered_list = next_list;
        self.gate_lane = gate_lane;
        if self.profile_report {
            self.prof_cur.store(
                crate::compiler::prof_sampler::KIND_OTHER,
                std::sync::atomic::Ordering::Relaxed,
            );
        }
        self.gate_queue = gate_queue;
        self.gate_queued = gate_queued;
        self.settle_dirty_ids = cur_list;
        self.settle_iters += total_iters;
        // Unconverged = the loop exhausted its budget with entries still
        // queued for the next pass. (The old `self.dirty_any` test here
        // was dead: every pass ends with `dirty_any = false`, so the
        // warning never actually fired even on a hard oscillator.)
        //
        // Diagnosability (§5): a design that hits the cap REPEATEDLY (the
        // customer's does, from t=151,310) used to warn once — the
        // max_settle_iters ratchet swallowed every later hit. Report the
        // first three hits verbosely, then every 1000th with the running
        // count; `finish_diagnostics` prints a final total.
        if total_iters >= limit && !converged && !self.settle_triggered_list.is_empty() {
            self.settle_limit_hits += 1;
            self.settle_limit_last_time = self.time;
            if self.settle_limit_hits <= 3 || self.settle_limit_hits % 1000 == 0 {
                eprintln!(
                    "[WARN] settle limit hit ({} iters) at time {} — signals may not have converged (hit #{}) . Use --settle-limit to increase.",
                    limit, self.time, self.settle_limit_hits
                );
                self.report_settle_churn(&churn, CHURN_TAIL);
            }
        }
        if total_iters > self.max_settle_iters {
            self.max_settle_iters = total_iters;
        }
        self.settling = false;
    }

    /// Attribution for the settle-limit warning: name the signals that were
    /// still CHANGING in the final settle iterations — current value, change
    /// count, and (when resolvable) the driving cont-assign / comb block's
    /// file:line and instance scope. Degrades gracefully: entries without a
    /// span print name + value only.
    pub(super) fn report_settle_churn(&self, churn: &[(usize, usize)], tail: u64) {
        if churn.is_empty() {
            return;
        }
        // (signal -> (change count, last writing entry)).
        let mut counts: HashMap<usize, (u64, usize)> = HashMap::default();
        for &(sig, eidx) in churn {
            let e = counts.entry(sig).or_insert((0, eidx));
            e.0 += 1;
            e.1 = eidx;
        }
        let total = counts.len();
        let mut top: Vec<(usize, u64, usize)> =
            counts.into_iter().map(|(s, (c, e))| (s, c, e)).collect();
        top.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        top.truncate(5);
        eprintln!(
            "       Still changing in the last {} iterations ({} of {} signals shown) — likely a zero-delay combinational loop through:",
            tail,
            top.len(),
            total
        );
        for (sig, cnt, eidx) in top {
            let name = self.name_for_id(sig);
            let width = self.signal_widths.get(sig).copied().unwrap_or(1);
            let val = match self.signal_table.get(sig) {
                Some(v) if width > 16 => format!("'h{}", v.to_hex_string()),
                Some(v) => format!("'b{}", v.to_bin()),
                None => "?".to_string(),
            };
            let mut line = format!("         {} = {} — changed {}x", name, val, cnt);
            if let Some(entry) = self.comb_entries.get(eidx) {
                let kind = match &entry.item {
                    CombItem::AlwaysBlock { is_always_comb, .. }
                    | CombItem::CompiledAlwaysBlock { is_always_comb, .. } => {
                        if *is_always_comb {
                            "always_comb block"
                        } else {
                            "always block"
                        }
                    }
                    _ => "cont-assign",
                };
                line.push_str(" — driven by ");
                line.push_str(kind);
                let src_file = self.scope_src_file(entry.cold.scope_hint.as_deref());
                if let Some(loc) = self.span_file_line_in(entry.cold.span, src_file) {
                    line.push_str(" at ");
                    line.push_str(&loc);
                }
                if let Some(scope) = entry.cold.scope_hint.as_deref().filter(|s| !s.is_empty()) {
                    match self.scope_def_module(Some(scope)) {
                        Some(m) if m != self.module.name => {
                            line.push_str(&format!(" ({}, module {})", scope, m));
                        }
                        _ => line.push_str(&format!(" ({})", scope)),
                    }
                }
            }
            eprintln!("{}", line);
        }
    }
}
