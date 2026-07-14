# xezim cross-platform benchmarks

Five benchmarks chosen so that each stresses a **different hardware axis**.
The point is not a single score — it is that when AMD / Intel / Graviton
diverge, you can attribute *why*.

| # | Benchmark | What it measures | Hardware axis it discriminates |
|---|-----------|------------------|--------------------------------|
| B1 | `c910-hello` (real RTL) | end-to-end throughput on a full XuanTie C910 boot | the headline number; memory- and branch-bound |
| B2a | `dispatch_regular` | interpreter rate, predictable block order | cache-resident execution throughput, IPC |
| B2b | `dispatch_branchy` | interpreter rate, data-dependent path (sequential) | **indirect-branch prediction** |
| B2c | `dispatch_branchy_par` | same design under xezim's auto-parallel dispatch | **thread fork/join + sync cost** |
| B3 | `mem-sweep` | ns/cycle as the working set walks L1 → LLC → DRAM | cache hierarchy, memory latency/bandwidth, TLB |
| B4 | `parallel-scaling` | edge-dispatch parallelism (`XEZIM_DISPATCHER`) | atomics, false sharing, core count vs SMT |
| B5 | `constraint-rand` | `randomize()` throughput (dist/foreach/unique) | branchy code, allocation, hashing, **i128 math** |

`B1 ÷ B2` tells you how much of xezim's real-world cost is memory versus
dispatch. B5's profile looks nothing like the others — it leans on the i128
exact arithmetic added for §18, which lowers very differently on aarch64.

## Running

```bash
python3 bench/gen_designs.py          # generate the synthetic designs
./bench/run_bench.sh                  # B2..B5, 5 reps, writes bench_<host>_<arch>.csv
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
  are enabled, since B4 depends on them.
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
  `XEZIM_DISPATCHER=pdes|perlp`, which is what B4 sweeps.
* On this dev box (6-core Intel i7-9800X), B4 showed **no speedup** from any
  dispatcher, and 4× the independent work produced only ~1.5× more unit-updates
  per second. If that reproduces on the other platforms, the limit is xezim's
  NBA merge, not the hardware — which is precisely what B4 exists to find.
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

* **A real xezim performance bug fell out of this.** xezim automatically enables
  parallel edge dispatch once a tick has >=10k bytecode insns across
  parallel-eligible blocks (`parallel_blocks >= 2 && parallel_insn_count >=
  10_000`). For many small blocks it then forks/joins **per clock edge**, and on
  this box that is **~6x SLOWER than sequential** for the identical design
  (total 10.3s vs 1.8s; `edge_exec` 9.8s vs 1.36s), while IPC collapses to 0.93.
  It also silently reports `insns=0`/`ns_per_insn=0`, because the parallel path
  does not increment those counters. `XEZIM_NO_PARALLEL=1` restores the fast
  path. This is very likely why B4 shows no scaling, and it is worth fixing in
  xezim (the heuristic should weigh per-block work, not the tick's total insns).
* For contrast, B5 (the constraint solver) runs at IPC 2.25 with a 1.1%
  branch-miss rate: it *is* the branchy, unpredictable workload of the set.
