# In-place worklist settle parallelization — scope

Pivot after the B2a negative result (replacing canonical settle with an
isolated full-cone re-settle: correct early, diverged late, inherently slow).
This scopes parallelizing the **canonical incremental settle worklist in
place**, and — critically — assesses whether it can actually win.

Goal: parallel settle that keeps hello/memcpy `TEST PASSED` (sim_time 44695 /
101965) with wall-clock < single-thread, on the dual-core c910 cut
(core0 = `x_ct_top_0` vs core1 + uncore).

---

## Code-grounded facts (verified this session)

- **The eval is NOT the bottleneck.** `exec_insns` (simulator.rs:8541) drives
  comb blocks off **pre-resolved signal IDs** (`Insn::LoadSignal(dest,
  sig_id)`), no per-insn name/HashMap lookup. The isolated evaluator
  (`exec_comb_block_isolated`) uses the same ID-based insns. So per-entry
  cost is comparable; B2a's ~4× slowness was the full-cone × multi-round
  shape and t=0/program-load, not per-entry eval.
- **Steady-state settle cones are SMALL.** Measured first time>0 settle:
  seed=7 entries → 162 signals changed, converged in 1 round (reseed=0). The
  canonical settle is incremental (worklist = dirty cone only), so typical
  per-call work is hundreds of entries, not the 438,580-entry full graph.
- **Partition (semantic cut):** comb entries total 438,580 — LP-A (core0)
  185,400, LP-B (core1+uncore) 253,180, straddle=0, orphan=0 (clean).
  Cross-LP comb edges = 115; boundary signals = 115. 6 AST-fallback entries
  (all raw `ContAssign`).
- **Cross-LP comb coupling:** acyclic DAG, wavefront depth 2 → iterated
  boundary exchange converges in ~3 rounds (prior `pdes_crosslp_cycle_analysis`).
- **Eval shared-state surface** (what must be per-LP or partitioned to run
  two LP drains concurrently): `signal_table` (read shared / write OWNED by
  `signal_owner_lp`), `vm_regs` (scratch → per-LP), the worklist
  (`settle_triggered` / `settle_triggered_list` / `cur_list` → per-LP),
  dirty propagation (`dirty_signals` / `dirty_list` → per-LP), `table_modified`
  (bool, OR-combine), prof counters (sum). Inactive for the c910 interpreter
  run: `signal_inline_bits` (JIT), `activity_mon`, `sdf_delays` — so
  `after_signal_write` is a no-op and there are no delayed comb writes.
- **The 6 AST ContAssign entries** need the AST interpreter
  (`eval_expr_ctx` + name resolution) → stay on the coordinator (already
  solved via `eval_ast_contassign_entries`).

---

## The naive design: partition the worklist

Split the canonical chaotic-iteration drain by entry ownership:
1. Seed per-LP worklists from the dirty cone (entry → LP via `signal_owner_lp`
   of its write target; AST entries → coordinator).
2. Round: both LP workers drain their ready entries in parallel — isolated
   eval, write OWNED signals, collect dirtied into a per-LP list. No
   write-write race (disjoint owners); cross-LP reads see prior-round values.
3. Barrier: hand each LP the cross-LP dirtied signals' dependents (the
   115-signal boundary set) as next-round seed; coordinator runs the 6 AST
   entries.
4. Repeat until all worklists empty (global fixpoint, ≤~3 rounds by depth-2).

Building blocks already exist: `settle_subset_tracked` (per-LP incremental
drain + changed-signal report + persistent scratch), `eval_ast_contassign_entries`,
`signal_owner_lp`, the step-1 persistent barrier harness.

### Why the naive design likely DOES NOT WIN (the granularity wall)

A typical steady-state settle touches ~100s of signals (162 in the sample).
Splitting that across 2 worker threads with a per-round barrier means the
barrier/handoff sync cost (cache-line ping-pong, condvar wakeups) is paid
per settle call, ~1–2× per tick × ~150k ticks. This is the SAME granularity
problem that made per-tick edge parallelism net ~0 (measured, conclusive:
seq 66.96s ≈ scoped 65.39s ≈ pool 65.29s). Parallelizing a 162-entry cone
across 2 threads + a barrier almost certainly loses to running it serially.

The settles that ARE big enough to parallelize (t=0 init, program-load,
full-graph) are **one-time**, not the steady-state hot path. So per-settle-call
worklist parallelism is the wrong granularity — confirmed by the same wall
that beat edge.

**Verdict: the naive in-place worklist split is not worth building.** It
inherits the granularity wall.

---

## The architecture that could actually win: per-LP full-tick independence

To amortize sync cost, a worker must stay busy across MANY entries between
barriers — i.e. do an LP's ENTIRE tick (edge exec + settle) before syncing,
not a per-phase barrier. This is the original PDES per-LP vision:

- Persistent per-LP workers, each owning one core's logic (core0 / core1)
  plus its share of uncore by read-affinity.
- Each worker runs its LP's full tick locally: apply inbound boundary, edge
  exec (clocked), settle (comb) to fixpoint — reading its own canonical
  per-LP view, writing owned signals.
- Sync ONCE per tick at a barrier: exchange the 115 boundary signals via
  lookahead (registered cross-core signals are 1-cycle-delayed → safe; the
  115 are comb-consumed AXI/BIU handshakes → need the iterated 2–3 round
  exchange, already proven correct: multi-tick value-only mismatch = 0).
- Coordinator owns time/clocks/testbench(`run_scheduled_process`)/`$finish`/IO
  and the 6 AST entries.

This makes the parallel region one whole tick of one LP (edge+settle, ~95% of
the work) per barrier — coarse enough to beat sync overhead. It is the
"combined Phase B" the project pointed at, done as full-tick LP independence
rather than per-phase offload.

### Hard problems, ranked

- **R1 — eval thread-safety (HIGHEST).** Running two full-tick LP drains
  concurrently needs the eval (`exec_insns` + the settle drain) to touch only
  per-LP state + owned signal writes. The eval is `&mut self` over a wide
  surface. Options: (a) thread per-LP context through `exec_insns`/the drain
  (big refactor); (b) `UnsafeCell`-shared `self` with disjoint-write
  discipline + per-LP scratch/worklists, barrier-guaranteed no concurrent
  conflict. The isolated evaluator already proves the eval CAN run on a view
  + per-LP scratch + ID-based insns — that is the seam to build on.
- **R2 — boundary correctness (HIGH).** The 115 boundary signals are
  comb-consumed (not a registered seam). The proven recipe (broadcast edge
  NBAs to both views + iterated cross-LP settle, ~3 rounds) gave value-only
  mismatch = 0 offline; must hold in the live loop driving the testbench.
- **R3 — testbench/IO coherence (HIGH).** `run_scheduled_process` reads core
  outputs (post-settle, merged) and drives inputs. Needs a coherent merged
  view at the coordinator each tick.
- **R4 — per-LP views vs shared table (MEDIUM).** Two persistent per-LP views
  (clones) avoid intra-round read/write races but cost memory (2× ~15 MB
  table) and per-tick boundary reconciliation; a shared table with the
  2-phase (read-boundary→barrier→write-owned) round avoids clones but needs
  careful discipline. Start with the safe model, optimize later.
- **R5 — load balance (MEDIUM).** core0 (185k entries) vs core1+uncore
  (253k); edge 3.39×@4thr but settle capped at 2 LPs. Uncore split by
  read-affinity for balance (prior `pdes_build_comb_partition_balanced`).

---

## Phased plan (each independently validatable by TEST PASSED)

- **P0 — decision gate (cheap).** Confirm the granularity verdict with a
  microbench: time a serial 162-entry cone settle vs a 2-thread+barrier
  split. If serial wins (expected), do NOT build the naive worklist split;
  commit to full-tick LP independence.
- **P1 — single-LP-on-worker skeleton.** Reuse the step-1 barrier harness:
  move ONE LP's full-tick work (edge+settle for its owned blocks/entries) onto
  a worker reading a per-LP view, coordinator does the rest + merge. Validate
  hello TEST PASSED (still effectively serial — proves the data plumbing +
  per-LP view + merge + the eval-on-view seam).
- **P2 — two LPs in parallel, lookahead boundary.** Both workers run their
  full tick concurrently; barrier-exchange the 115 boundary signals with the
  iterated 2–3 round scheme. Validate hello TEST PASSED; then memcpy.
- **P3 — perf + balance.** Measure wall-clock vs serial; shared-table model
  (R4) if view-clone cost dominates; uncore read-affinity split (R5); edge
  can use >2 threads (proven 3.39×@4thr) while settle stays 2-LP.

---

## Honest assessment

- The naive in-place worklist split is a **dead end** (granularity wall — same
  one that beat edge). Do not build it; this is the main result of this scope.
- The only shape that can win is **per-LP full-tick independence** (each core
  simulated locally, lookahead boundary sync each tick) — coarse enough to
  amortize sync.
- That is dominated by R1 (eval thread-safety) and R2/R3 (boundary + testbench
  coherence), NOT by the compute (already built + measured: edge 3.39×@4thr,
  settle ~2×, multi-tick value-only mismatch 0).
- Realistic end-state: ~1.7–2.0× at 2 LPs (Amdahl-limited by serial
  coordinator + 2-LP settle cap), more if edge dominates with >2 threads.
- This is a multi-week, research-grade effort. The cheap, decisive first step
  is P0 (the granularity microbench) — if it confirms the wall, every future
  hour goes to full-tick LP independence, not worklist tuning.

---

## Evaluation: per-LP full-tick independence

**What's already de-risked (stronger than it looks):**
- R1 (eval thread-safety) is LARGELY SOLVED for the bulk. Workers do NOT run
  `&mut self exec_insns`; they run the **already-built Send isolated
  evaluators** on per-LP views: `SendExecContext::pdes_exec_block` (edge —
  validated 0 cross-LP NBA writers, mismatches=0, 3.39×@4thr) and
  `CombSettleCtx::settle_subset(_tracked)` (settle — offline bit-identical
  217005/217013). Both use pre-resolved signal IDs (not name lookups), so
  per-entry cost ≈ canonical. The only coordinator-only bits: the 6 AST
  ContAssign comb entries + any StmtFallback edge blocks (few, already
  routed).
- Granularity is plausibly fine at full-tick: the parallel region is a whole
  tick of edge+settle (~95% of work). Barrier cost (~µs × ~3 boundary rounds
  × tick count) is small vs ~180s of compute — UNLIKE per-settle-call. This
  is the one decomposition that clears the wall.
- The threading/barrier/disjoint-write/boundary-channel plumbing already
  exists in `multikernel` (see below).

**The one gate that decides everything — UNRESOLVED:**
- B2a replaced only SETTLE with the partitioned scheme and **TEST FAILED in
  the live loop**, yet the OFFLINE multitick harness reported value-only
  mismatch = 0 (all diffs X-ordering, "benign"). These contradict, and the
  contradiction was never resolved (the clone-shadow couldn't reach the late
  divergence). Two possibilities, opposite implications:
  - (a) B2a integration/reseed bug → the partitioned/boundary scheme is
    actually correct live → full-tick independence is sound, proceed.
  - (b) The "X-ordering diffs are benign" assumption is FALSE in the full
    design — an X-resolution difference on an AXI/CDC handshake flips edge
    detection → functional divergence. If so, the partitioned settle (which
    produces different X-ordering than the monolith) is fundamentally
    unsound on THIS cut, and full-tick independence on the core0/core1 cut
    fails too — the cut must move to a REGISTERED seam (L2/BIU master), a
    different and harder partition.
- This single unknown (is B2a's first divergence real-value or X-only?) gates
  the entire per-LP direction. It is cheap to answer relative to building the
  architecture, and MUST be answered first.

**Secondary risks:** load imbalance (core0 185k vs core1+uncore 253k entries →
<2× even at perfect parallelism); testbench/IO coherence (R3 — needs the
testbench's signal footprint to be coordinator-owned or merged each tick,
unverified); shared-clock assumption (both cores likely one clock domain →
lockstep ticks → barrier-friendly, but unverified).

**Verdict:** architecturally the right and only winning shape, and most of the
compute machinery exists — but it is GATED on resolving the B2a divergence.
Do not build it before classifying that first divergence. Net: conditional
GO, pending the (a)-vs-(b) determination.

## Evaluation: the `multikernel` module

`multikernel` (PdesCoordinator/PdesKernel/SignalTable/ClockBarrier/
BoundaryChannel) IS the existing skeleton of per-LP full-tick independence —
one OS thread per kernel, shared `SignalTable` (UnsafeCell, disjoint-owner
writes), `ClockBarrier` sync, mpsc `BoundaryChannel` FIFOs, lookahead-K
batching. It works and is clean.

**But it is a PROTOCOL proof, not the real simulator — gaps that ARE the work:**
- **`SignalTable<u64>`**, not `Value` — no 4-state X, no wide (>64b) signals.
  c910 needs `SignalTable<Value>` (acknowledged in-code).
- **Stub blocks** (`KernelBlock = Box<dyn Fn(&[u64]) -> Vec<(usize,u64)>>`) —
  NOT the real edge/comb logic. The 438k comb entries, exec_insns, edge
  detection are absent.
- **No combinational settle** — the kernel does blocks→NBA→boundary only.
  The 40% settle (cone propagation, cascade) is not modeled at all.
- **No testbench / event queue / clocks / `$finish` / `$readmemh` / DPI / IO**
  — it just ticks `clock_period_ns` to `max_sim_time`. The entire c910
  testbench coordinator is absent.
- **CRITICAL: its boundary protocol is lookahead-1 FIFO** ("FIFO order
  supplies the previous-tick value") — i.e. it assumes a REGISTERED boundary.
  c910's 115 cross-LP signals are COMB-CONSUMED (zero lookahead, Phase A
  verdict). So multikernel's protocol is UNSOUND for the c910 cut as-is —
  building on it naively reproduces the multitick divergence. It must be
  replaced by the proven iterated comb-boundary exchange (broadcast edge NBAs
  to both views + settle→exchange→reseed ~3 rounds).

**Verdict:** multikernel's VALUE is the validated threading/barrier/channel/
disjoint-write mechanics + lookahead batching — a real head start on the
plumbing. But it solves the EASY case (registered boundary, u64, stub blocks,
no settle, no testbench); every HARD part of c910 (Value, real eval, settle,
comb-boundary correctness, testbench coherence) is outside it and is the
multi-week effort. Reuse its skeleton; do NOT trust its lookahead-1 protocol.

## Gating question ANSWERED — it's X-fidelity, not real values, not boundary

The cheap detector kept stalling in program-load, but the existing fast stub
(`--pdes-c910-stub` + `XEZIM_PDES_CHK_KINDS=1`) already isolates the
discrepancy without a full sim. `[PDES-CHK] checked=217013, bits_mismatch=8,
repr_diff=522`. Dumping them:

- **522 repr_diff**: bit-identical v/x, only the `is_signed` flag differs
  (APB GPIO/PMU peripheral regs). Functionally irrelevant (same bits).
- **8 bits_mismatch — ALL X-INVOLVED, ZERO real-value diffs:**
  - `x_plic_top...m/sclaim_eq_vec_exp`: canonical X (1 bit) vs isolated X
    (wider, nx=0x200000001) — the isolated eval OVER-widens the X.
  - `x_hart_mie_ff/sie_ff..._rd_idx_{0,2,4}_.data_in` (6): canonical X vs
    isolated 0 — the isolated eval resolves X→0.
  All in the PLIC / hart interrupt-enable (MIE/SIE) CSRs — uninitialized
  registers reading X at reset.

**Interpretation (UPDATED after bytecode tracing — supersedes the first read):**
the live divergence is NOT a real value error, NOT a boundary failure, and —
crucially — NOT an isolated-evaluator fidelity bug either. Tracing the failing
blocks (`XEZIM_PDES_DUMP_SIG`) shows the relevant opcodes (`RangeSelect`,
`Select`, `BlockingAssign`) are BIT-IDENTICAL between `exec_comb_block_isolated`
and canonical `exec_insns`. So re-eval-from-golden can only differ if golden
isn't a fixpoint for the block — and it isn't: e.g. `mie_ff..data_in =
sig397036[1][31:0]`; isolated reads golden `397036[1]=0` → `data_in=0`
(consistent), but canonical golden has `data_in=X` (a STALE X left when 397036
was X earlier and the block didn't re-fire). These 8 are exactly the
**X-convergence-ordering artifacts** the code already notes — canonical golden
carries residual X on uninitialized-at-reset PLIC/hart interrupt CSRs; a fresh
eval resolves them. They are **benign** (uninit-at-reset; the program writes
these CSRs before using interrupts).

**Verdict: GO on the core0/core1 cut, and the isolated evaluator is FAITHFUL.**
"Drive bits_mismatch to 0" was the WRONG goal — those 8 are benign canonical
convergence X, not evaluator bugs (a minor real fidelity win was still landed:
isolated `BlockingAssign` now masks-to-width + stamps is_signed like canonical,
repr_diff 522→517). The partitioned/per-LP settle is value-faithful modulo
benign X-ordering on uninit signals. **B2a's TEST FAILED is therefore an
INTEGRATION bug (the cross-LP reseed / dirty-seed handling in
settle_combinatorial_perlp), not eval fidelity** — that is where the next
debugging effort belongs (re-run the cheap detector after auditing the reseed,
or audit `push_cross_lp_dependents` + the round-loop seed for a missed
propagation).

## RESOLUTION (2026-05-29) — B2a correctness fixed; root cause = boot-X

The B2a partitioned settle's TEST FAILED was a HANG ("no instructions retired
in 50000 cycles"), not a data mismatch. Root cause, established by elimination:
- NOT reseed-incompleteness (made reseed conservative — all dependents).
- NOT non-convergence (MAX_ROUNDS hits = 0).
- NOT isolated-evaluator fidelity (opcodes bit-identical; the 8 stub
  bits_mismatch are benign X-convergence artifacts on uninit interrupt CSRs).
- IT IS: **order-dependent 4-state X-resolution at BOOT.** The partitioned
  settle's per-LP + rounds evaluation order resolves boot-time X
  (uninitialized flops) differently than canonical's serial worklist; that X
  difference propagates into core control logic and hangs the core. Intrinsic
  to parallelizing 4-state settle while boot-time X is present.

**FIX (confirmed): run reset/boot CANONICALLY, partitioned only post-boot**
(`XEZIM_PERLP_AFTER=<sim_time>`). Sweep: AFTER=40000 PASS (1286 perlp
settles), AFTER=2000 PASS (12793 settles), AFTER=200 FAIL (hang). Boot-X
window is (200, 2000]; AFTER>=2000 is safe. **The partitioned per-LP settle
is now FUNCTIONALLY CORRECT in the live loop** (hello TEST PASSED) — the
make-or-break correctness milestone.

Live gates in settle_combinatorial_perlp: t=0 canonical, dirty>256 canonical
(one-time big cones), time<perlp_after canonical (boot). All perlp-only;
default settle untouched.

REMAINING — and B3 (per-settle threading) is MEASURED FUTILE: instrumented
the perlp settle (hello AFTER=2000): total 17.9s, parallelizable (2x
settle_subset) 14.0s (78.1%), threading ceiling 1.64x. Threading saves ~7s on
settle but the partitioning overhead is ~8s (perlp 74s vs baseline 66s) => net
~baseline, NO win. Per-settle threading hits the granularity/overhead wall.
The ONLY shape that wins is FULL-TICK LP INDEPENDENCE: parallelize edge (~55%)
AND settle (~40%) together between per-tick barriers, so the partition
overhead is amortized across the whole tick (not paid per settle). That is the
multikernel/per-LP-full-tick rewrite scoped above — now with its hardest
prerequisite proven: the boot-gated partitioned per-LP settle is FUNCTIONALLY
CORRECT in the live loop (hello + memcpy TEST PASSED). Also needed: an auto
"boot-done / X-resolved" detector to replace the fixed AFTER threshold.

## FULL-TICK FEASIBILITY — measurement-backed verdict (2026-05-29)

Before committing to the multi-week full-tick rewrite, the economics were
re-examined against the session's measurements. Full-tick parallelizes two
regions; each FAILS independently in the live loop:

- **Edge (~55%)** is the only race-free-cheap parallelism (workers read a
  read-only signal snapshot, return NBAs — no clones, no write races). BUT it
  is ALREADY MEASURED at ~0 speedup three ways on real hello: sequential
  66.96s vs scoped 65.39s vs **persistent pool 65.29s**. The persistent
  ParallelWorkerPool — exactly full-tick's mechanism (no per-tick spawn) — was
  already tested and did NOT beat sequential. Edge is granularity-bound (small
  per-tick fired-block batches + the 10k-insn threshold + load imbalance), and
  persistence does not fix that.
- **Settle (~40%)** is overhead-bound (B3 measured ceiling 1.64x nets
  ~baseline) AND the race-free requirement is fatal for small cones: two LPs
  settling concurrently on a SHARED table race on cross-LP boundary READS
  (LP0 reads an LP1-owned signal while LP1 writes it). The only race-free
  options are (a) two full per-LP VIEW CLONES per settle (~14MB x2 x ~12793
  settles — the clone dwarfs the ~162-signal steady-state cone) or (b) the
  sequential current scheme. So per-settle parallelism is not viable for the
  small steady-state cones that dominate.

**VERDICT: the per-LP PDES speedup does NOT materialize on the c910 dual-core
cut.** Full-tick combines two individually-non-winning parallelisms (edge
granularity-bound — persistent pool already ~0; settle overhead+clone-bound).
The fine-grained per-tick structure (many small fired-block/cone batches) is
the intrinsic bottleneck, not the parallel machinery (which is built + proven:
edge 3.39x@4thr offline, settle correct). The Amdahl ceiling (~1.9x) is not
reachable because both components fail at live-loop granularity.

What IS achieved and durable: the per-LP partitioned settle is now
FUNCTIONALLY CORRECT in the live loop (hello+memcpy TEST PASSED, boot-gated) —
the long-standing correctness blocker is resolved. But pursuing speedup via
this cut/granularity is not justified by the measurements. A speedup would
require a fundamentally coarser decomposition (e.g. a registered L2/BIU seam
with long lookahead so each core runs MANY ticks between syncs — the original
PDES K>1 lookahead vision — not per-tick sync), which is a different,
larger re-architecture, or a different design with more inherent core
independence.

## Unifying conclusion

Per-LP full-tick independence and multikernel are the same architecture at
two maturities (design vs toy skeleton). The gating question — "is the
divergence real-value, X-only, or a fundamental boundary failure?" — is now
ANSWERED (see above): it's an X-fidelity gap in the isolated evaluator on 8
interrupt-controller signals, zero real-value divergence, no boundary
unsoundness. The core0/core1 cut is viable.

So the true prerequisite, the cheapest and most decisive next step, is:

  **Make `exec_comb_block_isolated` bit-exact (including X) with canonical —
  drive the stub's `bits_mismatch` from 8 to 0.**

That is a bounded evaluator-fidelity fix (width/X handling on PLIC claim
vectors + flop `data_in` X-vs-0), validatable fast via the stub (no full
sim). Once `bits_mismatch=0`, the partitioned/per-LP settle is value-identical
to canonical, and everything else (Value SignalTable, settle on workers,
testbench coordinator, load balance) is plumbing on a sound foundation.
