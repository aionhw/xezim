# PDES experiment session summary — branch `perlp-experiment`

End state of one session's worth of per-LP PDES architectural validation
on c910. Worktree at `/home/bondan/repo/sv2023/xezim-pdes` (branch
`perlp-experiment`), forked from main at commit `143e163` before any
PDES additions existed.

## What's been validated (7 unit tests + 4 c910-scale integrations)

### Unit tests — `cargo test --release --lib multikernel::`

| # | Test | What it proves |
|---|---|---|
| 1 | `signal_table_basic_read_write` | UnsafeCell + raw-pointer disjoint writes sound |
| 2 | `clock_barrier_sync_round_count` | N-thread barrier with generation counter correct |
| 3 | `two_counters_with_shared_signal_via_pdes` | 3-barrier CMB lookahead-1 in `PdesCoordinator` |
| 4 | `pdes_exec_block_runs_real_bytecode_on_flip_flop` | Real bytecode flows through PDES wrapper API |
| 5 | `pdes_exec_block_flop_toggles_across_5_ticks` | Per-tick exec+apply cycle (sequential) |
| 6 | `real_bytecode_toy_through_pdes_phase_protocol` | Real SV bytecode through 3-phase protocol (sequential threads) |
| 7 | `real_bytecode_toy_through_actual_parallel_threads` | Real SV bytecode through 2 OS threads via `std::thread::scope` |

All 7 pass. Toy 2-counter SV (`examples/perlp_toy.sv`) produces
`count_a=10, count_b=45` (= sum(0..10)) under PDES — identical to
xezim's normal `--simulate` result on the same file.

### c910-scale integrations

| Validation | Result |
|---|---|
| Parse + elaborate + compile c910 hello | 31 s (matches baseline) |
| Boundary classifier on c910 | **109 cross-LP signals** identified (canonical AXI4-ACE + IRQ + debug + reset) |
| 120 `BoundaryChannel` objects wired and exercised | OK, +12% coord overhead |
| Coordinator throughput at c910 scale | ~17 M block invocations/sec sustained to 100k ticks |
| `Value`-table full snapshot per tick | 629 ms (infeasible for hello's 4 470 ticks) |
| `Value`-table sparse snapshot per tick (read-set only) | **3.19 ms** (**197× faster**) |
| Per-LP read-set sizes | LP-A: 101 205 signals (3.1 MB); LP-B: 137 561 (4.2 MB) |
| `SendExecContext` extract at c910 | 440 ms, ~220 MB total |
| Real bytecode through PDES dispatcher on c910 hello (10 ticks) | 101 280 block invocations, 89 118 NBA writes, no crash |
| Real bytecode through PDES dispatcher (50 ticks) | 506 400 invocations, 445 598 NBA writes, real state evolution observable |
| `--multikernel-scope` flag on worktree | hello PASSES, sim_time 44 695 ns ✓ |
| **`XEZIM_DISPATCHER=pdes`** active on hello | **PASSES**, sim_time 44 695 ns ✓, 4 462 dispatches through PDES arm |

## Architectural data — c910 hello at full scale

```
Signals:                  35 955 017
Compiled edge blocks:     20 779
Parallel-eligible blocks: 10 128 (49%)
Comb entries (continuous assigns + always_comb): 438 580
Boundary signals (cross-LP, comb-traced):         109
  - LP-A → LP-B:                                   61
  - LP-B → LP-A:                                   37
  - bidirectional:                                 11

LP-A scope:               x_soc...x_ct_top_0 (CPU core 0)
LP-A blocks:              7 973 (38%)
LP-A parallel-eligible:   4 212
LP-A read-set:            101 205 signals (3.1 MB Value-table)
LP-A write-set:           147 838 signals

LP-B scope:               everything else
LP-B blocks:              12 806 (62%)
LP-B parallel-eligible:   5 916
LP-B read-set:            137 561 signals (4.2 MB Value-table)
LP-B write-set:           187 956 signals

Multi-LP-writer signals:   0 (perfect disjointness — required for PDES)
```

## Performance baseline (worktree)

c910 hello, normal `--simulate`:
```
[PHASE] simulator construction: 4 904 ms
[PHASE] time-0 settle:            188 ms
[PHASE] compilation:           30 811 ms
[PHASE] simulation:            83 568 ms    ← target for PDES speedup
[PHASE] total:                114 379 ms
```

Per-phase breakdown of the 83.6 s simulation:
```
edges:    55.99 s  (67.0%)  ← edge_detect 11.5s + edge_exec 44.3s
settle:   19.09 s  (22.8%)
nba:       4.14 s   (4.9%)
snap:      3.16 s   (3.8%)
process:   1.15 s   (1.4%)
sched:     5 ms     (0.0%)
```

## PDES speedup ceiling (Amdahl, 2-LP, idealized)

Assuming perfect 2× speedup on each parallelized phase + measured
sparse-snapshot overhead (~6 s) + ~1% channel coordination:

| Phases parallelized | Total wall | Speedup |
|---|---:|---:|
| (baseline) | 83.6 s | 1.00× |
| edge_exec only | 67.4 s | 1.24× |
| edge_exec + settle | 57.9 s | 1.44× |
| edge_exec + settle + nba | 55.9 s | 1.50× |
| All ÷ 2 (idealized) | 47.8 s | 1.75× |

**Realistic 2-LP PDES ceiling: ~1.5× wall on c910 hello.**

## Worktree code summary

New files:
- `src/multikernel.rs` (~620 LOC) — `PdesKernel`, `BoundaryChannel`, `ClockBarrier`, `SignalTable<T>`, `KernelSpec`, `PdesCoordinator`, `classify_lp_io`, `build_c910_stub_specs_with_channels`, `run_c910_real_bytecode`, `LpIoStats`, sparse/full snapshot benchmarks
- `src/multikernel/tests.rs` (~430 LOC) — 7 passing tests
- `examples/perlp_toy.sv` (~50 LOC) — toy 2-counter reference SV (verified identical-output to PDES)
- `SESSION-SUMMARY.md` (this file)

Modified files:
- `src/lib.rs` (+~280 LOC) — `pdes_c910_stub_multi` orchestrator with Value-table + sparse-snapshot + SendExecContext benchmarks; `multikernel_scope` plumbing in `simulate_multi`
- `src/main.rs` (+~95 LOC) — `--pdes-c910-stub`, `--pdes-c910-ticks`, `--multikernel-scope` flags
- `src/compiler/simulator.rs` (+~285 LOC) — PDES accessors (`edge_block_count`, `edge_block_compiled`, `edge_block_scope_at`, `edge_block_parallel_at`, `signal_table_len`, `compiled_edge_block_at`, `signal_name_at`, `signal_signed_slice`, `signal_table_slice`, `comb_entry_count`, `comb_entry_io_at`, `pdes_exec_block`, `extract_send_exec_context`), `SendExecContext` struct + impl + `unsafe impl Send/Sync` with documented soundness contract, `apply_multikernel_scope_partition`, `XEZIM_DISPATCHER=pdes` arm, `prof_par_dispatch_pdes` counter

Total new + modified: ~1 760 LOC.

## What's left to actually beat baseline

The PDES dispatcher arm is wired but currently delegates to the same
`std::thread::scope` mechanism as the default backend, so wall time is
equivalent (95.5 s std::scope ≈ 93.5 s pdes — within noise). Beating
baseline requires:

| Piece | Est LOC | What it delivers |
|---|---:|---|
| Sparse per-LP signal_table snapshot in PDES arm | ~600 | Requires rewriting `exec_insns_isolated` to use LP-local signal-id space + global↔local translation maps |
| Per-LP NBA buffer with boundary channel delivery | ~250 | Each LP's NBA writes target its own table; cross-LP writes ship via channel |
| Per-LP event_loop with multi-tick lookahead (CMB) | ~1 500 | **The actual speedup mechanism** — LPs advance many ticks between syncs |
| `$display`/`$finish`/`$readmemh` routing to LP-tb | ~300 | Required if LPs ride ahead independently |
| Settle parallelization per LP | ~400 | Settle is 23% of hello wall; parallelizing it doubles the speedup |

Total remaining: **~3 050 LOC, 3-5 focused sessions.**

The **single line of reasoning** for why dispatcher-swap alone doesn't
deliver speedup: all current dispatchers (`tbb`, `std::scope`, `pdes`,
`rayon` on main) implement the same per-tick chunked parallel exec
pattern. The architectural constraint that caps speedup is **the global
tick boundary** — every dispatcher returns to the serial event_loop
after one tick. Real PDES speedup requires LP threads to advance many
ticks between rendezvous, which means per-LP time advancement, which
means per-LP event_loops.

## Recommended next-session sequence

1. Implement per-LP local signal-table architecture in `multikernel.rs`
   (build the storage layer + global↔local id maps). ~400 LOC. Unit-test
   against classifier-derived read+write sets.
2. Add LP-local-id-aware variant of `exec_insns_isolated`
   (or refactor existing to be id-space-parameterized). ~300 LOC. Test
   against existing per-block test (`pdes_exec_block_runs_real_bytecode_on_flip_flop`).
3. Wire sparse snapshot + boundary channel into PDES dispatcher arm.
   ~250 LOC. Run hello — should still PASS, wall similar (no
   architectural change yet).
4. Build per-LP event_loop variant (the big piece). ~1 500 LOC.
   Validate toy SV first, then hello.
5. Measure hello wall under per-LP event_loops. Should hit ~55-70 s
   (vs 83.6 s baseline = 1.20-1.50×).

## Validation criteria

Any further integration step must preserve:
1. **Bit-for-bit sim_time match** with baseline (hello: 44 695 ns;
   memcpy: 101 965 ns; cmark: 2 007 365 ns)
2. **TEST PASSED** in the testbench output
3. **All 7 PDES unit tests still pass**

## Memcpy 3-way validation (post-commit)

Same worktree binary, sequential runs on c910 memcpy. Confirms the
PDES dispatcher arm extends to memcpy depth with identical behavior
to the std::scope dispatcher.

| Mode | Sim time | TEST | Sim wall | Δ vs baseline | Dispatcher hits |
|---|---:|---|---:|---:|---|
| Baseline `--simulate` (1T) | 101 965 ns | PASS | **223.7 s** | 1.00× | partition=0 |
| `--multikernel-scope` (std::scope) | 101 965 ns ✓ | PASS | **242.7 s** | 0.92× (+8.5%) | partition=10 189, pdes=0 |
| `XEZIM_DISPATCHER=pdes` | 101 965 ns ✓ | PASS | **243.6 s** | 0.92× (+8.9%) | partition=10 189, **pdes=10 189** |

All three modes produce bit-identical sim_time and TEST PASSED.
PROF counter `pdes=10 189` confirms every parallel-block dispatch
on memcpy routes through the new PDES arm. Wall-time gap PDES↔std::scope
is 0.4% (noise) — consistent with the central architectural finding
that dispatcher-swap alone cannot beat baseline.

Projected memcpy with full per-LP PDES (sparse tables + multi-tick
lookahead + parallelized settle, the ~3 050-LOC remaining work):
- edges parallel (50% saved on 56.8% phase) → -63 s
- settle parallel (50% saved on 35.1% phase) → -39 s
- + ~10 s sparse-snapshot overhead
- = **~131 s vs 223.7 s baseline → ~1.7× speedup**

Logs: `xezim/simtest/xuantie_c910/work/c910_memcpy_cmp_{base,mk,pdes}.log`.

## Honest limitations

- The PDES architecture as designed targets **dual-LP scope-based
  partitions** (e.g. CPU core 0 vs everything else). Generalizing to
  k > 2 LPs adds complexity not addressed in this experiment.
- Settle is currently NOT parallelized in the prototype. The 1.5×
  ceiling assumes settle parallelization in addition to edge_exec.
- `Value`-type cross-thread sharing relies on the
  `unsafe impl Send/Sync for SendExecContext` contract — sound for
  read-only dispatch (CompiledBlock's interior-mutable Cell/OnceCell
  fields are populated single-threaded during compile, only read at
  exec time). Generalizing to per-tick mutation would need
  Cell → Atomic or per-thread clones.
