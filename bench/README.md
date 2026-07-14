# xezim cross-platform benchmarks

Four benchmarks chosen so that each stresses a **different hardware axis**.
The point is not a single score — it is that when AMD / Intel / Graviton
diverge, you can attribute *why*.

| # | Benchmark | What it measures | Hardware axis it discriminates |
|---|-----------|------------------|--------------------------------|
| B1 | `c910-hello` (real RTL) | end-to-end throughput on a full XuanTie C910 boot | the headline number; memory- and branch-bound |
| B2a | `dispatch_regular` | interpreter rate, predictable block order | cache-resident execution throughput, IPC |
| B2b | `dispatch_branchy` | interpreter rate, data-dependent path (sequential) | **indirect-branch prediction** |
| B2c | `dispatch_branchy_par` | same design under xezim's auto-parallel dispatch | **thread fork/join + sync cost** |
| B3 | `mem-sweep` | ns/cycle as the working set walks L1 → LLC → DRAM | cache hierarchy, memory latency/bandwidth, TLB |
| B5 | `constraint-rand` | `randomize()` throughput (dist/foreach/unique) | branchy code, allocation, hashing, **i128 math** |

`B1 ÷ B2` tells you how much of xezim's real-world cost is memory versus
dispatch. B5's profile looks nothing like the others — it leans on the i128
exact arithmetic added for §18, which lowers very differently on aarch64.

## Running

```bash
python3 bench/gen_designs.py          # generate the synthetic designs
./bench/run_bench.sh                  # B2, B3, B5 — 5 reps; writes bench_<host>_<arch>.csv
./bench/run_bench.sh -b B1,B2 -r 9    # pick benchmarks / reps
./bench/summarize.py results/*.csv    # compare hosts side by side
```

B1 is opt-in (`-b B1`): it needs `simtest/xuantie_c910` set up with the
external RTL, so it is skipped where that isn't present.

Every row carries `host,arch,cpu,cores,xezim`, so CSVs from the three machines
can simply be concatenated and fed to `summarize.py`.

## Methodology (this matters more than the benchmark list)

* **Fix the work, not the time.** Every design does a fixed number of cycles /
  randomizations. Compare `items_per_sec` and `ns_per_insn` — wall-clock alone
  will just rank clock speeds.
* **Same toolchain on all three hosts** (identical rustc/LLVM). Report both
  stock and `RUSTFLAGS="-C target-cpu=native"`; on Graviton confirm LSE atomics
  are enabled, since B2c depends on them.
* **Pin cores, ≥5 reps, use the median.** `summarize.py` flags any row whose
  spread across reps exceeds 10% (`!`) — do not draw conclusions from those.
* **Keep the `[PROF]` split.** Each row records `settle / edges / nba /
  process` ms. That is what turns "Graviton is 20% slower" into "Graviton
  spends 20% more in `edges`".
* **Watch `fallbacks`.** If one platform shows more AST fallbacks, the runs are
  not doing the same work and are not comparable.

## Hardware counters

The runner wraps each run in `perf stat` when it is available and permitted,
recording **rates, not raw counts** (`ipc`, `branch_miss_pct`,
`cache_miss_pct`), because rates stay meaningful across machines with different
clock speeds and core counts. Only the *generic* perf events are used
(`cycles,instructions,branches,branch-misses,cache-references,cache-misses`) —
the kernel maps these on Neoverse/Graviton exactly as on x86, so the columns are
directly comparable. Arch-specific events (`LLC-load-misses` and friends) are
deliberately avoided.

If `perf` is missing or `perf_event_paranoid` is too high, the benchmarks still
run and those columns read 0.

Counters are how you answer *why* a platform is slower. A B2 that regresses on
Graviton with a **higher branch-miss rate** is an indirect-predictor story; the
same regression with a flat branch-miss rate but **higher cache-miss rate** is a
memory story. Without counters you can only observe the gap.

## Gotchas discovered while building this

* **`--threads n` is not parallel simulation.** Per `--help` it only offloads
  stdout writes to a background thread. Parallel edge dispatch is selected with
  xezim's own calibration (see below), or forced with `XEZIM_FORCE_PARALLEL=1`.
* B3 already shows a clean knee on this box: ~537k cycles/s at a 4 KiB working
  set → ~279k at 16 MiB.
* **The original B2 was not branch-bound.** With a block order that repeats
  every cycle the predictor learns it perfectly — IPC 3.07, branch-miss 0.04%.
  So it measures cache-resident throughput, not prediction. It is kept as
  `dispatch_regular`, and `dispatch_branchy` was added: an LFSR selects a
  different case arm *and* a different subset of firing blocks every cycle.
  Same footprint, same work, only predictability differs — so the pair isolates
  the predictor's cost. On the i7-9800X (median of reps):

  | variant | items/s | IPC | br-miss | cache-miss |
  |---|---:|---:|---:|---:|
  | `dispatch_regular` | 83,542 | 3.07 | 0.04% | 35.6% |
  | `dispatch_branchy` | 11,904 | 2.23 | 0.51% | 0.65% |
  | `dispatch_branchy_par` | 1,695 | 0.93 | 1.60% | 0.86% |

* **A real xezim performance bug fell out of this — and it is now fixed.**
  xezim used to enable parallel edge dispatch on a fixed rule (`>= 2 blocks and
  >= 10k bytecode insns in the tick`). That rule ignores block SHAPE: 512 blocks
  of ~40 insns clear 10k easily, yet each is far too small to amortize a thread
  hand-off, so xezim forked/joined **per clock edge** and ran **~6x SLOWER than
  sequential** on the identical design (10.3s vs 1.8s). It also reported
  `insns=0`/`ns_per_insn=0`, because worker threads never touched those counters
  — the profile was silently lying about the very path that was running.

  The gate is now **self-calibrating**: the first 64 qualifying ticks run
  sequential, the next 64 run parallel, ns/insn is compared, and the winner is
  locked for the rest of the run (parallel must win by >10% to be worth the
  nondeterminism). That is also the right answer for a cross-platform suite —
  the correct choice genuinely differs between a 6-core x86 and a 64-core
  Graviton, and now each machine decides for itself. `XEZIM_NO_PARALLEL=1`
  forces sequential; `XEZIM_FORCE_PARALLEL=1` forces threading (which is how
  B2c measures thread cost). `[PROF]` now also prints
  `parallel_dispatch ticks=… blocks=…` so the parallel path can never again
  masquerade as zero work.

  On this box calibration picks SEQUENTIAL for every design in the suite,
  including the one shape parallelism should suit (8 blocks x ~6k insns) — so
  xezim's parallel edge path is currently not profitable here at all. B2c
  (forced parallel) is what tells you whether that also holds on the other
  machines.
* For contrast, B5 (the constraint solver) runs at IPC 2.25 with a 1.1%
  branch-miss rate: it *is* the branchy, unpredictable workload of the set.
