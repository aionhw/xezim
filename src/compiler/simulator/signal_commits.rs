//! Simulator signal commits.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

impl Simulator {
    #[inline(always)]
    pub(super) fn exec_vector_gate(&mut self, op: &VectorGate) {
        let (dst, width, val, xz) = self.vector_gate_eval(op);
        let sdf_any = !self.sdf_delays.is_empty();
        if self.fused_vector_commit(dst, width, val, xz, sdf_any) {
            self.mark_dirty_id(dst.sig_id as usize);
        }
    }

    #[inline(always)]
    pub(super) fn scatter_gate_eval(&self, op: &ScatterGate) -> (u64, u64) {
        match op {
            ScatterGate::Buf {
                src, width, invert, ..
            } => {
                let (v, x) = self.vector_ref_bits(*src, *width);
                let mask = if *width == 64 {
                    u64::MAX
                } else {
                    (1u64 << *width) - 1
                };
                (if *invert { (!v) & !x & mask } else { v }, x)
            }
            ScatterGate::Bin {
                a,
                b,
                width,
                op,
                invert,
                ..
            } => {
                let (av, ax) = self.vector_ref_bits(*a, *width);
                let (bv, bx) = self.vector_ref_bits(*b, *width);
                let mask = if *width == 64 {
                    u64::MAX
                } else {
                    (1u64 << *width) - 1
                };
                let (rv, rx) = match op {
                    GateBin::And => {
                        let one = (av & !ax) & (bv & !bx);
                        let zero = (!av & !ax) | (!bv & !bx);
                        (one & mask, (!(one | zero)) & mask)
                    }
                    GateBin::Or => {
                        let one = (av & !ax) | (bv & !bx);
                        let zero = (!av & !ax) & (!bv & !bx);
                        (one & mask, (!(one | zero)) & mask)
                    }
                    GateBin::Xor => {
                        let unknown = (ax | bx) & mask;
                        ((av ^ bv) & !unknown & mask, unknown)
                    }
                };
                (if *invert { (!rv) & !rx & mask } else { rv }, rx)
            }
        }
    }

    #[inline]
    pub(super) fn exec_scatter_gate(&mut self, op: &ScatterGate) {
        let (val, xz) = self.scatter_gate_eval(op);
        let dsts = match op {
            ScatterGate::Buf { dsts, .. } | ScatterGate::Bin { dsts, .. } => dsts,
        };
        let sdf_any = !self.sdf_delays.is_empty();
        if let Some((edge_active, armed_active)) = self.fused_batch_commit_state() {
            for (lane, &id) in dsts.iter().enumerate() {
                let bit = (((val >> lane) & 1) | (((xz >> lane) & 1) << 1)) as u8;
                if self.fused_scalar_commit_planned(id, bit, sdf_any, edge_active, armed_active) {
                    self.mark_dirty_id(id as usize);
                }
            }
        } else {
            for (lane, &id) in dsts.iter().enumerate() {
                let bit = (((val >> lane) & 1) | (((xz >> lane) & 1) << 1)) as u8;
                if self.fused_bit_commit(BitRef { sig_id: id, bit: 0 }, bit, sdf_any) {
                    self.mark_dirty_id(id as usize);
                }
            }
        }
    }

    #[inline(always)]
    pub(super) fn fused_batch_commit_state(&self) -> Option<(bool, bool)> {
        if self.signal_commit_plan.is_empty()
            || self.value_trace_ids.is_some()
            || self.dump_dirty_active
            || !self.active_force_exprs.is_empty()
            || !self.signal_inline_bits.is_empty()
            || (self.event_measure && !self.armed_edge)
        {
            None
        } else {
            Some((
                self.dirty_edge
                    || self.dirty_edge_shadow
                    || self.in_edge_block
                    || self.in_edge_cont,
                self.armed_edge,
            ))
        }
    }

    #[inline(always)]
    pub(super) fn fused_scalar_commit_planned(
        &mut self,
        signal_id: u32,
        new_bit: u8,
        sdf_any: bool,
        edge_active: bool,
        armed_active: bool,
    ) -> bool {
        let id = signal_id as usize;
        if sdf_any && self.sdf_delays.get(id).copied().unwrap_or(0) > 0 && self.time > 0 {
            if self.signal_table[id].get_bit_code(0) != new_bit {
                let mut value = self.signal_table[id].clone();
                value.set_scalar_code(new_bit);
                self.schedule_delayed(id, value);
            }
            return false;
        }
        if !self.signal_table[id].set_scalar_code(new_bit) {
            return false;
        }
        self.table_modified = true;
        let plan = self.signal_commit_plan.get(id).copied().unwrap_or(3);
        if armed_active && plan & 2 != 0 {
            self.note_armed_write(id);
        }
        if edge_active && plan & 1 != 0 {
            self.note_edge_write(id);
        }
        true
    }

    /// Commit one 4-state bit to a fused-gate destination. Shared by every
    /// fused evaluator (single gate, buffer fanout, AND fanout) and by the
    /// settle loop, which needs the *outcome* of the write so it can trigger
    /// the destination's dependents inline instead of routing them through
    /// `dirty_list`. Returns true when `signal_table` actually changed; an
    /// SDF-delayed destination reschedules and reports false (the change
    /// lands later, via the delay queue's own dirty marking).
    ///
    /// `sdf_any` is `!self.sdf_delays.is_empty()`, hoisted by the caller out
    /// of its per-destination loop — SDF annotation cannot change while a
    /// fanout group is being written.
    #[inline(always)]
    /// Gate-lane commit for a ≤64-bit destination (always inline storage):
    /// direct plane update, and only the write notes the destination needs
    /// (`obs` bits from `build_gate_lane`). Falls back to the generic commit
    /// when any global observer is active.

    /// Whether a gate-lane commit must take the full `fused_bit_commit`
    /// path because a global observer is active. Nothing a lane commit does
    /// changes any of these, so the lane tests it once per drain.
    pub(super) fn gate_lane_slow(&self) -> bool {
        self.value_trace_ids.is_some()
            || self.dump_dirty_active
            || !self.active_force_exprs.is_empty()
            || !self.signal_inline_bits.is_empty()
            || (self.event_measure && !self.armed_edge)
            || !self.sdf_delays.is_empty()
            || !self.signal_commit_plan.is_empty()
    }

    /// `obs`: bit 0 armed input, bit 1 edge signal, bit 2 destination wider
    /// than 64 bits (`build_gate_lane`); `slow` is `gate_lane_slow()`.
    pub(super) fn gate_lane_commit(
        &mut self,
        dst: BitRef,
        new_bit: u8,
        obs: u8,
        slow: bool,
    ) -> bool {
        let id = dst.sig_id as usize;
        if slow || obs & 4 != 0 {
            let sdf_any = !self.sdf_delays.is_empty();
            return self.fused_bit_commit(dst, new_bit, sdf_any);
        }
        let entry: &mut Value = unsafe { self.signal_table.get_unchecked_mut(id) };
        let (v, x) = entry.raw_bits();
        let bit = dst.bit as usize;
        let m = 1u64 << bit;
        let cur = ((((x >> bit) & 1) << 1) | ((v >> bit) & 1)) as u8;
        if cur == new_bit {
            return false;
        }
        let nv = if new_bit & 1 != 0 { v | m } else { v & !m };
        let nx = if new_bit & 2 != 0 { x | m } else { x & !m };
        if !entry.set_inline_bits(nv, nx) {
            let sdf_any = !self.sdf_delays.is_empty();
            return self.fused_bit_commit(dst, new_bit, sdf_any);
        }
        self.table_modified = true;
        if obs & 1 != 0 {
            self.note_armed_write(id);
        }
        if obs & 2 != 0 {
            self.note_edge_write(id);
        }
        true
    }

    pub(super) fn fused_bit_commit(&mut self, dst: BitRef, new_bit: u8, sdf_any: bool) -> bool {
        let id = dst.sig_id as usize;
        // Late (runtime $sdf_annotate) SDF delay on a fused destination:
        // route the whole-signal updated value through schedule_delayed
        // instead of writing the bit immediately. Build-time (CLI --sdf)
        // annotation never fuses a delayed destination, so on that path
        // sdf_delays is either empty (no SDF at all — the hoisted bool) or
        // zero for every fused dst (one Vec index).
        if sdf_any && self.sdf_delays.get(id).copied().unwrap_or(0) > 0 && self.time > 0 {
            if self.signal_table[id].get_bit_code(dst.bit as usize) != new_bit {
                let mut v = self.signal_table[id].clone();
                v.set_bit_code(dst.bit as usize, new_bit);
                self.schedule_delayed(id, v);
            }
            return false;
        }
        if self.signal_table[id].set_bit_code(dst.bit as usize, new_bit) {
            // §10.6.2: a forced net keeps its forced value against its gate
            // driver. Checked only on a change, off the common path.
            if !self.forced_signals.is_empty() {
                if let Some(fv) = self.forced_signals.get(&id) {
                    let code = fv.get_bit_code(dst.bit as usize);
                    self.signal_table[id].set_bit_code(dst.bit as usize, code);
                    return false;
                }
            }
            self.sync_mirror(id);
            self.table_modified = true;
            // sync_mirror just refreshed the mirror — skip the repack
            // `after_signal_write` would redo (367M commits/run on c906).
            self.after_signal_write_premirrored(id);
            true
        } else {
            false
        }
    }

    /// Source bit of a fused buffer fanout, with the group's inversion applied.
    #[inline(always)]
    pub(super) fn fused_buf_src_bit(&self, src: BitRef, invert: bool) -> u8 {
        let source = self.signal_table[src.sig_id as usize].get_bit_code(src.bit as usize);
        if invert {
            match source {
                0 => 1,
                1 => 0,
                _ => 2,
            }
        } else {
            source
        }
    }

    /// §28.4 vs §10.3 — decided per destination: a fanout group can mix
    /// a gate-driven net with a plain assign's target.
    #[inline(always)]
    pub(super) fn fused_buf_dst_bit(&self, dst: BitRef, new_bit: u8, invert: bool) -> u8 {
        if !invert
            && new_bit == 3
            && self
                .signal_gate_driven
                .get(dst.sig_id as usize)
                .copied()
                .unwrap_or(false)
        {
            2
        } else {
            new_bit
        }
    }

    /// Execute a group of fused 1-bit buffers that share one source bit.
    #[inline]
    pub(super) fn exec_fused_buf_fanout(&mut self, src: BitRef, dsts: &[BitRef], invert: bool) {
        let new_bit = self.fused_buf_src_bit(src, invert);
        let sdf_any = !self.sdf_delays.is_empty();
        for &dst in dsts {
            let bit = self.fused_buf_dst_bit(dst, new_bit, invert);
            if self.fused_bit_commit(dst, bit, sdf_any) {
                self.mark_dirty_id(dst.sig_id as usize);
            }
        }
    }

    /// Result bit of one branch of a fused AND/NAND fanout.
    #[inline(always)]
    pub(super) fn fused_and_branch_bit(
        &self,
        common_code: u8,
        branch: &FusedAndBranch,
        invert: bool,
    ) -> u8 {
        let other =
            self.signal_table[branch.other.sig_id as usize].get_bit_code(branch.other.bit as usize);
        let and = if common_code == 0 || other == 0 {
            0
        } else if common_code == 1 && other == 1 {
            1
        } else {
            2
        };
        if invert {
            match and {
                0 => 1,
                1 => 0,
                _ => 2,
            }
        } else {
            and
        }
    }

    /// Execute fused one-bit AND/NAND gates that share one operand.
    #[inline]
    pub(super) fn exec_fused_and_fanout(
        &mut self,
        common: BitRef,
        branches: &[FusedAndBranch],
        invert: bool,
    ) {
        let common_code =
            self.signal_table[common.sig_id as usize].get_bit_code(common.bit as usize);
        let sdf_any = !self.sdf_delays.is_empty();
        for branch in branches {
            let new_bit = self.fused_and_branch_bit(common_code, branch, invert);
            if self.fused_bit_commit(branch.dst, new_bit, sdf_any) {
                self.mark_dirty_id(branch.dst.sig_id as usize);
            }
        }
    }

    /// Read-only half of `exec_fused_gate`: fetch the operand bits and compute
    /// the 4-state result. Split out so the settle loop can decide what to do
    /// with a successful write (trigger dependents inline) without duplicating
    /// the gate algebra.
    #[inline(always)]
    pub(super) fn fused_gate_eval(&self, op: &FusedGate) -> (BitRef, u8) {
        #[inline(always)]
        fn and4(a: u8, b: u8) -> u8 {
            if a == 0 || b == 0 {
                0
            } else if a == 1 && b == 1 {
                1
            } else {
                2
            }
        }
        #[inline(always)]
        fn or4(a: u8, b: u8) -> u8 {
            if a == 1 || b == 1 {
                1
            } else if a == 0 && b == 0 {
                0
            } else {
                2
            }
        }
        #[inline(always)]
        fn xor4(a: u8, b: u8) -> u8 {
            match (a, b) {
                (0, 0) | (1, 1) => 0,
                (0, 1) | (1, 0) => 1,
                _ => 2,
            }
        }
        #[inline(always)]
        fn not4(a: u8) -> u8 {
            match a {
                0 => 1,
                1 => 0,
                _ => 2,
            }
        }
        let (dst, new_bit) = match op {
            FusedGate::Buf1 { dst, src, invert } => {
                let s = self.signal_table[src.sig_id as usize].get_bit_code(src.bit as usize);
                // §28.4 `buf` primitive: a z input drives x. §10.3 continuous
                // assign: the value passes through, z included. The two rules
                // only ever DIFFER when the source bit is z, so test that
                // first — `signal_gate_driven` is one bool per signal (35M on
                // c906) and probing it on every buffer evaluation was a
                // guaranteed miss for a decision that is almost always "no
                // change". Same result for all four (invert, gate_driven,
                // s) combinations.
                let v = if *invert {
                    not4(s)
                } else if s == 3
                    && self
                        .signal_gate_driven
                        .get(dst.sig_id as usize)
                        .copied()
                        .unwrap_or(false)
                {
                    2
                } else {
                    s
                };
                (dst, v)
            }
            FusedGate::Bin2 {
                dst,
                a,
                b,
                op: gop,
                invert,
            } => {
                let va = self.signal_table[a.sig_id as usize].get_bit_code(a.bit as usize);
                let vb = self.signal_table[b.sig_id as usize].get_bit_code(b.bit as usize);
                let r = match gop {
                    GateBin::And => and4(va, vb),
                    GateBin::Or => or4(va, vb),
                    GateBin::Xor => xor4(va, vb),
                };
                (dst, if *invert { not4(r) } else { r })
            }
            FusedGate::Mux2 { dst, s, t, e } => {
                let vs = self.signal_table[s.sig_id as usize].get_bit_code(s.bit as usize);
                let vt = self.signal_table[t.sig_id as usize].get_bit_code(t.bit as usize);
                let ve = self.signal_table[e.sig_id as usize].get_bit_code(e.bit as usize);
                // IEEE 1800 §11.4.11 Table 11-21: ambiguous select merges the
                // branches per bit — 0/0→0, 1/1→1, every other pair (incl.
                // z/z) → x.
                let v = match vs {
                    0 => ve,
                    1 => vt,
                    _ => {
                        if vt == ve && vt < 2 {
                            vt
                        } else {
                            2
                        }
                    }
                };
                (dst, v)
            }
            FusedGate::UdpLut3 {
                dst,
                inputs,
                input_count,
                table,
            } => {
                let mut lut_idx = 0usize;
                let mut place = 1usize;
                for input in inputs.iter().take(*input_count as usize) {
                    let code =
                        self.signal_table[input.sig_id as usize].get_bit_code(input.bit as usize);
                    lut_idx += usize::from(code.min(2)) * place;
                    place *= 3;
                }
                (dst, ((table >> (lut_idx * 2)) & 3) as u8)
            }
        };
        (*dst, new_bit)
    }

    /// Execute a fused 1-bit gate. Reads operand bits from signal_table,
    /// computes 4-state result, writes single bit back if changed.
    #[inline(always)]
    pub(super) fn exec_fused_gate(&mut self, op: &FusedGate) {
        let (dst, new_bit) = self.fused_gate_eval(op);
        let sdf_any = !self.sdf_delays.is_empty();
        if self.fused_bit_commit(dst, new_bit, sdf_any) {
            self.mark_dirty_id(dst.sig_id as usize);
        }
    }

    /// Batch-sync signal_table → signals HashMap.
    /// Called lazily before any code that reads from the HashMap.
    pub(super) fn sync_table_to_hashmap(&mut self) {
        if !self.table_modified {
            return;
        }
        for (id, name) in self.id_to_name.iter() {
            if let Some(v) = self.signal_table.get(id) {
                self.signals.insert(name.to_string(), v.clone());
            }
        }
        // Elements of virtual-name arrays (see `NameMap`) were registered
        // names too; mirror them the same way.
        let mut elems: Vec<(String, usize)> = Vec::new();
        for (base, first, lo, hi) in self.signal_name_to_id.virtual_arrays() {
            for idx in lo..=hi {
                elems.push((format!("{}[{}]", base, idx), first + (idx - lo) as usize));
            }
        }
        for (name, id) in elems {
            if let Some(v) = self.signal_table.get(id) {
                self.signals.insert(name, v.clone());
            }
        }
        self.table_modified = false;
    }

    /// Mark a signal as dirty by name (for settle_combinatorial)
    #[inline]
    pub(super) fn mark_dirty(&mut self, name: &str) {
        if let Some(&id) = self.signal_name_to_id.get(name) {
            let has_comb_deps = id + 1 >= self.comb_dep_offsets.len()
                || self.comb_dep_offsets[id] != self.comb_dep_offsets[id + 1];
            if has_comb_deps {
                if self.ts_direct_writes {
                    self.ts_direct_dirty(id, u64::MAX);
                } else if !self.dirty_signals[id] {
                    self.dirty_signals[id] = true;
                    self.dirty_list.push(id);
                }
                if !self.ts_direct_writes {
                    self.dirty_any = true;
                }
            }
        }
    }

    /// Dirty-driven edge detect: record a write to signal `id` so check_edges
    /// can later scan only changed edge-sensitive positions. No-op (one bool
    /// load) unless XEZIM_DIRTY_EDGE / _SHADOW is active.
    pub(super) fn rebuild_signal_commit_plans(&mut self) {
        self.signal_commit_plan.clear();
        if std::env::var("XEZIM_COMMIT_PLAN").ok().as_deref() != Some("1") {
            return;
        }
        self.signal_commit_plan.resize(self.signal_table.len(), 0);
        for id in 0..self.signal_commit_plan.len() {
            if self.sig_to_edge_pos.get(id).is_some_and(|&p| p >= 0) {
                self.signal_commit_plan[id] |= 1;
            }
            if self.armed_input_set(id) {
                self.signal_commit_plan[id] |= 2;
            }
        }
    }

    #[inline(always)]
    pub(super) fn note_edge_write(&mut self, id: usize) {
        // §9.2 re-trigger recording (always on) — see the write_sig! twin.
        if self.in_edge_block || self.in_edge_cont {
            if let Some(&p) = self.sig_to_edge_pos.get(id) {
                if p >= 0 {
                    let p = p as usize;
                    if !self.edge_exec_seen[p] {
                        self.edge_exec_seen[p] = true;
                        self.edge_exec_wrote.push(p);
                    }
                }
            }
        }
        if !self.dirty_edge && !self.dirty_edge_shadow {
            return;
        }
        if let Some(&p) = self.sig_to_edge_pos.get(id) {
            if p >= 0 {
                let p = p as usize;
                if !self.edge_pos_seen[p] {
                    self.edge_pos_seen[p] = true;
                    self.changed_edge_pos.push(p);
                }
            }
        }
    }

    #[inline(always)]
    pub(super) fn armed_input_set(&self, id: usize) -> bool {
        match self.armed_input_bitmap.get(id >> 6) {
            Some(w) => (w >> (id & 63)) & 1 != 0,
            None => false,
        }
    }

    #[inline(always)]
    pub(super) fn note_armed_write(&mut self, id: usize) {
        if !self.armed_edge || !self.armed_input_set(id) {
            return;
        }
        let (lo, hi) = self.armed_input_ranges[id];
        if let Some(c) = self.arm_census.as_mut() {
            if c.len() <= id {
                c.resize(id + 1, (0, 0));
            }
            c[id].0 += 1;
            c[id].1 += (hi - lo) as u64;
        }
        for k in lo as usize..hi as usize {
            let bi = self.armed_input_blocks[k] as usize;
            self.edge_block_armed[bi] |= EDGE_ARMED;
        }
    }

    /// Canonical post-write hook for in-place `signal_table[id]` mutations.
    /// It intentionally leaves `signal_has_xz` stale-conservative; tightening
    /// that hint can enable JIT paths that do not yet support every X/Z case.
    /// `--warn-x`: a write is putting an x bit into `id` that was not x
    /// before. Reports once per signal, with the instance/module it lives in
    /// and every driver that can write it.
    ///
    /// X after time 0 is the classic gate-level/RTL-integration failure —
    /// an unconnected port, a register never reset, a bus with no active
    /// driver — and a waveform only shows WHERE it surfaced, not what drove
    /// it there. Reporting the driver list at the moment the bit turns x
    /// points straight at the cause.
    /// Record a fresh x on `id` from raw (val, xz) bit pairs. Takes `&self`
    /// and records through `RefCell`s, so it can be called from the comb-settle
    /// fast paths (which write `signal_table` directly and never reach
    /// `write_sig!`) and from the `&self` isolated evaluators.
    #[inline]
    pub(super) fn warn_x_note_bits(&self, id: usize, ov: u64, ox: u64, nv: u64, nx: u64) {
        let _ = ov;
        // x is (nx & !nv). Only a VALID 0/1 bit (old xz clear) going x is
        // reported — see the write_sig! macro's twin check.
        let fresh = (nx & !nv) & !ox;
        if fresh == 0 {
            return;
        }
        if !self.warn_x_seen.borrow_mut().insert(id) {
            return;
        }
        let w = self.signal_widths.get(id).copied().unwrap_or(64).max(1);
        let mut v = Value::zero(w);
        // Only the low 64 bits are available here; the report caps its own
        // rendering at 64 bits anyway.
        let _ = v.set_inline_bits(nv, nx);
        self.warn_x_pending
            .borrow_mut()
            .push((id, self.time, v, self.current_pid, None));
    }

    /// Record a fresh x on a value stored by NAME rather than by signal id —
    /// an associative-array element, and anything else kept in the untyped
    /// `signals` map. Those never pass through `write_sig!` or the raw-bit
    /// fast paths, so without this an x landing in `aa["k"]` went unreported
    /// while the very same x in a queue element (which does have an id) was
    /// reported — an inconsistency more confusing than silence.
    pub(super) fn warn_x_note_named(&self, name: &str, old: Option<&Value>, new: &Value) {
        // Fresh only when the bit previously held a VALID 0/1: a value born x
        // (no prior element, or an x/z bit) is not a corruption to report.
        let has_new_x = (0..new.width as usize).any(|b| {
            new.get_bit(b) == LogicBit::X
                && old.is_some_and(|o| {
                    b < o.width as usize && matches!(o.get_bit(b), LogicBit::Zero | LogicBit::One)
                })
        });
        if !has_new_x {
            return;
        }
        if !self.warn_x_seen_named.borrow_mut().insert(name.to_string()) {
            return;
        }
        self.warn_x_pending.borrow_mut().push((
            usize::MAX,
            self.time,
            new.clone(),
            self.current_pid,
            Some(name.to_string()),
        ));
    }

    pub fn drain_warn_x(&mut self) {
        if !self.warn_x {
            return;
        }
        let pending: Vec<(usize, u64, Value, usize, Option<String>)> =
            std::mem::take(&mut *self.warn_x_pending.borrow_mut());
        for (id, t, val, pid, named) in pending {
            self.report_x_one(id, t, &val, pid, named);
        }
    }

    pub(super) fn report_x_one(
        &mut self,
        id: usize,
        when: u64,
        new_val: &Value,
        pid: usize,
        named: Option<String>,
    ) {
        if named.is_none() && id >= self.signal_table.len() {
            return;
        }
        let lim = warn_x_limit();
        if lim != 0 && self.warn_x_count >= lim {
            return;
        }
        let w = new_val.width.max(1) as usize;
        let mut fresh_x: Vec<usize> = Vec::new();
        for b in 0..w {
            if new_val.get_bit(b) == LogicBit::X {
                fresh_x.push(b);
            }
        }
        if fresh_x.is_empty() {
            return;
        }
        self.warn_x_count += 1;

        let name = match &named {
            Some(n) => n.clone(),
            None => self.name_for_id(id).to_string(),
        };
        // `aa[key]` / `q[3]`: the instance path is what precedes the element
        // name, so strip the subscript before splitting off the scope.
        let name_for_scope = name
            .split_once('[')
            .map(|(base, _)| base.to_string())
            .unwrap_or_else(|| name.clone());
        // `a.b.c.sig` -> instance path `a.b.c`; empty for a top-level signal.
        let inst = name_for_scope.rsplit_once('.').map(|(p, _)| p.to_string());
        let module = self
            .scope_def_module(inst.as_deref())
            .unwrap_or_else(|| self.module.name.clone());
        let where_ = match &inst {
            Some(p) if !p.is_empty() => format!("instance {} (module {})", p, module),
            _ => format!("module {}", module),
        };

        let bits = if fresh_x.len() == w {
            "all bits".to_string()
        } else if fresh_x.len() == 1 {
            format!("bit {}", fresh_x[0])
        } else {
            let hi = fresh_x.iter().copied().max().unwrap_or(0);
            let lo = fresh_x.iter().copied().min().unwrap_or(0);
            if fresh_x.len() == hi - lo + 1 {
                format!("bits [{}:{}]", hi, lo)
            } else {
                format!("{} bits, highest {}", fresh_x.len(), hi)
            }
        };

        eprintln!(
            "[xezim][warning] X on '{}' at time {} — {} x ({})",
            name,
            when,
            bits,
            self.warn_x_bin(new_val, w)
        );
        eprintln!("                 in {}", where_);
        let drivers = if named.is_some() {
            Vec::new()
        } else {
            self.warn_x_driver_lines(id)
        };
        if drivers.is_empty() {
            // No continuous/comb/edge driver. Either a procedural process wrote
            // it — `process_origin` knows which, and that is the useful answer —
            // or nothing drives it at all, which is its own diagnosis.
            let by_proc = self.process_origin.get(&pid).map(|&(span, kind)| {
                let loc = self
                    .span_file_line_in(span, self.stall_pid_src_file(pid))
                    .map(|l| format!(" at {}", l))
                    .unwrap_or_default();
                format!("{}{}", kind, loc)
            });
            match by_proc {
                Some(d) => {
                    eprintln!("                 written by:");
                    eprintln!("                   - {} (procedural)", d);
                }
                None => eprintln!(
                    "                 drivers: NONE FOUND — nothing drives this signal, so it \
                     reads x; check for an unconnected port or a missing assignment"
                ),
            }
        } else {
            eprintln!("                 driven by:");
            for d in drivers {
                eprintln!("                   - {}", d);
            }
        }
        if lim != 0 && self.warn_x_count == lim {
            eprintln!(
                "[xezim][warning] --warn-x limit of {} reached; further X reports suppressed \
                 (raise with --warn-x-limit N, 0 = unlimited)",
                lim
            );
        }
    }

    /// Per-bit rendering, capped so a 4096-bit bus does not fill the terminal.
    pub(super) fn warn_x_bin(&self, v: &Value, w: usize) -> String {
        let shown = w.min(64);
        let mut out = String::with_capacity(shown + 8);
        if shown < w {
            out.push_str("low 64 bits: ");
        }
        for b in (0..shown).rev() {
            out.push(match v.get_bit(b) {
                LogicBit::Zero => '0',
                LogicBit::One => '1',
                LogicBit::X => 'x',
                LogicBit::Z => 'z',
            });
        }
        out
    }

    /// One line per thing that can write `id`. Built once, from the comb
    /// entries (continuous assigns, always_comb, fused gates) and the edge
    /// blocks (always_ff / always @(posedge …)).
    pub(super) fn warn_x_driver_lines(&mut self, id: usize) -> Vec<String> {
        if self.warn_x_drivers.is_none() {
            let mut map: HashMap<usize, Vec<String>> = HashMap::default();
            for e in self.comb_entries.iter() {
                let kind = match &e.item {
                    CombItem::ContAssign { .. } | CombItem::CompiledContAssign { .. } => {
                        "continuous assign"
                    }
                    CombItem::AlwaysBlock { is_always_comb, .. }
                    | CombItem::CompiledAlwaysBlock { is_always_comb, .. } => {
                        if *is_always_comb {
                            "always_comb"
                        } else {
                            "always @(*)"
                        }
                    }
                    CombItem::FusedGate { .. }
                    | CombItem::CtlGate { .. }
                    | CombItem::FusedBufFanout { .. }
                    | CombItem::FusedAndFanout { .. }
                    | CombItem::GateRegion { .. }
                    | CombItem::VectorGate { .. }
                    | CombItem::ScatterGate { .. } => "gate primitive",
                    CombItem::Udp { .. } | CombItem::UdpBatch { .. } => "UDP",
                    CombItem::TimingCheck { .. } => "timing check",
                    CombItem::DirectCopy { .. }
                    | CombItem::FastDirectCopy { .. }
                    | CombItem::FastDirectFanout { .. } => "continuous assign (copy)",
                    CombItem::Noop => continue,
                };
                let src_file = e
                    .cold
                    .scope_hint
                    .as_deref()
                    .and_then(|sc| self.scope_def_module(Some(sc)))
                    .and_then(|m| self.module.src_file_of_module.get(&m).copied());
                let at = self
                    .span_file_line_in(e.cold.span, src_file)
                    .map(|l| format!(" at {}", l))
                    .unwrap_or_default();
                let scope = match e.cold.scope_hint.as_deref() {
                    Some(sc) if !sc.is_empty() => format!(" [in {}]", sc),
                    _ => String::new(),
                };
                let line = format!("{}{}{}", kind, at, scope);
                for &wid in e.cold.write_signal_ids.iter() {
                    map.entry(wid).or_default().push(line.clone());
                }
            }
            // Edge blocks keep no resolved write set, so derive one.
            let blocks = Arc::clone(&self.edge_blocks);
            for b in blocks.iter() {
                let mut reads: HashSet<String> = HashSet::default();
                let mut writes: HashSet<String> = HashSet::default();
                Self::collect_stmt_reads(&b.stmt, &self.module, &mut reads, &mut writes);
                let kind = match b.kind {
                    AlwaysKind::AlwaysFf => "always_ff",
                    AlwaysKind::AlwaysLatch => "always_latch",
                    AlwaysKind::AlwaysComb => "always_comb",
                    AlwaysKind::Always => "always @(edge)",
                };
                let src_file = (!b.scope.is_empty())
                    .then(|| self.scope_def_module(Some(b.scope.as_str())))
                    .flatten()
                    .and_then(|m| self.module.src_file_of_module.get(&m).copied());
                let at = self
                    .span_file_line_in(b.stmt.span, src_file)
                    .map(|l| format!(" at {}", l))
                    .unwrap_or_default();
                let scope = if b.scope.is_empty() {
                    String::new()
                } else {
                    format!(" [in {}]", b.scope)
                };
                let line = format!("{}{}{}", kind, at, scope);
                for wname in writes.iter() {
                    let mut ids: Vec<usize> = Vec::new();
                    if let Some(&i) = self.signal_name_to_id.get(wname.as_str()) {
                        ids.push(i);
                    }
                    if !b.scope.is_empty() {
                        if let Some(&i) = self
                            .signal_name_to_id
                            .get(format!("{}.{}", b.scope, wname).as_str())
                        {
                            ids.push(i);
                        }
                    }
                    for i in ids {
                        map.entry(i).or_default().push(line.clone());
                    }
                }
            }
            // Procedural blocks. Without these a signal a testbench drives
            // from an `initial` was reported as having NO driver, which reads
            // as "unconnected port" — the opposite of the truth.
            let procs: Vec<(&'static str, Statement, String)> = self
                .module
                .initial_blocks
                .iter()
                .map(|ib| ("initial block", ib.stmt.clone(), ib.scope.clone()))
                .chain(
                    self.module
                        .final_blocks
                        .iter()
                        .map(|fb| ("final block", fb.stmt.clone(), fb.scope.clone())),
                )
                .collect();
            for (kind, stmt, scope) in procs {
                let mut reads: HashSet<String> = HashSet::default();
                let mut writes: HashSet<String> = HashSet::default();
                Self::collect_stmt_reads(&stmt, &self.module, &mut reads, &mut writes);
                let src_file = (!scope.is_empty())
                    .then(|| self.scope_def_module(Some(scope.as_str())))
                    .flatten()
                    .and_then(|m| self.module.src_file_of_module.get(&m).copied());
                let at = self
                    .span_file_line_in(stmt.span, src_file)
                    .map(|l| format!(" at {}", l))
                    .unwrap_or_default();
                let where_ = if scope.is_empty() {
                    String::new()
                } else {
                    format!(" [in {}]", scope)
                };
                let line = format!("{}{}{}", kind, at, where_);
                for wname in writes.iter() {
                    let mut ids: Vec<usize> = Vec::new();
                    if let Some(&i) = self.signal_name_to_id.get(wname.as_str()) {
                        ids.push(i);
                    }
                    if !scope.is_empty() {
                        if let Some(&i) = self
                            .signal_name_to_id
                            .get(format!("{}.{}", scope, wname).as_str())
                        {
                            ids.push(i);
                        }
                    }
                    for i in ids {
                        map.entry(i).or_default().push(line.clone());
                    }
                }
            }
            for v in map.values_mut() {
                v.sort();
                v.dedup();
            }
            self.warn_x_drivers = Some(map);
        }
        self.warn_x_drivers
            .as_ref()
            .and_then(|m| m.get(&id))
            .cloned()
            .unwrap_or_default()
    }

    /// Drain queued value-trace hits (see `value_trace_pending`).
    pub fn drain_value_trace(&mut self) {
        if self.value_trace_ids.is_none() {
            return;
        }
        let pending: Vec<(usize, u64, Value, Value, usize, &'static str)> =
            std::mem::take(&mut *self.value_trace_pending.borrow_mut());
        for (id, t, old, new, pid, phase) in pending {
            if self.value_trace_emitted > self.value_trace_limit {
                return;
            }
            if self.value_trace_emitted == self.value_trace_limit {
                self.value_trace_emitted += 1;
                eprintln!(
                    "[value-trace] output limit reached ({} lines) — raise XEZIM_VALUE_TRACE_LIMIT",
                    self.value_trace_limit
                );
                return;
            }
            self.value_trace_emitted += 1;
            let name = self.name_for_id(id).to_string();
            let render = |v: &Value| {
                let s = if v.width <= 8 {
                    v.to_bin()
                } else {
                    format!("'h{}", v.to_hex())
                };
                if s.len() > 80 {
                    format!("{}…", &s[..80])
                } else {
                    s
                }
            };
            let who = if phase == "nba" {
                "nba commit".to_string()
            } else {
                self.process_origin
                    .get(&pid)
                    .map(|(sp, kind)| {
                        let loc = self
                            .span_file_line_in(*sp, None)
                            .unwrap_or_else(|| "?".to_string());
                        format!("{} at {}", kind, loc)
                    })
                    .unwrap_or_else(|| "settle/elab".to_string())
            };
            // In-place mutation paths (bit/part-select, inline-bits fast
            // paths) can't capture the pre-write value; they queue old==new
            // and get the arrowless form.
            if old == new {
                eprintln!(
                    "[value-trace] t={} {} = {} ({}; {})",
                    t,
                    name,
                    render(&new),
                    phase,
                    who
                );
            } else {
                eprintln!(
                    "[value-trace] t={} {} {} -> {} ({}; {})",
                    t,
                    name,
                    render(&old),
                    render(&new),
                    phase,
                    who
                );
            }
        }
    }

    /// Resolve XEZIM_VALUE_TRACE patterns to signal ids (post-construction,
    /// when the name table exists). Prints the resolved watch set so a
    /// misspelled pattern is visible immediately instead of silently
    /// matching nothing.
    pub(super) fn init_value_trace(&mut self) {
        let Ok(pats) = std::env::var("XEZIM_VALUE_TRACE") else {
            return;
        };
        let pats: Vec<&str> = pats
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        if pats.is_empty() {
            return;
        }
        let mut ids = std::collections::HashSet::new();
        for pat in &pats {
            let mut hits = 0;
            self.signal_name_to_id.for_each_name(|n, id| {
                if n.contains(&pat[..]) {
                    ids.insert(id);
                    hits += 1;
                }
                true
            });
            if hits == 0 {
                eprintln!("[value-trace] pattern '{}' matched no signal", pat);
            } else {
                eprintln!("[value-trace] pattern '{}' matched {} signal(s)", pat, hits);
            }
        }
        if ids.len() > 256 {
            eprintln!(
                "[value-trace] {} signals watched — narrow the patterns if the output drowns",
                ids.len()
            );
        }
        self.value_trace_ids = Some(ids);
    }

    #[inline(always)]
    pub(super) fn after_signal_write(&mut self, id: usize) {
        self.after_signal_write_inner(id, false)
    }

    /// `after_signal_write` for callers that have ALREADY refreshed the
    /// `signal_inline_bits` mirror for `id` — skips the raw_bits repack
    /// (a 64-LogicBit walk on every wide signal).
    #[inline(always)]
    pub(super) fn after_signal_write_premirrored(&mut self, id: usize) {
        self.after_signal_write_inner(id, true)
    }

    #[inline(always)]
    pub(super) fn after_signal_write_inner(&mut self, id: usize, premirrored: bool) {
        if id >= self.signal_table.len() {
            return;
        }
        let (edge_sidecar, armed_sidecar) = if self.signal_commit_plan.is_empty() {
            // Quiet signal (no armed / edge readers): nothing below applies
            // unless a global observer is active.
            if self.quiet_valid
                && (self.quiet_bits[id >> 6] >> (id & 63)) & 1 != 0
                && self.value_trace_ids.is_none()
                && !self.dump_dirty_active
                && self.active_force_exprs.is_empty()
                && (premirrored || self.signal_inline_bits.is_empty())
                && !(self.event_measure && !self.armed_edge)
            {
                if self.profile_report {
                    self.prof_quiet_stores += 1;
                }
                return;
            }
            if self.profile_report {
                self.prof_observed_stores += 1;
            }
            (true, true)
        } else {
            let plan = self.signal_commit_plan.get(id).copied().unwrap_or(3);
            let edge = plan & 1 != 0
                && (self.dirty_edge
                    || self.dirty_edge_shadow
                    || self.in_edge_block
                    || self.in_edge_cont);
            let armed = plan & 2 != 0 && self.armed_edge;
            let global = self.value_trace_ids.is_some()
                || self.dump_dirty_active
                || !self.active_force_exprs.is_empty()
                || (!premirrored && !self.signal_inline_bits.is_empty())
                || (self.event_measure && !self.armed_edge);
            if !edge && !armed && !global {
                return;
            }
            (edge, armed)
        };
        self.after_signal_write_observed(id, premirrored, edge_sidecar, armed_sidecar);
    }

    #[inline]
    pub(super) fn after_signal_write_observed(
        &mut self,
        id: usize,
        premirrored: bool,
        edge_sidecar: bool,
        armed_sidecar: bool,
    ) {
        // XEZIM_VALUE_TRACE: this hook covers the write paths that mutate
        // signal_table in place (bit/part-select, inline-bits, parallel-NBA
        // raw pointers) and so never expand write_sig!. The pre-write value
        // is gone here; queue old==new for the arrowless print form.
        if let Some(vt_ids) = &self.value_trace_ids {
            if vt_ids.contains(&id) && id < self.signal_table.len() {
                let v = self.signal_table[id].clone();
                self.value_trace_pending.borrow_mut().push((
                    id,
                    self.time,
                    v.clone(),
                    v,
                    self.current_pid,
                    self.edge_dispatch_phase,
                ));
            }
        }
        if armed_sidecar {
            self.note_armed_write(id);
        }
        if edge_sidecar {
            self.note_edge_write(id);
        }
        // Incremental VCD: this is the post-write hook for the direct
        // signal_table writes (bit/part-select, struct/array element) and the
        // NBA/vpi fast paths, so mark the id dirty here too. Superset-safe.
        vcd_mark!(self, id);
        // Direct/in-place write paths reach this hook after the old value is
        // gone. Treat the write as a generation change while an expression-
        // backed override is active. A redundant wakeup is conservative; the
        // dependency filter still avoids all unrelated RHS evaluations.
        if !self.active_force_exprs.is_empty() && id < self.active_force_signal_epochs.len() {
            self.active_force_signal_epochs[id] =
                self.active_force_signal_epochs[id].wrapping_add(1);
            self.force_epoch_gen = self.force_epoch_gen.wrapping_add(1);
        }
        // `raw_bits()` is NOT free: its `Wide` arm (any signal over 64 bits)
        // re-packs up to 64 `LogicBit` bytes one at a time. Its ONLY consumer
        // here is the compact `signal_inline_bits` mirror, which is allocated
        // solely under XEZIM_INLINE_BITS=1 and is EMPTY otherwise — so
        // computing it before the length test ran that repack loop on every
        // wide signal write and then threw the answer away. (LLVM cannot sink
        // it itself: the mirror's length is a runtime value.) Moving the call
        // inside the guard makes the disabled case a single load+compare.
        // Callers that already refreshed the mirror themselves use
        // `after_signal_write_premirrored` to skip this repack.
        if !premirrored && id < self.signal_inline_bits.len() {
            let (new_v, new_x) = self.signal_table[id].raw_bits();
            self.signal_inline_bits[id] = [new_v, new_x];
        }
        // O1 measurement: stamp fast-path (set_inline_bits) writes too.
        if self.event_measure && !self.armed_edge && id < self.sig_last_change.len() {
            self.sig_last_change[id] = self.event_phase;
        }
        // Value-change callback dispatch lives in the write_sig! macro
        // so every write path sees it. after_signal_write is only
        // invoked from NBA writeback / vpi_put_value paths that have
        // already passed through write_sig! — firing here would cause
        // double-counting on those paths.
    }

    /// O1 (event-driven-edge) MEASUREMENT setup. Builds per-edge-block static
    /// data-read sets (LoadSignal sids minus the block's edge sensitivity),
    /// marks blocks with dynamic reads non-gateable, and picks the main clock
    /// (the posedge edge-signal with the most flop fanout). No behavior change.
    /// (v, x) raw bits of a `w`-bit slice of `signal_table[sid]` starting at
    /// bit `lo`. For narrow signals this is a shift of `raw_bits`; for wide
    /// ones the (<=64) bits are gathered individually — only slice entries of
    /// gateable blocks ever take that path, and they are 1-few bits wide.
    #[inline]
    pub(super) fn raw_bits_slice(v: &Value, lo: u16, w: u16) -> (u64, u64) {
        let lo = lo as usize;
        let w = w as usize;
        if v.width as usize <= 64 {
            let (rv, rx) = v.raw_bits();
            let mask = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
            ((rv >> lo) & mask, (rx >> lo) & mask)
        } else {
            // SWAR-chunked in core (out-of-range bits read 0/0, matching the
            // get_bit loop this replaces: get_bit past the width is Zero).
            v.slice_bits_swar(lo, w)
        }
    }

    /// Signal ids, and dense array element ranges `(first_id, len)`, that
    /// procedural code OUTSIDE the edge blocks and the settle entries can
    /// write: the processes still queued at event-loop start (initial
    /// blocks, always blocks run as processes, static initializers),
    /// process FSMs, program blocks, class methods, and the module tasks
    /// and functions those call by statement. Names resolve scope-first,
    /// then as written; a name that resolves nowhere is not something an
    /// edge block writes either.
    ///
    /// A task called only from edge blocks is part of those blocks (their
    /// compiled code carries its writes), so it is not walked unless a
    /// process calls it. The leading statements of a time-0 process that
    /// cannot suspend or defer (`initial begin q = 0; mem[i] = 0; ... end`)
    /// run before time advances; their writes are left out and instead
    /// `edge_skip_t0_refresh` is set so the first time advance invalidates
    /// every snapshot once (`refresh_edge_skip_after_t0`).
    pub(super) fn foreign_procedural_writes(
        &mut self,
        top_prefix: &str,
    ) -> (HashSet<usize>, HashSet<(usize, usize)>) {
        use super::bytecode::{ArrayOperand, Insn};
        type Out = (HashSet<usize>, HashSet<(usize, usize)>);
        let mut out: Out = (HashSet::default(), HashSet::default());
        let note = |name: &str, scope: &str, out: &mut Out| {
            let scoped = (!scope.is_empty()).then(|| format!("{}.{}", scope, name));
            for cand in scoped.as_deref().into_iter().chain(std::iter::once(name)) {
                let dense = self.array_first_id.get(cand).or_else(|| {
                    self.array_first_id
                        .get(format!("{}{}", top_prefix, cand).as_str())
                });
                if let Some(&(first, lo, hi)) = dense {
                    let len = ((hi - lo).max(0) as usize).saturating_add(1);
                    out.1.insert((first, len));
                } else if let Some(&(lo, hi, _)) = self.module.arrays.get(cand) {
                    for i in lo..=hi {
                        if let Some(id) =
                            self.resolve_read_name(&format!("{}[{}]", cand, i), top_prefix)
                        {
                            out.0.insert(id);
                        }
                    }
                } else if let Some(id) = self.resolve_read_name(cand, top_prefix) {
                    out.0.insert(id);
                }
            }
        };
        // Statement-position task/function calls: (raw dotted name, scope).
        let mut calls: Vec<(String, String)> = Vec::new();
        let note_stmt = |stmt: &Statement, scope: &str, out: &mut Out, calls: &mut Vec<_>| {
            for name in super::elaborate::stmt_write_targets(stmt).1 {
                note(&name, scope, out);
            }
            Self::collect_stmt_call_names(stmt, &mut |raw| {
                calls.push((raw.to_string(), scope.to_string()))
            });
        };
        let mut t0_prefix_skipped = false;
        let now = self.time;
        let mut queued: Vec<(usize, u64, &ProcCont)> = Vec::new();
        self.event_queue
            .for_each_pending(|pid, t, c| queued.push((pid, t, c)));
        for (pid, t, cont) in queued {
            let scope = self
                .process_scope_hint
                .get(&pid)
                .map(String::as_str)
                .unwrap_or("");
            let mut in_prefix = t == 0 && now == 0;
            let mut frame = Some(cont);
            while let Some(c) = frame {
                for st in c.stmts.iter() {
                    if in_prefix && self.stmt_is_instant(st) {
                        t0_prefix_skipped = true;
                        continue;
                    }
                    in_prefix = false;
                    note_stmt(st, scope, &mut out, &mut calls);
                }
                frame = c.next.as_deref();
            }
        }
        for (stmts, scope) in &self.pending_reactive {
            for st in stmts {
                note_stmt(st, scope, &mut out, &mut calls);
            }
        }
        for fsm in self.proc_fsm.values() {
            for insn in &fsm.compiled.instructions {
                match insn {
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
                    | Insn::NbaAssignArrayRead(id, ..) => {
                        out.0.insert(*id as usize);
                    }
                    Insn::NbaAssignArray(arr, ..)
                    | Insn::NbaAssignArrayRange(arr, ..)
                    | Insn::BlockingAssignArray(arr, ..)
                    | Insn::BlockingAssignArrayRange(arr, ..) => match &**arr {
                        ArrayOperand::Dense {
                            first_id, lo, hi, ..
                        } => {
                            let len = ((*hi - *lo).max(0) as usize).saturating_add(1);
                            out.1.insert((*first_id, len));
                        }
                        ArrayOperand::Named(name) => note(name, &fsm.scope, &mut out),
                    },
                    Insn::StmtFallback(payload) => {
                        note_stmt(&payload.0, &fsm.scope, &mut out, &mut calls)
                    }
                    Insn::CallFreeFunction(_, name, ..) => {
                        calls.push((name.0.to_string(), fsm.scope.clone()))
                    }
                    _ => {}
                }
            }
        }
        // Class methods run only from processes (or other methods).
        {
            use crate::ast::decl::ClassMethodKind as CMK;
            for c in self.module.classes.values() {
                for m in c.methods.values() {
                    let items = match &m.kind {
                        CMK::Function(f) | CMK::Extern(f) | CMK::PureVirtual(f) => &f.items,
                        CMK::Task(t) => &t.items,
                    };
                    for st in items {
                        note_stmt(st, "", &mut out, &mut calls);
                    }
                }
            }
        }
        // Module tasks/functions reachable from the code above, matched the
        // way `stmt_calls_blocking_task` resolves a callee (scoped, as
        // written, scoped leaf) plus any `*.leaf` key, an over-approximation.
        let mut walked: HashSet<String> = HashSet::default();
        while let Some((raw, scope)) = calls.pop() {
            let leaf = raw.rsplit('.').next().unwrap_or(&raw).to_string();
            let mut keys: Vec<String> = Vec::new();
            for cand in [
                (!scope.is_empty()).then(|| format!("{}.{}", scope, raw)),
                Some(raw.clone()),
                (!scope.is_empty()).then(|| format!("{}.{}", scope, leaf)),
            ]
            .into_iter()
            .flatten()
            {
                if self.module.tasks.contains_key(cand.as_str())
                    || self.module.functions.contains_key(cand.as_str())
                {
                    keys.push(cand);
                }
            }
            let dotted = format!(".{}", leaf);
            keys.extend(
                self.module
                    .tasks
                    .keys()
                    .chain(self.module.functions.keys())
                    .filter(|k| k.ends_with(dotted.as_str()))
                    .map(|k| k.to_string()),
            );
            for key in keys {
                if !walked.insert(key.clone()) {
                    continue;
                }
                let body_scope = key.rsplit_once('.').map_or("", |(s, _)| s).to_string();
                let items = self
                    .module
                    .tasks
                    .get(key.as_str())
                    .map(|t| &t.items)
                    .or_else(|| self.module.functions.get(key.as_str()).map(|f| &f.items));
                for st in items.into_iter().flatten() {
                    note_stmt(st, &body_scope, &mut out, &mut calls);
                }
            }
        }
        if t0_prefix_skipped {
            self.edge_skip_t0_refresh = true;
        }
        out
    }

    /// A statement that runs to completion in the instant it starts, writing
    /// nothing later: assignments without intra-assignment timing, system
    /// task calls, declarations, and control flow made only of those. Any
    /// timing control, wait, fork, event trigger or user subroutine call
    /// disqualifies it (a callee may suspend).
    fn stmt_is_instant(&self, stmt: &Statement) -> bool {
        match &stmt.kind {
            StatementKind::Null | StatementKind::VarDecl { .. } => true,
            StatementKind::BlockingAssign { rvalue, .. } => !Self::intra_timing_suspends(rvalue),
            StatementKind::NonblockingAssign { delay, rvalue, .. } => {
                delay.is_none() && !Self::intra_timing_suspends(rvalue)
            }
            StatementKind::Expr(e) => matches!(e.kind, ExprKind::SystemCall { .. }),
            StatementKind::If {
                then_stmt,
                else_stmt,
                ..
            } => {
                self.stmt_is_instant(then_stmt)
                    && else_stmt.as_ref().is_none_or(|e| self.stmt_is_instant(e))
            }
            StatementKind::Case { items, .. } => {
                items.iter().all(|it| self.stmt_is_instant(&it.stmt))
            }
            StatementKind::For { body, .. }
            | StatementKind::While { body, .. }
            | StatementKind::DoWhile { body, .. }
            | StatementKind::Repeat { body, .. }
            | StatementKind::Foreach { body, .. } => self.stmt_is_instant(body),
            StatementKind::SeqBlock { stmts, .. } => stmts.iter().all(|s| self.stmt_is_instant(s)),
            _ => false,
        }
    }

    /// Calls in statement position (`t(x);`, `t;`, `obj.m();`), reported as
    /// the dotted callee name, through every nested statement.
    fn collect_stmt_call_names(stmt: &Statement, f: &mut dyn FnMut(&str)) {
        let callee_name = |e: &Expression| -> Option<String> {
            let target = match &e.kind {
                ExprKind::Call { func, .. } => &**func,
                _ => e,
            };
            match &target.kind {
                ExprKind::Ident(h) => Some(
                    h.path
                        .iter()
                        .map(|s| s.name.name.as_str())
                        .collect::<Vec<_>>()
                        .join("."),
                ),
                ExprKind::MemberAccess { member, .. } => Some(member.name.clone()),
                _ => None,
            }
        };
        match &stmt.kind {
            StatementKind::Expr(e) => {
                if let Some(n) = callee_name(e) {
                    f(&n);
                }
            }
            StatementKind::If {
                then_stmt,
                else_stmt,
                ..
            } => {
                Self::collect_stmt_call_names(then_stmt, f);
                if let Some(e) = else_stmt {
                    Self::collect_stmt_call_names(e, f);
                }
            }
            StatementKind::Case { items, .. } => {
                for it in items {
                    Self::collect_stmt_call_names(&it.stmt, f);
                }
            }
            StatementKind::For { body, .. }
            | StatementKind::While { body, .. }
            | StatementKind::DoWhile { body, .. }
            | StatementKind::Repeat { body, .. }
            | StatementKind::Forever { body }
            | StatementKind::ForeverTail { body }
            | StatementKind::Foreach { body, .. }
            | StatementKind::ForeachTail { body, .. } => Self::collect_stmt_call_names(body, f),
            StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
                for st in stmts {
                    Self::collect_stmt_call_names(st, f);
                }
            }
            StatementKind::TimingControl { stmt, .. } | StatementKind::Wait { stmt, .. } => {
                Self::collect_stmt_call_names(stmt, f)
            }
            StatementKind::RandCase { items } => {
                for (_, st) in items {
                    Self::collect_stmt_call_names(st, f);
                }
            }
            _ => {}
        }
    }

    /// One-shot follow-up to the time-0 writes `foreign_procedural_writes`
    /// left out of the writer census: on the first time advance, make every
    /// gateable edge block run on its next event (the same invalidation a
    /// `release` performs). One branch per time advance otherwise.
    #[inline]
    pub(super) fn refresh_edge_skip_after_t0(&mut self) {
        if self.edge_skip_t0_refresh {
            self.edge_skip_t0_refresh = false;
            self.edge_block_snap_valid
                .iter_mut()
                .for_each(|v| *v = false);
            self.edge_block_armed
                .iter_mut()
                .for_each(|v| *v |= EDGE_ARMED);
        }
    }

    pub(super) fn build_event_measure_state(&mut self) {
        use super::bytecode::Insn;
        let nb = self.compiled_edge_blocks.len();
        // §9.2: unchanged inputs do not imply an unchanged output when another
        // edge process drives it. Count per-block writers, not instructions:
        // several conditional assignments in one process are still one owner.
        let mut block_writes = vec![HashSet::default(); nb];
        // Which BITS of each destination a block claims. Two blocks writing
        // one register are only two drivers of the same thing where their
        // slices overlap: a generate that gives `entry_expt[0]` its own arm
        // beside one for `entry_expt[6:1]` has each bit driven once, and both
        // arms may still skip an idle edge. Counting the signal was enough to
        // reject 832 blocks on a C910 SoC. Bit 1 of `q[2:1]` and `q[1:0]`, on
        // the other hand, has two drivers, and neither of those may skip — so
        // the test is overlap, not whether the write is partial. A slice whose
        // bounds are only known at run time claims the whole signal.
        let mut block_spans: Vec<Vec<(usize, u32, u32)>> = vec![Vec::new(); nb];
        // A whole-array write is kept as its element RANGE. Expanding it to
        // ids cost 19M hash inserts on a c906 SoC (1.8 s, ~400 MB) only to
        // ask "does anything else write this"; the counter below answers that
        // from a byte per signal, and the ranges are scanned in place.
        let mut block_arrays: Vec<Vec<(usize, usize)>> = vec![Vec::new(); nb];
        // Saturating at two: the only question asked of it is `> 1`.
        let mut writer_counts = vec![0u8; self.signal_widths.len()];
        let whole_hi = |widths: &[u32], id: usize| -> u32 {
            widths.get(id).copied().unwrap_or(1).saturating_sub(1)
        };
        let bump = |counts: &mut Vec<u8>, id: usize| {
            if let Some(c) = counts.get_mut(id) {
                *c = c.saturating_add(1);
            }
        };
        let top_prefix = format!("{}.", self.module.name);
        let retired: Vec<bool> = {
            let mut v = self.edge_block_retired.clone();
            v.resize(nb, false);
            v
        };
        for (bi, block) in self.edge_blocks.iter().enumerate().take(nb) {
            if retired[bi] {
                // Folded into a merged block, which already carries these
                // writes; counting them again would give every merged output
                // a second driver.
                continue;
            }
            let mut needs_ast = self.compiled_edge_blocks[bi].is_none();
            if let Some(cb) = &self.compiled_edge_blocks[bi] {
                for insn in &cb.instructions {
                    match insn {
                        Insn::NbaAssign(id, ..)
                        | Insn::NbaAssignConst(id, ..)
                        | Insn::NbaAssignArrayRead(id, ..)
                        | Insn::BlockingAssign(id, ..)
                        | Insn::NbaAssignRangeDyn(id, ..)
                        | Insn::NbaAssignBitDyn(id, ..)
                        | Insn::BlockingAssignRangeDyn(id, ..)
                        | Insn::BlockingAssignBitDyn(id, ..) => {
                            let id = *id as usize;
                            block_writes[bi].insert(id);
                            block_spans[bi].push((id, 0, whole_hi(&self.signal_widths, id)));
                        }
                        Insn::NbaAssignRange(id, hi, lo, _)
                        | Insn::BlockingAssignRange(id, hi, lo, _) => {
                            let id = *id as usize;
                            block_writes[bi].insert(id);
                            block_spans[bi].push((id, *lo.min(hi), *lo.max(hi)));
                        }
                        Insn::StmtFallback(..)
                        | Insn::EvalExprFallback(..)
                        | Insn::NbaAssignArray(..)
                        | Insn::BlockingAssignArray(..)
                        | Insn::NbaAssignArrayRange(..)
                        | Insn::BlockingAssignArrayRange(..) => {
                            needs_ast = true;
                        }
                        _ => {}
                    }
                }
            }
            // Compiled signal ids avoid another hierarchical AST walk for
            // ordinary flops. Keep interpreted writers in the census too.
            if needs_ast {
                // Only the WRITE set is used here, and a whole-array write is
                // a contiguous id range: collecting element names (and read
                // names that are then dropped) cost 33 s on a c906 SoC, all of
                // it in `String` building and hashing for the SRAM models.
                let mut writes = HashSet::default();
                let mut arrays: HashSet<String> = HashSet::default();
                Self::collect_stmt_rw(
                    &block.stmt,
                    &self.module,
                    None,
                    &mut writes,
                    Some(&mut arrays),
                );
                for name in writes {
                    if let Some(id) = self.resolve_read_name(&name, &top_prefix) {
                        block_writes[bi].insert(id);
                        // A name carries no slice, so it claims every bit.
                        block_spans[bi].push((id, 0, whole_hi(&self.signal_widths, id)));
                    }
                }
                for name in arrays {
                    let ids = self
                        .array_first_id
                        .get(name.as_str())
                        .or_else(|| {
                            self.array_first_id
                                .get(format!("{}{}", top_prefix, name).as_str())
                        })
                        .copied();
                    match ids {
                        Some((first, lo, hi)) => {
                            let len = ((hi - lo).max(0) as usize).saturating_add(1);
                            block_arrays[bi].push((first, len));
                        }
                        // Not a dense array after all: fall back to the
                        // per-element names, which is what this path did
                        // before, so the writer census stays exact.
                        None => {
                            if let Some(&(lo, hi, _)) = self.module.arrays.get(&name) {
                                for i in lo..=hi {
                                    if let Some(id) = self
                                        .resolve_read_name(&format!("{}[{}]", name, i), &top_prefix)
                                    {
                                        block_writes[bi].insert(id);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            for &id in &block_writes[bi] {
                bump(&mut writer_counts, id);
            }
            for &(first, len) in &block_arrays[bi] {
                let end = first.saturating_add(len).min(writer_counts.len());
                for c in &mut writer_counts[first.min(end)..end] {
                    *c = c.saturating_add(1);
                }
            }
        }
        for entry in &self.comb_entries {
            for &id in &entry.cold.write_signal_ids {
                bump(&mut writer_counts, id);
            }
        }
        // §9.2.2/§10.4.1: a PROCESS writing a block's output breaks the same
        // invariant. `always @(a) ext = 1;` with a testbench `ext = 0;`
        // between events reads no data input, so every later event of `a`
        // was skipped as "unchanged" and `ext` stayed 0; a flop whose `q` a
        // testbench overwrote kept the overwritten value across its next
        // clock edge. Count those writers like any other driver.
        let (foreign_ids, foreign_arrays) = self.foreign_procedural_writes(&top_prefix);
        for &id in &foreign_ids {
            bump(&mut writer_counts, id);
        }
        for &(first, len) in &foreign_arrays {
            let end = first.saturating_add(len).min(writer_counts.len());
            for c in &mut writer_counts[first.min(end)..end] {
                *c = c.saturating_add(1);
            }
        }
        // Who else claims the bits of a multiply driven signal. Only those
        // signals can conflict, so a design where every register has one
        // driver — the ordinary case — builds an empty index.
        let mut spans_by_sig: HashMap<usize, Vec<(usize, u32, u32)>> = HashMap::default();
        for (bi, spans) in block_spans.iter().enumerate() {
            for &(id, lo, hi) in spans {
                if writer_counts.get(id).is_some_and(|&c| c > 1) {
                    spans_by_sig.entry(id).or_default().push((bi, lo, hi));
                }
            }
        }
        // A settle-region writer drives the whole signal and is never one of
        // the edge blocks, so it conflicts with every span of it. So does a
        // process writer (its names carry no slice).
        for &id in self
            .comb_entries
            .iter()
            .flat_map(|e| e.cold.write_signal_ids.iter())
            .chain(foreign_ids.iter())
        {
            if let Some(v) = spans_by_sig.get_mut(&id) {
                v.push((usize::MAX, 0, whole_hi(&self.signal_widths, id)));
            }
        }
        // A whole-array claim covers every element id in its range; it is
        // kept as the range rather than expanded (see `block_arrays`), so the
        // span test below scans these for an element another block writes
        // by constant index. A `mem[0] <= x` beside a `mem[p] <= y` must not
        // skip: once p reaches 0, its element depends on which block ran
        // last.
        let array_claims: Vec<(usize, usize, usize)> = block_arrays
            .iter()
            .enumerate()
            .flat_map(|(bj, v)| {
                v.iter()
                    .map(move |&(first, len)| (bj, first, first.saturating_add(len)))
            })
            .chain(
                foreign_arrays
                    .iter()
                    .map(|&(first, len)| (usize::MAX, first, first.saturating_add(len))),
            )
            .collect();
        let mut data_reads: Vec<Vec<u32>> = vec![Vec::new(); nb];
        let mut data_metas: Vec<Vec<(u16, u16)>> = vec![Vec::new(); nb];
        let mut gateable: Vec<bool> = vec![false; nb];
        // Census of non-gateable reasons (XEZIM_EVENT_EDGE_CENSUS=1):
        // 0 uncompiled, 1 range-oob, 2 LoadArrayElem, 3 NbaAssignArrayRead,
        // 4 array write, 5 opaque (StmtFallback), 6 wide-without-armed,
        // 7 another block claims the same bits, 8 another block claims the
        // same array.
        let mut gate_census = [0usize; 9];
        let mut arm_extra: Vec<Vec<u32>> = vec![Vec::new(); nb];
        let mut arm_only: Vec<bool> = vec![false; nb];
        let mut wide_read: Vec<bool> = vec![false; nb];
        // Subtract ONLY true clock-generator signals (they toggle every cycle,
        // so they'd never let a flop skip). Keep reset/enable/gated-clock
        // reads in the change-check: a reset or enable CHANGE must ungate the
        // flop (correctness), and a gated clock toggling fires it (safe — it
        // just means gated-clock flops don't skip).
        let clock_sids: HashSet<u32> = self
            .clock_generators
            .iter()
            .map(|c| c.signal_id as u32)
            .collect();
        for bi in 0..nb {
            if retired[bi] {
                continue;
            }
            let Some(cb) = self.compiled_edge_blocks[bi].as_ref() else {
                gate_census[0] += 1;
                continue;
            };
            let mut reads: Vec<u32> = Vec::new();
            let mut metas: Vec<(u16, u16)> = Vec::new();
            let mut seen: HashSet<(u32, u16, u16)> = HashSet::default();
            let mut dynamic = false;
            let mut dyn_kind = 0usize;
            let mut arm_extra_b: Vec<u32> = Vec::new();
            let mut arm_only_b = false;
            let mut arm_seen: HashSet<usize> = HashSet::default();
            let armed = self.armed_edge;
            // A block that escapes to the AST interpreter (`StmtFallback`, e.g.
            // its only effect is `$display`/`$monitor`) has side effects and
            // reads the bytecode can't see. Gating it on "no tracked data-input
            // change" would drop every fire after the first (its read set is
            // empty), so such a block must never be gateable.
            let mut opaque = false;
            for insn in &cb.instructions {
                match insn {
                    // A counted block must run on every edge.
                    Insn::StmtFallback(..) | Insn::EvalExprFallback(..) | Insn::CovHit(..) => {
                        opaque = true
                    }
                    // Constant bit/part selects record only the SLICE they
                    // read (see `edge_block_reads_meta`); everything else is
                    // a full-signal read.
                    Insn::LoadSignalBit(_, s, idx) if *idx <= u16::MAX as u32 => {
                        if seen.insert((*s as u32, *idx as u16, 1)) {
                            reads.push(*s as u32);
                            metas.push((*idx as u16, 1));
                        }
                    }
                    Insn::LoadSignalRange(_, s, l, r) => {
                        let lo = (*l).min(*r);
                        let w = (*l).max(*r) - lo + 1;
                        if lo <= u16::MAX as u32 && w <= u16::MAX as u32 {
                            if seen.insert((*s as u32, lo as u16, w as u16)) {
                                reads.push(*s as u32);
                                metas.push((lo as u16, w as u16));
                            }
                        } else {
                            dynamic = true;
                            dyn_kind = 1;
                        }
                    }
                    Insn::LoadSignal(_, s)
                    | Insn::LoadSignalSigned(_, s)
                    | Insn::LoadSignalBit(_, s, _)
                    | Insn::BranchIfSignalFalse(s, _, _) => {
                        let w = self.signal_widths[*s as usize].min(u16::MAX as u32) as u16;
                        if seen.insert((*s as u32, 0, w)) {
                            reads.push(*s as u32);
                            metas.push((0, w));
                        }
                    }
                    // Partial (read-modify-write) assigns read the destination
                    // signal's current value as base, but NOT via a LoadSignal —
                    // so the preserved bits are an implicit input. Treat the dest
                    // as a read, else a change to the kept bits (e.g. another
                    // driver) would be false-skipped.
                    Insn::NbaAssignRange(s, ..)
                    | Insn::NbaAssignRangeDyn(s, ..)
                    | Insn::NbaAssignBitDyn(s, ..)
                    | Insn::BlockingAssignRange(s, ..)
                    | Insn::BlockingAssignRangeDyn(s, ..)
                    | Insn::BlockingAssignBitDyn(s, ..) => {
                        let w = self.signal_widths[*s as usize].min(u16::MAX as u32) as u16;
                        if seen.insert((*s as u32, 0, w)) {
                            reads.push(*s as u32);
                            metas.push((0, w));
                        }
                    }
                    Insn::LoadArrayElem(_, arr, _) => {
                        if Self::arm_array_elems(arr, armed, &mut arm_extra_b, &mut arm_seen) {
                            arm_only_b = true;
                        } else {
                            dynamic = true;
                            dyn_kind = 2;
                        }
                    }
                    // Fused `LoadSignal ; LoadArrayElem ; NbaAssign`. It READS
                    // the index signal (register it exactly as the LoadSignal
                    // it replaced would have) and WRITES the destination
                    // whole-width (so, exactly like plain `NbaAssign`, the
                    // destination is NOT an implicit read). The element read is
                    // dynamically addressed, so — as for the `LoadArrayElem` it
                    // replaced — the block must stay non-gateable.
                    Insn::NbaAssignArrayRead(_, arr, s, _) => {
                        let w = self.signal_widths[*s as usize].min(u16::MAX as u32) as u16;
                        if seen.insert((*s as u32, 0, w)) {
                            reads.push(*s as u32);
                            metas.push((0, w));
                        }
                        if Self::arm_array_elems(arr, armed, &mut arm_extra_b, &mut arm_seen) {
                            arm_only_b = true;
                        } else {
                            dynamic = true;
                            dyn_kind = 3;
                        }
                    }
                    // Array-element NBA/blocking writes (memory) use a dynamic
                    // index; conservatively make such blocks non-gateable.
                    Insn::NbaAssignArray(arr, ..)
                    | Insn::NbaAssignArrayRange(arr, ..)
                    | Insn::BlockingAssignArray(arr, ..)
                    | Insn::BlockingAssignArrayRange(arr, ..) => {
                        if Self::arm_array_elems(arr, armed, &mut arm_extra_b, &mut arm_seen) {
                            arm_only_b = true;
                        } else {
                            dynamic = true;
                            dyn_kind = 4;
                        }
                    }
                    _ => {}
                }
            }
            if dynamic {
                gate_census[dyn_kind] += 1;
            }
            arm_extra[bi] = arm_extra_b;
            arm_only[bi] = arm_only_b;
            if opaque {
                gate_census[5] += 1;
            }
            // Drop only true clock signals; keep reset/enable/data.
            {
                let mut i = 0;
                while i < reads.len() {
                    if clock_sids.contains(&reads[i]) {
                        reads.swap_remove(i);
                        metas.swap_remove(i);
                    } else {
                        i += 1;
                    }
                }
            }
            // Wide values cannot use the compact snapshot. Write-armed mode
            // can still gate them safely: no input write permits a skip, and
            // any input write forces execution without a value comparison.
            let has_wide = metas.iter().any(|&(_, w)| w > 64);
            if has_wide && !self.armed_edge {
                gate_census[6] += 1;
            }
            // Another driver of the same BITS, rather than merely of the same
            // signal. An overlap makes this block's output depend on when the
            // other one last ran, so it must fire on every edge.
            let span_shared = block_spans[bi].iter().any(|&(id, lo, hi)| {
                spans_by_sig.get(&id).is_some_and(|others| {
                    others
                        .iter()
                        .any(|&(bj, olo, ohi)| bj != bi && lo <= ohi && olo <= hi)
                }) || (writer_counts.get(id).is_some_and(|&c| c > 1)
                    && array_claims
                        .iter()
                        .any(|&(bj, first, end)| bj != bi && first <= id && id < end))
            });
            let array_shared = block_arrays[bi].iter().any(|&(first, len)| {
                let end = first.saturating_add(len).min(writer_counts.len());
                writer_counts[first.min(end)..end].iter().any(|&c| c > 1)
            });
            if span_shared {
                gate_census[7] += 1;
            } else if array_shared {
                gate_census[8] += 1;
            }
            let shared_output = span_shared || array_shared;
            gateable[bi] = !dynamic && !opaque && !shared_output && (!has_wide || self.armed_edge);
            wide_read[bi] = has_wide;
            data_reads[bi] = reads;
            data_metas[bi] = metas;
        }
        for bi in 0..nb {
            if !gateable[bi] {
                arm_only[bi] = false;
                arm_extra[bi].clear();
                wide_read[bi] = false;
            }
        }
        self.edge_block_gateable = gateable;
        self.edge_block_arm_only = arm_only;
        self.edge_block_wide_read = wide_read;
        if std::env::var_os("XEZIM_EVENT_EDGE_CENSUS").is_some() {
            let arm_only_n = self.edge_block_arm_only.iter().filter(|&&a| a).count();
            let arm_elems: usize = arm_extra.iter().map(|v| v.len()).sum();
            let wide_n = self.edge_block_wide_read.iter().filter(|&&a| a).count();
            eprintln!(
                "[EVENT-EDGE-CENSUS] non-gateable: uncompiled={} range_oob={} load_array_elem={} nba_array_read={} array_write={} opaque={} wide_unarmed={} shared_bits={} shared_array={}; arm-only blocks={} (element inputs={}); gateable wide-read blocks={}",
                gate_census[0],
                gate_census[1],
                gate_census[2],
                gate_census[3],
                gate_census[4],
                gate_census[5],
                gate_census[6],
                gate_census[7],
                gate_census[8],
                arm_only_n,
                arm_elems,
                wide_n
            );
        }
        let tracked_signal_len = if self.armed_edge {
            0
        } else {
            (0..nb)
                .filter(|&bi| self.edge_block_gateable[bi])
                .flat_map(|bi| data_reads[bi].iter().copied())
                .max()
                .map_or(0, |sid| sid as usize + 1)
        };
        self.sig_last_change = vec![0u64; tracked_signal_len];
        self.flop_last_fire = if self.armed_edge {
            Vec::new()
        } else {
            vec![0u64; nb]
        };
        let gateable_n = self.edge_block_gateable.iter().filter(|&&g| g).count();
        let empty_reads = (0..nb)
            .filter(|&bi| self.edge_block_gateable[bi] && data_reads[bi].is_empty())
            .count();
        let total_reads: usize = (0..nb)
            .filter(|&bi| self.edge_block_gateable[bi])
            .map(|bi| data_reads[bi].len())
            .sum();
        if self.event_skip {
            // Flatten gateable per-block reads into a CSR for cache locality.
            // Non-gateable blocks contribute zero entries (offsets equal).
            let mut off: Vec<u32> = Vec::with_capacity(nb + 1);
            let mut flat_reads: Vec<u32> = Vec::new();
            let mut flat_metas: Vec<(u16, u16)> = Vec::new();
            off.push(0);
            for bi in 0..nb {
                if self.edge_block_gateable[bi] {
                    flat_reads.extend_from_slice(&data_reads[bi]);
                    flat_metas.extend_from_slice(&data_metas[bi]);
                }
                off.push(flat_reads.len() as u32);
            }
            let total = flat_reads.len();
            self.edge_block_off = off;
            self.edge_block_reads_flat = flat_reads;
            self.edge_block_reads_meta = flat_metas;
            self.edge_block_snap_flat = vec![(0u64, 0u64); total];
            self.edge_block_snap_valid = vec![false; nb];
            self.edge_block_change_streak = vec![0; nb];
            self.edge_block_epoch_probe_left = vec![0; nb];
            self.edge_block_skip_streak = vec![0; nb];
            self.event_heal = std::env::var("XEZIM_EVENT_EDGE_HEAL")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(16);
            // The timestamp-only measurement path needs the nested read sets;
            // normal skip mode uses only the compact CSR above.
            self.edge_block_data_reads = Vec::new();
        } else {
            self.edge_block_data_reads = data_reads;
            self.edge_block_off = Vec::new();
            self.edge_block_reads_flat = Vec::new();
            self.edge_block_reads_meta = Vec::new();
            self.edge_block_snap_flat = Vec::new();
            self.edge_block_snap_valid = Vec::new();
            self.edge_block_change_streak = Vec::new();
            self.edge_block_epoch_probe_left = Vec::new();
            self.edge_block_skip_streak = Vec::new();
        }
        self.build_armed_edge_state(&arm_extra);
        self.rebuild_signal_commit_plans();
        chatter!(
            "[EVENT-EDGE] measure (timestamp): {} edge blocks, {} gateable, {} gateable-with-EMPTY-data-reads, avg data-reads/gateable={:.2}",
            nb,
            gateable_n,
            empty_reads,
            if gateable_n > 0 {
                total_reads as f64 / gateable_n as f64
            } else {
                0.0
            },
        );
    }

    /// Register every element of a dense unpacked array as an arm-only input
    /// of the block (see `edge_block_arm_only`). Returns false when the block
    /// must stay non-gateable: write-arming off, a name-resolved (non-dense)
    /// array, a packed-arena array (its element writes bypass `write_sig!`),
    /// or an array too large to fan out per element.
    pub(super) fn arm_array_elems(
        arr: &super::bytecode::ArrayOperand,
        armed: bool,
        out: &mut Vec<u32>,
        seen: &mut HashSet<usize>,
    ) -> bool {
        const MAX_ARM_ELEMS: i64 = 1 << 16;
        if !armed {
            return false;
        }
        match arr {
            super::bytecode::ArrayOperand::Dense {
                first_id, lo, hi, ..
            } => {
                let count = *hi - *lo + 1;
                if is_packed_id(*first_id) || count <= 0 || count > MAX_ARM_ELEMS {
                    return false;
                }
                if seen.insert(*first_id) {
                    out.extend((*first_id..*first_id + count as usize).map(|i| i as u32));
                }
                true
            }
            super::bytecode::ArrayOperand::Named(_) => false,
        }
    }

    pub(super) fn build_armed_edge_state(&mut self, arm_extra: &[Vec<u32>]) {
        self.armed_input_bitmap.clear();
        self.armed_input_ranges.clear();
        self.armed_input_blocks.clear();
        self.edge_block_armed.clear();
        if !self.armed_edge || !self.event_skip {
            return;
        }

        let nb = self.edge_block_gateable.len();
        let mut pairs: Vec<(u32, u32)> = Vec::with_capacity(self.edge_block_reads_flat.len());
        for bi in 0..nb {
            if !self.edge_block_gateable[bi] {
                continue;
            }
            let lo = self.edge_block_off[bi] as usize;
            let hi = self.edge_block_off[bi + 1] as usize;
            for &sid in &self.edge_block_reads_flat[lo..hi] {
                pairs.push((sid, bi as u32));
            }
            if let Some(extra) = arm_extra.get(bi) {
                for &eid in extra {
                    pairs.push((eid, bi as u32));
                }
            }
        }
        pairs.sort_unstable();
        pairs.dedup();

        let bitmap_len = pairs.last().map_or(0, |(sid, _)| *sid as usize + 1);
        self.armed_input_bitmap = vec![0u64; bitmap_len.div_ceil(64)];
        self.quiet_valid = false;
        self.gate_lane_valid = false;
        self.armed_input_ranges = vec![(0, 0); bitmap_len];
        self.armed_input_blocks.reserve(pairs.len());
        let mut cursor = 0usize;
        let mut input_count = 0usize;
        while cursor < pairs.len() {
            let sid = pairs[cursor].0;
            let lo = self.armed_input_blocks.len() as u32;
            while cursor < pairs.len() && pairs[cursor].0 == sid {
                self.armed_input_blocks.push(pairs[cursor].1);
                cursor += 1;
            }
            let hi = self.armed_input_blocks.len() as u32;
            self.armed_input_bitmap[(sid as usize) >> 6] |= 1u64 << (sid & 63);
            self.armed_input_ranges[sid as usize] = (lo, hi);
            input_count += 1;
        }
        self.edge_block_armed = self
            .edge_block_gateable
            .iter()
            .enumerate()
            .map(|(bi, &gateable)| {
                if !gateable {
                    EDGE_HOLD
                } else if self.edge_block_wide_read.get(bi).copied().unwrap_or(false) {
                    EDGE_ARMED
                } else {
                    EDGE_ARMED | EDGE_HOLD
                }
            })
            .collect();
        chatter!(
            "[EVENT-EDGE] ARMED mode ON: {} input signals, {} fanout edges{}",
            input_count,
            self.armed_input_blocks.len(),
            if self.armed_edge_shadow {
                " (shadow validation)"
            } else {
                ""
            }
        );
    }

    /// JIT-redesign Stage 1 invariant: walk every sid where
    /// `signal_inline_bits` is populated and verify it matches the
    /// canonical `signal_table[sid].raw_bits()`.  Returns the number
    /// of mismatched ids (0 = healthy).  Called by `event_loop_singlethread`
    /// when `XEZIM_VERIFY_INLINE_BITS=1` to detect missed write-site
    /// instrumentation.  Cost: O(num_signals), ~10 ms on c910 per call.
    pub(super) fn verify_inline_bits_invariant(&self, iter: u64) -> usize {
        if self.signal_inline_bits.is_empty() {
            return 0;
        }
        let n = self.signal_inline_bits.len().min(self.signal_table.len());
        let mut mismatches = 0usize;
        for id in 0..n {
            let (v, x) = self.signal_table[id].raw_bits();
            let [iv, ix] = self.signal_inline_bits[id];
            if iv != v || ix != x {
                mismatches += 1;
                if mismatches <= 5 {
                    eprintln!(
                        "[INLINE_BITS_MISMATCH] iter={} sid={} table=(v={:#x}, x={:#x}) inline=(v={:#x}, x={:#x})",
                        iter, id, v, x, iv, ix
                    );
                }
            }
        }
        if mismatches > 0 {
            eprintln!(
                "[INLINE_BITS_MISMATCH] iter={} total_mismatches={}/{}",
                iter, mismatches, n
            );
        }
        mismatches
    }

    /// Read a cell by id, transparently across the signal table and the
    /// packed arena (see `PackedMem`).
    #[inline]
    pub(super) fn cell_read(&self, id: usize) -> Value {
        if is_packed_id(id) {
            self.packed.read(id)
        } else {
            self.signal_table[id].clone()
        }
    }

    /// Store raw bits into a packed-arena cell; true when it changed. A
    /// packed cell has no comb dependents of its own (admission requires an
    /// unnamed bulk array), so the only readers to wake are the comb entries
    /// that re-evaluate on every settle (`comb_unresolved_idx`) — the way a
    /// memory read through a dynamic index is tracked.
    #[inline]
    pub(super) fn packed_store(&mut self, id: usize, v: u64, x: u64) -> bool {
        let changed = self.packed.set_raw(id, v, x);
        if changed {
            self.table_modified = true;
            if !self.comb_unresolved_idx.is_empty() {
                self.dirty_any = true;
            }
        }
        changed
    }

    /// `packed_store` of a whole `Value` (resized to the element width).
    #[inline]
    pub(super) fn packed_store_value(&mut self, id: usize, val: &Value) -> bool {
        let w = self.packed.width(id);
        let (v, x) = if val.width == w && !val.is_real && !val.is_fill {
            val.raw_bits()
        } else {
            val.resize(w).raw_bits()
        };
        self.packed_store(id, v, x)
    }

    /// §10.6.2: pin cell `id` at its current (just-assigned) value. `v` is
    /// the forced expression's value, recorded as-is only where the table
    /// holds no resized copy of it.
    pub(super) fn force_cell(&mut self, id: usize, v: Value) {
        if is_packed_id(id) {
            let (cv, cx, _) = self.packed.raw(id);
            self.packed.force_raw(id, cv, cx);
            return;
        }
        // Record the value as stored (post-resize), so the map mirrors the
        // signal table.
        let stored = self.signal_table.get(id).cloned().unwrap_or(v);
        self.forced_signals.insert(id, stored);
    }

    /// IEEE 1800-2023 §23.3.3 / §10.3: an input port is a continuous
    /// assignment from its actual, so the actual's time-0 value reaches the
    /// port as an update event at time 0 — also when the actual got it from
    /// its declaration initializer, which by itself is no event (§6.8). A
    /// level-sensitive `always @(...)` / `always @*` in the instance that
    /// reads such a port therefore runs once at time 0 instead of waiting for
    /// the first later change. The instance body reads the actual itself
    /// (the elaborator renames input ports), so the ports come from
    /// `input_port_actuals`. An actual still all x/z (0 for 2-state) made no
    /// change.
    pub(super) fn fire_time0_input_port_blocks(&mut self) {
        if self.module.input_port_actuals.is_empty() || self.comb_entries.is_empty() {
            return;
        }
        let top = format!("{}.", self.module.name);
        let cands: Vec<(usize, String)> = self
            .comb_entries
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                e.defer_at_time0
                    && matches!(
                        e.item,
                        CombItem::AlwaysBlock {
                            is_always_comb: false,
                            ..
                        } | CombItem::CompiledAlwaysBlock {
                            is_always_comb: false,
                            ..
                        }
                    )
            })
            .filter_map(|(i, e)| {
                let sc = e.cold.scope_hint.as_deref()?;
                let sc = sc.strip_prefix(top.as_str()).unwrap_or(sc);
                (!sc.is_empty()).then(|| (i, sc.to_string()))
            })
            .collect();
        if cands.is_empty() {
            return;
        }
        let scopes: HashSet<&str> = cands.iter().map(|(_, s)| s.as_str()).collect();
        let actuals: Vec<(String, Expression)> = self
            .module
            .input_port_actuals
            .iter()
            .filter(|(inst, _)| scopes.contains(inst.as_str()))
            .cloned()
            .collect();
        let mut port_ids: HashMap<String, Vec<usize>> = HashMap::default();
        for (inst, actual) in &actuals {
            let mut reads: HashSet<String> = HashSet::default();
            Self::collect_expr_reads(actual, &self.module, &mut reads);
            let ids: Vec<usize> = reads
                .iter()
                .filter_map(|r| self.resolve_read_name(r, &top))
                .collect();
            if ids.is_empty() {
                continue;
            }
            let v = self.eval_expr(actual);
            let two_state = ids
                .iter()
                .all(|&id| self.signal_two_state.get(id).copied().unwrap_or(false));
            let changed = if two_state {
                v.is_nonzero().unwrap_or(false)
            } else {
                (0..v.width as usize)
                    .any(|i| matches!(v.get_bit(i), LogicBit::Zero | LogicBit::One))
            };
            if changed {
                port_ids.entry(inst.clone()).or_default().extend(ids);
            }
        }
        if port_ids.is_empty() {
            return;
        }
        let fire: Vec<usize> = cands
            .iter()
            .filter(|(i, sc)| {
                port_ids.get(sc).is_some_and(|ids| {
                    self.comb_entries[*i]
                        .cold
                        .read_signal_ids
                        .iter()
                        .any(|r| ids.contains(r))
                })
            })
            .map(|(i, _)| *i)
            .collect();
        for eidx in fire {
            let entry = self.comb_entries[eidx].clone();
            self.eval_comb_entry_full(&entry);
        }
        if self.dirty_any {
            self.settle_combinatorial();
        }
    }

    /// Lift a force on cell `id` so a replacing force's own write lands.
    pub(super) fn unforce_cell(&mut self, id: usize) {
        if is_packed_id(id) {
            self.packed.release_forced(id);
        } else {
            self.forced_signals.remove(&id);
        }
    }

    /// Whether a bytecode array operand names a packed-arena array.
    pub(super) fn array_operand_is_packed(&self, a: &super::bytecode::ArrayOperand) -> bool {
        match a {
            super::bytecode::ArrayOperand::Dense { first_id, .. } => is_packed_id(*first_id),
            super::bytecode::ArrayOperand::Named(name) => self
                .array_first_id
                .get(name.as_str())
                .is_some_and(|&(first, _, _)| is_packed_id(first)),
        }
    }

    /// Write a cell by id; returns true when it changed. A packed cell has no
    /// comb dependents of its own (admission requires an unnamed bulk array),
    /// so only the table-modified flag needs updating.
    #[inline]
    pub(super) fn cell_write(&mut self, id: usize, val: &Value) -> bool {
        if is_packed_id(id) {
            return self.packed_store_value(id, val);
        }
        let w = self.signal_widths[id];
        let resized = val.resize(w);
        if self.signal_table[id] != resized {
            write_sig!(self, id, resized);
            self.mark_dirty_id(id);
            self.table_modified = true;
            return true;
        }
        false
    }

    /// A cell without a name (a bulk memory's) has no dependency edge: its
    /// readers are entries with unresolved reads, which re-evaluate on every
    /// settle — so its write must still make one run, as `packed_store` does
    /// for an arena cell.
    #[cold]
    #[inline(never)]
    pub(super) fn note_unnamed_cell_write(&mut self, id: usize) {
        if !self.ts_direct_writes && self.id_to_name.is_unnamed(id) {
            self.dirty_any = true;
        }
    }

    /// Mark a signal as dirty by ID.
    #[inline]
    pub(super) fn mark_dirty_id(&mut self, id: usize) {
        let has_comb_deps = id + 1 >= self.comb_dep_offsets.len()
            || self.comb_dep_offsets[id] != self.comb_dep_offsets[id + 1];
        if has_comb_deps {
            if self.ts_direct_writes {
                self.ts_direct_dirty(id, u64::MAX);
            } else if !self.dirty_signals[id] {
                self.dirty_signals[id] = true;
                self.dirty_list.push(id);
            }
            if !self.ts_direct_writes {
                self.dirty_any = true;
            }
        } else if !self.comb_unresolved_idx.is_empty() {
            self.note_unnamed_cell_write(id);
        }
        if self.activity_mon {
            if self.signal_toggle_counts.len() != self.signal_table.len() {
                self.signal_toggle_counts.resize(self.signal_table.len(), 0);
            }
            self.signal_toggle_counts[id] += 1;
        }
    }

    /// Return true if every signal touched by `cb`'s instructions has
    /// width ≤ 64. Used as a pre-JIT guard: blocks that read or write a
    /// wider signal cannot use the u64-only JIT bridges without silent
    /// truncation.
    /// Reason-reporting sibling of `block_signals_fit_u64` for the
    /// XEZIM_COMB_PATHS census: names the first disqualifying item.
    pub(super) fn block_fit_u64_reason(
        &self,
        cb: &super::bytecode::CompiledBlock,
    ) -> Option<String> {
        use super::bytecode::Insn;
        for insn in &cb.instructions {
            let sig_id = match insn {
                Insn::LoadSignal(_, id)
                | Insn::LoadSignalSigned(_, id)
                | Insn::NbaAssign(id, ..)
                | Insn::NbaAssignConst(id, ..)
                | Insn::BranchIfSignalFalse(id, _, _)
                | Insn::NbaAssignRangeDyn(id, ..)
                | Insn::NbaAssignBitDyn(id, ..)
                | Insn::BlockingAssign(id, ..)
                | Insn::BlockingAssignRangeDyn(id, ..)
                | Insn::BlockingAssignBitDyn(id, ..)
                | Insn::NbaAssignArrayRead(id, ..) => Some(*id),
                Insn::LoadArrayElem(_, arr, _) | Insn::BlockingAssignArray(arr, ..) => {
                    match arr.as_ref() {
                        super::bytecode::ArrayOperand::Dense { first_id, .. } => {
                            Some(*first_id as u32)
                        }
                        _ => None,
                    }
                }
                _ => None,
            };
            if let Some(id) = sig_id {
                let id = id as usize;
                if id < self.signal_widths.len() && self.signal_widths[id] > 64 {
                    return Some(format!(
                        "wide signal {} ({} bits) via {}",
                        self.name_opt(id)
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| format!("id{id}")),
                        self.signal_widths[id],
                        super::bytecode::insn_opcode_name(insn)
                    ));
                }
                if self
                    .signal_table
                    .get(id)
                    .map(|v| v.is_real)
                    .unwrap_or(false)
                {
                    return Some(format!("real signal id{id}"));
                }
            }
            if let Insn::LoadConst(_, v) = insn {
                if v.is_fill {
                    return Some(format!("fill const width {}", v.width));
                }
                if v.is_real {
                    return Some("real const".to_string());
                }
            }
            if let Insn::BinOpConst(_, _, v, _) | Insn::CmpBranchConst(_, _, v, _, _) = insn {
                if v.is_fill || v.is_real || v.width > 64 {
                    return Some("fill/real/wide constant operand".to_string());
                }
            }
        }
        None
    }

    /// Why two-state lowering gave up AT this instruction, when the opcode
    /// itself is one the lowering handles. The closure census showed 76.4% of
    /// interpreter-bound evaluations sit in blocks whose opcodes are ALL
    /// supported — they fail on a GATE inside the arm — so naming the gate is
    /// what decides where the next mechanism goes. Pure diagnostics: reads the
    /// same signal tables the gates consult and never affects lowering.
    pub(super) fn ts_gate_reason(&self, insn: &super::bytecode::Insn) -> &'static str {
        use super::bytecode::Insn;
        // §5.7.1: any x/z bit in a literal is a hard stop — a two-state block
        // cannot represent it at all, so these blocks are permanently out of
        // reach rather than waiting on a mechanism.
        if let Insn::LoadConst(_, v) = insn {
            if v.has_xz() {
                return "x-const (unreachable)";
            }
            if v.is_real {
                return "real-const";
            }
        }
        let mut sigs: Vec<usize> = Vec::new();
        let push = |id: u32, out: &mut Vec<usize>| out.push(id as usize);
        match insn {
            Insn::LoadSignal(_, id)
            | Insn::LoadSignalSigned(_, id)
            | Insn::LoadSignalBit(_, id, _)
            | Insn::LoadSignalRange(_, id, _, _)
            | Insn::BranchIfSignalFalse(id, _, _)
            | Insn::NbaAssign(id, ..)
            | Insn::NbaAssignConst(id, ..)
            | Insn::NbaAssignRange(id, ..)
            | Insn::NbaAssignRangeDyn(id, ..)
            | Insn::NbaAssignBitDyn(id, ..)
            | Insn::BlockingAssign(id, ..)
            | Insn::BlockingAssignRange(id, ..)
            | Insn::BlockingAssignRangeDyn(id, ..)
            | Insn::BlockingAssignBitDyn(id, ..) => push(*id, &mut sigs),
            Insn::NbaAssignArrayRead(dst, arr, idx, _) => {
                push(*dst, &mut sigs);
                push(*idx, &mut sigs);
                if let super::bytecode::ArrayOperand::Dense { first_id, .. } = arr.as_ref() {
                    sigs.push(*first_id);
                }
            }
            Insn::LoadArrayElem(_, arr, _)
            | Insn::NbaAssignArray(arr, ..)
            | Insn::BlockingAssignArray(arr, ..) => {
                if let super::bytecode::ArrayOperand::Dense { first_id, .. } = arr.as_ref() {
                    sigs.push(*first_id);
                }
            }
            _ => {}
        }
        if sigs.is_empty() {
            return "register-only gate";
        }
        for &id in &sigs {
            if self.signal_real.get(id).copied().unwrap_or(false) {
                return "real-signal";
            }
        }
        for &id in &sigs {
            if self.signal_widths.get(id).copied().unwrap_or(0) > 128 {
                return "signal >128b";
            }
        }
        for &id in &sigs {
            let w = self.signal_widths.get(id).copied().unwrap_or(0);
            if w > 64 {
                return "signal 65..128b";
            }
        }
        for &id in &sigs {
            if self.signal_signed.get(id).copied().unwrap_or(false) {
                return "signed-signal";
            }
        }
        "other (reg width/shape)"
    }

    pub(super) fn block_signals_fit_u64(&self, cb: &super::bytecode::CompiledBlock) -> bool {
        use super::bytecode::Insn;
        // §PACKED_MEM pre-pass: cells of a PACKED array live in the arena
        // above `PACKED_BASE` and have no `signal_table` slot at all, while
        // the JIT addresses array elements relative to `signal_table`. The
        // per-signal width test below narrows ids with `as u32`, which
        // TRUNCATES a packed id (`1<<48 + off` -> `off`) and so silently
        // aliases a whole RAM onto an unrelated signal — the opposite of the
        // loud out-of-bounds failure `PACKED_BASE` was chosen to give. Under
        // `XEZIM_PACKED_MEM=1` that miscompiled every JIT'd SRAM block on the
        // C906 SoC (a 524288x128 macro among them), so reject such blocks up
        // front, on EVERY array-bearing opcode.
        for insn in &cb.instructions {
            let arr = match insn {
                Insn::LoadArrayElem(_, a, _)
                | Insn::NbaAssignArray(a, ..)
                | Insn::BlockingAssignArray(a, ..)
                | Insn::NbaAssignArrayRange(a, ..)
                | Insn::BlockingAssignArrayRange(a, ..)
                | Insn::NbaAssignArrayRead(_, a, ..) => Some(a),
                _ => None,
            };
            if let Some(a) = arr {
                if let super::bytecode::ArrayOperand::Dense { first_id, .. } = a.as_ref() {
                    if is_packed_id(*first_id as usize) {
                        return false;
                    }
                }
            }
        }
        for insn in &cb.instructions {
            let sig_id = match insn {
                Insn::LoadSignal(_, id)
                | Insn::LoadSignalSigned(_, id)
                | Insn::NbaAssign(id, ..)
                | Insn::NbaAssignConst(id, ..)
                | Insn::BranchIfSignalFalse(id, _, _)
                | Insn::NbaAssignRangeDyn(id, ..)
                | Insn::BlockingAssign(id, ..)
                | Insn::BlockingAssignRangeDyn(id, ..)
                // Only the destination is checked here; the index signal this
                // variant also reads needs no separate check because
                // `JitModule::is_supported` rejects `NbaAssignArrayRead`
                // outright, so a block containing one never reaches codegen.
                | Insn::NbaAssignArrayRead(id, ..) => Some(*id),
                // Single-bit accesses tolerate a wide signal. Reads: bits
                // < 64 come off the inline plane's low word (maintained for
                // ALL ids, wide included), bits >= 64 go through the slice
                // bridge. Writes: both BitDyn forms move exactly one bit
                // through the range/bit bridges, whose per-bit fallback
                // handles any width (dyn RANGE writes stay gated — a runtime
                // span over 64 bits cannot ride a u64 register).
                // Real-typed signals stay rejected (planes are ints).
                Insn::NbaAssignBitDyn(id, ..)
                | Insn::BlockingAssignBitDyn(id, ..)
                | Insn::LoadSignalBit(_, id, _) => {
                    if self
                        .signal_table
                        .get(*id as usize)
                        .map(|v| v.is_real)
                        .unwrap_or(false)
                    {
                        return false;
                    }
                    None
                }
                // Dense-array element access in the JIT computes the element
                // id arithmetically; the first element's decl stands for all,
                // so gating on it gates the whole span.
                Insn::LoadArrayElem(_, arr, _)
                | Insn::BlockingAssignArray(arr, ..)
                | Insn::NbaAssignArray(arr, ..) => match arr.as_ref() {
                    super::bytecode::ArrayOperand::Dense { first_id, .. } => {
                        Some(*first_id as u32)
                    }
                    _ => None,
                },
                // STATIC-bound slice access tolerates a wide signal: the
                // slice bridges move only the addressed <= 64-bit window.
                // Whole-value and dynamic-bound forms above stay gated;
                // real-typed signals are still rejected (planes are ints).
                Insn::LoadSignalRange(_, id, l, r) => {
                    if l.abs_diff(*r) + 1 > 64
                        || self
                            .signal_table
                            .get(*id as usize)
                            .map(|v| v.is_real)
                            .unwrap_or(false)
                    {
                        return false;
                    }
                    None
                }
                Insn::BlockingAssignRange(id, hi, lo, _)
                | Insn::NbaAssignRange(id, hi, lo, _) => {
                    if hi.abs_diff(*lo) + 1 > 64
                        || self
                            .signal_table
                            .get(*id as usize)
                            .map(|v| v.is_real)
                            .unwrap_or(false)
                    {
                        return false;
                    }
                    None
                }
                _ => None,
            };
            if let Some(id) = sig_id {
                let id = id as usize;
                if id < self.signal_widths.len() && self.signal_widths[id] > 64 {
                    return false;
                }
                // §5 real-valued signals: the register file moves raw bit
                // patterns, so f64 payloads would go through integer ops.
                if self
                    .signal_table
                    .get(id)
                    .map(|v| v.is_real)
                    .unwrap_or(false)
                {
                    return false;
                }
            }
            // §5.7.1 unbased-unsized fills ('1/'0/'x) replicate to the width
            // of their CONSUMING context, and §5 real literals carry f64
            // payloads — both are `Value` behaviors the register file cannot
            // express. Blocks carrying either fall back to the interpreter.
            if let Insn::LoadConst(_, v) = insn {
                if v.is_fill || v.is_real {
                    return false;
                }
            }
            if let Insn::NbaAssignConst(_, k, _) = insn {
                if k.is_fill || k.is_real || k.width > 64 {
                    return false;
                }
            }
            // The fused array-read's INDEX is a signal the bridges load as
            // u64; its element span is gated via the Dense arm above, and
            // the destination via the id arm.
            if let Insn::NbaAssignArrayRead(_, arr, idx_sig, _) = insn {
                let i = *idx_sig as usize;
                if i < self.signal_widths.len() && self.signal_widths[i] > 64 {
                    return false;
                }
                match arr.as_ref() {
                    super::bytecode::ArrayOperand::Dense { first_id, .. } => {
                        let f = *first_id;
                        if f < self.signal_widths.len() && self.signal_widths[f] > 64 {
                            return false;
                        }
                        if self.signal_table.get(f).map(|v| v.is_real).unwrap_or(false) {
                            return false;
                        }
                    }
                    _ => return false,
                }
            }
            // Same constant, just folded into the ALU op that consumes it —
            // the peephole moves the `Value` out of the `LoadConst` and into
            // `BinOpConst` or `CmpBranchConst`, where the LoadConst check
            // no longer sees it. JIT codegen accepts embedded constants, so
            // validate their width and value kind before native lowering.
            if let Insn::BinOpConst(_, _, v, _) | Insn::CmpBranchConst(_, _, v, _, _) = insn {
                if v.is_fill || v.is_real || v.width > 64 {
                    return false;
                }
            }
        }
        true
    }

    /// JIT bridge: read `signal_table[id]` as a raw `u64` of val_bits.
    /// For 4-state (X/Z) values we still return a best-effort bit
    /// pattern (the val_bits directly) — the JIT'd block will compute
    /// on garbage but the caller re-runs the block via the interpreter
    /// afterwards if any X/Z is in play. (Phase-1 MVP: no XZ tracking
    /// in JIT code; we rely on post-execution fallback semantics which
    /// are fine for c910 post-reset where all signals are determinate.)
    ///
    /// **Width contract:** caller must ensure `signal_widths[id] <= 64`.
    /// `block_signals_fit_u64` is the pre-JIT gate that guarantees this.
    #[inline]
    pub(crate) fn jit_load_signal(&self, id: usize) -> u64 {
        if id >= self.signal_table.len() {
            return 0;
        }
        debug_assert!(
            self.signal_widths[id] <= 64,
            "jit_load_signal called on wide signal id={} width={}",
            id,
            self.signal_widths[id],
        );
        // Raw val plane, NOT `to_u64()`: to_u64 zeroes bits under X/Z, so a
        // Z bit (val=1, xz=1) entered JIT registers canonicalized to X and
        // came back out corrupted through plane-preserving insns (Concat,
        // CaseEq, stores). Every 4-state op table computes known-1 as
        // `v & ~x`, and truthiness consumers mask the same way, so the raw
        // plane is the correct feed.
        self.signal_table[id].raw_bits().0
    }

    /// Packed ELEMENT width of a declared type for the AST layer: direct
    /// packed-of-packed shapes resolve structurally; a bare typedef name
    /// resolves through the elaborator's typedef elem table.
    pub(super) fn ast_decl_elem_width(&self, dt: &crate::ast::types::DataType) -> Option<u32> {
        if let Some(ew) = super::elaborate::packed_inner_elem_width(
            dt,
            &self.module.parameters,
            &self.module.typedefs,
        ) {
            return Some(ew);
        }
        if let crate::ast::types::DataType::TypeReference {
            name, dimensions, ..
        } = dt
        {
            if dimensions.is_empty() {
                return self
                    .module
                    .typedef_elem_widths
                    .get(&name.name.name)
                    .copied();
            }
        }
        None
    }

    /// Re-sync the SoA plane mirror + has_xz for one signal after an
    /// in-place mutation that bypassed `write_sig!` (raw-bit fast paths,
    /// set_bit/splice paths, UDP bit writes, direct table stores). Cheap:
    /// two u64 stores + one byte.
    #[inline]
    pub(crate) fn sync_mirror(&mut self, id: usize) {
        if let Some(slot) = self.signal_inline_bits.get_mut(id) {
            let (v, x) = self.signal_table[id].raw_bits();
            *slot = [v, x];
            if let Some(hx) = self.signal_has_xz.get_mut(id) {
                *hx = if self.signal_table[id].has_xz() { 1 } else { 0 };
            }
        }
    }

    /// JIT bridge target: execute one AST fallback statement with the
    /// owning block's scope hint applied, mirroring the interpreter path's
    /// timescale/%m/name-resolution setup around fallback-bearing blocks.
    pub(crate) fn jit_exec_fallback_stmt(
        &mut self,
        stmt: &crate::ast::stmt::Statement,
        hint: Option<&str>,
    ) -> bool {
        let saved_ts = self.timescale_scope_override.take();
        self.timescale_scope_override = hint.map(|h| h.to_string());
        // `%m` wants the instance-relative scope the interpreter's edge path
        // records; a root block's hint is the top module's name.
        let m_scope = hint.map(|h| self.instance_relative_scope(h));
        self.set_m_block_scope(m_scope.as_deref());
        let saved_hint = {
            let prev = self.name_resolve_hint.borrow().clone();
            if let Some(sc) = hint {
                *self.name_resolve_hint.borrow_mut() = Some(sc.to_string());
            }
            prev
        };
        self.exec_statement(stmt);
        *self.name_resolve_hint.borrow_mut() = saved_hint;
        self.timescale_scope_override = saved_ts;
        self.finished
    }

    /// JIT bridge: both planes in one call (see `xezim_jit_load_signal2`).
    #[inline]
    pub(crate) fn jit_load_signal2(&self, id: usize) -> (u64, u64) {
        if id >= self.signal_table.len() {
            return (0, 0);
        }
        self.signal_table[id].raw_bits()
    }

    /// JIT bridge: read a bit-slice [lo +: w] of a signal of ANY width as
    /// u64 planes (w <= 64). The wide-signal escape hatch for
    /// `LoadSignalRange` — whole-value loads stay gated to <= 64 bits.
    #[inline]
    pub(crate) fn jit_load_signal_slice(&self, id: usize, lo: u32, w: u32) -> u64 {
        if id >= self.signal_table.len() {
            return 0;
        }
        Self::raw_bits_slice(&self.signal_table[id], lo as u16, w as u16).0
    }
}
