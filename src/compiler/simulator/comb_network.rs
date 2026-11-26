//! Combinational network items, gates, comb entries and prepared-comb caches.

use super::*;

/// Lazy two-state lowering state for one comb entry (see P5 lowering in
/// bytecode.rs). `Arc` so the eval site can clone the handle out of `self`
/// before running with `&mut self`.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(super) enum TsSlot {
    Untried,
    No,
    Yes(std::sync::Arc<super::bytecode::TwoStateBlock>),
}

/// §10.3.3 / §28.16: transition bits and rule before driver widening.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub(super) struct DelayTarget {
    pub(super) lo: u32,
    pub(super) width: u32,
    /// §10.3.3 vector assignment, rather than §28.16 gate/scalar rules.
    pub(super) vector: bool,
    /// The fall and turn-off delays of an assignment to a constant bit- or
    /// part-select, whose net other drivers share; the bits come from the
    /// select at each update. None for a whole-net target, whose delays are
    /// in the per-driver maps.
    pub(super) select_delays: Option<(Option<u64>, Option<u64>)>,
}

/// A combinatorial item (continuous assign or always @*/always_comb block)
/// with pre-computed sensitivity set for efficient evaluation.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(super) enum CombItem {
    Noop,
    ContAssign {
        // Interpreted fallbacks are cold. Keep their large ASTs out of the
        // enum so direct-copy/fused/compiled entries stay cache compact.
        lhs: Box<Expression>,
        rhs: Box<Expression>,
        delay: u64,
        /// §10.3.3 / §28.16: the original target before driver widening,
        /// boxed like the ASTs so the entry stays cache compact.
        delay_target: Option<Box<DelayTarget>>,
        // §10.3.3: expressions are evaluated when the right-hand side changes.
        delay_exprs: Option<Box<[Option<Expression>; 3]>>,
    },
    /// Fast path: direct signal-to-signal copy (assign b = a) with pre-resolved IDs.
    DirectCopy {
        dst_id: usize,
        src_id: usize,
        width: u32,
    },
    FastDirectCopy {
        dst_id: usize,
        src_id: usize,
    },
    /// Several exact-width, zero-delay copies with one common source. Port
    /// arrays frequently elaborate into dozens of these (notably replicated
    /// PLL clock inputs); one entry avoids repeated worklist dispatch while
    /// retaining an individual signal-table cell for every destination.
    FastDirectFanout {
        src_id: usize,
        dst_ids: Box<[usize]>,
    },
    /// Batched one-bit buffers sharing the same source bit and inversion.
    FusedBufFanout {
        src: BitRef,
        dsts: Box<[BitRef]>,
        invert: bool,
    },
    /// Batched one-bit AND/NAND gates sharing one input. Clock trees commonly
    /// instantiate many `gclk = parent_clk & enable` cells; one dependency
    /// entry avoids repeated worklist dispatch while every enable remains a
    /// sensitivity and every destination remains independent.
    FusedAndFanout {
        common: BitRef,
        branches: Box<[FusedAndBranch]>,
        invert: bool,
    },
    /// Bytecode-compiled cont_assign: RHS compiled to VM instructions,
    /// result written to pre-resolved dst_id via BlockingAssign insn.
    CompiledContAssign {
        compiled: super::bytecode::CompiledBlock,
    },
    AlwaysBlock {
        stmt: Box<Statement>,
        is_always_comb: bool,
    },
    /// Bytecode-compiled comb always block. Skips the AST interpreter per
    /// settle iteration — dominant hot path in RTL-heavy designs.
    CompiledAlwaysBlock {
        compiled: super::bytecode::CompiledBlock,
        is_always_comb: bool,
    },
    /// Fused 1-bit gate: recognizes yosys-generated patterns
    /// `assign d[i] = a op b`, `assign d[i] = ~a`, `assign d[i] = s ? t : e`.
    /// Skips bytecode VM entirely — reads operand bits from signal_table,
    /// computes 4-state logic, writes single bit back.
    FusedGate {
        op: FusedGate,
    },
    /// Aligned slices of independent one-bit gates represented as one plane
    /// operation and one destination commit.
    VectorGate {
        op: VectorGate,
    },
    /// Adjacent source lanes whose destinations remain separate scalar cells.
    /// The vector operation is shared while each destination keeps its own
    /// change detection and dependency dispatch.
    ScatterGate {
        op: ScatterGate,
    },
    /// A coact-clustered batch of fused gates executed in one worklist
    /// dispatch, in topological member order, through the SAME per-gate
    /// evaluator as `FusedGate` — zero semantic novelty, machinery amortized
    /// over the batch. Formed by the region pass (class 3) from
    /// profile-measured co-activation edges. c906: gates are 49.3% of all
    /// comb evals and were untouched by bytecode fusion.
    GateRegion {
        gates: Box<[FusedGate]>,
    },
    /// IEEE 1800-2017 §29 User-Defined Primitive instance. Evaluated by
    /// `eval_udp(idx)` against `self.udp_runtime[idx]` (truth table + per-
    /// instance sequential state). Always evaluated on the serial settle path.
    Udp {
        idx: usize,
    },
    /// Edge-filtered sequential UDPs sharing one event terminal. Their data
    /// terminals are proven state-holding, so one dependency dispatch can
    /// evaluate the whole clock-domain batch.
    UdpBatch {
        event_ref: BitRef,
        indices: Box<[usize]>,
    },
    /// §31 timing checks watching one signal: `eval_timing_watch(idx)`
    /// against `self.timing_watches[idx]`. Reads that signal and writes
    /// nothing directly (a notifier toggles through the NBA region).
    TimingCheck {
        idx: usize,
    },
    /// A fused AND/OR, inverted gate, or mux whose high-fanout input is ignored
    /// while the other input holds the controlling value: the gate is
    /// `ctl_gates[slot]` (see `CtlGate`). Made from `FusedGate` entries at
    /// the first settle, never before the prepared-comb cache is written.
    CtlGate {
        slot: u32,
    },
}

/// Controlling-value skip for a binary gate or one mux data arm.
/// For muxes, only the inactive arm is masked. Select and active-arm edges stay live.
///
/// `wl = clk & rs` cannot change while `rs` is 0, however `clk` toggles, so
/// the gate's dependency edges from `clk` (the high-fanout input) carry mask
/// 0 while `rs` sits at the controlling value (0 for AND/NAND, 1 for
/// OR/NOR) and the static mask otherwise. Every evaluation of the gate
/// recomputes that mask from the controlling input's value it read, so:
/// - a zero mask always means the controlling input held the controlling
///   value at the gate's last evaluation, and the output already equals the
///   forced result (x AND 0 = 0; x and z never control);
/// - the controlling input's own edge is never masked, so its change always
///   re-evaluates the gate, which restores the mask with the current value
///   of the masked input in hand (same-delta changes of both inputs are
///   read together).
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub(super) struct CtlGate {
    /// The original binary gate or mux.
    pub(super) op: FusedGate,
    /// The input whose controlling value masks the other one.
    pub(super) other: BitRef,
    /// The masked (high-fanout) input's signal.
    pub(super) hot: u32,
    /// `comb_dep_edges[pos..pos + npos]`: the edges from `hot` to the gate.
    pub(super) pos: u32,
    pub(super) npos: u32,
    /// Their static (bit-sensitivity) mask, restored when not controlled.
    pub(super) smask: u64,
    /// Known control code that makes changes on the hot input irrelevant.
    pub(super) ctl: u8,
}

/// Dynamic controlling-value edge mask for specialized two-state arena
/// entries (`TS_AND_EQ`, `TS_AND_BIT`, `TS_AND2`, `TS_OR2`).
/// While the controlling input holds its controlling value (`0` for AND,
/// definite `1` for OR) and no forces are active, the non-controlling data
/// inputs' dependency edges have `mask = 0` so broadcast toggles skip waking
/// the entry completely. The controlling input's own edge is never masked.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub(super) struct TsCtl {
    /// First masked data edge span `(pos0..pos0+npos0)` in `comb_dep_edges`.
    pub(super) pos0: u32,
    pub(super) npos0: u32,
    pub(super) smask0: u64,
    /// Optional second masked data edge span (`npos1 == 0` when unused).
    pub(super) pos1: u32,
    pub(super) npos1: u32,
    pub(super) smask1: u64,
    /// Dynamic mask state of `(pos0, pos1)`:
    /// `0` = neither masked, `1` = `pos0` masked / `pos1` active,
    /// `2` = `pos0` active / `pos1` masked, `3` = both masked.
    pub(super) mask_state: u8,
    /// For `TS_AND2` / `TS_OR2`: true when `sig2` is the controlling input
    /// (and `sig1` is masked); false when `sig1` is controlling.
    pub(super) ctl_is_sig2: bool,
}

/// Dynamic per-arm edge mask for single-writer combinational `always` blocks
/// dispatched by a `case` or `if` selector (`CaseJump`, `CmpBranchConst`
/// chain, or `BranchIfSignalFalse`). For each selector value `v`, bit `i` of
/// `arm_masks[v]` (or `default_mask` when `v >= arm_masks.len()`) is `1` iff
/// edge `i` is NOT read by the arm taken for `v` and can carry `mask = 0`
/// while the selector stays steady.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(super) struct CaseCtl {
    /// Primary selector signal (`sel_lo0..sel_lo0+sel_w0`).
    pub(super) sel_sig0: u32,
    pub(super) sel_lo0: u16,
    pub(super) sel_w0: u16,
    /// Optional second 1-bit selector signal for `{sel_sig0[sel_lo0], sel_sig1[sel_lo1]}`
    /// concats (`u32::MAX` for a single-signal selector).
    pub(super) sel_sig1: u32,
    pub(super) sel_lo1: u16,
    /// Per-selector-value maskable-edge bitset (up to 64 maskable edges).
    pub(super) arm_masks: Box<[u64]>,
    /// Maskable-edge bitset for selector values `>= arm_masks.len()`.
    pub(super) default_mask: u64,
    /// Dependency edge spans `(pos, npos, smask)` in `comb_dep_edges`.
    pub(super) edges: Box<[(u32, u32, u64)]>,
    /// Bit `i` is 1 iff `edges[i]` currently carries `mask = 0`.
    pub(super) cur_masked: u64,
}

/// A continuous mux with equal-width, in-bounds descending ranges. Arms
/// are ordered false/true. The ordinary evaluator still owns every write.
#[derive(serde::Serialize)]
pub(super) struct MuxGuard {
    pub(super) selector: usize,
    pub(super) arms: [(usize, usize); 2],
    pub(super) dst: usize,
    pub(super) dst_lo: usize,
    pub(super) width: usize,
}

/// Per-instance runtime state for a §29 UDP. Terminals are resolved to single
/// bit refs; sequential state and previous-input levels drive edge detection.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct UdpRuntime {
    pub(super) udp_name: String,
    pub(super) inst_path: String,
    pub(super) out_ref: BitRef,
    pub(super) in_refs: Vec<BitRef>,
    pub(super) is_sequential: bool,
    /// Rise delay, and every delay unless `delay_fall` differs (§29.3).
    pub(super) delay: u64,
    /// §28.16: the delay of a change to 0; equal to `delay` for one value.
    pub(super) delay_fall: u64,
    /// Shared by every instance of the same UDP definition. The elaborator
    /// attaches rows to each flattened instance, but retaining those clones
    /// wastes substantial memory on cell fabrics.
    pub(super) rows: Arc<[crate::ast::decl::UdpTableRow]>,
    /// Precomputed `(state, previous inputs, current inputs) -> next state`
    /// table for small UDPs. Values are 0, 1, or 2 (=x).
    pub(super) transition_lut: Option<Arc<[u8]>>,
    /// Number of ternary input vectors (`3 ^ input_count`) in the LUT.
    pub(super) lut_input_states: usize,
    /// Cached ternary encoding of the last input vector. The LUT path can
    /// compare and index this directly instead of rebuilding previous inputs.
    pub(super) input_state_idx: usize,
    /// When only one input can cause a state transition, identify that port
    /// and the previous/current trit pairs worth evaluating. Other pairs can
    /// update the cached trit and return without reading the remaining ports.
    pub(super) event_input: Option<u8>,
    pub(super) event_transition_mask: u16,
    /// Current output/state level: 0,1,2(=x).
    pub(super) state: u8,
    /// Previous input levels (0,1,2); 255 = uninitialised (first eval).
    pub(super) prev_inputs: Vec<u8>,
    pub(super) initialized: bool,
    /// True once we've warned about a no-match with a delayed multibit output.
    pub(super) warned: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct BitRef {
    pub(super) sig_id: u32,
    pub(super) bit: u32,
}

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct FusedAndBranch {
    pub(super) dst: BitRef,
    pub(super) other: BitRef,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub(super) enum GateBin {
    And,
    Or,
    Xor,
}

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub(super) enum FusedGate {
    /// dst = src, or dst = ~src when invert
    Buf1 {
        dst: BitRef,
        src: BitRef,
        invert: bool,
    },
    /// dst = a op b, or dst = ~(a op b) when invert
    Bin2 {
        dst: BitRef,
        a: BitRef,
        b: BitRef,
        op: GateBin,
        invert: bool,
    },
    /// dst = s ? t : e
    Mux2 {
        dst: BitRef,
        s: BitRef,
        t: BitRef,
        e: BitRef,
    },
    /// Zero-delay combinational UDP lowered to a compact current-input LUT.
    /// Three ternary inputs fit in 54 packed bits, and the immutable node can use
    /// the same direct dependency dispatch as built-in fused gates.
    UdpLut3 {
        dst: BitRef,
        inputs: [BitRef; 3],
        input_count: u8,
        table: u64,
    },
}

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub(super) struct VectorRef {
    pub(super) sig_id: u32,
    pub(super) lo: u32,
}

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub(super) enum VectorGate {
    Buf {
        dst: VectorRef,
        src: VectorRef,
        width: u8,
        invert: bool,
    },
    Bin {
        dst: VectorRef,
        a: VectorRef,
        b: VectorRef,
        width: u8,
        op: GateBin,
        invert: bool,
    },
    Mux {
        dst: VectorRef,
        s: VectorRef,
        t: VectorRef,
        e: VectorRef,
        width: u8,
    },
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(super) enum ScatterGate {
    Buf {
        dsts: Box<[u32]>,
        src: VectorRef,
        width: u8,
        invert: bool,
    },
    Bin {
        dsts: Box<[u32]>,
        a: VectorRef,
        b: VectorRef,
        width: u8,
        op: GateBin,
        invert: bool,
    },
}

// See the §9.2.2.2 note in Simulator::collect_expr_reads' Call arm: set
// while collecting reads for a PLAIN `always` (star or explicit list),
// whose sensitivity excludes called-function bodies (only
// always_comb/always_latch descend into them).
/// Continuous-assignment targets indexed by leaf name, for the edge-select
/// alias sharing in `rewrite_edge_select_sensitivities` (built once, on the
/// first edge term that selects a bit of a vector).
pub(super) mod edgesel_writers {
    use super::*;

    pub(in crate::compiler::simulator) fn base_ident(e: &Expression) -> Option<String> {
        match &e.kind {
            ExprKind::Paren(inner) => base_ident(inner),
            ExprKind::Index { expr, .. } | ExprKind::RangeSelect { expr, .. } => base_ident(expr),
            ExprKind::Ident(h) => Some(
                h.path
                    .iter()
                    .map(|s| s.name.name.as_str())
                    .collect::<Vec<_>>()
                    .join("."),
            ),
            _ => None,
        }
    }
    pub(in crate::compiler::simulator) fn leaf(n: &str) -> &str {
        n.rsplit('.').next().unwrap_or(n)
    }

    #[derive(Default)]
    pub(in crate::compiler::simulator) struct WriterIndex {
        /// Leaf name -> indices into `continuous_assigns`.
        pub cont: HashMap<String, Vec<usize>>,
        /// Leaf name (of the target, or of the parent net a port target
        /// connects to) -> indices into `pending_cont_assign`.
        pub pend: HashMap<String, Vec<usize>>,
        /// Leaf names some procedural `force` or `assign` (§10.6) targets:
        /// its bits need not follow their continuous drivers.
        pub forced: HashSet<String>,
    }

    /// The targets of `force` / procedural `assign` statements in `s`.
    fn proc_cont_targets<'a>(s: &'a Statement, out: &mut Vec<&'a Expression>) {
        use crate::ast::stmt::ProceduralContinuous as PC;
        match &s.kind {
            StatementKind::ProceduralContinuous(PC::Force { lvalue, .. })
            | StatementKind::ProceduralContinuous(PC::Assign { lvalue, .. }) => {
                match &lvalue.kind {
                    ExprKind::Concatenation(parts) => out.extend(parts.iter()),
                    _ => out.push(lvalue),
                }
            }
            StatementKind::If {
                then_stmt,
                else_stmt,
                ..
            } => {
                proc_cont_targets(then_stmt, out);
                if let Some(e) = else_stmt {
                    proc_cont_targets(e, out);
                }
            }
            StatementKind::Case { items, .. } => {
                for it in items {
                    proc_cont_targets(&it.stmt, out);
                }
            }
            StatementKind::ForeachTail(ft) => proc_cont_targets(&ft.body, out),
            StatementKind::For { body, .. }
            | StatementKind::Foreach { body, .. }
            | StatementKind::While { body, .. }
            | StatementKind::DoWhile { body, .. }
            | StatementKind::Repeat { body, .. }
            | StatementKind::Forever { body }
            | StatementKind::ForeverTail { body }
            | StatementKind::RsAction { body } => proc_cont_targets(body, out),
            StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
                for x in stmts {
                    proc_cont_targets(x, out);
                }
            }
            StatementKind::TimingControl { stmt, .. } | StatementKind::Wait { stmt, .. } => {
                proc_cont_targets(stmt, out)
            }
            StatementKind::RandCase { items } => {
                for (_, x) in items {
                    proc_cont_targets(x, out);
                }
            }
            StatementKind::WaitOrder { pass, fail, .. } => {
                if let Some(p) = pass {
                    proc_cont_targets(p, out);
                }
                if let Some(f) = fail {
                    proc_cont_targets(f, out);
                }
            }
            _ => {}
        }
    }

    impl WriterIndex {
        pub(in crate::compiler::simulator) fn build(module: &ElaboratedModule) -> Self {
            let mut ix = WriterIndex::default();
            fn targets(e: &Expression) -> Vec<&Expression> {
                match &e.kind {
                    ExprKind::Concatenation(parts) => parts.iter().collect(),
                    _ => vec![e],
                }
            }
            for (i, ca) in module.continuous_assigns.iter().enumerate() {
                match ca {
                    crate::compiler::elaborate::ContinuousAssignment::PlainLink { lhs, .. } => {
                        ix.cont.entry(leaf(lhs.as_str()).to_string()).or_default().push(i);
                    }
                    crate::compiler::elaborate::ContinuousAssignment::Full(f) => {
                        for t in targets(&f.lhs) {
                            if let Some(b) = base_ident(t) {
                                ix.cont.entry(leaf(&b).to_string()).or_default().push(i);
                            }
                        }
                    }
                }
            }
            let mut forced: Vec<&Expression> = Vec::new();
            for b in &module.always_blocks {
                proc_cont_targets(&b.stmt, &mut forced);
            }
            for b in module
                .initial_blocks
                .iter()
                .chain(module.final_blocks.iter())
            {
                proc_cont_targets(&b.stmt, &mut forced);
            }
            let mut routine_forced: HashMap<usize, Vec<String>> = HashMap::default();
            for (k, t) in &module.tasks {
                let ptr = std::sync::Arc::as_ptr(t.items.as_arc()) as usize;
                let bases = routine_forced.entry(ptr).or_insert_with(|| {
                    let mut f: Vec<&Expression> = Vec::new();
                    for x in &t.items {
                        proc_cont_targets(x, &mut f);
                    }
                    f.into_iter().filter_map(base_ident).collect()
                });
                let dctx = module.deferred_task_bodies.get(k);
                for b in bases.iter() {
                    ix.forced.insert(leaf(b).to_string());
                    if let Some(dctx) = dctx
                        && let Some(mb) = dctx.ctx.port_map.get(b).and_then(base_ident)
                    {
                        ix.forced.insert(leaf(&mb).to_string());
                    }
                }
            }
            for (k, f) in module.functions.iter() {
                let ptr = std::sync::Arc::as_ptr(f.items.as_arc()) as usize;
                let bases = routine_forced.entry(ptr).or_insert_with(|| {
                    let mut fv: Vec<&Expression> = Vec::new();
                    for x in &f.items {
                        proc_cont_targets(x, &mut fv);
                    }
                    fv.into_iter().filter_map(base_ident).collect()
                });
                let dctx = module.deferred_function_bodies.get(k);
                for b in bases.iter() {
                    ix.forced.insert(leaf(b).to_string());
                    if let Some(dctx) = dctx
                        && let Some(mb) = dctx.ctx.port_map.get(b).and_then(base_ident)
                    {
                        ix.forced.insert(leaf(&mb).to_string());
                    }
                }
            }
            for e in forced {
                if let Some(b) = base_ident(e) {
                    ix.forced.insert(leaf(&b).to_string());
                }
            }
            let pend_sources = module
                .pending_always
                .iter()
                .map(|p| (&*p.source, &p.ctx))
                .chain(module.pending_initial.iter().map(|p| (&*p.source, &p.ctx)));
            for (src, ctx) in pend_sources {
                let mut f: Vec<&Expression> = Vec::new();
                proc_cont_targets(src, &mut f);
                for e in f {
                    if let Some(b) = base_ident(e) {
                        ix.forced.insert(leaf(&b).to_string());
                        if let Some(mb) = ctx.port_map.get(&b).and_then(base_ident) {
                            ix.forced.insert(leaf(&mb).to_string());
                        }
                    }
                }
            }
            for (i, p) in module.pending_cont_assign.iter().enumerate() {
                for t in targets(&p.source_lhs()) {
                    let Some(b) = base_ident(t) else {
                        continue;
                    };
                    let mut leaves = vec![leaf(&b).to_string()];
                    if let Some(mb) = p.ctx.port_map.get(&b).and_then(base_ident) {
                        leaves.push(leaf(&mb).to_string());
                    }
                    leaves.dedup();
                    for l in leaves {
                        let v = ix.pend.entry(l).or_default();
                        if v.last() != Some(&i) {
                            v.push(i);
                        }
                    }
                }
            }
            ix
        }
    }
}

thread_local! {
    pub(super) static SKIP_FN_BODY_READS: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct CombEntry {
    pub(super) item: CombItem,
    /// Everything the settle HOT LOOP does not read, behind one pointer.
    ///
    /// The eval loop's `entries[eidx]` is an indirect access; at 144 bytes an
    /// entry straddled 2–3 cache lines and every evaluation paid the extra
    /// misses (4.5k evals per simulated cycle on the C906 SoC). With the
    /// three cold containers boxed the entry is 64 bytes — the match on
    /// `item` plus the two flags touch one line. `cold` is dereferenced only
    /// during entry construction, diagnostics, and rebuild passes.
    pub(super) cold: Box<CombEntryCold>,
    /// True when dependency extraction could not resolve all read signals.
    /// Such entries are conservatively re-evaluated each settle pass.
    pub(super) has_unresolved_reads: bool,
    /// True for a plain `always @(sensitivity_list)` block whose body can
    /// reach `$finish` / `$stop`. xezim routes level-only `always @(...)`
    /// blocks through the comb-settle path, which fires them at time 0
    /// with their (typically X) initial values — but per IEEE 1800
    /// §9.2.2.1 such a block actually suspends at its event control and
    /// only runs when a sensitivity input changes. Firing it at time 0
    /// is usually harmless (the block computes garbage that gets
    /// recomputed later), but if the body hits `$finish` the whole
    /// simulation aborts before the testbench has initialised its
    /// stimulus. So: don't seed these from the time-0 dirty set; let the
    /// first real sensitivity transition trigger them.
    pub(super) defer_at_time0: bool,
}

impl CombEntry {
    /// A procedural block that reads a signal it also writes, whose own writes
    /// must not re-queue it. A continuous assignment re-runs until it settles.
    pub(super) fn reads_own_writes(&self) -> bool {
        matches!(
            self.item,
            CombItem::AlwaysBlock { .. } | CombItem::CompiledAlwaysBlock { .. }
        ) && (self.cold.read_signal_ids.iter()).any(|r| self.cold.write_signal_ids.contains(r))
    }
}

/// Cold half of a `CombEntry` — see `CombEntry::cold`.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct CombEntryCold {
    /// Lexical instance of a continuous RHS that may enter the interpreter.
    /// An empty scope selects the top module independently of the waking process.
    pub(super) rhs_time_scope: Option<Arc<str>>,
    /// Preferred hierarchical scope for resolving unqualified identifiers.
    pub(super) scope_hint: Option<String>,
    /// Pre-resolved signal IDs for reads (for fast dependency lookup).
    pub(super) read_signal_ids: Vec<usize>,
    /// Pre-resolved signal IDs for writes. The original layout
    /// stored a `(usize, String)` tuple but every read site
    /// destructures the String with `_` — name was dead weight.
    /// At c910 scale (~467K entries × ~5 writes each) the dropped
    /// String per write reclaims tens of MB.
    pub(super) write_signal_ids: Vec<usize>,
    /// Source span of the originating construct (cont-assign LHS or always
    /// block statement) so diagnostics — notably the settle-limit warning —
    /// can name file:line for a non-converging driver. `Span::dummy()` when
    /// the origin has no source (synthesized entries).
    pub(super) span: crate::ast::Span,
    /// Code coverage counter of an interpreted continuous assignment (the
    /// compiled form counts with an `Insn::CovHit`). Never cached.
    #[serde(skip)]
    pub(super) cov: Option<u32>,
    /// §9.2.2.2: evaluating the entry is observable beyond its outputs
    /// (see `time0_effects`); it waits for the time-0 start batch.
    pub(super) time0_effects: bool,
    /// Nonzero `alias_t0_kind` tags (`1` = 4-state `X`, `2` = 2-state `0`) for
    /// read signals accessed through a collapsed identity input port formal,
    /// for top-scope non-`always_comb` entries checked by `fire_time0_input_port_blocks`.
    pub(super) alias_t0_reads: Option<Box<Vec<(usize, u8)>>>,
}

/// One `comb_dep_edges` element: a dependent entry and the bits of the
/// signal it reads (`bytecode::sig_span_mask` chunks; all ones = any bit).
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub(super) struct DepEdge {
    pub(super) mask: u64,
    pub(super) entry: u32,
    /// Zero for ordinary work, otherwise the terminal gate's lane index plus
    /// one. Derived from the lane map and stored next to the sensitivity mask.
    #[serde(skip)]
    pub(super) gate_slot: u32,
}

#[derive(serde::Serialize)]
pub(super) struct PreparedCombCacheRef<'a> {
    pub(super) signal_count: usize,
    pub(super) entries: &'a [CombEntry],
    pub(super) dep_offsets: &'a [u32],
    pub(super) dep_entries: &'a [u32],
    pub(super) unresolved_idx: &'a [usize],
    pub(super) time0_idx: &'a [usize],
    pub(super) time0_deferred: &'a [CombEntry],
    pub(super) cont_driven: &'a HashSet<usize>,
    pub(super) gate_fall_delay_by_id: &'a HashMap<usize, u64>,
    pub(super) gate_off_delay_by_id: &'a HashMap<usize, u64>,
}

#[derive(serde::Deserialize)]
pub(super) struct PreparedCombCache {
    pub(super) signal_count: usize,
    pub(super) entries: Vec<CombEntry>,
    pub(super) dep_offsets: Vec<u32>,
    pub(super) dep_entries: Vec<u32>,
    pub(super) unresolved_idx: Vec<usize>,
    pub(super) time0_idx: Vec<usize>,
    pub(super) time0_deferred: Vec<CombEntry>,
    pub(super) cont_driven: HashSet<usize>,
    pub(super) gate_fall_delay_by_id: HashMap<usize, u64>,
    pub(super) gate_off_delay_by_id: HashMap<usize, u64>,
}

/// Mid-exec ts aborts tolerated per block before demotion.
pub(super) const TS_ABORT_DEMOTE: u16 = 8;

/// Flag bit in a `ts_save_list` id marking a wide save entry (signal ids
/// stay below it).
pub(super) const TS_SAVE_WIDE: u32 = 1 << 31;

/// Flag bit in a `ts_save_list` id marking a code coverage count to take back
/// when the block bails (the four-state re-run counts it again).
pub(super) const TS_SAVE_COV: u32 = 1 << 30;

/// Consecutive x-read bails before a block is put to sleep.
pub(super) const TS_XBAIL_STREAK: u8 = 8;

/// First sleep, in successful two-state evaluations design-wide (about half
/// a clock tick on a CPU SoC); doubles per further streak, up to 2^12 units.
pub(super) const TS_XBAIL_SLEEP_UNIT: u64 = 256;

/// A gap this long between two bails of one block counts as a clean stretch
/// and restarts its streak.
pub(super) const TS_XBAIL_FORGET: u64 = 1 << 16;

/// A sleeping two-state block: what was swapped out, and where.
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct TsSleepRec {
    pub(super) kind: u8,
    pub(super) idx: u32,
    pub(super) len_kind: u16,
    pub(super) slept_at: u64,
    pub(super) plan: Option<CombPlan>,
    pub(super) slot: Option<TsSlot>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(super) enum CombPlan {
    Unresolved,
    /// Two-state island. The per-eval guard can still bail dynamically
    /// (forces, X inputs); the plan then falls through JIT-then-interp,
    /// exactly like the unplanned dispatch.
    Ts(std::sync::Arc<super::bytecode::TwoStateBlock>),
    /// Native slot; its function is stored in `comb_jit_fns`. Keep the same
    /// checkpoint discriminants in builds with and without the JIT feature.
    Jit,
    /// Template slot; its function and mapping window are in `comb_tpl_fns`.
    Tpl,
    Interp,
}
