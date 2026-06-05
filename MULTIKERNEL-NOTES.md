# Multikernel / PDES improvement notes

Hand-off notes for the next engineer working on multikernel speedup in
`xezim-pdes` (branch `perlp-experiment`). Captures what's been shipped,
what was tried and abandoned, measured numbers from c910 hello/memcpy/cmark,
and concrete next-step ROI ranking.

Companion to:
- [PERLP-EVENTLOOP-PLAN.md](PERLP-EVENTLOOP-PLAN.md) — original 5-phase plan
- [SESSION-SUMMARY.md](SESSION-SUMMARY.md) — prior session's PDES architecture
  validation (sparse snapshot, boundary classifier, etc.)

Last branch tip: `30c8d19 pdes: Phase 2.3 + 2.4 — snapshot_for_tick + pdes_exec_block_local`.
Everything below this commit is uncommitted in the worktree.

---

## TL;DR — what works and what doesn't on c910

All measurements `--multikernel-scope`, sequential runs (no concurrent
sims — see Methodology section).  Treat sub-3% deltas as run-to-run
noise; the variance band on this machine is ~5–10%.

| Knob | hello sim wall | memcpy sim wall | Status |
|---|---:|---:|---|
| baseline `--simulate` (1T) | ~83.6 s | ~223.7 s | reference (older measurement) |
| baseline `--multikernel-scope` (std::scope arm) | **82.5 s** | **235.3 s** | clean re-measurement; was reported 86.8 / 236.1 earlier — variance |
| `XEZIM_DISPATCHER=pdes` | 90.8 s | 235.1 s | bit-identical to std::scope arm (parallel-block dispatch only) |
| `+ XEZIM_PDES_PAR_APPLY=1` | 98.4 s (+13%) | 264.0 s (+12%) | **REGRESSION** — NBA work too small to amortize spawn |
| `+ all PDES flags stacked` | 108.9 s (+25%) | 267.5 s (+13%) | larger regression |
| Simple NBA-elision-at-eval | ~77 s | ~233 s | early win, 35% of NBAs |
| **+ Partial-range NBA-elision (shipped)** | **74.1 s (−10%)** | **209.8 s (−11%)** | **big win** — 89% of NBAs elided; nba phase −94% / −97% |
| Quiescent-tick skip (snap+check_edges) | 87.9 s | 259.7 s | dead path on c910 (0 firings); reverted |
| Clocks-only check_edges subset | 87.0 s | 234.0 s | safe but 0 firings on c910; kept as scaffolding |
| `write_sig!` macro touch-tracker | **TEST FAILED** | — | unsafe: some hot write paths bypass write_sig!. **DO NOT REPEAT** |
| **JIT (Cranelift, `--features jit` + XEZIM_JIT=1)** | 91.7 s **(+11%)** | n/a | **REGRESSION** — 72% block coverage but FFI bridge per signal access exceeds dispatch savings on load-heavy c910 blocks |
| **JIT (LLVM, `--features jit-llvm` + XEZIM_JIT=1)** | 87.1 s **(+5%)** | n/a | smaller regression than Cranelift; same FFI bottleneck |
| **LTO + native CPU** (no PGO) | 80.3 s (−2.7%) | 222.9 s (−5.3%) | modest stand-alone |
| **PGO** (release build, profile gen against release) | 69.3 s | 195.8 s | stacks on partial-NBA: **−6.5% / −6.7% on top** |
| **LTO+PGO MISMATCHED** (LTO build using non-LTO profile) | 71.9 s | 210.9 s | regression — profile points at wrong inline graph |
| **LTO+PGO MATCHED** (profile gen against LTO build) | 66.0 s (−20.0%) | 189.6 s (−19.4%) | session-2 ship |
| **+ Phase 3 (run_one_tick extraction)** | **67.4 s (−18.3%)** | **184.0 s (−21.8%)** | session-3 final — Phase 3 cost-neutral, Phase 4 entry stub gated by `XEZIM_DISPATCHER=perlp` |

`cmark` iterations=1 with INIT_ZERO=1 + multikernel-scope + NBA-elision
(non-LTO): **PASSED, sim_time 2 007 365 ns, 1:35:26 elapsed, 1.39B NBAs elided.**
LTO cmark measurement pending.

---

## Architectural finding that determines everything

**c910 does not have idle iters.** Every event_loop iter has either a
clock toggle that wakes thousands of blocks, or a settle that propagates
the clock through comb fanout. Any "fast-path predicate" coarser than
per-individual-write tracking trips on settle running every iter.

Concretely on hello:
- 8 938 iters total
- 89 485 155 edges fired (~10 k per iter average)
- **2 234 iters fire zero edges** (the negedge iters of mostly posedge-sensitive flops)
- But all 2 234 zero-edge iters ALSO run apply_nba (queued from previous posedge) and
  settle (clock toggle dirtied the clk signal → settle propagates)

Implication: the **edge_detect cost on the 2 234 zero-edge iters is the
real opportunity (~2.8 s on hello / ~6.6 s on memcpy)** but capturing it
requires distinguishing "settle wrote a non-clock edge signal" from
"settle only wrote non-edge signals" — which means instrumenting every
direct `signal_table[id]` write. The naive `write_sig!` macro approach
fails because some hot paths (JIT bridge, settle eval) bypass the macro.

---

## What's shipped this session (uncommitted)

All in `src/compiler/simulator.rs`. Build is clean; tests in
`src/multikernel/tests.rs` all pass (20 tests).

### 1. NBA-elision-at-eval (simple `Insn::NbaAssign`)

**Lines: bytecode.rs:96 (`Insn::NbaAssign(usize, RegId, u32)` unchanged) +
simulator.rs:5794, simulator.rs:6210** (two interpreter paths).

Both `Insn::NbaAssign` evaluation sites now check
`signal_table[sig_id] != val` BEFORE pushing the `NbaFast` into the
queue. The check is identical to what `apply_nba_entry` was already
doing — we're moving it earlier.

```rust
Insn::NbaAssign(sig_id, val_reg, width) => {
    let val = vm_regs[*val_reg as usize].resize_for_assign(*width);
    if signal_table[*sig_id] != val {  // <-- elision
        nba_out.push(NbaFast { signal_id: *sig_id, value: val, block_index });
    }
}
```

**Correctness rationale.** `apply_nba_entry` already had this check at
apply time. Moving it to eval time is a pure no-op for correctness —
the same writes get applied, the same writes get dropped. The win is
all in queue overhead (push, traversal, apply scan iteration).

**Counter:** `prof_nba_elided: u64` (simulator.rs near other prof
fields), printed via `[PROF] clocks_only_detect=N nba_elided=M`.

**Measured fraction elided (simple only):** ~34–35% of NBA traffic on
all three c910 workloads. The full picture (after #1.5) is much bigger.

### 1.5. Partial-range NBA-elision (this session — the BIG win)

**Lines: simulator.rs:5818, :5841, :5905 (parallel-safe path —
`exec_insns_isolated`); simulator.rs:6245, :6299, :6368, :6670, :6698
(main interpreter — Insn::NbaAssignRange, NbaAssignRangeDyn,
NbaAssignBitDyn, NbaAssignArray, NbaAssignArrayRange).**

Same idea as #1, extended to partial-bit / partial-range / array NBA
variants. For each, after computing the merged final value (range bits
overlaid on either an existing queue entry OR signal_table[id]), elide
the queue push if and only if there is NO existing queue entry AND
the merged value equals signal_table[id]:

```rust
Insn::NbaAssignRange(sig_id, hi, lo, val_reg) => {
    // ... (compute merged new_val) ...
    if has_existing_queue_entry {
        nba_out[i].value = new_val;          // unchanged behavior
    } else if new_val != signal_table[*sig_id] {
        nba_out.push(NbaFast { ... });       // real change → push
    } else {
        prof_nba_elided += 1;                // no-op → drop
    }
}
```

For the inline-bits fast path (signal_widths[id] ≤ 64 && !signal_real),
the comparison is `(new_v, new_x) == (base_v, base_x)` against the
raw bits returned by `compose_inline_range_bits`. Zero-allocation
elision in the hot path.

**Why only-when-no-existing-entry:** when there IS an existing entry
in nba_fast, it was pushed because a previous NBA in the same block
made a real change (otherwise we'd have elided it). The update may or
may not return to no-op state but apply_nba's existing slot check
catches that — we don't need to scan the queue to remove it.

**Measured fraction elided (with partial-range):** ~89% of NBA traffic
on hello (84.3M of ~95M NBA writes). Memcpy: 192M elided. cmark
projected proportional.

**Per-phase impact on hello (cumulative with simple #1):**

| phase | baseline | After NBA-elision (simple + partial) | Δ |
|---|---:|---:|---:|
| nba | 4.4 s | **0.138 s** | **−94%** |
| edges | 60.8 s | 53.6 s | −12% |
| └ edge_exec | 49.4 s | 42.4 s | −14% |
| settle | 17.2 s | 16.0 s | −7% |
| **total sim** | **82.5 s** | **74.1 s** | **−10%** |

Memcpy nba dropped 10 s → 0.34 s (−97%). Same compound-downstream
effect as #1 but bigger absolute: less dirty propagation → less settle
→ less edge_exec. Both wins compound.

**Estimate was way off.** The IMPROVEMENT-SUGGESTIONS.md guess was
"−1 to −3 s." Actual was −8 s on hello, −25 s on memcpy. The partial-
range variants (especially `NbaAssignBitDyn` for individual flop bit
writes) turned out to dominate c910's NBA traffic. The simple
`Insn::NbaAssign` is a minority of writes on this design.

### 2. Clocks-only `check_edges` subset (scaffold, 0 firings on c910)

**Lines: simulator.rs:673** (ClockGen + edge_signal_position),
**simulator.rs:5147** (fire_clock_generators fills toggled_clock_positions),
**simulator.rs:9867** (check_edges_inner + PosIter stack enum),
**simulator.rs:9882** (check_edges_clocks_only wrapper).

Refactored `check_edges` into `check_edges_inner(detect_subset:
Option<&[usize]>)`. Full-scan path iterates `0..edge_signal_ids.len()`;
subset path iterates a slice of positions. **Uses a stack-allocated
`PosIter` enum — NOT `Box<dyn Iterator>`** (the first attempt with Box
regressed sim wall by 9% due to per-iter heap alloc; the enum is
zero-cost).

```rust
enum PosIter<'a> {
    Full(std::ops::Range<usize>),
    Subset(std::slice::Iter<'a, usize>),
}
```

`fire_clock_generators` now also fills a reusable scratch
`toggled_clock_positions: Vec<usize>` with each toggled clock's position
inside `edge_signal_ids`. Position is cached in
`ClockGen.edge_signal_position` once at end of elaboration.

**Dispatch in event_loop:** `if !non_clock_change &&
!toggled_clock_positions.is_empty() { check_edges_clocks_only() } else
{ check_edges() }`.

**Why it doesn't fire on c910:** `non_clock_change` is set true whenever
`apply_delayed_updates`, batch !is_empty (processes), apply_nba (with
real edge write — see #3), OR `settle` runs. On c910, settle runs every
iter because the clock toggle dirties the clock signal → conservative
true → fast path never fires.

**Counter:** `prof_clocks_only_detect: u64`, printed in the same line as
`nba_elided`.

**Cost when not firing: zero.** Verified by re-running hello: 86.5 s
(vs 86.8 s baseline) — within run-to-run noise. The Range vs
slice::Iter abstraction compiles to identical asm to the original
range loop when LLVM sees the enum variant.

### 3. NBA-only touch tracker — `is_edge_signal_non_clock` + `nba_touched_edge_non_clock`

**Lines: simulator.rs:~1235** (struct field defs),
**simulator.rs:~4510** (is_edge_signal_non_clock built from
edge_signal_ids minus clock signals, after sort+dedup),
**simulator.rs:9675** (apply_nba_entry sets flag on real edge write),
**simulator.rs:~9628** (parallel apply path sets flag in dirty merge).

Tracks whether `apply_nba` actually moved a non-clock edge signal this
iter. Used by the event_loop predicate above. Correctly conservative —
doesn't fire on c910 because settle still flips `non_clock_change` true.

**DO NOT** attempt to extend this tracker into `write_sig!` macro. See
the "What was tried and failed" section.

### 4. EventLoopState + run_one_tick extraction (from prior session)

Status from PERLP-EVENTLOOP-PLAN.md Phase 3: complete but uncommitted.
`run_one_tick(&mut self, state)` at simulator.rs:~8123 and
`event_loop_report()` at ~8232. `event_loop` is now a thin wrapper that
constructs `EventLoopState` and loops `run_one_tick`. Bit-identical
sim_time preserved.

This is the architectural prep for the per-LP event_loop runner that
the original plan targets — see "TODO" section.

### 5. LTO + native CPU release profile

**Cargo.toml: new `[profile.release-lto]`**, inheriting from release
with `lto = "fat"` and `codegen-units = 1`. The existing
`[profile.release]` is left as-is to preserve the JIT-LLVM safety
note (LTO miscompiles inkwell/llvm-sys).

Build invocation:
```bash
RUSTFLAGS="-C target-cpu=native" cargo build --profile release-lto --bin xezim
# Binary lands at target/release-lto/xezim
```

**Measured impact (sequential, clean):**
| test | release | release-lto + native | Δ |
|---|---:|---:|---:|
| hello | 82.5 s | 80.3 s | −2.7% |
| memcpy | 235.3 s | 222.9 s | −5.3% |

ns_per_insn drops 81.6 → 78.9 on hello, 58.1 → 54.9 on memcpy. Cross-
crate inlining of `Value` arithmetic and signal_table access produces
tighter codegen; `target-cpu=native` enables AVX2/BMI2 for Value math.

Bigger wins on longer workloads (memcpy > hello) because more time
spent in the inlined hot path. cmark LTO measurement pending.

No correctness risk — just a build-flag change. The two profiles
coexist; the existing `--features jit-llvm` path stays safe.

### 5.5. Phase 3 — `run_one_tick` extraction (shipped)

**Lines: simulator.rs:~683 (`PerTickAccum` struct), :~8534
(`run_one_tick` method), :~8653 (`event_loop` dispatcher), :~8678
(`event_loop_singlethread`), :~8631 (`event_loop_perlp` stub).**

Pure refactor that prepares for Phase 4 per-LP threading. The
event_loop body that previously ran inline is now a callable
`run_one_tick(&mut self, accum, cascade_limit, iters, trace_loop)`
method. `event_loop` dispatches to either `event_loop_singlethread`
(default — runs `run_one_tick` in a while loop) or `event_loop_perlp`
(stub gated by `XEZIM_DISPATCHER=perlp`, currently falls through to
single-thread with a notice; Phase 4 implementation pending).

**Critical detail:** `#[inline(always)]` on `run_one_tick` is
**mandatory**. Without it LLVM's auto-inline heuristic refuses the
100-line body, function-call overhead becomes per-tick × 8938 ticks,
AND inner-call inlining decisions for `snapshot_edge_signals` /
`apply_nba` / `settle_combinatorial` change because the optimizer no
longer sees them as part of a single hot function. A debugging
incident this session attributed a "15× slowdown" to this — turned
out to be unrelated (stale cmark pats in workdir) but the
`#[inline(always)]` hint is still required for the refactor to be
performance-neutral.

**Validation:** hello 67.4 s with Phase 3 + LTO + matched-PGO vs
66.0 s without Phase 3 — within run-to-run noise. Memcpy 184.0 s vs
189.6 s — slightly faster (also noise). Bit-identical sim_time.

### 6. PGO + LTO matched-profile build (best combo this session)

**No code changes — pure toolchain. Stacks on #1.5 + #5.**

PGO instrumentation is a Rust compiler feature; `llvm-profdata` ships
with the rustc toolchain. Two builds required: instrumented (profile
generate) then optimized (profile use). **The profile must be
generated against the same compiler flags as the final use build** —
including LTO — or the inlining graph differs and the profile points
at the wrong functions (measured: −7% regression on hello when the
profile was mis-targeted, see TL;DR table).

**Steps:**
```bash
PROFDATA="/home/bondan/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/x86_64-unknown-linux-gnu/bin/llvm-profdata"

# Stage 1: build instrumented LTO binary
rm -rf /tmp/pgo-data && mkdir -p /tmp/pgo-data
RUSTFLAGS="-C target-cpu=native -Cprofile-generate=/tmp/pgo-data" \
    cargo build --profile release-lto --bin xezim

# Stage 2: collect profile (hello takes ~100 s instrumented vs ~74 s normal)
LLVM_PROFILE_FILE=/tmp/pgo-data/xezim-%p-%m.profraw \
    ./target/release-lto/xezim --simulate ...   # hello workload

# Stage 3: merge raw profiles
"$PROFDATA" merge -o /tmp/pgo-data/merged.profdata /tmp/pgo-data/*.profraw

# Stage 4: build final optimized LTO+PGO binary
RUSTFLAGS="-C target-cpu=native -Cprofile-use=/tmp/pgo-data/merged.profdata" \
    cargo build --profile release-lto --bin xezim
# Final binary at target/release-lto/xezim — use this for production runs
```

**Measured impact (matched profile, sequential clean runs):**

| stage | hello | memcpy |
|---|---:|---:|
| baseline (release) | 82.5 s | 235.3 s |
| + simple+partial NBA-elide | 74.1 s | 209.8 s |
| + PGO (release) | 69.3 s | 195.8 s |
| + LTO (matched profile) | **66.0 s** | **189.6 s** |
| **cumulative Δ** | **−20% / −16.5 s** | **−19% / −45.7 s** |

ns_per_insn 81.6 → 64.8 on hello (−21%), 58.1 → 46.0 on memcpy.

**Gotcha (avoided here, would have cost time):** profile-use builds
require the SAME RUSTFLAGS / profile setting as the profile-generate
binary. Mixing release-generated profile with release-lto build cost
hello +3 s and memcpy +15 s in one accidental experiment — visible as
`pgo-warn-missing-function` warnings (388 of them) at link time. If
you see those warnings, your profile is mis-targeted.

**Recommended use:** when running long sims, ALWAYS use the
LTO+matched-PGO binary. For ad-hoc dev runs the plain release binary
is fine. The matched-PGO build takes 4–5 min (vs 30 s for plain
release).

---

## What was tried and failed (do not repeat)

### A. `write_sig!` macro touch-tracker (TEST FAILED)

Attempted to instrument the `write_sig!` macro itself to set
`nba_touched_edge_non_clock` on every write to a non-clock edge signal.
Plan was: capture ALL writes via the macro, then settle's writes to
non-edge signals wouldn't disqualify the fast path.

**Result: TEST FAILED on hello.** sim_time drifted to 500 195 ns (vs
correct 44 695 ns) and `clocks_only_detect = 100 037` (fired aggressively
but incorrectly). The testbench's 50 000-cycle no-instructions-retired
watchdog fired.

**Root cause: several hot write paths bypass `write_sig!` and write
directly to `signal_table[id]`.** Suspected: the JIT bridge functions
(see `compiler/jit.rs`), parallel apply_nba (already instrumented
separately), settle eval expressions, scheduled-process writes. The
macro is NOT the single point of write — anyone planning a tracker via
the macro must FIRST audit every direct-`signal_table[id] = val`
write in the codebase.

**Reverted.** Lesson: if you can't enumerate every direct
`signal_table[id]` write site, don't build a fast-path predicate that
assumes the macro catches them all.

### B. Quiescent-tick fast-forward (0 firings on c910)

Tracked `prev_iter_quiescent: bool` — true when the previous iter's
check_edges fired 0 blocks AND no NBA/settle/process ran. Skipped snap
+ check_edges on subsequent quiescent iters.

**Result: PASSED, but 0 skips on c910.** Hello has 25% zero-edge iters
but they're interleaved with high-edge posedge iters. `prev_iter_quiescent`
flips off every other iter. Also tried treating clock toggle as
"non-work" — still 0 because settle runs every iter.

**Removed.** The code path was dead weight; the cleaner replacement is
#2 (clocks-only subset) which keeps the same plumbing but is bounded.

### C. Snap-skip when prev iter quiescent (UNSAFE)

Skipping `snapshot_edge_signals` when previous iter was quiescent. Looks
safe but if the clock toggled in either iter, `prev_val[clk]` goes stale
and the NEXT check_edges may miss or invent an edge.

**Removed.** Snap must run every iter to track each clock toggle.

### D. Cranelift + LLVM JIT (`--features jit` / `--features jit-llvm`) (REGRESSION)

Built xezim with both `--features jit` (Cranelift) and `--features jit-llvm`
(LLVM 18 via inkwell). Both backends activate at runtime via
`XEZIM_JIT=1` (also honors `XEZIM_JIT_BACKEND={cranelift,llvm}`).

**Coverage:** 15 040 / 20 779 edge blocks JIT'd on c910 hello (72%).
The uncovered 28% are blocks containing Insns not yet implemented in
the JIT Phase 1 MVP (per the phase plan in `src/compiler/jit.rs:30+`)
or signals >64 bits (block_signals_fit_u64 gate).

**Result on c910 hello, multikernel-scope:**
| Backend | sim wall | ns_per_insn | JIT compile time |
|---|---:|---:|---:|
| no JIT (baseline) | 82.5 s | 81.6 | — |
| Cranelift JIT (XEZIM_JIT=1) | **91.7 s (+11%)** | 97.5 | 5.5 s |
| LLVM JIT (XEZIM_JIT=1 XEZIM_JIT_BACKEND=llvm) | **87.1 s (+5%)** | 89.5 | 44.3 s |

**Root cause: FFI bridge per signal access.** The JIT design (see
`src/compiler/jit.rs` header) uses FFI calls into Rust for every
signal load/store (`jit_load_signal`, `jit_store_signal`, etc.) —
`~10-20 ns FFI overhead per call`, designed assuming many arithmetic
ops between loads/stores save more than the FFI cost. c910 RTL
blocks are the opposite: many signal accesses with few arithmetic
ops between them (typical RTL pattern: read 3 signals, compute,
write 1 signal). Every access pays FFI cost the interpreter
doesn't need.

LLVM produces ~5% tighter code than Cranelift but pays 8× more in
JIT-compile time (44 s vs 5.5 s) — still a net loss.

**Verdict: DO NOT enable JIT on c910 with the current design.** Fix
would require inlining signal_table access into JIT-generated code
(no FFI per access), which means careful interior-mutability
discipline crossing the JIT boundary. Multi-week project. The
Phase 1 MVP likely stalled here for the same reason — whoever
wrote it probably hit the same wall on a real workload.

**Sim_time was bit-identical TEST PASSED in both backends.** The
JIT is correctly implemented; it's just architecturally wrong for
c910 patterns. Keep the feature gated and OFF by default.

### E. PDES dispatcher arm flags stacking (REGRESSION)

`XEZIM_PDES_PAR_APPLY=1`, `XEZIM_PDES_POOL=1`, `XEZIM_PDES_SCAN_MERGE=1`,
`XEZIM_PDES_BUCKET_NBA=1`, `XEZIM_PDES_CHUNK_TARGET=3000`,
`XEZIM_PDES_SUBCHUNK=4`. All knobs ON:
- hello: 86.8 s → 108.9 s (+25%)
- memcpy: 236.1 s → 267.5 s (+13%)

PAR_APPLY in particular is a c910-killer: NBA work is only 4–10 s on
c910 (after NBA-elision now even smaller) — std::thread::scope spawn
overhead per tick (~100 µs × thousands of ticks) exceeds the actual NBA
parallel speedup. Documented in PROF: nba=4.4s → 15.5–18s with PAR_APPLY
on hello.

**Recommendation: ship with default PDES knobs OFF on c910-scale
designs.** They were designed for a different scale assumption.

---

## Where the time goes after NBA-elision

c910 hello, NBA-elision shipped, sim wall 77.3 s:

```
edges      54.9 s  (71%)    ← still the dominant phase
└ edge_detect   10.8 s
└ edge_exec     44.1 s    ← #1 target
settle     16.0 s  (21%)
nba         2.2 s   (3%)   ← halved by elision
snap        3.0 s   (4%)
process     1.2 s   (2%)
```

`ns_per_insn=77` on hello (was 86 pre-elide; the drop is because elided
NBAs no longer count in the insn cost but DO count in queueing time
saved). `ns_per_insn=32.5` on cmark — when each tick wakes more state,
the bytecode dispatch loop amortizes better.

**Top remaining levers by ROI (from earlier analysis):**

| Idea | Hello est | Effort | Notes |
|---|---:|---|---|
| Cranelift JIT (already `--features jit`) | −25 to −35 s | medium | Already in tree at `src/compiler/jit.rs`. Build with `--features jit`, validate, measure. ns_per_insn 77 → ~25. **Highest single ROI.** Risk: feature has been out of CI; may have bit-rotted or have uncovered insns falling back to slow path. Coverage assertions needed. |
| Persistent compile cache | 0 sim, −29 s compile | medium-hard | Saves the 29 s bytecode-compilation phase on every repeat run. Requires serde on `Insn` enum which holds `Arc<Statement>`. `xezim-core` already has `write_compiled`/`read_compiled` for elaboration; extend to bytecode. **Best dev-loop ergonomics.** |
| Quiescent-skip extension via clock-derivation static analysis | −2 to −3 s | hard | Detect clock-fanout comb outputs at compile time; treat them as "clock-equivalent" so settle writes to them don't disqualify the fast path. Captures the 2.8 s on hello / 6.6 s on memcpy that the current clocks-only scaffold sees but can't claim. |
| Per-LP event_loop (PDES Phase 4-5) | up to −30 s | very hard | The architectural ceiling fix. ~3 050 LOC per SESSION-SUMMARY.md. Per-LP threads each run `run_one_tick` on local `EventLoopState`, synchronize every K ticks via `ClockBarrier`. Phase 3 extraction is done; Phase 4 needs wiring. |
| Partial-range NBA-elision | −1 to −3 s | low | Extend the NBA-elide check to `NbaAssignRange`/`NbaAssignBitDyn`/etc. Each writes a merged Value; compare final merged value against signal_table[id] before queueing. |
| Snap thinning (Vec replaces prev_wide HashMap) | −1 s | low | snap=3 s on hello; replacing the wide-signal HashMap with a Vec indexed by edge_signal_ids position cuts wide-signal snap roughly in half. |
| Dead-block elision via edge_block_exec_counts | unsafe single-shot | medium | edge_block_exec_counts is already collected. **DO NOT** prune blocks based on runtime counts within a single run — error-handling paths might not fire until late. Only useful with cross-run profile persistence (= compile cache). |

---

## Concrete next-step instructions for the next implementer

### Immediate: ship what's working

1. **Commit the EventLoopState + run_one_tick extraction (Phase 3)** — it's
   complete, bit-identical, and unlocks reviewable Phase 4 diffs.
2. **Commit the NBA-elision-at-eval** — the only measured win this session.
   Two-line change in two interpreter call sites + one counter field +
   one PROF line. Low review burden, high impact.
3. **Commit the clocks-only scaffolding** — even though it doesn't fire on
   c910, it's correctness-preserving zero-cost and prep for future static
   clock-derivation work.

Suggested commit order:

```
pdes: Phase 3 — EventLoopState + run_one_tick extraction
pdes: NBA-elision at eval — drop no-op writes before queue push (−11% hello, −1% memcpy)
pdes: clocks-only check_edges subset scaffolding (zero-cost; 0 firings on c910 — see notes)
```

### Next session: pick ONE

Recommended ranking:

1. **Try `--features jit` first.** Build with the existing Cranelift JIT
   feature, run hello + memcpy. If it works cleanly, this is the single
   biggest measurable win available (potentially halving sim wall).

   ```bash
   cargo build --release --features jit --bin xezim
   cd simtest/xuantie_c910/work && /usr/bin/time -v ../../../target/release/xezim ...
   ```

   Look for fallback rate in PROF. High fallback = JIT coverage gap. Patch
   the gaps incrementally.

2. **Partial-range NBA-elision** if JIT bit-rotted. Bounded ~30 LOC; capture
   the remaining 1–3% on the NBA path. Same pattern as the simple
   NbaAssign elision; only the merge case is more involved.

3. **Per-LP event_loop wiring** if both above land cleanly and you're
   ready for a multi-session integration. PERLP-EVENTLOOP-PLAN.md has the
   detailed file-by-file plan. The infrastructure (sparse snapshot,
   boundary classifier, BoundaryChannel, ClockBarrier, EventLoopState
   extraction, lookahead-K helpers) is all in place — Phase 4 just needs
   to wire per-LP threads through `run_one_tick`.

### Things to NOT do

- Don't instrument `write_sig!` for fast-path tracking. Many writes bypass it.
- Don't enable any `XEZIM_PDES_*` flag besides `DISPATCHER=pdes` by
  default — they all regress c910 wall.
- Don't prune compiled edge blocks based on within-run exec_counts. Use
  cross-run profile data only (= persistent compile cache).
- Don't try snap-skip optimizations — every clock toggle requires a snap.
- Don't reach for the per-LP event_loop before validating the JIT
  baseline. The Amdahl ceiling for 2-LP PDES depends on the
  single-threaded baseline; halving that baseline via JIT effectively
  doubles the PDES win required to be worth the complexity.

---

## Key file/line references

| What | Path | Line |
|---|---|---|
| `Insn::NbaAssign` definition | `src/compiler/bytecode.rs` | 96 |
| NBA-elide in main interpreter | `src/compiler/simulator.rs` | 6210 |
| NBA-elide in `exec_insns_isolated` (parallel-safe) | `src/compiler/simulator.rs` | 5794 |
| `prof_nba_elided` counter | `src/compiler/simulator.rs` | ~1250 (struct), printed at PROF block |
| `ClockGen` struct (incl. `edge_signal_position`) | `src/compiler/simulator.rs` | 673 |
| `fire_clock_generators` + `toggled_clock_positions` fill | `src/compiler/simulator.rs` | 5147 |
| `check_edges` (thin wrapper) | `src/compiler/simulator.rs` | 9867 |
| `check_edges_clocks_only` (fast-path wrapper) | `src/compiler/simulator.rs` | 9876 |
| `check_edges_inner` + `PosIter` enum | `src/compiler/simulator.rs` | 9890 |
| `is_edge_signal_non_clock` build site | `src/compiler/simulator.rs` | ~4510 (after sort+dedup of edge_signal_ids) |
| `nba_touched_edge_non_clock` + `apply_nba_entry` instrumentation | `src/compiler/simulator.rs` | 9675 |
| Parallel apply_nba path instrumentation | `src/compiler/simulator.rs` | ~9628 (dirty merge loop) |
| Event_loop dispatch (clocks-only vs full) | `src/compiler/simulator.rs` | ~8480 |
| PROF line | `src/compiler/simulator.rs` | ~8530 (`[PROF] clocks_only_detect=N nba_elided=M`) |
| Cranelift JIT (feature-gated) | `src/compiler/jit.rs` | top of file |
| EventLoopState struct (Phase 3) | `src/compiler/simulator.rs` | ~277 |
| `run_one_tick` (Phase 3) | `src/compiler/simulator.rs` | ~8123 |
| `event_loop_report` | `src/compiler/simulator.rs` | ~8232 |
| Lookahead-K helpers (Phase 5, unused) | `src/multikernel.rs` | 155–207 |
| BoundaryChannelTopology + build (Phase 4) | `src/multikernel.rs` | 230–290 |

---

## Measurement methodology

Lesson learned the hard way this session: **run-to-run variance on
the same build is ~5–10%.** This is bigger than several of the
optimizations we're investigating.

Rules to avoid the trap I fell into:

1. **Always run sequentially.** Two concurrent sims on the same
   machine contend for all cores and degrade *both* by 10–20%.
   This invalidates any A/B comparison. (I lost ~20 minutes
   measuring a memcpy LTO run that was actually contending with a
   "baseline" run launched concurrently — both came back at ~95 s
   on hello-equivalent workloads vs the true ~82 s.)
2. **Treat sub-3% deltas as noise.** Three runs minimum if a
   sub-5% win matters.
3. **Re-measure both variants in the same session.** Numbers from
   different days/sessions can drift due to background load,
   thermal state, or other variance sources. The "77.3 s hello"
   that this notes file originally referenced was a low-outlier
   run; the actual clean baseline range on this branch is 80–87 s.
4. **`/usr/bin/time -v ...` reports wall-clock elapsed.** Use that
   number for comparisons, not the `[PHASE] simulation` PROF line
   (which excludes setup/teardown). Both numbers should match
   within ~1 s; if they don't, something is wrong.

The baseline numbers in this notes file have been re-measured
sequentially (most recent: hello 82.5 s, memcpy 235.3 s on the
release build) but the older outliers may still be referenced
contextually (e.g. "77.3 s" appears in NBA-elision discussion
because that's the run that motivated the change).

## Validation criteria for any further change

Per SESSION-SUMMARY.md, preserve:

1. **Bit-for-bit sim_time match** with baseline:
   - hello: 44 695 ns
   - memcpy: 101 965 ns
   - cmark iter=1: 2 007 365 ns
2. **TEST PASSED** in testbench output
3. **All 20 PDES unit tests pass** (`cargo test --release --lib multikernel::`)
4. **No regression beyond noise** on c910 hello/memcpy walls vs
   recent baselines (hello ~82 s, memcpy ~235 s — see TL;DR table
   for exact numbers).

Run commands (always sequential — see Methodology):

```bash
# Recommended build: LTO + native CPU (shipped this session)
RUSTFLAGS="-C target-cpu=native" cargo build --profile release-lto --bin xezim
XEZIM=./target/release-lto/xezim
# (or fall back to ./target/release/xezim for the plain build)

# hello, multikernel-scope
cp /home/bondan/repo/rtlmeter/designs/XuanTie-C910/tests/hello/{inst,data}.pat \
   simtest/xuantie_c910/work/
cd simtest/xuantie_c910/work
/usr/bin/time -v $XEZIM --simulate --max-time 80000000 \
   --multikernel-scope x_soc.x_cpu_sub_system_axi.x_rv_integration_platform.x_cpu_top.x_ct_top_0 \
   -s tb -I . -I /home/bondan/repo/rtlmeter/designs/XuanTie-C910/src \
   -f c910.fl &> c910_hello_<variant>.log

# memcpy: same command with memcpy/{inst,data}.pat staged
# cmark: same command + INIT_ZERO=1 env + +iterations=1 + --max-time 200000000
```

Check PROF lines for:
- `nba_elided=N` — should be ~34% of total NBA writes (consistent across workloads)
- `clocks_only_detect=N` — currently 0 on c910; non-zero means a future change
  enabled the fast path
- `par_dispatch partition=X pdes=Y` — verify the multikernel-scope partition
  applied (LP-A=7973 blocks, LP-B=12806 blocks on c910)
- `quiescent_skips` / `zero_edge_iters` — these counters were temporary
  diagnostics that have been removed; if you see them, you're on an old
  build
