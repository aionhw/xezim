# Functional parallel event_loop — scope

Goal: a real `event_loop_perlp` that drives the c910 testbench to
`$finish` with parallel edge exec + per-LP settle, validated FUNCTIONALLY
(hello/memcpy `TEST PASSED`, `sim_time` matches the single-thread run),
not by per-tick bit-equality (which is the wrong metric for a lookahead
PDES — see PER-LP-NEXT-STEPS.md "metric insight").

Success metric: `hello` parallel run = TEST PASSED, sim_time 44695;
`memcpy` = TEST PASSED, sim_time 101965; wall-clock < single-thread.

---

## What's already built and validated (this session)

| component | status |
|---|---|
| comb partition (semantic, clean cut) | ✓ straddle=0, 115 cross-LP edges |
| `exec_comb_block_isolated` | ✓ 217005/217013 bit-identical |
| per-LP settle driver (`pdes_settle_subset_view`) | ✓ reconverges to global |
| `CombSettleCtx` (Send) | ✓ threaded settle ~2× |
| `exec_fused_gate_isolated`, `eval_comb_entry_isolated` | ✓ |
| edge partition + `SendExecContext::pdes_exec_block` | ✓ 0 cross-LP NBA writers |
| threaded edge exec | ✓ 3.39× @ 4 threads, mismatches=0 |
| multi-tick loop (clock-toggled, boundary channel) | ✓ runs, 2.94× @ 4 thr |

The compute machinery and speedup (~2.9× @ 4 threads) are proven. What's
missing is wiring it into the REAL loop with the serial coordinator work
and a correct boundary protocol.

---

## Architecture: coordinator + workers

`run_one_tick` (simulator.rs:10684) per tick does, in order:
1. `apply_delayed_updates` + settle      ← SERIAL (delays)
2. `fire_clock_generators`               ← SERIAL (coordinator)
3. `event_queue.remove` + `run_scheduled_process` loop  ← SERIAL (testbench,
   `$display`/`$finish`/`$readmemh`/`#delays`, drives inputs/reads outputs)
4. `apply_nba`                           ← SERIAL (merge NBAs)
5. `settle_combinatorial`                ← **PARALLEL** (per-LP settle)
6. `check_edges` / `drain_edge_cascade`  ← **PARALLEL** (edge exec) + serial detect
7. `check_monitor`, strobes, VCD/aitrace/xtrace writes  ← SERIAL (IO)

So the model is: a coordinator thread owns the canonical state + steps
1–4 + 7; it offloads steps 5–6 (edge exec + settle, the 95% compute) to
2 worker threads operating on per-LP views, then merges. The testbench
(step 3) MUST see a coherent merged state — it reads core outputs and
drives inputs.

Two viable state models:
- **(A) Global canonical + per-tick offload**: keep `self.signal_table`
  canonical; each tick, hand worker threads read-snapshots, collect
  NBA/settle writes, merge back. Simpler correctness (single source of
  truth), but the per-tick snapshot cost (~7ms snap in profile) recurs.
- **(B) Persistent per-LP views**: views are canonical, coordinator
  reads/merges only boundary + IO signals. Faster (no per-tick clone) but
  the coordinator's testbench access needs a coherent view — more complex.

Recommend (A) first (correctness-first), optimize to (B) later.

---

## Hard problems, ranked by risk

### R1 — Boundary lookahead (HIGHEST, gates everything)
Conservative PDES is correct ONLY if every cross-LP signal has ≥1 cycle
lookahead (registered). The 9 observed boundary mismatches are BIU0/CIU
AXI handshakes (`ibiu0_pad_back/_bready/_rack`). **Are they registered or
combinational?** If ALL ~110 cross-LP boundary signals are registered →
lookahead-1 channel is functionally correct, integration is plumbing. If
ANY are zero-lookahead comb → must merge their cones into one LP (extend
the partition) or the parallel sim is wrong. THIS MUST BE ANSWERED FIRST
(cheap: trace each boundary signal to its driver — flop vs comb).

### R2 — Testbench/state coherence (HIGH)
`run_scheduled_process` runs the testbench: reads core outputs (must be
post-settle, merged), drives core inputs. Under model (A) it reads/writes
`self.signal_table` which the workers just merged into — coherent. Under
(B), needs explicit boundary+IO sync. Getting the read/write ordering vs
the worker offload right is the main correctness surface.

### R3 — Serial fraction caps Amdahl (MEDIUM)
Profile (memcpy): edges 106s + settle 76s = 95% parallelizable, but
process 1.2s + snap 7s + sched + traces are serial. With xtrace/VCD
enabled the serial IO can dominate — must measure serial fraction WITH
the test's actual tracing. Realistic loop speedup likely 1.7–2.0× at 2
LPs (not the 2.94× compute-only number), capped by the serial coordinator
work + settle's 2-LP limit.

### R4 — Delays / SDF / #-delays (MEDIUM)
`apply_delayed_updates` + the event_queue handle timed updates. These are
global/serial. Per-LP delayed updates would need per-LP queues; initially
keep delays on the coordinator (serial) — correct but a serial tax.

### R5 — The 6 AST comb entries + non-parallel edge blocks (LOW)
ast_ca=6 + non-parallel-eligible edge blocks run on the coordinator.
Already identified; trivial to route.

---

## Phased plan

### Phase A — De-risk: boundary lookahead analysis — DONE, verdict NO-GO
`pdes_boundary_lookahead_report` on the core0-vs-rest cut (c910):
```
boundary=147  comb_consumed(true blocker)=115  registered-only-consumed=32
```
115 of 147 boundary signals are consumed COMBINATIONALLY across the cut
(zero lookahead) — the `pad_ibiu0_*` AXI/BIU interface + clock/reset/
config/debug, read same-cycle by core0. This is the SAME 115 as the comb
partition's cross-LP edge count. **The core0/core1 boundary is NOT a
registered seam — the cores communicate through the CIU/L2 via
combinational AXI handshakes at the cut point.** Naive lookahead-1 is
therefore unsound here (root cause of the multi-tick divergence).

→ NO-GO for Phase B on this cut. Prerequisite (Phase A.2): re-cut at the
REGISTERED seam. Candidates:
  (a) Cut at the L2/memory interface: LP = core+L1+BIU (the BIU registers
      the AXI master side), shared L2/memory on the coordinator or a 3rd
      LP. The AXI master/slave register stages give the lookahead.
  (b) Co-locate the 115 comb-consumed cones into one LP (pull the BIU/CIU
      interface wires to core0's LP) — but these cones reach into the
      shared CIU, so this likely collapses toward (a).
  (c) Within-tick producer-ordered settle for feedforward cross-LP comb
      paths + iteration only for true cross-LP comb CYCLES. Needs a
      cycle-vs-feedforward analysis of the 115 (not yet done). The naive
      iterate-to-fixpoint attempt did not converge to the monolith,
      suggesting cycles and/or the ordering wasn't producer-first.

Further analysis worth doing before Phase B: classify the 115 into
feedforward vs cross-LP comb cycles (option c), and locate the BIU's
registered AXI stage (option a). Both are bounded analyses.

### Phase A.2 — cycle-vs-feedforward analysis — DONE → conditional-GO
`pdes_crosslp_cycle_analysis` on the core0-vs-rest cut (c910):
```
cross-LP comb edges: A->B=61  B->A=54  (BIDIRECTIONAL)
wavefront: max_crossings=2  converged=true (NO comb cycle)
```
- **No combinational cycle** — the cross-LP comb coupling is an acyclic
  DAG, so iterated boundary exchange is guaranteed to TERMINATE.
- **Wavefront depth = 2** — longest comb chain crosses the boundary ≤2×,
  so a correct partitioned settle needs only ~2–3 iterated exchange rounds
  per tick (settle→exchange→settle→exchange), NOT unbounded iteration.
- Bidirectional, so producer-ordering alone fails; but bidirectional +
  shallow + acyclic is the easy case for iteration.

**Verdict upgraded NO-GO → CONDITIONAL-GO.** The boundary isn't registered,
but a correct scheme is cheap and well-defined: option (c) =
CONSISTENT partition + 2–3 round iterated cross-LP settle. This also
explains the failed earlier iteration attempt: not a fundamental obstacle
(depth-2 converges) but two bugs — (1) partition inconsistency (edge_part
vs comb_part disagreed on uncore), (2) the EDGE phase still read
lookahead-1 boundary. The settle coupling itself converges in 2 rounds.

### Revised Phase B prerequisites (the correct scheme)
1. ONE consistent partition: assign edge blocks by comb signal ownership
   (so views agree on every signal's LP).
2. Per tick: edge exec → NBA apply → iterated settle (settle both LPs →
   exchange ALL cross-LP signals → reseed dependents → repeat until the
   boundary is stable, ≤3 rounds by the depth-2 result).
3. The iterated settle converges each tick to the monolithic fixpoint, so
   next tick's edge reads see correct values — no accumulating skew.
4. Validate functionally (TEST PASSED), but per-tick bit-equality vs
   monolithic should now ALSO hold (the 2-round settle resolves the
   boundary same-tick, matching lookahead-0 monolithic).
This is materially more tractable than the L2-reseam (option a); recommend
option (c) for Phase B.

### Phase B step-1 attempt (recipe in the multitick harness) — REVERTED
Applied the recipe to `pdes_validate_parallel_multitick`: consistent edge
partition (edge blocks by comb `signal_owner_lp`) + full cross-LP exchange
set + iterated cross-LP settle (settle→exchange→reseed→repeat ≤16 rounds).
Per-tick mismatch trajectory (8 ticks, vs naive lookahead-1):
```
iterated:  [10993, 12476, 12502, 19373, ..., 19391]   plateau 19391
naive:     [  930,  1556,  4406, 39147, ..., 39197]   plateau 39197
```
The recipe HALVES the plateau (39197→19391, direction confirmed) BUT
REGRESSES tick 1 (930→10993). Tick-1 regression is diagnostic of an
IMPLEMENTATION BUG (more settle work must move toward, not away from, the
monolithic fixpoint), not a convergence limit — the depth-2 cycle analysis
proves the recipe can reach 0. Also tanked speedup (1.07x @ 2 thr: the
iteration + per-round full-exchange + reseed is expensive). REVERTED (don't
ship a buggy diagnostic).

Prime suspect for the bug: edge-written boundary signals have
`signal_owner_lp[sid] == 0xFF` (signal_owner_lp is computed only from comb
lp_entries' write targets, NOT edge NBA writes), so the exchange's
`match owner { 0=>.., 1=>.., _=>skip }` SILENTLY DROPS them — the registered
BIU handshakes never get delivered. Fix: compute a full owner map including
edge-NBA writers, and exchange by that. Secondary: verify the full `exch`
set delivery + reseed don't corrupt (isolate via a single-tick monolithic-
vs-iterated-partitioned settle diff, listing the diverging signals — extend
`pdes_validate_perlp_tick` with the iteration + a mismatch dump).

This is a BOUNDED debugging task (find the tick-1 regression on one tick),
NOT a fundamental obstacle. Best done fresh, not at a session tail.

### RESOLVED — multi-tick loop is FUNCTIONALLY CORRECT (commit 2cded55)
The fix was NOT owner==0xFF. Two changes made the multitick correct:
1. **Broadcast edge NBAs to BOTH views** (not own-view-only). The single-
   tick test was clean because it reads a SHARED snapshot for edges; the
   multitick reads per-LP views, so edge outputs must be broadcast to
   eliminate cross-LP edge staleness.
2. **Iterated cross-LP settle** (settle→exchange comb boundary→reseed→
   repeat, ~2-3 rounds per the depth-2 result) — proven on a single tick
   to drive mismatches 9→0 (semantic) and 925→0 (balanced).

c910 result (8 ticks, comb-boundary exchange):
```
[PDES-MT-MM] last-tick mismatches=328  X-involved=328  VALUE-ONLY=0
[PDES-MULTITICK et=2] speedup=1.54x   et=4 speedup=1.81x
```
final_mismatch 39197 → 328, and ALL 328 are X-RESOLUTION-ORDERING
artifacts (value-only=0) — AXI bready/rready + CDC synchronizer flops
(jtag2pmu_sync), X by nature. ZERO actual logic divergence from monolithic.
The parallel loop matches the monolith on every resolved value; the only
differences are benign 4-state X-ordering (the documented metric issue).

Trade-off: 1.54x@2thr / 1.81x@4thr vs the naive (wrong) 2.9x — iteration
is the price of correctness. Comb-boundary exchange suffices (the full
cross-LP exch set gave the same 328, slower).

### What remains (now: production integration, correctness de-risked)
The validation harness proves the scheme. Remaining = wire it into the
live `event_loop` (FUNCTIONAL-PARLOOP-SCOPE Phase B/C): per-LP check_edges
+ real event/IO routing + $finish, driving the testbench, validated by
TEST PASSED (not per-tick bits — the 328 X-ordering diffs are expected and
functionally irrelevant). The hard correctness questions are now answered.

### MEASURED: the existing per-tick edge dispatch gives ~0 speedup
The live `event_loop` ALREADY parallelizes edge exec — `check_edges`
spawns `std::thread::scope` threads when a tick fires ≥2 parallel blocks
totaling ≥10k insns. Measured on real hello (full sim, TEST PASSED both):
```
NO_PARALLEL=1 (sequential): sim 66.96s  wall 1:32.52
default (parallel dispatch): sim 65.39s  wall 1:32.85  CPU 97% (~1 core)
                             par_dispatch legacy=4462
```
~0 speedup (2.3% = noise; 97% CPU = effectively single-core) despite 4462
parallel dispatches. WHY: (1) per-tick thread SPAWN overhead — each batch
is small relative to spawn/join; (2) the 10k-insn threshold — sparse ticks
stay sequential; (3) settle (40% of the loop) is fully sequential.
Per-tick-spawn granularity is too fine to win.

→ This is exactly why the SESSION SCHEME is the right design and what the
integration must change vs the existing dispatch:
  - PERSISTENT per-LP threads spanning many ticks (not per-tick spawn).
  - Parallelize SETTLE too (not just edge).
The validation harness (persistent views, parallel edge+settle, iterated
boundary) measured 1.5–1.8× AND is functionally correct (value-only=0).
Wiring THAT into the loop (replacing the per-tick-spawn dispatch) is the
real remaining work — a focused multi-session integration, not a tweak.

### CONCLUSIVE: per-tick edge parallelism is exhausted (3-way measured)
Real hello full sim, all TEST PASSED:
```
sequential (XEZIM_NO_PARALLEL=1):   sim 66.96s
scoped parallel edge (default):     sim 65.39s   97% CPU
persistent pool (XEZIM_PDES_POOL=1): sim 65.29s   97% CPU
```
All three within noise, all ~1 effective core. The persistent pool
(ParallelWorkerPool, already in-tree) does NOT beat scoped — confirming
the bottleneck is per-tick GRANULARITY (small batches + the 10k threshold)
and the fully-sequential settle, NOT thread-spawn. Per-tick edge
parallelism cannot win. The rewrite is mandatory.

### The rewrite — architecture + first sub-unit (for a fresh session)
Architecture: self.signal_table behind a shared UnsafeCell-style table
(see multikernel::SignalTable<T>); PERSISTENT per-LP worker threads each
OWN a disjoint write partition (per signal_owner_lp) and run the validated
per-tick scheme (edge exec + iterated cross-LP settle) reading shared /
writing owned — across the WHOLE sim, synchronized by a per-tick barrier,
NOT re-dispatched per tick. Coordinator (main) owns time advance, clocks,
check_edges DETECTION, event queue, run_scheduled_process (testbench /
$display / $finish / #delays), IO, and the boundary/IO signal sync.

FIRST SUB-UNIT (bounded, runnable, the safe start):
1. A 2-worker barrier harness: persistent threads + a per-tick
   `barrier.wait()` rendezvous with the coordinator (reuse
   multikernel::ClockBarrier). No work yet — just prove the persistent
   coordinator↔worker tick handshake runs hello to $finish with the
   workers as no-ops (coordinator still does all work). Validates the
   threading skeleton + shutdown + $finish, zero correctness risk.
2. Move EDGE exec of fired blocks to the workers (shared-read snapshot,
   owned-write), barrier-synced. Validate TEST PASSED.
3. Move SETTLE to the workers (iterated cross-LP, the proven scheme on the
   shared table). Validate TEST PASSED + measure speedup.
Each step is independently runnable + validatable. Step 1 is the safe
"start" that proves the architecture without touching correctness.

### Phase B step 1 DONE (commit c3d40e3) — barrier harness
`event_loop_perlp` spawns N=2 persistent worker threads that rendezvous
with the coordinator twice per tick via `ClockBarrier` (tick-start +
tick-end). Workers are no-ops; coordinator runs the real loop.
`event_loop_singlethread` gained `Option<&ClockBarrier>` (default path
passes `None` — two if-let no-ops/tick, behaviorally identical).
Shutdown: loop only exits at the top, so workers are always parked at
tick-start on exit; coordinator sets a flag + one final `wait()` to
release them. Validated: hello TEST PASSED under XEZIM_DISPATCHER=perlp
(needs `--multikernel-scope=<core0>` to set edge_block_partition_count>=2
so the perlp path engages); default path TEST PASSED, no regression.

### Phase B — code-grounded findings (2026-05-28, before B2/B3)
Studied run_one_tick (simulator.rs:11042) + settle_combinatorial (13558)
+ CombSettleCtx::settle_subset (23014) + check_edges_inner (12696):
- **Edge exec is ALREADY parallel + correct** in check_edges_inner (scoped
  dispatch ~12879; persistent ParallelWorkerPool gated XEZIM_DISPATCHER=pdes).
  The net-new prize is parallelizing SETTLE (40%, fully serial today).
- **Live settle is INCREMENTAL** (worklist: seeds settle_triggered from
  dirty_list's dependents, evals only the dirty cone). The validated
  offline harness seeded settle_subset with ALL LP entries (part.lp_entries)
  = a FULL re-settle of ~217k entries every call → would be far slower than
  the incremental worklist in the live loop, erasing the parallel gain.
  RESOLUTION: settle_subset DOES propagate from its seed via the dep CSR
  (23047), so seed it with the per-LP DIRTY CONE (dirty_list dependents
  restricted to the LP) → incremental partitioned settle.
- **settle_subset allocates per call**: `vec![false;n_entries]` + worklist +
  next + vm_regs each invocation (~217KB zeroing/LP/call). The hot loop
  needs PERSISTENT per-worker scratch (refactor settle_subset to take
  &mut scratch, or a stateful per-LP settle struct).
- **Race-free shared-table settle** (avoids full per-tick view clones):
  disjoint write partitions (signal_owner_lp) + 2-phase rounds:
  (R) each worker copies its needed cross-LP boundary INPUTS (~147 sigs)
  into a thread-local buffer [shared table read-only here] → barrier →
  (W) each worker settles its OWNED entries, reading own signals from the
  shared table + boundary inputs from the local buffer, writing only OWNED
  signals → barrier → repeat ~3 rounds (depth-2 acyclic ⇒ converges).
  No concurrent read+write of the same slot ⇒ no data race, no UB.
- **Contract**: replacing settle_combinatorial under perlp must still clear
  dirty_any/dirty_list and leave settle_triggered consistent, and produce
  signal_table values that check_edges + drain_edge_cascade read. Validate
  by TEST PASSED (X-ordering diffs benign, value-only=0 proven offline).

Refined sub-steps (each a small validated commit):
- **B2a**: cache CombPartition + CombSettleCtx on self (built in
  event_loop_perlp from the multikernel-scope prefix; store the prefix on
  self when apply_multikernel_scope_partition runs). Compute per-LP
  incremental seed from dirty_list. Single-threaded partitioned+iterated
  settle replacing settle_combinatorial under perlp. Validate hello TEST
  PASSED (proves the algorithm in the LIVE loop incl. cascade/edge-detect).
- **B2b**: persistent per-worker scratch (kill per-call allocs).
- **B3**: drive the 2-phase shared-table settle on the step-1 persistent
  workers (unsafe shared pointer + barrier discipline). Validate + measure.

### Phase B — Minimal functional loop, model (A) (1–2 weeks)
- `event_loop_perlp` real body: keep `self` canonical; per tick run
  steps 1–4,7 on the coordinator; offload steps 5–6 to 2 workers via the
  validated `SendExecContext` (edge) + `CombSettleCtx` (settle) against
  read-snapshots; merge NBA + settle writes back; apply boundary channel
  (lookahead per R1 verdict).
- Route the 6 AST entries + non-parallel edge blocks to the coordinator.
- Validate: hello → TEST PASSED, sim_time 44695. Iterate until functional.
- This is the make-or-break milestone: a correct parallel hello run.

### Phase C — Robustness + perf (1 week)
- memcpy + cmark functional pass.
- Measure real loop speedup with tracing on/off (R3).
- Optimize to model (B) (persistent views, eliminate per-tick snapshot)
  if the snapshot cost is significant.
- Settle's 2-LP cap stands; edge can use 4 threads (proven 3.39×).

---

## Honest assessment

- The compute parallelism is DONE and proven (~2.9× @ 4 threads on the
  edge+settle phases). 
- The functional integration is a genuine multi-week effort, dominated by
  R1 (lookahead correctness) and R2 (testbench coherence), NOT by the
  parallel compute (already built).
- Phase A is cheap and decisive — do it before committing to B/C. If any
  boundary signal is zero-lookahead comb, the partition must change first.
- Realistic end-state loop speedup: ~1.7–2.0× at 2 LPs (Amdahl-limited by
  serial coordinator + 2-LP settle cap), more if edge dominates and uses
  4 threads. The 2.94× compute number is an upper bound, not the loop
  number.
- Recommend NOT starting Phase B at a session tail — it's an
  all-or-nothing correctness integration (the C1 lesson). Start with
  Phase A (analysis), which is bounded and informs everything.
