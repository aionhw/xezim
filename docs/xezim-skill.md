---
name: xezim
description: Run, debug, and benchmark SystemVerilog simulations with the xezim simulator — invocation patterns, the flags and env vars that matter, waveform dumps, UVM, performance guidance, and the debugging workflows that actually localize a failure. Use when compiling or simulating SV/UVM designs with xezim, comparing its results against another simulator, or triaging a hang, X-storm, or wrong value.
---

# Using xezim

xezim is a SystemVerilog (IEEE 1800-2017/2023) simulator: parser, elaborator,
bytecode interpreter with compiled fast paths, plus UVM, DPI/VPI, and early
Verilog-AMS (`wreal`) support. This guide is for *using* it; `AGENTS.md`
covers developing it. Drop this file into `.claude/skills/xezim/SKILL.md` to
reuse it as a Claude Code skill.

## Build

```sh
cargo build --release --features jit     # the normal build (jit feature ≠ jit on)
./target/release/xezim -V                # version + git hash/date + release tag
```

Rust ≥ 1.92. The `xezim-core` dependency is pinned by exact rev and fetched
automatically; for local co-development of both repos run
`./scripts/use-local-core.sh` once. A maximum-performance binary comes from
profile-guided optimization: `./scripts/build-pgo.sh <training-command>`
(measured −15..18% wall on large SoCs; train on a workload shaped like the
one you care about). Do **not** stack BOLT on a PGO build — measured net
negative; PGO alone wins.

## Running a simulation

```sh
xezim --simulate -s top design.sv tb.sv                  # explicit top
xezim --simulate -f files.fl -I include_dir -D NAME=1    # filelist (recursive, may contain options)
xezim --simulate --sv2017 --max-time 80000000ns -l run.log ...
xezim --simulate top.sv +iterations=2 +seed=1            # plusargs; +seed=random prints the seed
```

Facts worth knowing before the first run:

- **Default mode is `--simulate`**; `--parse` and `--compile` stop earlier
  and are the fast way to check a design front-end only.
- **`--max-time` default is 100000 ns** — long benches silently stop there;
  always set it explicitly. Bare numbers are ns.
- **Runs are deterministic**: same inputs + same `+seed` ⇒ byte-identical
  output. `+seed=1` is the default. Use two runs + `diff` as a free sanity
  check after any config change.
- **Timescale**: modules with no `` `timescale ``/`timeunit` get the tool
  default, `1ns/1ns` for delays and `$realtime` alike, matching the reference
  simulator; the public AVIP benches run on it with no flag (uart alone needs
  `--module-timescale 1ps/1ps`). When a testbench assumes a different
  default, pass `--timescale 1ns/1ps` (a default, never an override)
  or the named `--module-timescale mod=unit/prec` form; `--dump-timescales`
  prints every module's resolution.
- **Exit codes**: parse errors, elaboration errors, `$fatal`, and a `-s`
  naming a nonexistent top all exit nonzero; add `--error-exit` to make any
  `$error` fail the run too — essential in scripts and CI. A generated
  corpus with known-stale top names can restore auto-detection with
  `--no-strict-top`.
- **Reserved-word caveat**: `cell` and `wreal` are keywords in this lexer
  even where IEEE 1800 would allow them as identifiers.
- `-l/--log file` redirects *everything*, including DPI/VPI C-side prints.

## UVM

See `docs/uvm-guide.md` for depth. The invocation that runs the public AVIP
benches unchanged:

```sh
xezim --sv2017 -D UVM_NO_DPI -D UVM_REPORT_DISABLE_FILE_LINE \
  +incdir+$UVM_SRC $UVM_SRC/uvm_pkg.sv -f compile.f +UVM_TESTNAME=my_test
```

`+seed=<n>` reproduces a random test exactly; `+seed=random` explores and
prints the seed for replay.

## Waveforms and traces

```sh
xezim --simulate --fst wave.fst --fst-scope tb.dut ...        # GTKWave FST
xezim --simulate --xtrace t.xt --xtrace-scope tb.dut \
      --xtrace-from 10000 --xtrace-to 20000 ...               # windowed text dump
```

Dumping is OPT-IN: pass `--wave` to compile the model with waveform support.
Without it the `$dump*` tasks warn once and are ignored (the run still
simulates). `--fst`/`--xtrace` imply `--wave`.

`$dumpfile/$dumpvars` VCD works too. For big designs, always scope and
window the dump — the `--xtrace-from/to` window around a known-bad time is
how divergences get diffed against another simulator's VCD.

## Performance

The defaults are the fast path — recent optimization work ships enabled.
What's left for the user:

- **`XEZIM_PACKED_MEM=1`** — packed storage for large RAM arrays; ~3× RSS
  reduction on RAM-heavy SoCs, wall-neutral. Use it whenever a design
  carries megabyte memories. (Known gap: NBAs into packed arrays commit
  immediately; designs whose RAM read ports read back the same element in
  the same timestep would see the new value a delta early.)
- **`XEZIM_BUF_COLLAPSE=1`** — folds whole-net identity continuous assigns
  (`assign y = x;`) onto their source net, the transform commercial
  optimizers apply by default to clock and buffer trees. **The single
  largest opt-in win available**: measured on wall-clock, c906 memcpy ×100
  50.2 s → 42.7 s (−14.9%) and ibex CoreMark 48.2 s → 43.0 s (−10.8%);
  combined with `XEZIM_EDGE_MERGE=8`, c906 reaches 40.6 s (−19.1%). It also
  cuts combinational entries (c906 35,267 → 29,728; ibex 1,553 → 1,130).
  Both designs stay bit-exact against the reference.

  It stays OPT-IN for a measured reason, not caution. Forcing it on turns 7
  suite tests red, and they name exactly what it trades: `force`/`release`
  on a collapsed name now reaches the shared net (4 tests), a continuous
  assign's Z pass-through stops being distinguishable from a `buf` gate,
  a parked waiter sees the post-collapse value because the buffer's delta
  step is gone, and a VPI/DPI backdoor can no longer find the folded name.
  Those are the same properties that make it fast, so it cannot be both.
  Reach for it on gate-level or clock-tree-heavy designs whose testbench
  does not force, probe, or delta-observe buffer nets. Skipped automatically
  under SDF, where a collapsed net would lose its annotated delay.
- **`XEZIM_EDGE_MERGE=<N>`** — merges edge blocks with identical
  sensitivities into one compiled block, at most `N` per block (8 measures
  best; large values lose more to coarser gating than they save in
  dispatch). −4% instructions on a gate-level SoC. Use it with the default
  engine: it measured net-negative under `XEZIM_AOT`, where wider blocks
  shrink native coverage and defeat template deduplication.
- **PGO build** (above) for long runs; the build cost amortizes quickly.
  Re-measured on current code: c906 memcpy ×100 **−14.5% instructions and
  −14.6% wall** (176.92 B → 151.31 B, 51.5 s → 44.0 s, interleaved, bit-exact).
  The WALL gain needs the design to be instruction-bound — ibex CoreMark sheds
  ~11% of its instructions with no wall change, being memory-bound — so judge
  by `perf stat` instructions first and confirm wall separately. The profile
  transfers well: a c906-trained profile beat an ibex-trained one ON IBEX
  (−11.1% vs −10.6%), so one representative trainer suffices.
- **Native compilation (`XEZIM_JIT=1`, optionally `+XEZIM_AOT=1`) pays on
  designs with FEW, VERY HOT blocks — measure before adopting it.** Warm
  cache, wall-clock:

  | | interp | JIT | +AOT | +AOT +FSM |
  |---|---|---|---|---|
  | ibex CoreMark (1,553 comb entries) | 50.8 s | **39.0 s** (−23%) | 38.8 s | 39.1 s |
  | c906 memcpy ×100 (35,267 entries) | 49.3 s | 55.7 s (**+13%**) | 49.5 s | 47.8 s |

  The c906 loss is compile time, not slower simulation — JIT takes its sim
  phase 43.6 s → 42.9 s but spends 7.0 s more compiling, because the work is
  spread over ~23× more blocks that are each ~37× colder. Do not try to fix
  that with a hotness threshold: measured negative both ways (see the
  perf-notes entry). Elaboration-dominated runs do not benefit either — a
  c910 hello (≈16 s elaboration vs ≈8 s simulation) is 25.5 s against 24.7 s
  for the default engine. The first run after any design or binary change
  pays a one-time rustc compile (c910: ~4 min); `XEZIM_AOT_TEMPLATE=1` cuts
  that by ~16× and is runtime-neutral.
- Elaboration of huge SoCs (hundreds of files, millions of signals) can
  dominate short runs; `--cache` (experimental warm-start design cache) and
  `--artifact-compression` help repeated runs of an unchanged design.
- `--report-stats[=json]` prints an end-of-run footer for dashboards.
- There is no user-facing thread knob; the engine parallelizes internally where safe.

## Debugging a misbehaving simulation

Work down this list; each step localizes further at near-zero cost.

1. **Read the warnings.** xezim's diagnostics are load-bearing:
   - `settle limit hit … likely a zero-delay combinational loop through: <signals>`
     names the oscillating nets. A *genuine* deep ripple chain (e.g. a
     128-stage prefix-OR) needs one settle iteration per stage — the default
     cap is 1000; raise with `--settle-limit N` if a legitimate chain is
     deeper.
   - `DEAD-CLOCK WATCHDOG` means a process is parked on a clock that never
     changes — almost always an undriven net, an unresolved module
     (`-v`/`-y` library miss), or an ungenerated behavioral clock upstream.
   - `[IMPLICIT NET …]` warnings frequently explain an X that appears
     downstream.
2. **`--x-warn`** (`--x-warn-limit N`) reports the first signals that *turn*
   x after time 0, with their drivers — the fastest X-origin finder.
3. **`XEZIM_PROFILE_REPORT=1`** prints where time and evaluations went;
   `XEZIM_COMPILE_FAIL_STATS=1` prints why hot statements stayed on the slow
   AST path, with a sample of the offending statement — the two together
   answer "why is this slow" in one run.
4. **Make a standalone repro**: `--dump-merged-sv repro.sv -s top` writes
   one self-contained, fully-preprocessed file reachable from the top — the
   single most useful artifact to attach to an issue.
5. **Bisection knobs** (all runtime, no rebuild): `XEZIM_TS_DENY=<Opcode,…>`
   sends suspect compiled two-state blocks back to the interpreter;
   `XEZIM_JIT_DENY`/`XEZIM_JIT_COMB_RANGE` do the same for the JIT;
   `XEZIM_NO_PARALLEL=1` forces single-threaded execution. If a wrong value
   disappears under one of these, you've named the machinery — file that.
6. `--show-env-avail` lists every `XEZIM_*` variable with a description
   (~200 of them; the ones above are the user-facing core).

## Comparing against another simulator

The technique that repeatedly finds real divergences in minutes:

1. Add a small **probe block to the testbench** (or a second `-s` top) that
   `$fdisplay`s an architectural stream — retired PCs, a bus handshake, a
   state register — every N events, to a file.
2. Run the *identical* instrumented source on both simulators.
3. `diff` the two text streams; the first differing line gives you the
   time, the state, and usually the module to look at. Then window a
   waveform dump (`--xtrace-from/to`) around that time on both sides.

Program output should be compared with simulator banners stripped; xezim's
lines are prefixed (`[PHASE]`, `[PROF]`, `[WARN]`, …) and easy to filter.

## Real-number / AMS modelling

- `wreal` nets are supported; multiple drivers **sum** (the KCL reading —
  Verilog-AMS leaves resolution tool-defined). Packed ranges on `wreal` are
  rejected.
- §6.6.7 user-defined nettypes with real resolution functions
  (`nettype real n with sum_f`) are supported and their resolver calls
  compile — resolver-heavy RNM models run at compiled speed.
- `real`-returning functions in continuous assigns are compiled, including
  dynamic-array formals fed by assignment patterns.

## Filing a good issue

Include: `xezim -V` output; the exact command line; a repro
(`--dump-merged-sv` output or a minimal case); observed vs expected, ideally
with another simulator's or a formal tool's verdict; and any relevant
`[WARN]`/profile lines. Issues in this repo regularly close same-day when
the repro is standalone.
