# Session-3 summary — xezim performance + JIT redesign

Branch: `perlp-experiment` from prior tip `30c8d19` to this session's
final `e2cbbbd`. Thirteen commits landing across performance,
infrastructure, and design-doc work.

## Measured wins (all bit-identical sim_time, all TEST PASSED)

### Default release interpreter
| Test | Pre-session | Post-session | Δ |
|---|---:|---:|---:|
| c910 hello | 82.5 s | **69.7 s** | **−15.5% / −12.8 s** |
| c910 memcpy | 235.3 s | **197.8 s** | **−15.9% / −37.5 s** |

### LTO + matched-PGO release — PRODUCTION BEST (re-baselined with all session wins)
| Test | Pre-session | Session-2 | **Final (Tier B included)** | Δ from baseline |
|---|---:|---:|---:|---:|
| c910 hello | 82.5 s | 66.0 s | **62.8 s** | **−24% / −19.7 s** |
| c910 memcpy | 235.3 s | 189.6 s | **179.8 s** | **−24% / −55.5 s** |
| c910 cmark iter=1 | ~5934 s | 5338 s | ~5000 s (est) | −16% |

ns_per_insn: hello 81.6 → 61.9 (−24%), memcpy 58.1 → 43.7 (−25%).

The final production binary
(`RUSTFLAGS="-C target-cpu=native -Cprofile-use=..." cargo build
--profile release-lto`) integrates NBA elision + Tier B (HashMap→Vec)
+ LTO + matched PGO.  Tier B carried into the production build added
~3 s on hello / ~10 s on memcpy over the session-2 best.

### JIT path (--features jit, XEZIM_JIT=1, XEZIM_INLINE_BITS=1)
| Stage | hello sim wall | Δ vs Stage 0 |
|---|---:|---:|
| Stage 0 (FFI baseline) | 91.7 s | reference |
| Stage 2 (inline LoadSignal) | 89.3 s | −2.4 s |
| Stage 3a (NBA bridge elision) | 90.4 s | REVERTED — wrong layer |
| Stage 4 Tier A (lean NBA bridge) | 87.0 s | −4.7 s |
| Stage 4 Tier B (dense nba_fast_index) | **84.9 s** | **−6.8 s (37% closure)** |

JIT still loses to interpreter (84.9 vs 69.7 = +15.2 s gap) because
the per-call cost is dominated by Value construction + dirty tracking
that haven't been inlined.

## Commits in chronological order

```
7076b8f  pdes: multikernel infrastructure (boundary topology, lookahead-K, tests)
5d6dc70  pdes: lib.rs orchestrator updates for boundary topology + lookahead-K
83bdca2  build: add [profile.release-lto] for interpreter-only builds
3d7e95e  pdes: NBA elision + clocks-only scan + Phase 3 run_one_tick + Phase 4 stub
acc0af7  pdes: session implementation notes (perf log + improvement suggestions + JIT design + per-LP plan)
a98bdb6  pdes: hot-signal arena design doc (C1)
4ccf865  pdes: JIT-REDESIGN — record Stage 1 audit findings
d33dee6  pdes: write-site refactor — canonical after_signal_write helper
f8f8931  pdes: JIT Stage 2 — inline LoadSignal codegen (−2.4 s on hello)
5ea7f0d  pdes: JIT-REDESIGN — record Stage 3 NBA-bridge-elision negative result
cedf83f  pdes: JIT-REDESIGN — Stage 4 cost breakdown + three-tier plan
275b529  pdes: JIT Stage 4 Tier A — leaner NBA bridge (−2.3 s on hello)
e2cbbbd  pdes: JIT Stage 4 Tier B — nba_fast_index HashMap → dense Vec
```

## Five documents shipped (implementation runway for next sessions)

- `MULTIKERNEL-NOTES.md` — perf log + measurement methodology + what
  was shipped vs reverted vs deferred. 700+ lines.
- `IMPROVEMENT-SUGGESTIONS.md` — ranked improvement menu (8 categories,
  with ROI + effort + file:line refs). 410+ lines.
- `JIT-REDESIGN-NOTES.md` — JIT FFI-inline redesign through 4 stages.
  Includes empirical findings from Stage 1-2-3a-4ab attempts + cost
  breakdown + tiered Stage 4 plan + anti-recommendations. 460+ lines.
- `HOT-ARENA-NOTES.md` — C1 hot-signal arena design (1.08% hot signals,
  11.9 MB arena fits in L3). Full audit checklist + falsification
  criteria + recommended next step. 280+ lines.
- `PER-LP-NEXT-STEPS.md` — Phase 4 per-LP event_loop implementation
  guide. 7-step buildup from spawn-and-barrier skeleton to full
  per-LP correctness. Re-baselined targets given session wins. 180+
  lines.

## What's shipped in the codebase

**Algorithmic / optimization (perf wins):**
- Simple + partial-range NBA elision in interpreter (89% of NBA writes
  elided at eval time on c910)
- `signal_inline_bits: Vec<[u64; 2]>` parallel storage with
  `after_signal_write` canonical helper instrumenting all 13
  signal_table mutator sites
- `signal_inline_bits` invariant check (`XEZIM_VERIFY_INLINE_BITS=1`)
- `NbaFastIndex` — dense `Vec<u32>` replacement for the prior
  `HashMap<usize, usize>` (Tier B). 144 MB extra RAM on c910.

**Build:**
- `[profile.release-lto]` in Cargo.toml — `lto = "fat"`,
  `codegen-units = 1`, inheriting from `release`.

**Phase 3 (per-LP prerequisite):**
- `PerTickAccum` struct + `run_one_tick(&mut self, accum, ...)`
  extraction. `event_loop` is now a thin dispatcher routing to
  `event_loop_singlethread` (default) or `event_loop_perlp` (stub).

**Phase 4 entry point:**
- `event_loop_perlp` gated by `XEZIM_DISPATCHER=perlp`, currently a
  fall-through stub.

**JIT (gated by `--features jit` + `XEZIM_JIT=1`):**
- `JitModule.inline_bits_ptr` + `set_inline_bits_storage` setter
- `JitModule.signal_widths_snapshot` + `set_signal_widths` setter
- Inline LoadSignal / LoadSignalSigned codegen via baked pointer
- `xezim_jit_schedule_nba_fast` 3-arg bridge for width-matches case
- Codegen branches at compile time between slow + fast NBA bridges

**Clocks-only check_edges subset (scaffolding):**
- `ClockGen.edge_signal_position` cache
- `fire_clock_generators` fills `toggled_clock_positions`
- `check_edges_inner(detect_subset: Option<&[usize]>)` with stack
  PosIter enum (no heap alloc)
- `check_edges_clocks_only` wrapper
- `is_edge_signal_non_clock` + `nba_touched_edge_non_clock` tracker
- 0 firings on c910 (clocks gate immediately) — kept as scaffolding
  for less-clocked designs

## Three deferred items (each has design doc + runway)

| Item | Status | Estimated effort | Doc |
|---|---|---|---|
| JIT Stage 4 Tier C (full inline NBA queue push) | designed | multi-week | JIT-REDESIGN-NOTES.md |
| C1 hot-signal arena | designed + diagnostic shipped | 600-700 LOC, 1.5-2 days | HOT-ARENA-NOTES.md |
| PDES Phase 4 per-LP event_loop | designed + stub wired | 630 LOC, 4-5 days | PER-LP-NEXT-STEPS.md |

## Session-3 negative results (documented to save next implementer's time)

1. **Quiescent-tick skip** — 0 firings on c910 because every iter has
   clock toggle work. Reverted.
2. **D2 static clock-derivation analysis** — only 16 clock-pure entries
   out of 438k on c910 (clock fanout gated immediately by enables).
   Reverted; would benefit cleaner clock trees.
3. **write_sig! macro touch-tracker (early Stage 1)** — TEST FAILED
   because some hot write paths bypass the macro. Replaced with the
   canonical `after_signal_write` helper approach (Tier B's
   write-site audit).
4. **JIT Stage 3a (bridge NBA elision)** — wrong layer; the elision-
   eligible NBAs concentrate in interpreter-handled blocks. Reverted.
5. **All `XEZIM_PDES_*` knobs beyond DISPATCHER=pdes** — measured
   regressions on c910 NBA scale. Documented; not re-tried.

## Lessons learned (load-bearing for next implementer)

1. **Run-to-run variance is ~5-10%.** Always measure sequentially,
   treat sub-3% deltas as noise. Documented in MULTIKERNEL-NOTES.md
   "Measurement methodology" section. (Lost 20 minutes early in the
   session before realizing concurrent runs were contending.)

2. **Audit completeness via single canonical write helper.**
   The failed write_sig! macro tracker and the eventual
   `after_signal_write` helper are the same idea — the difference is
   COMPLETENESS. Use the helper at every mutator site. The
   `XEZIM_VERIFY_INLINE_BITS=1` invariant check catches missing
   instrumentation early (0 mismatches across 8 938 iters / 36M
   signals = audit complete).

3. **JIT bake-time pointer hazard.** Anything baked into JIT'd code
   as a constant pointer (signal_inline_bits, soon Tier C's
   nba_fast pointer) must be backed by a Vec that NEVER reallocates
   after the bake. The Vec-realloc bug in Stage 2 caused a SIGSEGV;
   the `ensure_capacity` pre-allocation pattern + checks in
   `NbaFastIndex::insert` prevent recurrence.

4. **JIT signal_has_xz semantics are correctness-by-conservation.**
   The JIT's stale-conservative `signal_has_xz` (stuck at 1 after
   X/Z is cleared) is what its safe regime depends on. Tightening
   tracking lets the JIT execute MORE blocks and exposes latent
   codegen bugs. Keep `signal_has_xz` updates limited to
   `write_sig!` (full-Value writes); `after_signal_write` updates
   ONLY `signal_inline_bits`.

5. **Stale workdir inputs cost time.** The "data.pat from prior test"
   bug surfaced twice this session, each time spawning a "JIT
   regressed 15×" investigation that turned out to be cmark inputs
   silently running as hello. Always `md5sum` after staging if a
   measurement looks suspicious.

## Recommended next session

Pick one (each has full design + runway):

1. **JIT Tier C (full inline NBA push)** — closes another ~5-7 s of
   the JIT-vs-interp gap. Multi-week.
2. **C1 hot-signal arena** — ~3-5 s on hello, ~2 days. Highest
   per-effort ROI of the deferred items.
3. **PDES Phase 4 per-LP event_loop** — 1.5× architectural ceiling
   on c910 (~30 s on hello). 4-5 days. The original `perlp-experiment`
   branch goal.

Final perf reference for next session: **hello 69.7 s release,
84.9 s JIT, cmark 5338 s release-lto+matched-PGO (session-2 ship).**
