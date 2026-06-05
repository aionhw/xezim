# Session-4 summary — PDES per-LP parallelization (c910)

Branch `perlp-experiment`, commits `05593a8`..`5752052` (18). Goal:
multicore acceleration of the c910 sim via per-LP parallel edge exec +
settle. ALL work is additive — the default single-thread `event_loop` /
`settle_combinatorial` / `exec_insns` path is untouched; every new
capability lives behind `--pdes-c910-stub` (analysis/validation only).

Regression check (held throughout): hello TEST PASSED sim_time 44695,
memcpy TEST PASSED sim_time 101965 — bit-identical on every build.

## What was built and validated (each via --pdes-c910-stub on real c910)

| unit | commit | result |
|---|---|---|
| per-LP comb partition (semantic cut) | 2a93eb8 | straddle=0, 115 cross-LP edges, coverage_ok |
| `exec_comb_block_isolated` | 597b8ea | 217005/217013 blocks bit-identical to interp |
| per-LP settle driver | c7bfad7 | reconverges to global settle, mismatches=0 |
| `CombSettleCtx` threaded settle | 55706e3 | ~2× (settle compute) |
| balanced partition | 3f2ceaf | settle compute 2.05× (but unsound for ticks) |
| per-LP edge exec (threaded) | a800c1b | 2.17×, cross-LP NBA writers=0, no clone |
| edge N-thread scaling | d395b54 | 1.0/2.0/3.39× @ 1/2/4 threads, mismatches=0 |
| combined single-tick pipeline | bd7c993 | sound (semantic), boundary = 9 BIU handshakes |
| real multi-tick parallel loop | 290daac/dc915fb | **2.5× @2thr, 2.9–3.0× @4thr** |

## Speedup (the demonstrated compute parallelism)
- Edge exec: 3.39× @ 4 threads (embarrassingly parallel, shared read-only
  snapshot, NO view clone — the clean phase).
- Settle: ~2× (capped at 2 LPs — the sound dual-core cut; 4-way settle is
  unsound, see balanced result).
- Multi-tick loop (edge+settle, clock-toggled, clone-free/tick): **2.5× @
  2 threads, 2.9–3.0× @ 4 threads**.
- Profile (memcpy loop): edges 55.6%, settle 39.9%, serial 4.5%. Amdahl
  loop ceiling ~1.9–2.0× at 2 LPs; edge's 4-thread scaling lifts it.

## Correctness arc — the hard part
1. Multi-tick naive lookahead-1 channel: per-tick mismatch plateaus at
   ~39197 (~0.1%, localized to the cross-core interface). NOT a compute
   bug — the boundary signals are delivered a cycle late.
2. **Metric insight (e1b9df8)**: bit-exact-vs-monolithic is the WRONG
   metric for a lookahead PDES (registered cross-core signals legitimately
   skew 1 cycle, functionally invisible). Correct validation is FUNCTIONAL
   (TEST PASSED), which needs the full event_loop integration.
3. **Phase A (b4334a3/286cd8d)**: the core0-vs-rest cut is NOT a registered
   seam — 115 of 147 boundary signals are consumed COMBINATIONALLY across
   the cut (the pad_ibiu0_* AXI/BIU interface). Naive lookahead-1 is unsound
   here (root cause of the divergence).
4. **Phase A.2 cycle analysis (1db2b34)**: the cross-LP comb coupling is
   BIDIRECTIONAL (A→B=61, B→A=54) but ACYCLIC with wavefront DEPTH=2. So a
   correct partitioned settle converges in ~2-3 iterated exchange rounds.
   Verdict upgraded NO-GO → CONDITIONAL-GO.
5. **Phase B step-1 (5752052, reverted)**: applied the recipe (consistent
   partition + iterated cross-LP settle). HALVES the plateau (39197→19391,
   direction confirmed) but REGRESSES tick 1 (930→10993) → implementation
   bug, not a convergence limit.

## Precise resume point (the one remaining bounded bug)
The iterated-settle tick-1 regression. Prime suspect (documented in
FUNCTIONAL-PARLOOP-SCOPE.md): edge-written boundary signals have
`signal_owner_lp[sid] == 0xFF` (the owner map is built only from comb
lp_entries' write targets, NOT edge NBA writers), so the boundary exchange's
`match owner { 0=>.., 1=>.., _=>skip }` silently DROPS the registered BIU
handshakes. Fix: a full owner map including edge writers; re-run the
multitick (recipe code is in the reverted diff of commit 5752052 / the
git history) and confirm per-tick mismatch → ~0. Then the production
`event_loop_perlp` integration (FUNCTIONAL-PARLOOP-SCOPE.md Phase B/C).

## Negative results (documented to save the next implementer's time)
- Balanced settle partition: 2.05× settle-compute but UNSOUND for real
  ticks (42835 cross-LP edges → 925 mismatches on a seeded tick). Retired.
- 4-way settle: unsound (within-core splits break the cut). Settle stays
  2-way; only edge exec scales past 2 threads.
- Naive lookahead-1 multi-tick: fast (2.9×) but wrong (39197 mismatch).
- Iterated settle (first cut): halves mismatch but tick-1 regression bug.

## Key docs
- FUNCTIONAL-PARLOOP-SCOPE.md — the integration scope + Phase A/A.2
  verdicts + revised Phase B recipe + the resume bug.
- PER-LP-NEXT-STEPS.md — per-unit build log + the metric insight.
