//! Simulator isolated gates.

use super::*;

/// A signal view may publish in-place on the coordinator or stage writes in
/// a worker-local overlay. No worker obtains a mutable reference to the base.
pub(super) trait IsolatedSignalView:
    std::ops::Index<usize, Output = Value> + std::ops::IndexMut<usize>
{
    fn signal_len(&self) -> usize;
}
impl IsolatedSignalView for [Value] {
    fn signal_len(&self) -> usize {
        self.len()
    }
}
impl IsolatedSignalView for Vec<Value> {
    fn signal_len(&self) -> usize {
        self.len()
    }
}

/// Owned worker instructions carry no shared mutable AST/cache payloads. The
/// positive opcode check is also the transport contract of the edge workers.
pub(super) struct OwnedBspItem(CombItem);
// SAFETY: construction is private and rejects AST-bearing opcodes and mutable
// class/collection operations. Allowed payloads are owned or immutable values.
unsafe impl Send for OwnedBspItem {}
impl OwnedBspItem {
    pub(super) fn supported(item: &CombItem) -> bool {
        use super::super::bytecode::Insn;
        match item {
            CombItem::Noop
            | CombItem::FastDirectCopy { .. }
            | CombItem::FastDirectFanout { .. }
            | CombItem::DirectCopy { .. }
            | CombItem::FusedGate { .. }
            | CombItem::FusedBufFanout { .. }
            | CombItem::FusedAndFanout { .. } => true,
            CombItem::CompiledContAssign { compiled }
            | CombItem::CompiledAlwaysBlock { compiled, .. } => {
                compiled.instructions.iter().all(|i| {
                    Simulator::isolated_instruction_supported(i)
                        || matches!(
                            i,
                            Insn::BlockingAssign(..)
                                | Insn::BlockingAssignRange(..)
                                | Insn::BlockingAssignBitDyn(..)
                        )
                })
            }
            _ => false,
        }
    }
    pub(super) fn new(item: &CombItem) -> Self {
        assert!(Self::supported(item));
        Self(item.clone())
    }
    pub(super) fn item(&self) -> &CombItem {
        &self.0
    }
}

pub(super) struct StagedSignals<'a> {
    base: &'a [Value],
    writes: HashMap<usize, Value>,
}
impl<'a> StagedSignals<'a> {
    pub(super) fn new(base: &'a [Value]) -> Self {
        Self {
            base,
            writes: HashMap::default(),
        }
    }
    pub(super) fn finish(mut self, dirtied: Vec<u32>) -> Vec<(u32, Value)> {
        let mut result = Vec::with_capacity(self.writes.len());
        for id in dirtied {
            if let Some(value) = self.writes.remove(&(id as usize)) {
                result.push((id, value));
            }
        }
        result
    }
}
impl IsolatedSignalView for StagedSignals<'_> {
    fn signal_len(&self) -> usize {
        self.base.len()
    }
}
impl std::ops::Index<usize> for StagedSignals<'_> {
    type Output = Value;
    fn index(&self, index: usize) -> &Value {
        self.writes.get(&index).unwrap_or(&self.base[index])
    }
}
impl std::ops::IndexMut<usize> for StagedSignals<'_> {
    fn index_mut(&mut self, index: usize) -> &mut Value {
        self.writes
            .entry(index)
            .or_insert_with(|| self.base[index].clone())
    }
}

impl Simulator {
    /// Isolated counterpart of `exec_fused_gate`: evaluates a fused 1-bit
    /// gate against a mutable signal `view`, recording the dirtied id.
    pub(super) fn exec_fused_gate_isolated<V: IsolatedSignalView + ?Sized>(
        op: &FusedGate,
        view: &mut V,
        dirtied: &mut Vec<u32>,
        gate_driven: &[bool],
    ) {
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
                let s = view[src.sig_id as usize].get_bit_code(src.bit as usize);
                let v = if *invert {
                    not4(s)
                } else if gate_driven
                    .get(dst.sig_id as usize)
                    .copied()
                    .unwrap_or(false)
                {
                    // §28.4 `buf` primitive: a z input drives x.
                    match s {
                        3 => 2,
                        other => other,
                    }
                } else {
                    // §10.3 continuous assign passes z through.
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
                let va = view[a.sig_id as usize].get_bit_code(a.bit as usize);
                let vb = view[b.sig_id as usize].get_bit_code(b.bit as usize);
                let r = match gop {
                    GateBin::And => and4(va, vb),
                    GateBin::Or => or4(va, vb),
                    GateBin::Xor => xor4(va, vb),
                };
                (dst, if *invert { not4(r) } else { r })
            }
            FusedGate::Mux2 { dst, s, t, e } => {
                let vs = view[s.sig_id as usize].get_bit_code(s.bit as usize);
                let vt = view[t.sig_id as usize].get_bit_code(t.bit as usize);
                let ve = view[e.sig_id as usize].get_bit_code(e.bit as usize);
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
                    let code = view[input.sig_id as usize].get_bit_code(input.bit as usize);
                    lut_idx += usize::from(code.min(2)) * place;
                    place *= 3;
                }
                (dst, ((table >> (lut_idx * 2)) & 3) as u8)
            }
        };
        let id = dst.sig_id as usize;
        if view[id].set_bit_code(dst.bit as usize, new_bit) {
            dirtied.push(id as u32);
        }
    }

    #[inline]
    pub(super) fn exec_fused_and_fanout_isolated<V: IsolatedSignalView + ?Sized>(
        common: BitRef,
        branches: &[FusedAndBranch],
        invert: bool,
        view: &mut V,
        dirtied: &mut Vec<u32>,
    ) {
        let common_code = view[common.sig_id as usize].get_bit_code(common.bit as usize);
        for branch in branches {
            let other = view[branch.other.sig_id as usize].get_bit_code(branch.other.bit as usize);
            let and = if common_code == 0 || other == 0 {
                0
            } else if common_code == 1 && other == 1 {
                1
            } else {
                2
            };
            let result = if invert {
                match and {
                    0 => 1,
                    1 => 0,
                    _ => 2,
                }
            } else {
                and
            };
            let id = branch.dst.sig_id as usize;
            if view[id].set_bit_code(branch.dst.bit as usize, result) {
                dirtied.push(id as u32);
            }
        }
    }

    /// Evaluate one comb entry against a mutable signal `view`, appending
    /// the ids of any signals it changes to `dirtied`. Returns false if the
    /// entry is an AST-fallback variant the isolated path can't handle (the
    /// caller routes those to the main thread). Used by the BSP settle.
    pub(super) fn eval_comb_entry_isolated<V: IsolatedSignalView + ?Sized>(
        &self,
        eidx: usize,
        view: &mut V,
        vm_regs: &mut Vec<Value>,
        dirtied: &mut Vec<u32>,
    ) -> bool {
        match &self.comb_entries[eidx].item {
            CombItem::Noop => true,
            // §29 UDPs are never evaluated on this isolated (parallel) path —
            // `has_udp` forces the serial settle. Present only for exhaustiveness.
            // Timing checks likewise (`has_timing_checks`).
            CombItem::Udp { .. } | CombItem::UdpBatch { .. } | CombItem::TimingCheck { .. } => true,
            // Serial-only in v1: fall back to the full-simulator path.
            CombItem::GateRegion { .. }
            | CombItem::VectorGate { .. }
            | CombItem::ScatterGate { .. }
            | CombItem::CtlGate { .. } => false,
            CombItem::FastDirectCopy { dst_id, src_id } => {
                // LRM §9.3.1: skip if the destination is forced
                if self.forced_signals.is_empty() || !self.forced_signals.contains_key(dst_id) {
                    let (mut sv, mut sx) = view[*src_id].raw_bits();
                    // §6.11.1/§10.7: a 2-state destination drops X/Z on every
                    // write (X/Z -> 0). The raw-bit copy would otherwise keep it.
                    if sx != 0 && self.signal_two_state.get(*dst_id).copied().unwrap_or(false) {
                        sv &= !sx;
                        sx = 0;
                    }
                    let (dv, dx) = view[*dst_id].raw_bits();
                    if (sv != dv || sx != dx) && view[*dst_id].set_inline_bits(sv, sx) {
                        dirtied.push(*dst_id as u32);
                    }
                }
                true
            }
            CombItem::FastDirectFanout { src_id, dst_ids } => {
                let (src_v, src_x) = view[*src_id].raw_bits();
                let any_forced = !self.forced_signals.is_empty();
                for &dst_id in dst_ids.iter() {
                    if any_forced && self.forced_signals.contains_key(&dst_id) {
                        continue;
                    }
                    let (mut sv, mut sx) = (src_v, src_x);
                    if sx != 0 && self.signal_two_state.get(dst_id).copied().unwrap_or(false) {
                        sv &= !sx;
                        sx = 0;
                    }
                    let (dv, dx) = view[dst_id].raw_bits();
                    if (sv != dv || sx != dx) && view[dst_id].set_inline_bits(sv, sx) {
                        dirtied.push(dst_id as u32);
                    }
                }
                true
            }
            CombItem::DirectCopy {
                dst_id,
                src_id,
                width,
            } => {
                // LRM §9.3.1: skip if the destination is forced
                if self.forced_signals.is_empty() || !self.forced_signals.contains_key(dst_id) {
                    let dst_w = self.signal_widths[*dst_id];
                    let mut handled = false;
                    // Inline raw-bit copy only when src and dst widths match;
                    // set_inline_bits does not mask, so narrowing must go through
                    // the resize branch below (pr2224949).
                    if *width <= 64
                        && dst_w == *width
                        && self.signal_widths[*src_id] == *width
                        && !self.signal_two_state.get(*dst_id).copied().unwrap_or(false)
                    {
                        let (sv, sx) = view[*src_id].raw_bits();
                        let (dv, dx) = view[*dst_id].raw_bits();
                        if sv == dv && sx == dx {
                            handled = true;
                        } else if view[*dst_id].set_inline_bits(sv, sx) {
                            dirtied.push(*dst_id as u32);
                            handled = true;
                        }
                    }
                    if !handled {
                        // §10.7: fit the source to the destination NET's type —
                        // drop X/Z for a 2-state net, sign/X-extend a signed
                        // source, real<->int convert, and stamp the net's own
                        // signedness (a bare `resize` kept the source's, so an
                        // unsigned net driven by a signed value read negative).
                        let resized = self.fit_value_to_signal(*dst_id, &view[*src_id]);
                        if view[*dst_id] != resized {
                            view[*dst_id] = resized;
                            dirtied.push(*dst_id as u32);
                        }
                    }
                }
                true
            }
            CombItem::CompiledContAssign { compiled, .. }
            | CombItem::CompiledAlwaysBlock { compiled, .. } => {
                if vm_regs.len() < compiled.num_regs as usize {
                    vm_regs.resize(compiled.num_regs as usize, Value::zero(1));
                }
                let (_nba, unsupported) = Self::exec_comb_block_isolated(
                    &compiled.instructions,
                    view,
                    &self.signal_widths,
                    &self.signal_signed,
                    &self.signal_name_to_id,
                    &self.array_first_id,
                    vm_regs,
                    dirtied,
                    0,
                    compiled.nba_dup_targets,
                );
                // Deferred NBAs from comb blocks (rare; deferred_nba=0 on
                // c910) would be applied at end-of-tick by the coordinator;
                // for the settle fixpoint they don't participate.
                !unsupported
            }
            CombItem::FusedGate { op } => {
                Self::exec_fused_gate_isolated(op, view, dirtied, &self.signal_gate_driven);
                true
            }
            CombItem::FusedBufFanout { src, dsts, invert } => {
                for &dst in dsts.iter() {
                    Self::exec_fused_gate_isolated(
                        &FusedGate::Buf1 {
                            dst,
                            src: *src,
                            invert: *invert,
                        },
                        view,
                        dirtied,
                        &self.signal_gate_driven,
                    );
                }
                true
            }
            CombItem::FusedAndFanout {
                common,
                branches,
                invert,
            } => {
                Self::exec_fused_and_fanout_isolated(*common, branches, *invert, view, dirtied);
                true
            }
            // AST fallback (ContAssign / AlwaysBlock) — needs the full
            // Simulator (eval_expr / exec_statement). ast_ca=6 on c910.
            CombItem::ContAssign { .. } | CombItem::AlwaysBlock { .. } => false,
        }
    }

    /// Fully-static (no `&self`) isolated comb-entry evaluator for the parallel
    /// BSP level dispatch — worker threads call this with shared refs to the
    /// design arrays and a private staged `view`, so it must not borrow the
    /// (non-Sync) Simulator. Only ever called for `comb_par_safe` items, so the
    /// AST / unsupported arms are unreachable (return false defensively).
    /// Pushes changed signal ids into `dirtied`; bookkeeping/propagation is done
    /// serially by the caller after the parallel join.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn eval_comb_item_isolated<V: IsolatedSignalView + ?Sized>(
        item: &CombItem,
        signal_widths: &[u32],
        signal_signed: &[bool],
        signal_two_state: &[bool],
        signal_gate_driven: &[bool],
        signal_name_to_id: &NameMap,
        array_first_id: &HashMap<Arc<str>, (usize, i64, i64)>,
        view: &mut V,
        vm_regs: &mut Vec<Value>,
        dirtied: &mut Vec<u32>,
    ) -> bool {
        match item {
            CombItem::Noop => true,
            // §29 UDPs and timing checks never run on this isolated path
            // (serial forced).
            CombItem::Udp { .. } | CombItem::UdpBatch { .. } | CombItem::TimingCheck { .. } => true,
            CombItem::GateRegion { .. }
            | CombItem::VectorGate { .. }
            | CombItem::ScatterGate { .. }
            | CombItem::CtlGate { .. } => false,
            CombItem::FastDirectCopy { dst_id, src_id } => {
                let (mut sv, mut sx) = view[*src_id].raw_bits();
                // §6.11.1/§10.7: a 2-state destination drops X/Z (X/Z -> 0).
                if sx != 0 && signal_two_state.get(*dst_id).copied().unwrap_or(false) {
                    sv &= !sx;
                    sx = 0;
                }
                let (dv, dx) = view[*dst_id].raw_bits();
                if (sv != dv || sx != dx) && view[*dst_id].set_inline_bits(sv, sx) {
                    dirtied.push(*dst_id as u32);
                }
                true
            }
            CombItem::FastDirectFanout { src_id, dst_ids } => {
                let (src_v, src_x) = view[*src_id].raw_bits();
                for &dst_id in dst_ids.iter() {
                    let (mut sv, mut sx) = (src_v, src_x);
                    if sx != 0 && signal_two_state.get(dst_id).copied().unwrap_or(false) {
                        sv &= !sx;
                        sx = 0;
                    }
                    let (dv, dx) = view[dst_id].raw_bits();
                    if (sv != dv || sx != dx) && view[dst_id].set_inline_bits(sv, sx) {
                        dirtied.push(dst_id as u32);
                    }
                }
                true
            }
            CombItem::DirectCopy {
                dst_id,
                src_id,
                width,
            } => {
                let dst_w = signal_widths[*dst_id];
                let dst_two_state = signal_two_state.get(*dst_id).copied().unwrap_or(false);
                let mut handled = false;
                // Inline raw-bit copy only when src and dst widths match;
                // set_inline_bits does not mask, so narrowing must go through
                // the resize branch below (pr2224949).
                if *width <= 64
                    && dst_w == *width
                    && signal_widths[*src_id] == *width
                    && !dst_two_state
                {
                    let (sv, sx) = view[*src_id].raw_bits();
                    let (dv, dx) = view[*dst_id].raw_bits();
                    if sv == dv && sx == dx {
                        handled = true;
                    } else if view[*dst_id].set_inline_bits(sv, sx) {
                        dirtied.push(*dst_id as u32);
                        handled = true;
                    }
                }
                if !handled {
                    let src_val = view[*src_id].clone();
                    let mut resized = if src_val.width != *width {
                        src_val.resize(*width)
                    } else {
                        src_val
                    };
                    if dst_two_state {
                        resized = resized.to_two_state();
                    }
                    if view[*dst_id] != resized {
                        view[*dst_id] = resized;
                        dirtied.push(*dst_id as u32);
                    }
                }
                true
            }
            CombItem::CompiledContAssign { compiled, .. }
            | CombItem::CompiledAlwaysBlock { compiled, .. } => {
                if vm_regs.len() < compiled.num_regs as usize {
                    vm_regs.resize(compiled.num_regs as usize, Value::zero(1));
                }
                let (nba, unsupported) = Self::exec_comb_block_isolated(
                    &compiled.instructions,
                    view,
                    signal_widths,
                    signal_signed,
                    signal_name_to_id,
                    array_first_id,
                    vm_regs,
                    dirtied,
                    0,
                    compiled.nba_dup_targets,
                );
                !unsupported && nba.is_empty()
            }
            CombItem::FusedGate { op } => {
                Self::exec_fused_gate_isolated(op, view, dirtied, signal_gate_driven);
                true
            }
            CombItem::FusedBufFanout { src, dsts, invert } => {
                for &dst in dsts.iter() {
                    Self::exec_fused_gate_isolated(
                        &FusedGate::Buf1 {
                            dst,
                            src: *src,
                            invert: *invert,
                        },
                        view,
                        dirtied,
                        signal_gate_driven,
                    );
                }
                true
            }
            CombItem::FusedAndFanout {
                common,
                branches,
                invert,
            } => {
                Self::exec_fused_and_fanout_isolated(*common, branches, *invert, view, dirtied);
                true
            }
            CombItem::ContAssign { .. } | CombItem::AlwaysBlock { .. } => false,
        }
    }
}
