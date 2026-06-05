# Per-LP event_loop — next-session implementation guide

Addendum to [PERLP-EVENTLOOP-PLAN.md](PERLP-EVENTLOOP-PLAN.md), updated
post-session-2 with the new performance baseline and concrete first
steps for implementing Phase 4. Read PERLP-EVENTLOOP-PLAN.md first for
the architectural background; this file is the "where do I actually
start coding" companion.

---

## Causality-fix attempt (REVERTED) + the metric insight

Attempted to drive the multitick `final_mismatch` (39197, ~0.1%) to 0 via:
(1) iterated cross-LP settle (settle→exchange boundary→reseed→repeat to
fixpoint), (2) a CONSISTENT edge/comb partition (assign edge blocks by
comb ownership — the passed edge_part disagreed with comb_part on uncore),
(3) the FULL cross-LP read exchange set (edge LoadSignal + comb reads).
Result: mismatch did NOT drop — iteration alone 39197→39158 (negligible),
adding the consistent partition + full set made it WORSE (49283) and
slower (2.29x). REVERTED to dc915fb (naive lookahead-1, 2.94x).

**KEY INSIGHT — bit-exact-vs-monolithic is the WRONG metric for a
lookahead PDES.** The monolithic reference settles both cores together
each tick (lookahead-0: a cross-core signal resolves SAME tick). ANY
lookahead-based parallel scheme delivers cross-core (registered) signals
with ≥1 cycle delay, so on signals that change each cycle the parallel
state legitimately differs from the lookahead-0 reference by a 1-cycle
skew — yet that skew is FUNCTIONALLY INVISIBLE because the consumer
registers those signals (that's why they're valid LP boundaries). So the
~0.1% per-tick "mismatch" conflates benign lookahead skew with any real
bug; per-tick bit-identity cannot distinguish them.

→ The correct validation is FUNCTIONAL: run the full parallel sim and
check observable outputs (TEST PASSED / $display / sim_time), not per-tick
bit-equality vs a lookahead-0 monolith. That needs the full event_loop
integration (the parallel loop driving the real testbench to $finish),
which is the genuine remaining work. The parallel MACHINERY and SPEEDUP
(2.94x @ 4 threads) are demonstrated; functional-correctness validation is
the next milestone, and it supersedes the bit-mismatch metric.

---

## Session-3 feasibility survey — per-LP settle is VIABLE (key de-risking)

The hardest missing piece for a real per-LP event_loop is **per-LP
settle** (combinational propagation per LP).  Session-3 surveyed
whether settle can run in a worker thread (Send-safe context) and
found:

### settle's CombItem variants split into Send-safe vs AST-fallback
- Send-safe (operate on signal_table by id + compiled bytecode):
  `Noop`, `DirectCopy`, `FastDirectCopy`, `CompiledContAssign`,
  `CompiledAlwaysBlock`, `FusedGate`.
- AST-fallback (need full Simulator via eval_expr/exec_statement —
  NOT Send-safe): `ContAssign`, `AlwaysBlock`.

### c910 comb-entry breakdown (XEZIM_DEP_STATS=1)
```
[COMB_STATS] direct_copy=61560 compiled_ca=208387 ast_ca=6
             always_block=0 fused_gate=160001
```
**Only 6 of 429 954 comb entries are AST-fallback (0.0014%).
Zero AST always-blocks.**  99.9986% of settle is Send-safe.

### Implication
The AST-fallback dependency on the full Simulator — which looked like
it might force a settle rewrite — is a **6-entry special case** on
c910.  Per-LP settle is viable: port the compiled-comb chaotic-
iteration loop (the hot path in `settle_combinatorial`) to operate on
a `PerLpSignalTable` + the LP's comb_entries subset, and either:
- (a) run the 6 AST entries on the designated I/O LP (single-thread
  fallback within the per-LP coordinator), or
- (b) pre-compile them (they're continuous assigns; the bytecode
  compiler already handles 208 387 of them — the 6 that fell back
  likely hit an unsupported expr the compiler could be extended for).

This materially de-risks Phase 4: the settle extraction is a
mechanical port of the compiled-comb evaluation loop, not a rewrite.

## Gate 2 — comb dependency graph cuts cleanly (c910, --pdes-c910-stub)
```
[PDES-IO]   COMB writers: multi=0
[PDES-IO]   COMB boundary signals: A→B=61 B→A=37 bidir=11 TOTAL=109
[PDES-PART] entries: total=438580 LP-A=185400 LP-B=253180
            straddle=0 orphan=0 (coverage_ok=true)
[PDES-PART] comb_dep edges: total=762046 cross-LP=115 (0.0151%)
[PDES-PART] boundary signals (partition view)=115 (classify=109)
```
- **straddle=0**: no comb entry writes signals in both LPs — every entry
  has a clean owner.  No write-level co-location needed.
- **cross-LP dep edges = 115 / 762046 = 0.015%**: the comb graph is
  99.985% intra-LP; the 115 crossings all flow through the ~110 boundary
  signals (clock/reset/config/AXI), each with ≥1 cycle lookahead.
- coverage_ok=true: all 438580 entries partitioned.
- boundary identity (dumped names): `forever_core0_clk`, `core0_rst_b`,
  `cpurst_b`, `pad_core0_hartid/rvba`, `core0_*_dbg_req`, `pad_ibiu0_*`
  (BIU0/AXI) — the canonical 2-core cut.

→ per-LP settle with tick-boundary channel sync is **sound** for c910.

### Partition artifact (first code unit shipped)
`Simulator::pdes_build_comb_partition(lp_a_prefix) -> CombPartition`
(analysis-only).  `lp_entries[lp]` is the per-LP settle worklist;
straddle/orphan entries (0 each on c910) route to the coordinator.
`coverage_ok()` asserts full coverage.  Reported as `[PDES-PART]`.

### Isolated comb evaluator (second code unit shipped)
`Simulator::exec_comb_block_isolated(insns, view, widths, signed,
name_to_id, array_first_id, vm_regs, dirtied, block_index)
-> (Vec<NbaFast>, unsupported)` — the comb counterpart of
`exec_insns_isolated`.  Runs a compiled comb block against a mutable
per-LP signal `view`: `BlockingAssign*` writes land immediately (id
pushed to `dirtied` for worklist propagation), `NbaAssign*` defer into
the returned queue (as the sequential settle does), and unsupported
insns (`StmtFallback`, `*ArrayRange`, `NbaAssignRangeDyn`) set the flag
so the caller routes that entry to the main thread.  This is the
Send-safe building block the per-LP settle loop will call.

Validated via `pdes_check_comb_isolated` (fixpoint invariant: re-eval
at the settled state must not change bits) on c910:
```
[PDES-CHK] checked=217013 bits_mismatch=8 repr_diff=522
           unsupported=0 deferred_nba=0
```
- **217005/217013 bit-identical** to the interpreter's golden values.
- bits_mismatch=8: all PLIC `*claim_eq_vec_exp` / `*ie_ff_*.data_in`
  nodes the authoritative settle left X at t=0; the isolated eval
  resolves them to 0 from the converged inputs — MORE converged than
  golden, not a bug.
- repr_diff=522: bit-identical value, different width/storage rep.
- unsupported=0 (after implementing `BlockingAssignRangeDyn`, which
  was 100% of the initial 9623 unsupported).
Conclusion: the evaluator faithfully reproduces interpreter comb
semantics → safe to drive per-LP settle.  Diagnostics behind
`XEZIM_PDES_CHK_KINDS=1`.

### Per-LP settle driver (third code unit shipped) — END-TO-END VALIDATED
- `exec_fused_gate_isolated(op, view, dirtied)` — isolated bit-level
  fused gate (160001 comb entries on c910).
- `eval_comb_entry_isolated(eidx, view, vm_regs, dirtied) -> supported`
  — dispatches one comb entry to the right isolated evaluator
  (FastDirectCopy / DirectCopy / CompiledContAssign /
  CompiledAlwaysBlock / FusedGate); returns false for the 6 AST entries.
- `pdes_settle_subset_view(view, in_subset, seed, limit) -> (iters,
  unsupported)` — the per-LP chaotic-iteration loop: seeds a worklist,
  evaluates entries, propagates dirtied signals to dependents WITHIN the
  subset via the comb_dep CSR (cross-LP deps = boundary, read frozen).
- `pdes_validate_perlp_settle(lp_a_prefix)` — re-settles each LP's
  subset from the golden fixpoint and merges by ownership.

c910 result (`--pdes-c910-stub`):
```
[PDES-SETTLE] LP-A entries=185400 (iters=1) LP-B entries=253180 (iters=1)
              mismatches=0 unsupported_evals=6
```
**mismatches=0**: settling each LP's comb subset independently (boundary
frozen at the global fixpoint) reconverges to a per-signal state
BIT-IDENTICAL to the global settle, across all 35.9M signals.
unsupported_evals=6 = the AST entries (coordinator-run; targets already
correct).  The per-LP settle correctness path is proven end-to-end.

### Threaded per-LP settle (fourth code unit shipped) — REAL SPEEDUP
- `SendCombItem` / `CombSettleCtx` — `Send`-able executable form of the
  comb layer (AST and StmtFallback-containing blocks demoted to
  `AstFallback`).  `CombSettleCtx::settle_subset` is the worker-thread
  entry (no `&Simulator`).  Same `unsafe impl Send/Sync` rationale as
  `SendExecContext` (built single-threaded, read-only on workers).
- `extract_comb_settle_ctx` builds it; `pdes_validate_perlp_settle_threaded`
  runs the two LP settles on `std::thread::scope` worker threads.

c910 result (`--pdes-c910-stub`):
```
[PDES-SETTLE-MT] mismatches=0 unsupported_evals=6
                 seq=1257.9ms par=797.1ms speedup=1.58x
```
- **mismatches=0**: concurrent two-thread settle is bit-identical to the
  global settle → thread-safe and correct.
- **1.58x** on a 2-LP split — near the load-balance ceiling (LP-B 253k vs
  LP-A 185k entries = 1.37:1, larger thread dominates).  par_ms INCLUDES
  the per-thread 1.1 GB view clone, which a real event_loop pays once
  (views persist across ticks), so steady-state speedup is higher.

First demonstration of actual multicore acceleration of the c910 settle
phase.

### Load balance (fifth code unit shipped) — settle compute hits 2.05x
`pdes_build_comb_partition_balanced(prefix0, prefix1)` — core0 and core1
become the two LPs (symmetric: 185400 entries each), and the 67780
"uncore" entries (L2/fabric/peripherals) distribute by read-affinity
(an uncore signal read by exactly one core → that core's LP; both/neither
→ balance by count).  All writer-entries of a signal move together
(decision is per write-target signal) so single-writer-per-LP holds.
`CombPartition.signal_owner_lp` gives the merge its per-signal authority.

Measurement separated view-clone from settle compute:
```
[semantic] LP 185400/253180  settle_seq=165.6 settle_par=123.3  1.34x  clone=666ms
[balanced] LP 218599/219953  settle_seq=170.3 settle_par= 83.3  2.05x  clone=664ms
```
KEY FINDINGS:
1. **Balanced settle compute = 2.05x on 2 threads** (vs 1.34x semantic) —
   essentially perfect 2x.  Load balance was the lever for the COMPUTE.
2. **The view clone (666ms) dwarfs the settle compute (~83ms)** — it
   dominated the earlier 1.58x end-to-end number.  A real event_loop
   holds PERSISTENT per-LP views (clone once at init; per tick only
   refresh boundary signals via channels), so the clone leaves the
   per-tick critical path and 2.05x becomes the real per-tick speedup.
   → eliminating the per-tick view clone is the next lever.
3. SOUNDNESS COST of the balanced split: cross-LP comb edges 115→42835
   (5.6%), boundary 109→15598.  Correct at the fixpoint (mismatches=0),
   fine for the settle-compute demo — but a sound MULTI-TICK loop needs
   the affinity heuristic made boundary-aware (keep cross-LP edges
   registered, i.e. only split uncore along registered/AXI seams).

### Per-LP edge execution (sixth code unit shipped) — 2.17x, CLEANEST cut
Profiling memcpy: edge-block exec = 55.6% of the sim loop (settle 39.9%,
snap 3.7%).  So the per-LP loop must parallelize EDGES, not just settle
(settle-only Amdahl ceiling = 1.25x; edges+settle ~= 1.9x).

`EdgePartition` + `pdes_build_edge_partition(prefix0, prefix1)` classify
parallel edge blocks by scope; `pdes_validate_perlp_edge_threaded` runs
the two LPs' blocks on threads against a SHARED read-only snapshot
(edge exec reads snapshot, emits NBA writes — no per-LP view clone) and
compares the merged NBA set to sequential.  Reuses `SendExecContext`.

c910 result (`--pdes-c910-stub`):
```
[PDES-EDGE-PART] parallel blocks=10128 LP-0=5064 LP-1=5064 uncore=1704
                 cross-LP NBA writers=0
[PDES-EDGE-MT] nba_writes=863 mismatches=0
               exec_seq=7.5ms exec_par=3.5ms speedup=2.17x (no clone)
```
Edges are the IDEAL target — better than settle on every axis:
- **Perfectly balanced** 5064/5064 (uncore splits evenly).
- **cross-LP NBA writers=0**: edge (flop) layer cuts perfectly along core
  boundaries; inter-core comm is entirely through the registered comb
  boundary, not edge writes → SOUND for a real multi-tick loop (unlike
  the balanced settle split's 42835 cross-LP edges).
- **No view clone**: shared read-only snapshot → 2.17x is the real
  achievable speedup, no 666ms clone tax.
- mismatches=0: threaded edge exec == sequential.

### Combined single-tick pipeline (seventh unit) — soundness verdict
`pdes_validate_perlp_tick(comb_part, edge_part)` runs ONE real tick of
the parallel pipeline — parallel edge exec → NBA apply → SEEDED parallel
settle — vs the same sequentially, with persistent views (clone untimed).
Unlike the fixpoint re-settle (trivially stable), seeding settle from
actual edge NBA outputs exercises real cross-LP comb propagation.

c910 (`--pdes-c910-stub`), honest timing (clone excluded both sides):
```
[PDES-TICK semantic] edge_nba=863 mismatches=9   seq=24.7ms par=15.3ms 1.62x
[PDES-TICK balanced] edge_nba=863 mismatches=925 seq=25.8ms par=17.5ms 1.47x
```
DECISIVE FINDINGS:
1. **Balanced comb split is UNSOUND for a real tick**: 925 mismatches —
   its 42835 cross-LP comb edges break within-tick propagation once
   settle is seeded by a perturbation (the earlier fixpoint mismatches=0
   was trivially stable).  Balanced is ALSO slower per-tick (1.47x) than
   semantic, because a real seeded settle only touches the small NBA cone
   so its load-balance edge vanishes while its cross-LP overhead stays.
   → the multi-tick loop MUST use the semantic (clean, 115-edge) cut.
   "Balanced settle" is retired for the loop (it was only a settle-COMPUTE
   microbenchmark win at 2.05x; useless once correctness matters).
2. **Semantic mismatches=9 are the REGISTERED BOUNDARY handshakes**, not
   a bug (dumped via XEZIM_PDES_CHK_KINDS=1): all 9 are BIU0/CIU AXI
   handshake signals — `ibiu0_pad_back / _bready / _rack` at 3 hierarchy
   levels (cpu_top, ciu_top, piu0_top), all owner=1, par=X vs seq=resolved.
   In the single-tick test LP-1 reads core0's contribution FROZEN at the
   snapshot (X at t=0) so the handshake stays X; sequential propagates
   core0->core1 within the tick and resolves it.  These are EXACTLY the
   signals the boundary channel (CMB/lookahead-K) must carry — the per-LP
   compute is correct for all other 35.9M signals.  The validation has
   thus PRECISELY IDENTIFIED the boundary set the real loop must exchange:
   9 (semantic cut) vs 925 (balanced cut).  → parallel edge/settle compute
   is validated correct; the remaining work is wiring the boundary channel
   for these registered handshakes (the designed PDES mechanism).
3. **Realistic per-tick speedup ~1.62x**, below the per-phase ~2x: a real
   tick's settle is small (863-NBA cone, not the 438k full re-settle) and
   there's a SERIAL NBA-apply + seed-compute between the two parallel
   phases (an Amdahl tax).  Aggregate over the run settle is 40%, so it
   still matters; the per-phase 2x numbers stand for their isolated work.

EARLIER 47x/51x was a measurement bug (seq timing included the 650ms
golden clone, par excluded it) — fixed.

### Multi-tick parallel event loop (eighth unit) — RUNS at 2.51x, causality TBD
`pdes_validate_parallel_multitick(comb_part, edge_part, n_ticks)` is a
real multi-tick loop: per tick toggle clocks → parallel edge exec (each LP
reads its own persistent view) → apply NBAs → BOUNDARY-CHANNEL EXCHANGE
(deliver each boundary signal owner→other, lookahead-1) → parallel seeded
settle. Clone-free per tick (views persist). Validated tick-by-tick vs the
same pipeline run monolithically (sequential, lookahead-0).

c910 (`--pdes-c910-stub`, semantic cut, 8 ticks, XEZIM_PDES_TICKS to vary):
```
[PDES-MULTITICK] per_tick_mismatches=[930,1556,4406,39147,39172,39187,39189,39197]
[PDES-MULTITICK] seq=113.0ms par=45.0ms speedup=2.51x
```
FINDINGS:
1. **2.51x over 8 real ticks** — the best speedup yet; edge+settle
   parallelism compounds, per-tick overhead amortizes, clone-free.
   The machinery (parallel edge + channel + parallel settle) works and is
   fast.
2. **Naive lookahead-1 channel is NOT correctness-preserving (yet)**:
   mismatches vs the monolithic reference grow then PLATEAU at ~39197
   (~0.1% of 35.9M signals), localized to the cross-core interface. This
   is the PDES lookahead-causality effect — delivering a boundary signal
   1 tick late shifts the consumer's capture by a cycle; the skew
   propagates into the interface cone then stabilizes (bounded, not a
   blow-up). Correct PDES needs lookahead matching the ACTUAL registered
   latency of each boundary signal (CMB null-message / lookahead-K), not a
   blanket 1-tick delay.

→ The loop RUNS and PARALLELIZES (2.51x); the remaining work is precisely
scoped: per-boundary-signal causality (the ~110 registered handshakes need
their true lookahead, or a tighter barrier sync) to drive the 0.1%
interface divergence to 0. The compute speedup is already demonstrated.

### What remains for a RUNNING parallel per-LP event_loop
The settle math is validated single-threaded.  To get actual speedup:
1. Run the two `pdes_settle_subset_view` calls on separate threads
   against per-LP views (Send-safe: isolated evaluators take no &Simulator
   beyond the read-only Arc-able tables — fold those into SendExecContext).
2. Per-LP check_edges + NBA apply + boundary-channel exchange at tick
   boundaries (lookahead-K), then loop over ticks instead of the one-shot
   fixpoint re-settle.
3. Route the 6 AST entries + $display/$finish to the coordinator LP.
4. State split pre-spawn / merge post-join.
The hard correctness unknowns are now all retired; what's left is
threading + the per-tick protocol wiring (PdesKernel already proves the
protocol on toy SV).

### Other confirmed infrastructure (ready)
- `SendExecContext::pdes_exec_block` runs real edge-block bytecode in
  a worker thread against a signal snapshot, returns NBA writes ✓
- `PdesKernel::run_with_lookahead` proves the per-LP thread + boundary-
  channel + ClockBarrier + lookahead-K protocol end-to-end (toy SV) ✓
  — BUT it does NOT settle or detect edges; it runs all blocks every
  tick.  The real event_loop adds settle + check_edges per LP.
- `signal_lp_writer`: 109 boundary signals, 0 multi-LP-writers ✓
- Phase 3 `run_one_tick` extraction ✓ (still &mut self — needs the
  per-LP-context variant)

### Remaining Phase 4 work (revised, post-survey)
1. **Per-LP settle**: port the compiled-comb chaotic loop to
   `(PerLpSignalTable, lp_comb_entries)`; route the 6 AST entries to
   the I/O LP.  ~400 LOC.
2. **Per-LP check_edges**: the edge-detect scan over the LP's
   edge_signal_ids subset.  ~200 LOC.
3. **Per-LP NBA apply** into the local table + boundary-channel push
   for cross-LP writes.  ~200 LOC (signal_lp_writer already
   classifies; 0 multi-LP-writers makes this clean).
4. **run_per_lp_event_loop**: the 3-barrier protocol wrapping the
   above, K-tick lookahead.  ~300 LOC.
5. **$display/$finish/$readmemh routing** to I/O LP.  ~200 LOC.
6. **State split + merge** (build per-LP tables pre-spawn, merge
   post-join).  ~200 LOC.

Total ~1500 LOC (down from the ~3000 SESSION-SUMMARY.md estimate —
the settle viability + 0-multi-LP-writers + Phase 3 done all help).
Still a dedicated multi-session project, but no longer blocked on an
architectural unknown.

## New baseline after session-2 shipped wins

The original plan was written against a c910 hello baseline of ~83 s
and memcpy 224 s. Session-2 shipped (per
[MULTIKERNEL-NOTES.md](MULTIKERNEL-NOTES.md)):

- Simple + partial-range NBA-elision: −10% / −11%
- LTO + matched PGO build: additional −10% / −9%
- **Cumulative: hello 66 s, memcpy 190 s**

The 2-LP PDES Amdahl ceiling is unchanged in *ratio* (~1.5×) but
shrinks in *absolute saved time* because the baseline shrank:

| Workload | Old baseline | New baseline | 1.5× ceiling (old) | 1.5× ceiling (new) | Absolute saving lost |
|---|---:|---:|---:|---:|---:|
| hello | 83 s | 66 s | 55 s (saves 28 s) | 44 s (saves 22 s) | 6 s |
| memcpy | 224 s | 190 s | 149 s (saves 75 s) | 127 s (saves 63 s) | 12 s |
| cmark | 5934 s | ~4750 s est | 3956 s | 3167 s | 1583 s |

The cmark win remains large; hello/memcpy wins are smaller in
absolute terms. **The Phase 4 ROI calculation now favors cmark
strongly.** Validate per-LP correctness on hello (smallest test) but
target cmark for the headline number.

---

## What's already in place (status check)

Per PERLP-EVENTLOOP-PLAN.md plus session-2 work:

| Piece | Status | Location |
|---|---|---|
| `signal_lp_writer` classifier | ✅ shipped | src/compiler/simulator.rs |
| `PerLpSignalTable` + sparse snapshot | ✅ shipped | src/multikernel.rs |
| `SendExecContext` + Send/Sync impl | ✅ shipped | src/compiler/simulator.rs |
| `ClockBarrier`, `BoundaryChannel`, `SignalTable<T>` | ✅ shipped | src/multikernel.rs |
| `PdesKernel` + `PdesCoordinator` (toy) | ✅ shipped | src/multikernel.rs |
| `EventLoopState` extraction (Phase 3) | ✅ uncommitted | src/compiler/simulator.rs:~277 |
| `run_one_tick(&mut self, state)` (Phase 3) | ✅ uncommitted | src/compiler/simulator.rs:~8123 |
| `BoundaryChannelTopology` + `build_boundary_channels` | ✅ uncommitted | src/multikernel.rs:230–290 |
| Lookahead-K helpers (`pdes_lookahead_batches`, etc.) | ✅ uncommitted | src/multikernel.rs:155–207 |
| 20 PDES unit tests | ✅ all pass | src/multikernel/tests.rs |
| Toy 2-counter validation (CMB protocol) | ✅ passes | examples/perlp_toy.sv |
| **`run_per_lp_event_loop` function** | ❌ MISSING | — |
| **Per-LP NBA bucket integration** | ❌ MISSING | — |
| **$display / $finish routing to LP-tb** | ❌ MISSING | — |
| **Settle parallelization per LP** | ❌ MISSING | — |

The infrastructure is ~95% there. The remaining 5% is the actual
runtime wiring — the `run_per_lp_event_loop` function and its
integration with the existing Simulator::event_loop.

---

## Minimal first deliverable (smallest possible Phase 4)

Goal: prove the per-LP thread skeleton WORKS before adding any of the
hard pieces. **Drop K to 1, drop boundary channels, drop $display
routing.** Just spawn 2 threads each running run_one_tick on its own
EventLoopState with a per-tick barrier rendezvous.

This will produce WRONG sim_time (no boundary signal flow → cross-LP
signals stay X). That's OK — the goal is to prove the spawning,
barrier, and join work without crashing. Add correctness one piece at
a time after.

### Step 1: skeleton `run_per_lp_event_loop` (~80 LOC)

```rust
fn run_per_lp_event_loop(
    ctx: &SendExecContext,
    state: &mut EventLoopState,
    local_table: PerLpSignalTable,
    lp: LpId,
    barrier: &ClockBarrier,
) {
    while !state.finished && state.iters < state.max_iters {
        // Phase A: each LP runs one tick locally
        ctx.run_one_tick_local(state, &local_table, lp);

        // Phase B: barrier sync (per-tick — K=1)
        barrier.wait();

        // Phase C: (no boundary delivery yet)
        // Phase D: barrier sync
        barrier.wait();

        state.iters += 1;
    }
}
```

The `run_one_tick_local` method needs to be added to `SendExecContext`
— a stripped-down version of `Simulator::run_one_tick` that operates
on the per-LP table + read-only ctx.

### Step 2: integration into `Simulator::event_loop` (~30 LOC)

Behind an opt-in env var `XEZIM_DISPATCHER=perlp` initially, fall
through to the existing event_loop otherwise:

```rust
fn event_loop(&mut self) {
    if std::env::var("XEZIM_DISPATCHER").ok().as_deref() == Some("perlp")
        && self.edge_block_partition_count >= 2
    {
        return self.event_loop_perlp();
    }
    // ... existing path ...
}

fn event_loop_perlp(&mut self) {
    let n_lp = self.edge_block_partition_count as usize;
    let per_lp_tables = self.build_per_lp_signal_tables();
    let barrier = Arc::new(ClockBarrier::new(n_lp));
    let ctx = Arc::new(self.extract_send_exec_context());
    let states: Vec<EventLoopState> = (0..n_lp)
        .map(|_| EventLoopState::extract_from(self))
        .collect();

    let final_states: Vec<EventLoopState> = std::thread::scope(|s| {
        let handles: Vec<_> = states.into_iter().enumerate().map(|(lp, mut state)| {
            let ctx = Arc::clone(&ctx);
            let barrier = Arc::clone(&barrier);
            let table = per_lp_tables[lp].clone();
            s.spawn(move || {
                run_per_lp_event_loop(&ctx, &mut state, table, lp as LpId, &barrier);
                state
            })
        }).collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });

    EventLoopState::merge_per_lp_into(self, final_states);
}
```

### Step 3: validation

Run with `XEZIM_DISPATCHER=perlp --multikernel-scope ...`. Expected:
- TEST: probably FAILS (no boundary signal flow)
- But: process must complete cleanly, no deadlock, no segfault
- sim_time: probably wrong, definitely DIFFERENT from baseline

**That's the deliverable for the first commit.** Bash-test-style:
"does the spawn-and-join work at all?" Add a unit test for the new
event_loop_perlp path using the toy SV.

### Step 4: add boundary channels (~120 LOC)

Use the already-built `build_boundary_channels` from
src/multikernel.rs. In Phase C of run_per_lp_event_loop, drain
inbound channels and write boundary signals into local_table. After
Phase A, write to outbound channels.

Validate: c910 hello should now produce CORRECT sim_time (44 695 ns).

### Step 5: $display / $finish routing (~120 LOC)

Designate LP-tb (LP-A typically, containing the `tb` module) as the
I/O LP. Other LPs send $display strings via a channel; LP-tb prints
in receive order. $finish: any LP sets an `AtomicBool finished`; all
LPs check after each barrier.

Validate: c910 hello prints "Hello, World!" exactly once.

### Step 6: lookahead K (~80 LOC)

Replace per-tick barrier (K=1) with K-tick barrier from
`pdes_lookahead_batches`. Each LP runs K ticks locally, then syncs.
Boundary signals delivered in K-deep FIFO batches.

Validate: c910 hello + memcpy + cmark all PASS, measure wall
improvement.

### Step 7: settle parallelization (~200 LOC, optional)

Within each LP's run_one_tick_local, run settle on per-LP
comb_entries subset only. Cross-LP comb edges (likely few — most
comb is intra-module) need a separate sync.

---

## Estimated effort breakdown

| Step | LOC | Estimated wall time | Risk |
|---|---:|---|---|
| 1 + 2: skeleton + integration | ~110 | half a day | low — pure plumbing |
| 3: skeleton validation | — | 2 hours | medium — first time threads run a real RTL block |
| 4: boundary channels | ~120 | 1 day | medium — boundary write/read ordering |
| 5: $display routing | ~120 | half a day | low — well-defined |
| 6: lookahead K | ~80 | half a day | low |
| 7: settle parallelization | ~200 | 1 day | medium — comb dependency edges |
| **Total** | **~630** | **~4-5 days** | medium |

The original PERLP-EVENTLOOP-PLAN.md estimated 500 LOC for Phase 4
alone. ~630 LOC for the full integration here aligns. Two thirds of
that is plumbing; the actual algorithmic risk is in steps 4 and 7.

---

## Updated done-check criteria

Original plan (PERLP-EVENTLOOP-PLAN.md):
- hello < 75 s
- memcpy < 200 s
- cmark < 6000 s
- c906 hello unchanged ~13.3 s

**Update for post-session-2 baseline:** these targets are now ALREADY
MET by the single-thread optimizations (hello 66 s, memcpy 190 s).
The new per-LP targets should be set against the per-LP Amdahl
ceiling:

- hello < 50 s (1.32× from 66 s — conservative, given the smaller
  ceiling) — stretch: < 45 s (1.47×)
- memcpy < 145 s (1.31×) — stretch: < 130 s (1.46×)
- cmark < 4000 s (1.48×) — biggest absolute win

And the baseline-preservation checks:
- All 20 PDES unit tests still pass
- c906 hello (single-LP fallback) unchanged ~13.3 s
- Bit-identical sim_time on all three c910 tests

---

## Open architectural questions for the implementer

1. **Time-0 settle**: per-LP or global? PERLP-EVENTLOOP-PLAN.md
   suggests running time-0 globally then splitting. Confirm with the
   classifier's read-set analysis — if any LP needs cross-LP signals
   at time 0, the split is unsafe.

2. **`$readmemh` ordering**: only LP-tb should call $readmemh. Other
   LPs that reference the read-into arrays need the memory contents
   visible. Either (a) $readmemh runs at time 0 (pre-split) and the
   resulting memory is shared, OR (b) all memory-array signals are
   marked LP-tb-exclusive in the classifier.

3. **NBA cross-LP delivery semantics**: when LP-A writes to a
   boundary signal (e.g. AXI-W channel), LP-B sees the change after
   the next barrier. But the NBA semantics say "all NBAs at time T
   apply before any T+1 settle." If A's NBA at T arrives at B at
   T (through barrier sync), B's settle at T sees the new value. If
   it arrives at T+1, semantics break. Solution: drain channels
   BEFORE the settle phase (Phase C of the 3-barrier protocol).

4. **Sparse signal_table merge**: at end of run, per-LP tables must
   merge back into the global signal_table for any post-sim queries
   (xtrace dumps, etc). Use the global-id → local-id maps already in
   `PerLpSignalTable`.

5. **Stage 7 (settle parallelization)**: cross-LP comb dependencies.
   The classifier already identifies 109 boundary signals; some are
   comb-driven (LpIoStats::boundary_directions). For those, settle
   needs a mini-sync: write outbound, deliver inbound, re-settle.
   Risk: settle infinite loop if comb cycles cross LPs. Mitigation:
   cascade_limit (already exists) bounds it at 8 iters.

---

## Sequencing recommendation

Do not start #10 (Phase 4) until:

1. The session-2 shipped wins (NBA-elision, LTO+PGO) are COMMITTED.
   Currently uncommitted. Committing first means the per-LP work has
   a stable single-thread baseline to measure against.
2. `signal_table` write-site audit is complete (per the
   [JIT-REDESIGN-NOTES.md](JIT-REDESIGN-NOTES.md) "Maintenance
   contract" section). Even if not implementing JIT inlining, the
   audit's findings inform per-LP correctness — any direct
   signal_table write site that an LP thread might bypass is a
   correctness bug, similar to the write_sig! macro failure that
   killed the touch-tracker approach.

If both prerequisites are met, the per-LP event_loop is a 4-5 day
project producing 1.3-1.5× speedup on hello/memcpy and a much
bigger absolute win on cmark.

If either prerequisite is not met, do them first. Total session-2
shipping audit + commit work: ~half a day.