# Per-LP event_loop integration plan

Concrete file-by-file plan for the architectural refactor that lets c910
beat its single-thread baseline via PDES. Designed to be executed across
**2-3 focused sessions** of ~800-1200 LOC each, with hello → memcpy →
cmark as the validation ladder. Branch state at start of this work:
`perlp-experiment` at commit `b37f651` (DDG analysis landed).

## Architectural goal

Replace `Simulator::event_loop`'s single-threaded "advance time, run all
blocks per tick, settle, apply NBAs" pattern with **N independent per-LP
event_loops** running on N host threads, synchronized only through
bounded `BoundaryChannel`s at clock-edge sync points. LPs can ride K
ticks ahead between syncs (CMB lookahead-K — validated on toy in
`d55831f`).

The validation ladder:

1. **Phase 0 (done):** dispatcher-arm-only PDES. Hello passes (+4.6%
   vs baseline). All 14 unit tests pass.
2. **Phase 1 (this plan):** per-LP NBA buckets + per-LP signal_lp_writer
   classifier port. ~500 LOC. No event_loop change. Validates the
   per-LP write infrastructure end-to-end on c910.
3. **Phase 2:** per-LP local signal_table allocation + sparse-snapshot
   per LP + global↔local id translation. ~600 LOC. event_loop still
   global; per-LP snapshots feed exec_insns_isolated through translated
   ids.
4. **Phase 3:** per-LP event_loop body extraction. Pure refactor of
   the 2000-line event_loop into `run_one_tick(state: &mut LpState)`.
   ~700 LOC. Single LP still; behavior identical.
5. **Phase 4:** spawn N LP threads, each runs its own
   `run_one_tick` loop on its own `LpState`. ~500 LOC. ClockBarrier +
   BoundaryChannel handle sync. Hello + memcpy should pass with same
   sim_time.
6. **Phase 5:** multi-tick lookahead K=10 → 100. ~200 LOC. Sync
   overhead amortized; speedup emerges. Target ~1.25-1.5× wall on
   c910 hello.

Total: ~2 500 LOC over 5 phases. Each phase is independently
validateable (must preserve sim_time + TEST PASSED).

## Phase 1 — Per-LP NBA buckets + classifier port (~500 LOC)

### Files to change

#### `xezim/src/compiler/simulator.rs` (~400 LOC)

**1.1.** Add `signal_lp_writer: Vec<Option<u32>>` field on `Simulator`
(near `edge_block_partition`, line ~880). Populated by classifier.

**1.2.** Port `classify_signal_lp_writers` from main branch — scans every
parallel-eligible compiled block for NBA targets, attributes the signal
to the writing LP. Multi-LP writers marked None (boundary). ~80 LOC.

**1.3.** Port `relax_parallel_eligibility_for_multikernel` (~90 LOC) from
main — re-promotes blocks containing NbaAssignBitDyn / NbaAssignRange to
parallel-eligible when their writes target LP-exclusive signals
(safe because no cross-LP race).

**1.4.** Call both passes from `apply_multikernel_scope_partition` after
the partition is set.

**1.5.** Refactor `apply_nba` to bucket fast-path NBAs by LP (port from
main branch's Phase 1 work). ~140 LOC. New structure:

```rust
fn apply_nba(&mut self) {
    self.nba_fast_index.clear();
    let mut nba = std::mem::take(&mut self.nba_fast);
    if !self.signal_lp_writer.is_empty() && self.edge_block_partition_count > 0 {
        let n_lp = self.edge_block_partition_count as usize;
        let mut per_lp: Vec<Vec<NbaFast>> = (0..n_lp).map(|_| Vec::new()).collect();
        let mut boundary: Vec<NbaFast> = Vec::new();
        for entry in nba.drain(..) {
            match self.signal_lp_writer.get(entry.signal_id).copied().unwrap_or(None) {
                Some(lp) => per_lp[lp as usize].push(entry),
                None => boundary.push(entry),
            }
        }
        for lp_entries in per_lp.iter_mut() {
            for entry in lp_entries.drain(..) { self.apply_nba_entry(entry); }
        }
        for entry in boundary.drain(..) { self.apply_nba_entry(entry); }
    } else {
        for entry in nba.drain(..) { self.apply_nba_entry(entry); }
    }
    // nba_queue (slow path) unchanged
    let queue = std::mem::take(&mut self.nba_queue);
    for entry in queue {
        if let Some(lhs) = entry.lhs { self.assign_value(&lhs, &entry.value); }
    }
}

#[inline]
fn apply_nba_entry(&mut self, entry: NbaFast) {
    // extract the existing single-entry apply logic
}
```

**1.6.** Extend `[PROF] par_dispatch` reporting to add
`classifier_signals` (count of LP-A-only / LP-B-only / boundary writers).

### Validation order

1. Run `cargo test --release --lib multikernel::` — all 14 tests must
   still pass.
2. Run c910 hello with `--multikernel-scope` — must produce sim_time
   44 695 ns + TEST PASSED. Wall should be ≈ unchanged (Phase 1 is
   structural).
3. Run c910 hello with `--multikernel-scope XEZIM_DISPATCHER=pdes` —
   same result.
4. Verify `[PART] signal LP-writer classification` reports
   `LP-A-only=5214, LP-B-only=6975, boundary=0` (matches earlier
   classifier measurement at this point in the worktree).
5. Verify `[PART] multikernel NBA-exclusion lift` reports `+4 blocks`
   (matches main branch).
6. Run c910 memcpy with `--multikernel-scope` — sim_time 101 965 ns +
   TEST PASSED.

### Common failure modes

- **Different sim_time than baseline:** classifier mis-classified a
  signal; per-LP NBA bucketing drops or duplicates an entry. Diagnose
  by comparing per-LP bucket sizes to total nba_fast.len() before drain.
- **Bit-difference at apply time:** width/signed coercion in
  apply_nba_entry differs slightly from the original inline logic.
  Diagnose by running settle_dc/ca/ab profile diff.
- **Test ladder regression in toy:** Phase 1's changes touch generic
  apply_nba — must run multikernel::tests unconditionally.

## Phase 2 — Per-LP local signal_table + sparse snapshot (~600 LOC)

### Files to change

#### `xezim/src/multikernel.rs` (~400 LOC)

**2.1.** Add `PerLpSignalTable` struct holding:
- `local_to_global: Vec<u32>` — local idx → global signal_id (size = LP read+write set)
- `global_to_local: HashMap<usize, u32>` — sparse reverse lookup (only LP's signals)
- `values: Vec<Value>` — per-LP local table sized to read+write set
- `widths: Vec<u32>`, `signed: Vec<bool>` — per-LP local metadata mirrors

**2.2.** Add `Simulator::build_per_lp_tables(lp_a_prefix)` returning
`Vec<PerLpSignalTable>` (one per LP). Iterates classifier output to
derive each LP's read+write set, allocates the local table sized to
that set. ~200 LOC.

**2.3.** Add `PerLpSignalTable::snapshot_for_tick(global_table: &[Value])`
method that clones the LP's read-set values into a fresh `Vec<Value>`
indexed by local id. ~50 LOC. Validated by the existing
`benchmark_sparse_snapshot` infrastructure.

**2.4.** Add `Simulator::pdes_exec_block_local(per_lp_table, bi, snapshot)`
variant that translates LoadSignal global_id → local_id via
`global_to_local` before reading the snapshot. NBA writes returned with
LOCAL ids; caller translates to global before applying. ~150 LOC.

### Validation order

1. Toy 2-counter test runs through per_lp_table — count_a=10 / count_b=45
   still produced. Add a new test
   `pdes_exec_block_local_via_per_lp_table`.
2. c910 hello / memcpy unchanged (per_lp_table not yet integrated into
   real Simulator::event_loop).
3. Memory: per-LP local table sizes should match measured 3.1 MB
   (LP-A) + 4.2 MB (LP-B) — verify via PROF.

### Common failure modes

- **Global↔local id translation bug:** LoadSignal reads wrong value.
  Diagnose by comparing snapshot[local_id] to snapshot[global_id].
- **Read-set missing a signal:** translation table missing entry,
  exec_insns_isolated panics on out-of-bounds.

## Phase 3 — Extract event_loop into `run_one_tick(state)` (~700 LOC, pure refactor)

### Files to change

#### `xezim/src/compiler/simulator.rs` (~700 LOC)

**3.1.** Add `EventLoopState` struct containing what every tick mutates:
- `time: u64`
- `event_queue: EventQueue` (already a separate type)
- `nba_fast`, `nba_queue`, `nba_fast_index`
- `dirty_signals`, `dirty_list`, `dirty_any`
- `delayed_updates`, `event_waiters`
- `clock_generators`
- `finished: bool`
- Various profiler counters

Keep `signal_table`, `signal_widths`, `signal_signed`, `id_to_name`,
`comb_entries`, `compiled_edge_blocks` etc. on `Simulator` (immutable
per tick).

**3.2.** Refactor `event_loop` into:

```rust
fn event_loop(&mut self) {
    let mut state = EventLoopState::extract_from(self);
    while !state.finished && state.iters < state.max_iters {
        self.run_one_tick(&mut state);
    }
    EventLoopState::merge_into(self, state);
}

fn run_one_tick(&mut self, state: &mut EventLoopState) {
    // current tick body, all mutations through &mut state
}
```

This is a **pure refactor**. Zero behavior change. Must preserve every
existing test + c910/c906 hello/memcpy/cmark bit-identical.

### Validation order

1. All 14 multikernel unit tests pass.
2. c910 hello / memcpy / cmark all PASS with same sim_time + TEST PASSED.
3. Compare per-phase PROF wall (settle, edges, nba, snap) — must be
   within 5% of pre-refactor numbers.

### Common failure modes

- **State partition wrong:** some field that needs to mutate per-tick
  is left on Simulator (causing borrow-check failures or stale reads).
- **Profiler counter divergence:** counters that span multiple ticks
  (e.g., `prof_simulation_loop` totals) need careful state handling.

## Phase 4 — Per-LP threads with ClockBarrier sync (~500 LOC)

### Files to change

#### `xezim/src/compiler/simulator.rs` (~300 LOC)

**4.1.** Replace `event_loop` body with per-LP thread spawning:

```rust
fn event_loop(&mut self) {
    let n_lp = self.edge_block_partition_count as usize;
    if n_lp <= 1 {
        // Fall back to single-LP path.
        let mut state = EventLoopState::extract_from(self);
        while !state.finished && state.iters < state.max_iters {
            self.run_one_tick(&mut state);
        }
        return EventLoopState::merge_into(self, state);
    }
    // Per-LP path.
    let mut per_lp_states: Vec<EventLoopState> = (0..n_lp)
        .map(|lp| EventLoopState::for_lp(self, lp))
        .collect();
    let per_lp_tables = self.build_per_lp_tables(self.lp_a_prefix.clone());
    let barrier = Arc::new(ClockBarrier::new(n_lp));
    let channels = self.build_boundary_channels(); // 109 BoundaryChannels for c910
    let ctx = Arc::new(self.extract_send_exec_context());

    let final_states: Vec<EventLoopState> = std::thread::scope(|s| {
        per_lp_states.drain(..)
            .enumerate()
            .map(|(lp, state)| {
                let ctx = Arc::clone(&ctx);
                let barrier = Arc::clone(&barrier);
                let local_table = per_lp_tables[lp].clone();
                let lp_channels = channels.for_lp(lp);
                s.spawn(move || {
                    let mut state = state;
                    run_per_lp_event_loop(&ctx, &mut state, local_table, lp_channels, &barrier);
                    state
                })
            })
            .map(|h| h.join().unwrap())
            .collect()
    });
    // Merge LP states back into Simulator (one designated I/O LP owns
    // $display/$finish state; signal_table is the union of per-LP tables).
    EventLoopState::merge_per_lp_into(self, final_states);
}
```

**4.2.** `run_per_lp_event_loop` implements the 3-barrier protocol from
the multikernel toy tests, but with the Simulator's actual tick body
inside Phase B.

**4.3.** `build_boundary_channels` allocates the 109 channels per the
classifier output.

**4.4.** Routing $display / $finish / $readmemh: designate LP-tb (the
LP containing top-level testbench) as the I/O LP. Other LPs send any
$display call as a channel message; LP-tb processes them in order. For
$finish: any LP sets a global `finished` flag (atomic); next sync
detects and exits.

#### `xezim/src/multikernel.rs` (~200 LOC)

**4.5.** Promote `BoundaryChannel` and `ClockBarrier` to pub-crate
visibility for use from the Simulator's event_loop. (They're already
pub-crate; this is a no-op visibility check.)

**4.6.** Add `build_boundary_channels(io: &LpIoStats) -> BoundaryChannelTopology`
that consumes the classifier's 109 boundary signal ids and direction
tags, allocates one `Arc<BoundaryChannel>` per signal (or one per
unique direction), and returns a `BoundaryChannelTopology` struct
with per-LP inbound/outbound vecs.

### Validation order

1. **Toy first:** ensure the existing 14 unit tests all still pass after
   per-LP event_loop refactor. They use the PdesCoordinator (not
   Simulator::event_loop), so this should be no-op.
2. **c910 hello:** must pass with same sim_time 44 695 ns. Wall is
   expected ~85-95s (Phase 4 has K=1, per-tick sync, no speedup yet —
   should match Phase 0 dispatcher numbers).
3. **c910 memcpy:** must pass with same sim_time 101 965 ns. Wall
   ~230-250s (similar to Phase 0).
4. **c906 hello:** must pass with same sim_time 33 255 ns. Wall ≈
   13.3 s (single-LP fallback path).
5. **cmark:** must pass with same sim_time 2 007 365 ns. Wall
   ~6700-7400s.

### Common failure modes

- **Cross-LP $display ordering:** if 2 LPs both call $display at the
  same sim_time, the I/O LP receives both but order is unspecified.
  Test by hello (single $display before $finish — order doesn't matter)
  before cmark (many $display in loops).
- **Boundary signal hash collision:** 109 channels seems small; ensure
  classifier's bidirectional channels are split into separate
  uni-directional Arc<BoundaryChannel>s.
- **Time-0 init divergence:** time-0 settle is global today; per-LP
  time-0 might produce different X-prop. Solution: run time-0 globally
  (single-threaded), then split state into per-LP tables before
  spawning threads.
- **Settle deadlock:** if LP-A's settle depends on LP-B's boundary
  signal AND vice versa, both block on recv(). Solution: settle is
  PER-LP — boundary-signal updates only flow once per clock-edge, not
  per settle iteration.

## Phase 5 — Multi-tick lookahead K (~200 LOC)

### Files to change

#### `xezim/src/compiler/simulator.rs` (~200 LOC)

**5.1.** Add `XEZIM_PDES_K=N` env var (already exists in
`run_c910_real_bytecode`) to set lookahead K.

**5.2.** Modify `run_per_lp_event_loop` (from Phase 4) to advance K
ticks between barriers:

```rust
fn run_per_lp_event_loop(ctx, state, local_table, channels, barrier) {
    let k = std::env::var("XEZIM_PDES_K").unwrap_or("1").parse().unwrap_or(1).max(1);
    while !state.finished {
        // Phase A: drain channels into local boundary mirror
        channels.drain_inbox(local_table);
        barrier.wait();
        // Phase B: run K ticks locally
        for _ in 0..k {
            if state.finished { break; }
            ctx.run_one_tick(state, local_table);
        }
        // Phase D: send boundary outbound
        channels.flush_outbox(local_table);
        barrier.wait();
    }
}
```

**5.3.** Validate that K=1 gives same wall as Phase 4. Then sweep
K=10, K=100 and measure wall reduction.

### Validation order

1. Hello passes with K=1, K=10, K=100 — all sim_time 44 695 ns.
2. Wall comparison: K=10 should be ≤ K=1 (fewer barriers).
3. Memcpy / cmark same validation.

### Expected results

| Workload | K=1 (Phase 4) | K=10 (Phase 5) | K=100 |
|---|---:|---:|---:|
| c910 hello | ~85 s | ~70 s | ~65 s |
| c910 memcpy | ~230 s | ~175 s | ~150 s |
| c910 cmark | ~6800 s | ~5200 s | ~4500 s |

These are projections from the Amdahl analysis + measured 2× edge_exec
+ settle parallelism + reduced barrier cost. Real numbers will reveal
how much of the projection is realistic.

## Determinism criteria

Every phase must preserve:

1. **Bit-identical sim_time** for all validation tests
2. **TEST PASSED** in the testbench output
3. **All 14 PDES unit tests still pass**
4. **No new compiler warnings** that weren't there at start of phase
5. **No new unsafe blocks** without documented safety contracts

If any criterion fails, the phase is incomplete; do not advance to the
next phase until resolved.

## Estimated session breakdown

| Session | Phases | LOC | Risk |
|---|---|---:|---|
| 1 | Phase 1 + Phase 2 | ~1100 | Medium (per-LP infrastructure is new) |
| 2 | Phase 3 (refactor only) | ~700 | High (event_loop is intricate) |
| 3 | Phase 4 | ~500 | Medium-high (cross-thread + boundary channels) |
| 4 | Phase 5 (measurement-driven) | ~200 | Low (small change, big payoff) |

If a session runs out of context mid-phase, leave the WIP commit in a
"won't break tests" state (mark new features behind env-var gates so
default behavior is unchanged). Next session resumes from there.

## File-by-file reference summary

| File | Phase 1 | Phase 2 | Phase 3 | Phase 4 | Phase 5 |
|---|---:|---:|---:|---:|---:|
| `src/compiler/simulator.rs` | +400 | +0 | +700 | +300 | +200 |
| `src/multikernel.rs` | +50 | +400 | +0 | +200 | +0 |
| `src/multikernel/tests.rs` | +30 | +50 | +0 | +50 | +50 |
| `src/main.rs` | +5 | +0 | +0 | +0 | +0 |
| **Total** | **~485** | **~450** | **~700** | **~550** | **~250** |
| **Cumulative** | **485** | **935** | **1635** | **2185** | **2435** |

Across 4 sessions: **~2 435 LOC total**, matching the original estimate.

## Done check

A future session is done with the per-LP event_loop integration when:

- [ ] c910 hello via `--multikernel-scope XEZIM_DISPATCHER=pdes XEZIM_PDES_K=10`
      runs in < 75 seconds (under baseline 83.6 s)
- [ ] c910 memcpy under same flags runs in < 200 seconds (under baseline 223.7 s)
- [ ] c910 cmark under same flags runs in < 6000 seconds (under baseline 6711 s)
- [ ] c906 hello unchanged (~13.3 s)
- [ ] All 14 PDES unit tests pass
- [ ] No regressions in the existing xezim test suite

When all 6 boxes are checked, the PDES architecture has delivered on
its projected speedup and the worktree branch is ready to merge back
to main.
