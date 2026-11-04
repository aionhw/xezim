//! Simulator jit runtime.

use super::*;

impl Simulator {
    #[inline]
    pub(crate) fn jit_load_signal_slice_xz(&self, id: usize, lo: u32, w: u32) -> u64 {
        if id >= self.signal_table.len() {
            return 0;
        }
        Self::raw_bits_slice(&self.signal_table[id], lo as u16, w as u16).1
    }

    /// JIT bridge: Path B X/Z pre-check. Returns true if ANY signal in
    /// `ids` currently has X or Z bits set (i.e. its raw_bits xz part
    /// is non-zero). Called from the JIT-emitted prelude. When true,
    /// the JIT bails (returns 1) without writing anything, and the
    /// dispatch in `exec_bytecode` runs the interpreter for that
    /// execution while keeping the JIT armed for next time.
    pub(crate) fn jit_inputs_have_xz(&self, ids: &[u32]) -> bool {
        for &id in ids {
            let i = id as usize;
            if i >= self.signal_table.len() {
                continue;
            }
            let (_v, x) = self.signal_table[i].raw_bits();
            if x != 0 {
                return true;
            }
            // Wide signals: raw_bits truncates at 64 bits. Check the
            // tail explicitly so we don't miss X/Z above bit 63.
            if self.signal_widths[i] > 64 {
                let val = &self.signal_table[i];
                for bit in 64..val.width as usize {
                    match val.get_bit(bit) {
                        LogicBit::X | LogicBit::Z => return true,
                        _ => {}
                    }
                }
            }
        }
        false
    }

    /// JIT bridge: mirror `Insn::BlockingAssign` — width-mask the
    /// incoming val_bits, compare against current signal value, mark
    /// dirty + propagate if changed. Preserves `is_signed` from the
    /// signal's declared sign so readers get correct arithmetic.
    #[inline]
    /// JIT bridge: the X/Z plane of a signal (see `xezim_jit_load_signal_xz`).
    pub(crate) fn jit_load_signal_xz(&self, id: usize) -> u64 {
        if id >= self.signal_table.len() {
            return 0;
        }
        self.signal_table[id].raw_bits().1
    }

    /// JIT bridge: 4-state blocking write. Mirrors `jit_store_signal` but
    /// carries the X/Z plane instead of forcing the value 2-state.
    pub(crate) fn jit_store_signal_4s(
        &mut self,
        id: usize,
        val_bits: u64,
        xz_bits: u64,
        width: u32,
    ) {
        if id >= self.signal_table.len() {
            return;
        }
        // §6.11.1: a two-state destination drops x/z, as the VM's stores do.
        let (val_bits, xz_bits) = self.ts_scrub_two_state(id, val_bits, xz_bits);
        let sig_w = self.signal_widths[id];
        let w = if width == 0 { sig_w } else { width };
        let mask = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
        // No-change fast exit on raw planes — the overwhelmingly common case
        // in a settled design — before any Value is built. Only exact-width
        // Inline storage takes it; everything else falls through to the
        // full compare (resize semantics, Wide, real).
        if w == sig_w {
            if let Some([ov, ox]) = self.signal_inline_bits.get(id) {
                if *ov == val_bits & mask && *ox == xz_bits & mask {
                    return;
                }
            } else if let Some((ov, ox)) = self.signal_table[id].inline_planes() {
                if ov == val_bits & mask && ox == xz_bits & mask {
                    return;
                }
            }
        }
        let mut new_val = Value::from_inline(val_bits & mask, xz_bits & mask, w);
        if w != sig_w {
            new_val = new_val.resize(sig_w);
        }
        new_val.is_signed = self.signal_signed[id];
        if self.signal_table[id] != new_val {
            self.mark_dirty_id(id);
            write_sig!(self, id, new_val);
            self.table_modified = true;
        }
    }

    pub(crate) fn jit_store_signal(&mut self, id: usize, val_bits: u64, width: u32) {
        if id >= self.signal_table.len() {
            return;
        }
        let sig_w = self.signal_widths[id];
        let w = if width == 0 { sig_w } else { width };
        let mut new_val = Value::from_u64(val_bits, w);
        if w != sig_w {
            new_val = new_val.resize(sig_w);
        }
        new_val.is_signed = self.signal_signed[id];
        if self.signal_table[id] != new_val {
            self.mark_dirty_id(id);
            write_sig!(self, id, new_val);
            self.table_modified = true;
        }
    }

    /// JIT bridge: mirror `Insn::NbaAssign` — push an `NbaFast` entry.
    /// `apply_nba` later commits to `signal_table` at the end-of-cycle.
    #[inline]
    /// JIT bridge: 4-STATE non-blocking schedule. Mirrors `jit_schedule_nba`
    /// but carries the X/Z plane. Without it the JIT dropped every unknown on
    /// its way to `signal_table` — an uninitialised `val <= (val << 1) + 1`
    /// settled to a concrete number instead of staying x.
    pub(crate) fn jit_schedule_nba_4s(
        &mut self,
        id: usize,
        val_bits: u64,
        xz_bits: u64,
        width: u32,
    ) {
        if id >= self.signal_table.len() {
            return;
        }
        // §6.11.1: a two-state destination drops x/z, as the VM's stores do.
        let (val_bits, xz_bits) = self.ts_scrub_two_state(id, val_bits, xz_bits);
        let sig_w = self.signal_widths[id];
        let w = if width == 0 { sig_w } else { width };
        let mask = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
        // Unchanged-elision, mirroring the interpreter's NbaAssign arm: a
        // value already equal to the signal is not queued — UNLESS an entry
        // for this id is already pending, where §10.4.2 last-write-wins
        // requires superseding it.
        if w == sig_w {
            if let Some([ov, ox]) = self.signal_inline_bits.get(id) {
                if *ov == val_bits & mask
                    && *ox == xz_bits & mask
                    && self.nba_fast_index.get(id).is_none()
                {
                    return;
                }
            } else if let Some((ov, ox)) = self.signal_table[id].inline_planes() {
                if ov == val_bits & mask
                    && ox == xz_bits & mask
                    && self.nba_fast_index.get(id).is_none()
                {
                    return;
                }
            }
        }
        let mut val = Value::from_inline(val_bits & mask, xz_bits & mask, w);
        if w != sig_w {
            val = val.resize(sig_w);
        }
        val.is_signed = self.signal_signed[id];
        self.nba_fast_index.insert(id, self.nba_fast.len());
        self.nba_fast.push(NbaFast {
            block_index: 0,
            signal_id: id,
            value: val,
        });
    }

    pub(crate) fn jit_schedule_nba(&mut self, id: usize, val_bits: u64, width: u32) {
        if id >= self.signal_table.len() {
            return;
        }
        let sig_w = self.signal_widths[id];
        let w = if width == 0 { sig_w } else { width };
        let mut val = Value::from_u64(val_bits, w);
        if w != sig_w {
            val = val.resize(sig_w);
        }
        val.is_signed = self.signal_signed[id];
        self.nba_fast_index.insert(id, self.nba_fast.len());
        self.nba_fast.push(NbaFast {
            block_index: 0,
            signal_id: id,
            value: val,
        });
    }

    /// JIT Stage 4 Tier A: leaner NBA scheduling.  Caller (JIT codegen)
    /// guarantees that `val_bits` is already masked to
    /// `signal_widths[id]` and that `id` is in range — so we skip the
    /// bounds check, width compare, and Value::resize.  The bulk of
    /// the per-call cost (HashMap insert + Vec push) remains.
    #[inline]
    pub(crate) fn jit_schedule_nba_fast(&mut self, id: usize, val_bits: u64) {
        let sig_w = self.signal_widths[id];
        let mut val = Value::from_u64(val_bits, sig_w);
        val.is_signed = self.signal_signed[id];
        self.nba_fast_index.insert(id, self.nba_fast.len());
        self.nba_fast.push(NbaFast {
            block_index: 0,
            signal_id: id,
            value: val,
        });
    }

    /// JIT bridge: mirror `Insn::NbaAssignRange` / `Insn::NbaAssignRangeDyn`
    /// — read-modify-write bits `[hi:lo]` of the target signal (or
    /// the latest in-flight NbaFast entry) with `val_bits` occupying
    /// the low `(hi-lo+1)` bits.
    #[inline]
    pub(crate) fn jit_schedule_nba_range(
        &mut self,
        id: usize,
        hi: u32,
        lo: u32,
        val_bits: u64,
        xz_bits: u64,
    ) {
        if id >= self.signal_table.len() {
            return;
        }
        let (low, high) = if hi >= lo { (lo, hi) } else { (hi, lo) };
        let w = high - low + 1;
        // 4-state: the X/Z plane rides along — dropping it here silently
        // forced every JIT-stored unknown to 0 the moment the block ran
        // (the old 2-state pre-check masked this by never running on X).
        let val = Value::from_inline(val_bits, xz_bits, w);
        // Compose onto latest nba_fast entry if any, else onto
        // signal_table's current value — exactly matches the
        // interpreter's NbaAssignRange read-modify-write pattern.
        let sig_w = self.signal_widths[id];
        let high_eff = high.min(sig_w.saturating_sub(1));
        if oob_write_discard(high as i64, low as i64, sig_w) {
            return;
        }
        if let Some(i) = self.nba_fast_index.get(id) {
            let target = &mut self.nba_fast[i].value;
            for bit_pos in low..=high_eff {
                target.set_bit(bit_pos as usize, val.get_bit((bit_pos - low) as usize));
            }
            return;
        }
        let mut new_val = self.signal_table[id].clone();
        for bit_pos in low..=high_eff {
            new_val.set_bit(bit_pos as usize, val.get_bit((bit_pos - low) as usize));
        }
        self.nba_fast_index.insert(id, self.nba_fast.len());
        self.nba_fast.push(NbaFast {
            block_index: 0,
            signal_id: id,
            value: new_val,
        });
    }

    /// JIT bridge: schedule non-blocking assignment to a dynamic bit-index.
    #[inline]
    pub(crate) fn jit_schedule_nba_bit(&mut self, id: usize, idx: usize, v: u64, x: u64) {
        if id >= self.signal_table.len() {
            return;
        }
        let bit = match (v & 1, x & 1) {
            (0, 0) => LogicBit::Zero,
            (1, 0) => LogicBit::One,
            (0, _) => LogicBit::X,
            _ => LogicBit::Z,
        };
        if let Some(i) = self.nba_fast_index.get(id) {
            self.nba_fast[i].value.set_bit(idx, bit);
            return;
        }
        let mut new_val = self.signal_table[id].clone();
        new_val.set_bit(idx, bit);
        self.nba_fast_index.insert(id, self.nba_fast.len());
        self.nba_fast.push(NbaFast {
            block_index: 0,
            signal_id: id,
            value: new_val,
        });
    }

    /// JIT bridge: perform a blocking assign to a dynamic bit-range.
    #[inline]
    pub(crate) fn jit_blocking_assign_range(
        &mut self,
        id: usize,
        hi: u32,
        lo: u32,
        val_bits: u64,
        xz_bits: u64,
    ) {
        if id >= self.signal_table.len() {
            return;
        }
        let (low, high) = if hi >= lo { (lo, hi) } else { (hi, lo) };
        let w = high - low + 1;
        let sig_w = self.signal_widths[id];
        // Word path: masked insert on the raw planes when everything fits
        // one u64 — replaces the per-bit get_bit/set_bit loop (which was a
        // top-five profile entry on ibex: every RAM byte-lane write walks
        // this). Semantics identical: bits beyond the signal's width are
        // silently dropped.
        if sig_w <= 64 && low < 64 {
            if let Some((ov, ox)) = self.signal_table[id].inline_planes() {
                let span_mask = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
                let sig_mask = if sig_w >= 64 {
                    u64::MAX
                } else {
                    (1u64 << sig_w) - 1
                };
                let m = (span_mask << low) & sig_mask;
                let nv = (ov & !m) | ((val_bits << low) & m);
                let nx = (ox & !m) | ((xz_bits << low) & m);
                if nv != ov || nx != ox {
                    self.signal_table[id].set_inline_planes(nv, nx);
                    self.sync_mirror(id);
                    self.signal_table[id].is_signed = self.signal_signed[id];
                    self.mark_dirty_id(id);
                    self.table_modified = true;
                    self.after_signal_write(id);
                }
                return;
            }
        }
        // Single-bit write to a WIDE signal (the BitDyn bridges): patch the
        // storage bit, the mirror's low word, and the has_xz hint
        // incrementally — the general path below costs two full raw_bits
        // repacks plus a whole-value has_xz walk per write, which dominated
        // the profile once wide-signal blocks went native. The hint is only
        // ever tightened via a scan when an xz bit was actually cleared
        // (reset transitions), so the steady state never scans.
        if w == 1 && sig_w > 64 {
            if low >= sig_w {
                return;
            }
            let src_bit = Value::from_inline(val_bits & 1, xz_bits & 1, 1).get_bit(0);
            let old_bit = self.signal_table[id].get_bit(low as usize);
            if old_bit == src_bit {
                return;
            }
            self.signal_table[id].set_bit(low as usize, src_bit);
            if let Some(slot) = self.signal_inline_bits.get_mut(id) {
                if low < 64 {
                    let m = 1u64 << low;
                    slot[0] = (slot[0] & !m) | ((val_bits & 1) << low);
                    slot[1] = (slot[1] & !m) | ((xz_bits & 1) << low);
                }
            }
            if let Some(hx) = self.signal_has_xz.get_mut(id) {
                if xz_bits & 1 != 0 {
                    *hx = 1;
                } else if *hx != 0 {
                    // The written bit went xz -> valid; other bits may still
                    // be xz. Scan only on this clearing transition — leaving
                    // the hint stuck at 1 would permanently demote every
                    // native block reading this signal.
                    *hx = if self.signal_table[id].has_xz() { 1 } else { 0 };
                }
            }
            self.signal_table[id].is_signed = self.signal_signed[id];
            self.mark_dirty_id(id);
            self.table_modified = true;
            self.after_signal_write_premirrored(id);
            return;
        }
        let changed = if w <= 64 {
            self.signal_table[id].splice_bits64(low as usize, val_bits, xz_bits, w as usize)
        } else {
            let val = Value::from_inline(val_bits, xz_bits, w);
            let high_eff = high.min(sig_w.saturating_sub(1));
            if oob_write_discard(high as i64, low as i64, sig_w) {
                return;
            }
            let mut changed = false;
            for bit_pos in low..=high_eff {
                let src_bit = val.get_bit((bit_pos - low) as usize);
                if self.signal_table[id].get_bit(bit_pos as usize) != src_bit {
                    self.signal_table[id].set_bit(bit_pos as usize, src_bit);
                    changed = true;
                }
            }
            changed
        };
        if changed {
            // ONE mirror sync for the whole span (raw_bits walks 64
            // LogicBits and has_xz the full value — never per changed bit),
            // and the premirrored hook skips the duplicate repack
            // `after_signal_write` would otherwise redo.
            self.sync_mirror(id);
            self.signal_table[id].is_signed = self.signal_signed[id];
            self.mark_dirty_id(id);
            self.table_modified = true;
            self.after_signal_write_premirrored(id);
        }
    }

    /// JIT bridge: load an array element as u64.
    ///
    /// **Width contract:** array element widths must be ≤ 64.
    /// `block_signals_fit_u64` does not currently scan array element
    /// widths (only direct signal references); if a JIT-compiled block
    /// reads a wide unpacked-array element, the read truncates silently.
    /// The current JIT lowering does not emit array-element loads for
    /// any block that would hit a wide element width, so this is a
    /// belt-and-suspenders assert.
    #[inline]
    pub(crate) fn jit_load_array_elem(&mut self, name: &str, idx: i64) -> u64 {
        if let Some(eid) = self.get_array_elem_id(name, idx) {
            if is_packed_id(eid) {
                return self.packed.read(eid).to_u64().unwrap_or(0);
            }
            debug_assert!(
                self.signal_widths[eid] <= 64,
                "jit_load_array_elem called on wide elem array={} idx={} width={}",
                name,
                idx,
                self.signal_widths[eid],
            );
            self.signal_table[eid].to_u64().unwrap_or(0)
        } else {
            0
        }
    }

    /// True when NO class object can exist: the heap holds only the index-0
    /// null sentinel (`heap: vec![None]`), so `heap.is_empty()` is NEVER true
    /// after construction — every guard spelled that way was dead code, and
    /// class-free designs paid the class-resolution probes on every indexed
    /// access.
    #[inline(always)]
    pub(super) fn no_class_objects(&self) -> bool {
        // §8.9: a static unpacked struct of a parameterized class gets its
        // per-specialization holder object on first use, so it counts too.
        self.heap.len() <= 1 && self.static_struct_names.is_empty()
    }

    /// Id-keyed write with the same bookkeeping as `fast_signal_write`'s
    /// resolved branches: element-width resize, declared signedness, change
    /// compare, `write_sig!` (which respects `forced_signals`), dirty mark.
    #[inline]
    pub(super) fn fast_signal_write_id(&mut self, id: usize, val: &Value) -> bool {
        // Packed-arena cell: no per-signal table slots (the indexing below
        // panics on the arena id — found by the issue #147 write/read-race
        // test's plain `mem[5] = v` init). A whole-element blocking write
        // commits immediately, like assign_value's packed range splice.
        if is_packed_id(id) {
            return self.packed_store_value(id, val);
        }
        let width = self.signal_widths[id];
        let mut resized = val.resize(width);
        // §6.11.1/§10.7: a 2-state destination drops X/Z on every write.
        if self.signal_two_state.get(id).copied().unwrap_or(false) && resized.has_xz() {
            resized = resized.to_two_state();
        }
        resized.is_signed = self.signal_signed[id];
        let changed = self.signal_table[id] != resized;
        if changed {
            write_sig!(self, id, resized);
            self.table_modified = true;
            self.mark_dirty_id(id);
        }
        changed
    }

    /// Fast signal write: update both signal_table and signals HashMap.
    #[inline]
    pub(super) fn fast_signal_write(&mut self, name: &str, val: Value) -> bool {
        if let Some(&id) = self.signal_name_to_id.get(name) {
            let width = self.signal_widths[id];
            let mut resized = val.resize(width);
            if self.signal_two_state.get(id).copied().unwrap_or(false) && resized.has_xz() {
                resized = resized.to_two_state();
            }
            resized.is_signed = self.signal_signed[id];
            let changed = self.signal_table[id] != resized;
            if changed {
                write_sig!(self, id, resized);
                self.table_modified = true;
                // Avoid a second signal_name_to_id lookup via mark_dirty(name)
                // — we already resolved id above.
                self.mark_dirty_id(id);
            }
            changed
        } else if let Some(id) = resolve_array_elem_id(name, &self.array_first_id) {
            // Array-element fast path. Required because per-element entries
            // are no longer materialised in signal_name_to_id; without this,
            // every $readmemh element falls into the slow path that runs
            // sync_table_to_hashmap (O(num_signals) per call), which on E902
            // ballooned simulation memory past 6 GB.
            if is_packed_id(id) {
                return self.packed_store_value(id, &val);
            }
            let width = self.signal_widths[id];
            let mut resized = val.resize(width);
            if self.signal_two_state.get(id).copied().unwrap_or(false) && resized.has_xz() {
                resized = resized.to_two_state();
            }
            resized.is_signed = self.signal_signed[id];
            let changed = self.signal_table[id] != resized;
            if changed {
                write_sig!(self, id, resized);
                self.table_modified = true;
                self.mark_dirty_id(id);
            }
            changed
        } else {
            // Fallback
            self.sync_table_to_hashmap();
            let width = self
                .widths
                .get(name)
                .copied()
                .filter(|w| *w > 0)
                .unwrap_or(val.width);
            let mut resized = val.resize(width);
            resized.is_signed = self.signed_signals.contains(name);
            let changed = self.signals.get(name).is_none_or(|p| *p != resized);
            if changed {
                self.mark_dirty(name);
            }
            self.signals.insert(name.to_string(), resized);
            changed
        }
    }

    /// Look up a signal value by name. Consults the indexed signal_table
    /// first (zero-clone branch returns from a Vec read), falling back to
    /// the legacy `signals` HashMap for dynamically created entries
    /// (queue elems, foreach loop vars, in-process var decls, etc.).
    /// This is the single read path that lets us drop the dual-store of
    /// static signals without breaking the fallback callers.
    #[inline]
    pub(super) fn lookup_signal_value(&self, name: &str) -> Option<Value> {
        if let Some(&id) = self.signal_name_to_id.get(name) {
            return Some(self.signal_table[id].clone());
        }
        self.signals.get(name).cloned()
    }

    /// Look up declared width by signal name.
    #[inline]
    pub(super) fn lookup_signal_width(&self, name: &str) -> Option<u32> {
        if let Some(&id) = self.signal_name_to_id.get(name) {
            return Some(self.signal_widths[id]);
        }
        self.widths.get(name).copied()
    }

    /// Whether the named signal is signed.
    #[inline]
    pub(super) fn lookup_signal_signed(&self, name: &str) -> bool {
        if let Some(&id) = self.signal_name_to_id.get(name) {
            return self.signal_signed[id];
        }
        self.signed_signals.contains(name)
    }

    /// Whether the named signal is real (`real`/`shortreal`).
    #[inline]
    pub(super) fn lookup_signal_real(&self, name: &str) -> bool {
        if let Some(&id) = self.signal_name_to_id.get(name) {
            return self.signal_real[id];
        }
        self.real_signals.contains(name)
    }

    /// The LRM §11.6.1 SELF-determined width of an expression — the "max of
    /// operand widths" rule with NO carry headroom. `infer_width` deliberately
    /// over-reports (`a+a` -> 9) so temporaries never truncate, but the width
    /// that a shift/divide OPERAND takes from its context must be the LRM
    /// width: using the carry-aware one evaluated `(a<<4)>>2` at 12 bits and
    /// shifted the dropped carry back into range (0x8c for 0x0c).
    /// Returns None for any shape it cannot size WITHOUT evaluation —
    /// crucially it must NEVER fall back to `infer_width`, whose Call arm
    /// EXECUTES the call to learn its width: sizing the operands of
    /// `h.q.sz() == 2 && h.q.popf() == 10` that way ran every side-effectful
    /// method an extra time (the queue came out empty, $urandom sequences
    /// shifted, assoc-array cursors advanced twice). Callers fall back to
    /// their historical behavior on None.
    pub(super) fn lrm_self_width(&mut self, expr: &Expression) -> Option<u32> {
        match &expr.kind {
            ExprKind::Paren(i) => self.lrm_self_width(i),
            ExprKind::SystemCall { name, args }
                if super::bytecode::system_function_carries_arg(name) && !args.is_empty() =>
            {
                self.lrm_self_width(&args[0])
            }
            ExprKind::SystemCall { name, .. }
                if super::bytecode::system_function_result(name).is_some() =>
            {
                super::bytecode::system_function_result(name).map(|(w, _)| w)
            }
            ExprKind::Number(NumberLiteral::Integer { size: Some(sz), .. }) => Some(*sz),
            ExprKind::Number(NumberLiteral::Integer { size: None, .. }) => Some(32),
            ExprKind::Number(NumberLiteral::UnbasedUnsized(_)) => Some(1),
            ExprKind::Ident(h) => {
                let n = self.resolve_hier_name(h);
                if let Some(w) = self
                    .lookup_signal_width(&n)
                    .or_else(|| self.packed_field_width(&n))
                {
                    return Some(w);
                }
                // §11.6.1 (#565): a subroutine formal or local is not a
                // module-scope signal; its frame value carries the declared
                // width (`logic [8:0] s` as a formal sizes `s === x + y`).
                if h.path.len() == 1
                    && h.path[0].selects.is_empty()
                    && let Some(v) = self
                        .local_stack
                        .last()
                        .and_then(|f| f.get(&h.path[0].name.name))
                    && !v.is_real
                    && v.width > 0
                {
                    return Some(v.width);
                }
                // §11.6.1 (#564): a dotted path rooted at a class VARIABLE
                // (`e.st.sa`, `e.arr[0]`) is sized by walking the declared
                // property types — no object is read.
                if h.path.len() > 1
                    && h.root.is_none()
                    && let Some(w) = self.class_var_path_width(h)
                {
                    return Some(w);
                }
                // A CLASS-FIELD operand (e.g. a `time` / `longint` member of
                // a `uvm_transaction`) isn't a module-scope signal, so the
                // width tables above miss it. Falling back to the live
                // receiver lets a comparison against a literal size the
                // operand correctly: `begin_time != -1` on a 64-bit `time`
                // field otherwise sized `-1` as 32-bit and compared
                // 64-bit-all-ones against 32'hFFFFFFFF as NOT equal.
                // `class_prop_receiver` resolves the receiver to a heap handle,
                // which for a call-rooted chain (`w.me().x`) would RUN the
                // callee. §13.4 lets a call be evaluated once per evaluation and
                // the owning statement owns that one, so only a call-free
                // operand may be probed here.
                if !Self::expr_contains_call(expr) {
                    if let Some((handle, prop)) = self.class_prop_receiver(expr) {
                        if let Some(w) = self.class_prop_width_of(handle, &prop) {
                            return Some(w);
                        }
                    }
                }
                // §11.6.1: ELABORATION FLATTENS an owner-qualified name. `E::ta`
                // does not stay a `MemberAccess` node — it arrives as a single
                // `Ident` whose path is ["E","ta"]. Every probe above keys on a
                // signal name or a live receiver, so a class-STATIC property read
                // from module scope matches none of them: the operand fell back to
                // its self-determined width and `s === E::ta + E::tb` compared the
                // 9-bit 300 against an 8-bit 44.
                //
                // Ask the class declaration table directly. `class_prop_width_impl`
                // with `handle = None` is the static path — no object, no
                // evaluation — so the §13.4 rule that a sizing probe must never RUN
                // a method still holds, and it stays None for every shape it cannot
                // size, which is what the single-segment cases above rely on.
                if h.root.is_none()
                    && h.path.len() > 1
                    && h.path.iter().all(|s| s.selects.is_empty())
                {
                    // The owner is the first segment and the property the last --
                    // exactly what joining the path with '.' and re-splitting it
                    // would yield, without the Vec and String allocation on a path
                    // this hot (`lrm_self_width` runs per operand per evaluation).
                    let owner = h
                        .path
                        .first()
                        .map(|s| s.name.name.as_str())
                        .unwrap_or("");
                    let prop = h
                        .path
                        .last()
                        .map(|s| s.name.name.as_str())
                        .unwrap_or("");
                    if !owner.is_empty() && !prop.is_empty() && owner != prop {
                        if let Some(w) = self
                            .class_prop_width_impl(owner, None, prop)
                            .filter(|w| *w > 0)
                        {
                            return Some(w);
                        }
                    }
                }
                None
            }
            ExprKind::Unary { op, operand } => match op {
                UnaryOp::Plus | UnaryOp::Minus | UnaryOp::BitNot => self.lrm_self_width(operand),
                UnaryOp::LogNot
                | UnaryOp::BitAnd
                | UnaryOp::BitOr
                | UnaryOp::BitXor
                | UnaryOp::BitNand
                | UnaryOp::BitNor
                | UnaryOp::BitXnor => Some(1),
                _ => None,
            },
            ExprKind::Binary { op, left, right } => match op {
                BinaryOp::Add
                | BinaryOp::Sub
                | BinaryOp::Mul
                | BinaryOp::Div
                | BinaryOp::Mod
                | BinaryOp::BitAnd
                | BinaryOp::BitOr
                | BinaryOp::BitXor
                | BinaryOp::BitXnor => {
                    Some(self.lrm_self_width(left)?.max(self.lrm_self_width(right)?))
                }
                BinaryOp::ShiftLeft
                | BinaryOp::ShiftRight
                | BinaryOp::ArithShiftLeft
                | BinaryOp::ArithShiftRight
                | BinaryOp::Power => self.lrm_self_width(left),
                BinaryOp::Eq
                | BinaryOp::Neq
                | BinaryOp::CaseEq
                | BinaryOp::CaseNeq
                | BinaryOp::WildcardEq
                | BinaryOp::WildcardNeq
                | BinaryOp::Lt
                | BinaryOp::Leq
                | BinaryOp::Gt
                | BinaryOp::Geq
                | BinaryOp::LogAnd
                | BinaryOp::LogOr => Some(1),
                _ => None,
            },
            ExprKind::Conditional {
                then_expr,
                else_expr,
                ..
            } => Some(
                self.lrm_self_width(then_expr)?
                    .max(self.lrm_self_width(else_expr)?),
            ),
            // A signal read in a concurrent assertion (`sva_resolve_signals`):
            // the signal's width, kept on the node, or a local variable's
            // (§16.10).
            ExprKind::SystemCall { name, args } if name == SVA_SIG_MARKER => {
                if let Some(w) = expr.cached_width.get() {
                    return Some(w);
                }
                let w = self.sva_sig_width(args)?;
                if args.len() == 1 {
                    expr.cached_width.set(Some(w));
                }
                Some(w)
            }
            // A select or a member of an aggregate: from the declaration when
            // that says. Else, in a concurrent assertion, the width of its
            // value: evaluating it for that runs once a tick, and is safe
            // when nothing in it has an effect (a call would run again).
            ExprKind::Index { .. }
            | ExprKind::RangeSelect { .. }
            | ExprKind::MemberAccess { .. } => {
                if let Some(w) = self.select_width(expr) {
                    return Some(w);
                }
                // A CLASS-MEMBER operand (`c.a`, `this.x`, `arg.a`) is not a
                // module signal, so `select_width` misses it and the comparison
                // used to fall to None — i.e. never widen. Size it from the
                // property's DECLARATION through the live receiver, as the
                // Ident arm above does for a bare class field.
                // `class_prop_receiver` resolves the receiver to a heap handle,
                // which for a call-rooted chain (`w.me().x`) would RUN the
                // callee. §13.4 allows a call ONE evaluation per operand and the
                // owning statement holds that one, so only a call-free operand
                // may be probed here.
                if !Self::expr_contains_call(expr) {
                    if let Some((handle, prop)) = self.class_prop_receiver(expr) {
                        if let Some(w) = self.class_prop_width_of(handle, &prop) {
                            return Some(w);
                        }
                    }
                }
                // Still unresolved: a packed-struct field, an array element, a
                // class STATIC, a package variable or a virtual-interface
                // member. All of those size from a DECLARATION table.
                if let Some(w) = self.declared_select_width(expr) {
                    return Some(w);
                }
                // §11.6.1 (#564): an element of an array, queue or
                // associative property reached from a class VARIABLE
                // (`e.arr[0]`), sized from the declared property.
                if let ExprKind::Index { expr: base, .. } = &expr.kind
                    && let ExprKind::Ident(h) = &base.kind
                    && h.root.is_none()
                    && h.path.len() > 1
                    && let Some(w) = self.class_var_path_width_impl(h, true)
                {
                    return Some(w);
                }
                (self.active_sva_site.is_some() && Self::cond_expr_is_effect_free(expr, false))
                    .then(|| self.eval_expr(expr).width)
            }
            // A function-call operand: the callee's DECLARED return width,
            // never by running the call (`call_return_width` is what
            // `infer_width` uses for exactly this reason). Without it
            // `f8a() + f8b()` sized as None and the comparison against a
            // 9-bit RHS stayed 8-bit, so 200+100 wrapped to 44.
            ExprKind::Call { func, .. } => self.call_return_width(func),
            // §6.24.1: a size cast `N'(x)` is N bits wide.
            ExprKind::SystemCall { name, args }
                if name == "$__xz_size_cast" || name == "$__xz_named_cast" =>
            {
                self.size_cast_width(name, args)
            }
            // §11.4.12: the operands of a concatenation side by side, `count`
            // times over for a replication.
            ExprKind::Concatenation(parts) => parts
                .iter()
                .try_fold(0, |w, p| Some(w + self.lrm_self_width(p)?)),
            ExprKind::Replication { count, exprs }
                if Self::cond_expr_is_effect_free(count, false) =>
            {
                let n = u32::try_from(self.eval_expr(count).to_u64()?).ok()?;
                let w = exprs
                    .iter()
                    .try_fold(0u32, |w, p| Some(w + self.lrm_self_width(p)?))?;
                n.checked_mul(w)
            }
            _ => None,
        }
    }

    /// Declaration-table width of a class-member / handle-member / select
    /// operand, without evaluating anything.
    ///
    /// `lrm_self_width` already resolves a live class member through
    /// `class_prop_receiver` + `class_prop_width_of`; what is left are the
    /// shapes that walk past that: a packed-struct FIELD of a class property
    /// (`c.st.sa`), an element of a class-owned array (`c.arr[i]`), a package
    /// variable (`pkg::pa`), a class static (`E::ta`) and a virtual-interface
    /// member (`vif.data`). All of them are answerable from the elaboration
    /// tables, so the width path stays side-effect free.
    /// The declared width of `v.p1.p2...` where `v` is a class-typed
    /// variable: each segment is a property of the class so far, the
    /// walk descends through class-typed properties and, for a packed
    /// struct property, ends at a field; an indexed last segment is an
    /// element of an array, queue or associative property (§11.6.1).
    fn class_var_path_width(&mut self, h: &HierarchicalIdentifier) -> Option<u32> {
        self.class_var_path_width_impl(h, false)
    }

    /// `class_var_path_width` for a path whose last segment is indexed by
    /// an enclosing `Index` node (`e.arr[0]` parses that way).
    fn class_var_path_width_impl(
        &mut self,
        h: &HierarchicalIdentifier,
        indexed_last: bool,
    ) -> Option<u32> {
        let decl = self.class_of_var(&h.path[0].name.name)?;
        let mut cls = decl.split('#').next().unwrap_or(&decl).to_string();
        let mut i = 1;
        while i < h.path.len() {
            let seg = &h.path[i];
            let name = seg.name.name.clone();
            let last = i + 1 == h.path.len();
            let owners = self.prop_owners(&cls);
            let (_, cd) = owners.get(&name)?;
            let elem_w = cd
                .array_properties
                .get(&name)
                .map(|&(_, _, w)| w)
                .or_else(|| cd.queue_properties.get(&name).map(|(w, _)| *w))
                .or_else(|| cd.assoc_index_props.get(&name).map(|(w, _)| *w));
            let type_name = cd.properties.get(&name).and_then(|s| s.type_name.clone());
            let whole_w = cd.properties.get(&name).map(|s| s.value.width);
            if last {
                return if seg.selects.is_empty() && !indexed_last {
                    whole_w.filter(|w| *w > 0)
                } else {
                    elem_w.filter(|w| *w > 0)
                };
            }
            if !seg.selects.is_empty() {
                return None;
            }
            let tn = type_name?;
            if self.module.classes.contains_key(&tn) {
                cls = tn;
                i += 1;
                continue;
            }
            // A packed struct property: the rest of the path names a field.
            let dt = self.p_type_by_name(&tn)?;
            let fields = xezim_core::elaborate::packed_struct_field_layout(
                &dt,
                &self.module.parameters,
                &self.module.typedefs,
                &self.module.typedef_types,
            )?;
            let rest: Vec<&str> = h.path[i + 1..].iter().map(|s| s.name.name.as_str()).collect();
            if h.path[i + 1..].iter().any(|s| !s.selects.is_empty()) {
                return None;
            }
            let leaf = *rest.last()?;
            let joined = rest.join(".");
            return fields
                .iter()
                .find(|(f, _, _)| f.trim_start_matches('.') == joined)
                .or_else(|| fields.iter().find(|(f, _, _)| f.rsplit('.').next() == Some(leaf)))
                .map(|(_, _, w)| *w)
                .filter(|w| *w > 0);
        }
        None
    }

    fn declared_select_width(&mut self, expr: &Expression) -> Option<u32> {
        use crate::ast::expr::ExprKind;
        // A chain rooted at a CALL (`w.me().x`, `q.pop_front().addr`) is owned
        // by the statement that memoized its receiver: probing it here would
        // run the callee again and break §13.4's "one call per evaluation".
        // Those shapes stay on the memoized path.
        if Self::expr_contains_call(expr) {
            return None;
        }
        // (a) `base.member`: the field width comes from the packed layout of
        //     the property that `base` names.
        if let ExprKind::MemberAccess { expr: base, member } = &expr.kind {
            if let Some((_cls, _prop, _h, dt)) = self.declared_prop_chain_type(base) {
                if let Some(fields) = xezim_core::elaborate::packed_struct_field_layout(
                    &dt,
                    &self.module.parameters,
                    &self.module.typedefs,
                    &self.module.typedef_types,
                ) {
                    if let Some((_, _, w)) = fields.iter().find(|(f, _, _)| {
                        f.rsplit('.').next() == Some(member.name.as_str())
                    }) {
                        if *w > 0 {
                            return Some(*w);
                        }
                    }
                }
            }
        }
        // (b) `base[i]` / `base[a:b]`: the element width of the collection
        //     that `base` names.
        if let ExprKind::Index { expr: base, .. } | ExprKind::RangeSelect { expr: base, .. } =
            &expr.kind
        {
            if let Some((cls, prop, _h, dt)) = self.declared_prop_chain_type(base) {
                if let Some(cd) = self.module.classes.get(&cls) {
                    let hit = cd
                        .array_properties
                        .get(&prop)
                        .map(|&(_, _, w)| w)
                        .or_else(|| cd.queue_properties.get(&prop).map(|(w, _)| *w))
                        .or_else(|| cd.assoc_index_props.get(&prop).map(|(w, _)| *w))
                        .or_else(|| {
                            cd.static_fixed_arrays
                                .iter()
                                .find(|(n, _, _, _)| *n == prop)
                                .map(|&(_, _, _, w)| w)
                        })
                        .or_else(|| {
                            cd.static_collections
                                .iter()
                                .find(|(n, _, _)| *n == prop)
                                .map(|&(_, _, w)| w)
                        });
                    if let Some(w) = hit.filter(|w| *w > 0) {
                        return Some(w);
                    }
                }
                // A declared packed type narrower than the whole property is
                // the ELEMENT type of an array-like property. A plain vector
                // select is answered by `select_width` before reaching here.
                let whole = self.class_prop_width_impl(&cls, None, &prop).unwrap_or(0);
                let ew = xezim_core::elaborate::resolve_type_width(
                    &dt,
                    Some(&self.module.parameters),
                    Some(&self.module.typedefs),
                );
                if ew > 0 && ew < whole {
                    return Some(ew);
                }
            }
        }
        // (c) Owner-qualified names: `pkg::pa`, `E::ta`, `ifc.q1`.
        let cands: Vec<String> = match self.declared_dotted_name(expr) {
            Some(d) => vec![d.clone(), format!("{}.{}", d, d)],
            None => Vec::new(),
        };
        for cand in &cands {
            let owner = cand.split('.').next().unwrap_or("");
            let prop = cand.rsplit('.').next().unwrap_or("");
            if owner.is_empty() || prop.is_empty() || owner == prop {
                continue;
            }
            if let Some(w) = self.class_prop_width_impl(owner, None, prop).filter(|w| *w > 0) {
                return Some(w);
            }
            if let Some(Some(h)) = self.this_stack.last().copied() {
                if let Some(w) = self.class_prop_width_impl(owner, Some(h), prop).filter(|w| *w > 0) {
                    return Some(w);
                }
            }
            if self.module.packages.contains(owner) {
                if let Some(w) = self.lookup_signal_width(prop).filter(|w| *w > 0) {
                    return Some(w);
                }
            }
            if let Some(w) = self.lookup_signal_width(cand).filter(|w| *w > 0) {
                return Some(w);
            }
        }
        // (d) Virtual-interface member: `vif.data` sizes from the interface
        //     instance the handle's `vif` property is bound to.
        if let ExprKind::MemberAccess { expr: base, member } = &expr.kind {
            // The receiver of a virtual-interface member is the property that
            // HOLDS the vif (`vif` in `vif.data`); it is declared in
            // `virtual_iface_properties`, not `property_types`, so the class is
            // taken from the live receiver (or the enclosing class) instead of
            // from a property table lookup.
            let live_cls = self
                .heap
                .get(self.this_stack.last().copied().flatten().unwrap_or(0))
                .and_then(|o| o.as_ref().map(|i| i.class_name.clone()))
                .or_else(|| self.class_context_stack.last().and_then(|c| c.clone()));
            // The vif property is the RECEIVER (`vif` in `vif.data`), not the
            // member being read.
            let recv_prop = match &base.kind {
                ExprKind::Ident(h) => h.path.last().map(|s| s.name.name.clone()),
                ExprKind::MemberAccess { member, .. } => Some(member.name.clone()),
                _ => None,
            };
            if let Some(recv_prop) = recv_prop {
            if let Some(cls) = self
                .declared_prop_chain_class(base)
                .or_else(|| live_cls)
            {
                if self.class_vif_decl(&cls, &recv_prop).is_some() {
                    if let Some(h) = self.this_stack.last().copied().flatten() {
                        if let Some((bound, _mp)) = self
                            .virtual_iface_bindings
                            .get(&(h, recv_prop.clone()))
                            .cloned()
                        {
                            let sig = format!("{}.{}", bound, member.name);
                            if let Some(w) = self.lookup_signal_width(&sig).filter(|w| *w > 0) {
                                return Some(w);
                            }
                        }
                    }
                }
            }
            if let Some(bound) = self.iface_alias_for(&recv_prop) {
                let sig = format!("{}.{}", bound, member.name);
                if let Some(w) = self.lookup_signal_width(&sig).filter(|w| *w > 0) {
                    return Some(w);
                }
            }
            }
        }
        None
    }

    /// The class that owns the property named at the end of a plain property
    /// chain (`c.st`, `st`, `this.st`), the property name, the receiver handle
    /// when one is known, and the property's declared `DataType`.
    fn declared_prop_chain_type(
        &mut self,
        expr: &Expression,
    ) -> Option<(String, String, Option<usize>, crate::ast::types::DataType)> {
        use crate::ast::expr::ExprKind;
        let (base, prop) = match &expr.kind {
            ExprKind::MemberAccess { expr, member } => (Some(&**expr), member.name.as_str()),
            ExprKind::Ident(name) => (None, name.path.last()?.name.name.as_str()),
            _ => return None,
        };
        fn this_cls(sim: &mut Simulator) -> Option<(usize, String)> {
            let h = sim.this_stack.last().copied().flatten()?;
            let cls = sim.heap.get(h)?.as_ref()?.class_name.clone();
            Some((h, cls))
        }
        let (handle, cls) = match base {
            None => {
                let (h, cls) = this_cls(self)?;
                (Some(h), cls)
            }
            Some(b) => match &b.kind {
                ExprKind::This => {
                    let (h, cls) = this_cls(self)?;
                    (Some(h), cls)
                }
                ExprKind::Ident(name) => {
                    let n = name.path.last()?.name.name.as_str();
                    if n == "this" {
                        let (h, cls) = this_cls(self)?;
                        (Some(h), cls)
                    } else {
                        let v = if let Some(locals) = self.local_stack.last() {
                            locals.get(n).cloned()
                        } else {
                            None
                        }
                        .or_else(|| self.get_signal_value_by_name(n))?;
                        let h = v.to_u64()? as usize;
                        if h == 0 {
                            return None;
                        }
                        (Some(h), self.heap.get(h)?.as_ref()?.class_name.clone())
                    }
                }
                _ => return None,
            },
        };
        let owners = self.prop_owners(&cls);
        let (ocn, cd) = owners.get(prop)?;
        let dt = cd.property_types.get(prop)?.clone();
        Some((ocn.clone(), prop.to_string(), handle, dt))
    }

    /// Just the owning class of [`Self::declared_prop_chain_type`], for the
    /// shapes that only need the class (virtual-interface declarations).
    fn declared_prop_chain_class(&mut self, expr: &Expression) -> Option<String> {
        use crate::ast::expr::ExprKind;
        let inner = match &expr.kind {
            ExprKind::MemberAccess { expr: base, .. } | ExprKind::Index { expr: base, .. } => {
                base.clone()
            }
            ExprKind::Ident(_) => Box::new(expr.clone()),
            _ => return None,
        };
        self.declared_prop_chain_type(&inner).map(|(c, _, _, _)| c)
    }

    /// Dotted name of a plain member-access chain (`c.st.sa` -> `"c.st.sa"`,
    /// `"e.st.sa"`, `arr` -> `"arr"`), built from the AST alone. None for any
    /// shape that is not a pure chain of bare identifiers — a select, a call,
    /// a scoped `::` name — so callers never mistake those for a struct field.
    fn declared_dotted_name(&self, expr: &Expression) -> Option<String> {
        match &expr.kind {
            ExprKind::Ident(h) => {
                if h.root.is_some()
                    || h.path.is_empty()
                    || h.path.iter().any(|s| !s.selects.is_empty())
                {
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
            ExprKind::MemberAccess { expr, member } => {
                let base = self.declared_dotted_name(expr)?;
                Some(format!("{base}.{}", member.name))
            }
            _ => None,
        }
    }

    /// The width of a select of a signal or a one-dimensional array, from the
    /// declaration alone: a bit of a vector, an element of an array, `|l - r|
    /// + 1` bits for a part-select `[l:r]` and `w` for `[i +: w]`. None for
    /// any other base (a packed array of vectors, a local variable) and for
    /// bounds that are not unsized decimal literals. A select of a vector is
    /// as wide as its value always is, so the answer is kept on the node, as
    /// `infer_width` keeps its own.
    pub(super) fn select_width(&mut self, expr: &Expression) -> Option<u32> {
        if let Some(w) = expr.cached_width.get() {
            return Some(w);
        }
        // An unsized decimal literal is its digits. A sized one can be
        // truncated (`3'd15` is 7), so it is left to the caller.
        let lit = |e: &Expression| match &e.kind {
            ExprKind::Number(NumberLiteral::Integer {
                size: None,
                base: NumberBase::Decimal,
                value,
                ..
            }) => value.replace('_', "").parse::<i64>().ok(),
            _ => None,
        };
        let (base, bits) = match &expr.kind {
            ExprKind::Index { expr: base, .. } => (base, None),
            ExprKind::RangeSelect {
                expr: base,
                kind,
                left,
                right,
            } => {
                let w = match kind {
                    RangeKind::Constant => lit(left)?.abs_diff(lit(right)?) + 1,
                    RangeKind::IndexedUp | RangeKind::IndexedDown => lit(right)?.unsigned_abs(),
                };
                (base, Some(u32::try_from(w).ok()?))
            }
            _ => return None,
        };
        let ExprKind::Ident(h) = &base.kind else {
            return None;
        };
        if h.root.is_some() || h.path.iter().any(|s| !s.selects.is_empty()) {
            return None;
        }
        if let [seg] = h.path.as_slice()
            && self
                .local_stack
                .last()
                .is_some_and(|m| m.contains_key(&seg.name.name))
        {
            return None;
        }
        let n = self.resolve_hier_name(h);
        let n = n.as_ref();
        if let Some(t) = self.module.arrays.get(n) {
            return bits.is_none().then_some(t.2);
        }
        let id = *self.signal_name_to_id.get(n)?;
        let vector = !self.signal_real[id]
            && !self.module.packed_signal_elem_widths.contains_key(n)
            && !self.module.packed_full_dims.contains_key(n)
            && !self.module.arrays_2d.contains_key(n)
            && !self.module.arrays_nd.contains_key(n)
            && !self.module.dynamic_arrays.contains(n)
            && !self.module.associative_arrays.contains_key(n)
            && !self.string_signals.contains(n);
        if !vector {
            return None;
        }
        let w = bits.unwrap_or(1);
        expr.cached_width.set(Some(w));
        Some(w)
    }

    /// The width of a size cast `N'(x)` (§6.24.1): N, a literal or a name
    /// that is a constant (a parameter), as the cast's evaluation reads it.
    /// None for a cast to a type.
    pub(super) fn size_cast_width(&mut self, name: &str, args: &[Expression]) -> Option<u32> {
        let n = args.first()?;
        if name == "$__xz_named_cast" {
            let key = self.named_cast_key(n)?;
            let constant = !self.module.typedef_types.contains_key(&key)
                && !self.module.typedefs.contains_key(&key)
                && (self.module.parameters.contains_key(&key)
                    || self.get_signal_value_by_name(&key).is_some()
                    || self.class_value_param_in_scope(&key));
            if !constant {
                return None;
            }
        }
        let v = self.eval_expr(n);
        if v.has_xz() {
            return None;
        }
        u32::try_from(v.to_u64()?).ok().filter(|&w| w > 0)
    }

    /// The width of a packed struct or union member (`s.f`), which has no
    /// signal of its own: from its parent's field layout.
    pub(super) fn packed_field_width(&self, name: &str) -> Option<u32> {
        let (parent, field) = name.rsplit_once('.')?;
        let fields = self.module.packed_struct_fields.get(parent)?;
        fields.iter().find(|(m, _, _)| m == field).map(|f| f.2)
    }

    /// §11.4.13: `expr inside {items}` compares like `==`, so every operand
    /// is sized to the widest of them all. Returns that width when it exceeds
    /// the tested expression's own (None: evaluate as before — a string or
    /// real operand, an operand the LRM width rules cannot size without
    /// evaluating it, or nothing wider). An unpacked-array item contributes
    /// its element width; a `$` bound contributes nothing.
    pub(super) fn inside_joint_width(
        &mut self,
        expr: &Expression,
        items: &[Expression],
    ) -> Option<u32> {
        let mut bounds: Vec<(&Expression, bool)> = Vec::with_capacity(items.len());
        for it in items {
            match &it.kind {
                ExprKind::Range(lo, hi) => {
                    bounds.push((lo, false));
                    bounds.push((hi, false));
                }
                _ => bounds.push((it, true)),
            }
        }
        self.inside_joint_width_of(expr, &bounds)
    }

    /// `inside_joint_width` for a constraint's range list.
    pub(super) fn inside_joint_width_cr(
        &mut self,
        expr: &Expression,
        ranges: &[ConstraintRange],
    ) -> Option<u32> {
        if !matches!(
            Self::unparen(expr).kind,
            ExprKind::Binary { .. } | ExprKind::Unary { .. } | ExprKind::Conditional { .. }
        ) {
            return None;
        }
        let mut bounds: Vec<(&Expression, bool)> = Vec::with_capacity(ranges.len());
        for r in ranges {
            match r {
                ConstraintRange::Range { lo, hi } => {
                    bounds.push((lo, false));
                    bounds.push((hi, false));
                }
                ConstraintRange::Value(e) => bounds.push((e, true)),
            }
        }
        self.inside_joint_width_of(expr, &bounds)
    }

    /// The joint width of `expr` and `bounds` (each flagged when it is a
    /// whole set item, which may name an unpacked array).
    pub(super) fn inside_joint_width_of(
        &mut self,
        expr: &Expression,
        bounds: &[(&Expression, bool)],
    ) -> Option<u32> {
        // §11.4.13: EVERY operand is sized to the widest of them all, and the
        // widening matters for any operand whose RESULT depends on the width
        // it is computed at — an arithmetic/relational/conditional operation.
        // Either side can be that operation: `x inside {a+b}` and
        // `a+b inside {x}` both have to size `a+b` at the joint width, so the
        // tested expression being a plain variable is no reason to bail.
        let op_shaped = |e: &Expression| {
            matches!(
                Self::unparen(e).kind,
                ExprKind::Binary { .. } | ExprKind::Unary { .. } | ExprKind::Conditional { .. }
            )
        };
        if self.operand_is_string(expr) {
            return None;
        }
        let own = self.lrm_self_width(expr)?;
        // §11.4.13: 64-bit integers still supply context to narrower
        // range expressions. Width alone does not make an operand real.
        if self.expr_is_real_static(expr) || self.call_returns_real(expr) {
            return None;
        }
        let mut w = own;
        // The narrowest operand that would change value if computed at `w`.
        let mut narrowest = op_shaped(expr).then_some(own);
        for &(b, item) in bounds {
            if matches!(b.kind, ExprKind::Dollar) {
                continue;
            }
            if item && let Some(nm) = self.array_operand_name(b) {
                if self.get_queue_size(&nm) > 0 {
                    let ev = self.get_signal_value_by_name(&format!("{}[0]", nm))?;
                    if ev.is_real {
                        return None;
                    }
                    w = w.max(ev.width);
                }
                continue;
            }
            if self.operand_is_string(b)
                || self.expr_is_real_static(b)
                || self.call_returns_real(b)
            {
                return None;
            }
            let bw = self.lrm_self_width(b)?;
            w = w.max(bw);
            if op_shaped(b) {
                narrowest = Some(narrowest.map_or(bw, |n| n.min(bw)));
            }
        }
        (w > own || narrowest.is_some_and(|n| w > n)).then_some(w)
    }

    /// §12.5: a case expression and its item expressions are sized to the
    /// widest of them, so the selector is evaluated at that width: `case
    /// (a8 + 8'd1) 256: ...` adds at 32 bits. A `case inside` selector is
    /// sized by its value items only — the reference simulator does not
    /// widen it for a range item.
    pub(super) fn case_selector_value(
        &mut self,
        kind: &CaseKind,
        expr: &Expression,
        items: &[crate::ast::stmt::CaseItem],
    ) -> Value {
        if !matches!(
            Self::unparen(expr).kind,
            ExprKind::Binary { .. } | ExprKind::Unary { .. } | ExprKind::Conditional { .. }
        ) {
            return self.eval_expr(expr);
        }
        let inside = matches!(kind, CaseKind::CaseInside);
        let bounds: Vec<(&Expression, bool)> = items
            .iter()
            .filter(|it| !it.is_default)
            .flat_map(|it| it.patterns.iter())
            .filter(|p| !matches!(p.kind, ExprKind::Range(..)))
            .map(|p| (p, inside))
            .collect();
        match self.inside_joint_width_of(expr, &bounds) {
            Some(w) => self.eval_expr_ctx(expr, w),
            None => self.eval_expr(expr),
        }
    }

    /// Self-determined width of `expr`. Cached on the node when the answer
    /// is a static property of it (signal and packed-layout widths, literals,
    /// declared system-function results, and compositions of those); a width
    /// that depends on runtime state — a local's current value, a user
    /// function's return type, or the evaluate-and-measure fallback — is
    /// recomputed every time. The binary arm of the evaluator asks for both
    /// operand widths on every arithmetic evaluation, each a full tree walk
    /// (7 % of an axi4 run before the cache).
    pub(super) fn infer_width(&mut self, expr: &Expression) -> u32 {
        if let Some(w) = expr.cached_width.get() {
            return w;
        }
        let (w, is_static) = self.infer_width_walk(expr);
        if is_static {
            expr.cached_width.set(Some(w));
        }
        w
    }

    /// `infer_width` for an operand that is about to be EVALUATED anyway.
    /// Shapes the walk can only size by evaluating (a property read, an
    /// element select, a method whose return type it cannot resolve) are
    /// evaluated here ONCE and the value handed back, so the caller does not
    /// evaluate them a second time. The binary arm probed both operands of
    /// every `+`/`&`/... and then evaluated them again: every `obj.prop + 1`
    /// read the property twice, and a method in an arithmetic expression ran
    /// twice (6 % of an axi4 run). Static shapes go through the cached walk
    /// and return no value.
    pub(super) fn width_probe(&mut self, expr: &Expression) -> (u32, Option<Value>) {
        if let Some(w) = expr.cached_width.get() {
            return (w, None);
        }
        let evaluates = match &expr.kind {
            ExprKind::Ident(_)
            | ExprKind::Number(NumberLiteral::Integer { .. })
            | ExprKind::Concatenation(_)
            | ExprKind::Paren(_)
            | ExprKind::AssignExpr { .. }
            | ExprKind::Binary { .. }
            | ExprKind::Unary { .. }
            | ExprKind::Conditional { .. } => false,
            ExprKind::SystemCall { name, args } => {
                !(vpi_systf_is_func(name)
                    || (super::bytecode::system_function_carries_arg(name) && !args.is_empty())
                    || super::bytecode::system_function_result(name).is_some())
            }
            ExprKind::Call { func, .. } => {
                if let Some(w) = self.call_return_width(func) {
                    return (w, None);
                }
                true
            }
            // A streaming concatenation pads to the CONTEXT width on the
            // right; a value taken here would be extended on the left.
            ExprKind::StreamOp { .. } => false,
            _ => true,
        };
        if evaluates {
            let v = self.eval_expr(expr);
            return (v.width, Some(v));
        }
        (self.infer_width(expr), None)
    }

    /// `infer_width` body; the flag says whether every branch taken was
    /// static, i.e. safe to pin on the node.
    pub(super) fn infer_width_walk(&mut self, expr: &Expression) -> (u32, bool) {
        match &expr.kind {
            ExprKind::Ident(h) => {
                let n = self.resolve_hier_name(h);
                // The resolution is node-stable only once the resolver has
                // pinned the name on the node.
                let stable = h.cached_resolved_name.get().is_some();
                if let Some(w) = self.lookup_signal_width(&n) {
                    return (w, stable);
                }
                // A method formal or local variable has no signal entry —
                // infer from the local's current Value width. Without this,
                // `infer_width` fell back to 1 for ref parameters of
                // class-handle type, truncating assoc-array `first(ref k)` /
                // `next(ref k)` key writeback to a single bit (handle 2 → 0).
                if h.path.len() == 1 {
                    let name = &h.path[0].name.name;
                    if let Some(w) = self
                        .local_stack
                        .last()
                        .and_then(|m| m.get(name))
                        .map(|v| v.width)
                        .filter(|w| *w > 1)
                    {
                        return (w, false);
                    }
                }
                // A packed struct/union member (`word3.high`) has no leaf
                // signal; its width comes from the parent's field layout. Needed
                // so a concatenation LHS (`{word3.high, word3.low} = ...`) splits
                // the rvalue at the right bit boundaries.
                if let Some(w) = self.packed_field_width(&n) {
                    return (w, stable);
                }
                // Class properties have no signal entry. In particular a
                // concatenation lvalue must split its RHS using each declared
                // property width, not the one-bit unknown-name fallback.
                // Receiver/parameter-dependent widths cannot be node-cached.
                if let Some(w) = self.lrm_self_width(expr) {
                    return (w, false);
                }
                (1, false)
            }
            ExprKind::Number(NumberLiteral::Integer { size, .. }) => (size.unwrap_or(32), true),
            ExprKind::Concatenation(p) => {
                let mut total = 0;
                let mut st = true;
                for x in p {
                    total += self.infer_width(x);
                    st &= x.cached_width.get().is_some();
                }
                (total, st)
            }
            ExprKind::Paren(inner) => (self.infer_width(inner), inner.cached_width.get().is_some()),
            ExprKind::AssignExpr { lvalue, .. } => (
                self.infer_width(lvalue),
                lvalue.cached_width.get().is_some(),
            ),
            ExprKind::Binary { left, right, .. } => {
                let w = self.infer_width(left).max(self.infer_width(right));
                (
                    w,
                    left.cached_width.get().is_some() && right.cached_width.get().is_some(),
                )
            }
            ExprKind::Unary { operand, .. } => (
                self.infer_width(operand),
                operand.cached_width.get().is_some(),
            ),
            ExprKind::Conditional {
                then_expr,
                else_expr,
                ..
            } => {
                let w = self.infer_width(then_expr).max(self.infer_width(else_expr));
                (
                    w,
                    then_expr.cached_width.get().is_some()
                        && else_expr.cached_width.get().is_some(),
                )
            }
            // A VPI system function has a declared return width. The fallback
            // below infers a width by EVALUATING, which for a `$systf` means
            // running its calltf — twice, once here and once for the value.
            ExprKind::SystemCall { name, .. } if vpi_systf_is_func(name) => {
                (vpi_sysfunc_ret_width(name), true)
            }
            // A built-in system function's width comes from its LRM result
            // type. Sizing it by EVALUATING (the fallback below) ran the
            // call twice — `$urandom % n` advanced the generator twice and
            // `$fgetc(fd) & mask` consumed two bytes.
            ExprKind::SystemCall { name, args }
                if super::bytecode::system_function_carries_arg(name) && !args.is_empty() =>
            {
                (
                    self.infer_width(&args[0]),
                    args[0].cached_width.get().is_some(),
                )
            }
            ExprKind::SystemCall { name, .. }
                if super::bytecode::system_function_result(name).is_some() =>
            {
                (
                    super::bytecode::system_function_result(name)
                        .map(|(w, _)| w)
                        .unwrap_or(32),
                    true,
                )
            }
            // Same for a user function: its width comes from its declared
            // return type, never from calling it. `f() + 1` used to call `f`
            // once for the width and once for the value, so any function with
            // a side effect ran twice.
            ExprKind::Call { func, .. } => {
                if let Some(w) = self.call_return_width(func) {
                    (w, false)
                } else {
                    (self.eval_expr(expr).width, false)
                }
            }
            _ => (self.eval_expr(expr).width, false),
        }
    }
    /// Cheaply test whether the method name of a call callee (`recv.m(...)`
    /// as `MemberAccess`, or a 2+-segment `Ident`) satisfies `pred`, WITHOUT
    /// running `flat_member_name` over the receiver. Used to gate the
    /// expensive `collection_method_of` so it only runs for names it could
    /// possibly answer (size/num/len/pop_*). See `call_return_width`.
    pub(super) fn callee_method_name_is<F: Fn(&str) -> bool>(
        &self,
        func: &Expression,
        pred: F,
    ) -> bool {
        match &func.kind {
            ExprKind::MemberAccess { member, .. } => pred(&member.name),
            ExprKind::Ident(h) if h.path.len() >= 2 => {
                pred(&h.path.last().expect("len>=2").name.name)
            }
            _ => false,
        }
    }
}
