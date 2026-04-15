# sisvsim Multithreading / Multicore Analysis

Scope: analysis only — no implementation. Goal: identify where parallelism
is worthwhile in sisvsim, what stands in the way today, and a staged path
to a safely multicore-capable simulator.

---

## 1. Current architecture (single-threaded assumptions)

Relevant code: `src/compiler/simulator.rs` (~6.1k lines), plus
`elaborate.rs`, `bytecode.rs`, `native_codegen.rs`.

Execution model today:

1. `Simulator::run()` (simulator.rs:1496) elaborates, builds comb graph,
   compiles edge blocks, then calls `event_loop()`.
2. The core loop drives a `TimingWheel` event queue, processes per-process
   statements, applies NBA updates, runs `settle_combinatorial`
   (simulator.rs:3213), handles edge-sensitive blocks, and dumps VCD.
3. The whole simulator state lives in one monolithic `Simulator` struct
   (simulator.rs:358–505): flat signal table, dirty bitvec, NBA queue,
   process contexts, heap for class instances, VCD writer, etc. Nearly
   every evaluator method takes `&mut self`.

Key data structures that currently require exclusive access:

- `signal_table: Vec<Value>` + `dirty_signals: Vec<bool>` +
  `dirty_list: Vec<usize>` (the hot path in `settle_combinatorial`).
- `nba_queue` / `nba_fast` (NBA buffering).
- `event_queue: TimingWheel`, `event_waiters`, `join_waiters`.
- `heap`, `mailboxes`, `semaphores`, `cg_heap`, `process_contexts`,
  `this_stack`, `local_stack`.
- VCD writer (`vcd_writer`, `vcd_prev_signals`), profiling counters,
  `name_resolve_hint: RefCell<...>`.
- DPI bindings (`libloading::Library`, `libffi`): foreign code, not
  automatically `Send`/`Sync`.

Implication: the simulator is effectively a single giant `&mut self`
actor. Any parallelism plan has to carve out sub-states that can be
accessed independently, or switch to a fork/join model where worker
threads borrow immutable slices plus local scratch buffers.

---

## 2. Where parallelism actually pays

Not every phase benefits. Ranked by expected ROI:

### 2.1 Combinational settle (highest ROI)
`settle_combinatorial` evaluates a triggered set of comb entries
(continuous assigns, `always_comb`, direct copies). On large designs this
dominates wall time. Each entry reads a fixed set of signals and writes
one destination. This is a classic **static dataflow DAG** — ideal for
data parallelism.

- Within one fixpoint iteration, entries that don't share write targets
  can evaluate concurrently using a read-only snapshot of `signal_table`.
- Writes land in a per-thread delta buffer, merged at the barrier, and
  the dirty-set for the next iteration is rebuilt from the merged deltas.
- Levelization (topological layers over the comb dep graph) lets each
  layer run fully in parallel and removes read/write conflicts entirely
  when the graph is acyclic.

### 2.2 Edge-sensitive (`always @(posedge clk)` etc.) block execution
`edge_blocks` + `compiled_edge_blocks` already has a per-block
representation. On a clock edge, many independent flops fire; today they
run sequentially and push to `nba_queue`. A parallel pass can:

- Partition edge blocks by clock domain.
- Execute blocks in parallel into **per-thread NBA buffers**.
- Concatenate into `nba_queue` at the barrier (order within a
  time-step is irrelevant for NBA — LRM says updates happen after all
  blocks in the region complete).

### 2.3 NBA apply
Applying `nba_fast` / `nba_queue` to `signal_table` is a pure scatter.
With per-signal write ownership (one writer per signal — typical for RTL)
this is embarrassingly parallel.

### 2.4 VCD / AITRACE dumping
`vcd_writer` runs after each time advance. Dumping is I/O-bound and
independent of the next step once the snapshot is taken. Natural fit for
a **single background dumper thread** fed by a ring buffer of
`(time, delta)` records. This alone can recover a large fraction of
runtime on dump-heavy workloads without touching the evaluator.

### 2.5 Elaboration / parsing
`sv-parser` + `elaborate.rs` are run once at startup. File-level
parallelism (parse each source in its own rayon task) is trivial,
low-risk, and helps short-simulation turnaround. Low ROI during steady
state, but cheap to do.

### 2.6 Poor ROI — do not parallelize
- The event wheel itself (ordering is sequential by design; parallelizing
  across time-steps requires speculative/Chandy–Misra/Time-Warp, which
  is a separate research-level project — see §6).
- `initial` blocks and single-process user code.
- DPI calls (serialize around foreign code unless the library declares
  reentrancy).

---

## 3. Obstacles in the current code

### 3.1 Monolithic `&mut self`
Every evaluator method mutates `Simulator`. Parallelizing requires
refactoring the hot path so that:

- Read-only state (comb graph, compiled bytecode, widths, signed flags,
  sdf_delays, dep_by_id) is held behind `Arc<...>` and shared.
- Mutable state is split into **shards** (signal range, NBA buffer,
  dirty buffer) owned by one worker at a time.

### 3.2 Interior mutability and non-`Send` fields
- `name_resolve_hint: RefCell<Option<String>>` — not `Sync`. Must be
  moved to a per-thread scratch or removed from hot eval.
- `libloading::Library` and `libffi::middle::*` — DPI handles are not
  guaranteed `Send`/`Sync`. A dedicated DPI thread or a mutex-guarded
  proxy is needed.
- `rand::rngs::StdRng` — fine, but must become per-thread to keep
  determinism (see §5).

### 3.3 HashMap-based auxiliary state
`signals`, `widths`, `signed_signals`, `prev_signals`,
`monitor_prev`, `vcd_id_map`, `vcd_prev_signals`,
`process_contexts` — these mirror or shadow the fast `Vec`-indexed
state. Under parallelism the mirrors go stale (`table_modified` flag
already hints at this bug surface). The parallel path must either drop
the HashMap mirrors entirely on the hot path or rebuild them only at
barrier points.

### 3.4 Comb graph contains cycles / unresolved reads
`has_unresolved_reads` entries are re-triggered every iteration
conservatively. A parallel scheduler that wants deterministic layering
must first tighten the comb graph builder so unresolved-read entries
become a small, well-identified residual evaluated sequentially after
each parallel layer.

### 3.5 Process contexts and fibers
SV fork/join, mailboxes, semaphores, class `this`, and edge-block
wait/resume are implemented as a bespoke fiber layer over
`process_contexts`, `this_stack`, `local_stack`,
`class_context_stack`. These are inherently sequential *within a process*
but many processes can make progress independently between yields.
Refactor prerequisite: each process's context must be movable
(`Send`) between threads, and the scheduler becomes a work-stealing
queue of runnable PIDs.

### 3.6 Determinism guarantees
IEEE 1800 scheduling regions (active → inactive → NBA → observed →
reactive → postponed) are deterministic *within a region* only up to a
point, but users rely on run-to-run reproducibility. A parallel engine
must define and preserve a stable tie-break (e.g., by `pid`, by
`signal_id`) when flushing NBAs and dispatching processes. See §5.

### 3.7 VCD ordering
VCD writer must see events in time order. Parallel evaluation is fine as
long as the dumper consumes committed snapshots in monotonic `time`
order — a single-writer channel handles this.

---

## 4. Proposed staged plan

Each stage is independently shippable and reversible.

### Stage 0 — preparation (no threads yet)
1. Replace `HashMap` mirror state on the hot path with the existing
   `Vec`-indexed tables; move HashMaps behind a `rebuild_slow_views()`
   helper called only where needed (debug, introspection, VCD setup).
2. Eliminate `RefCell<name_resolve_hint>` from hot eval: thread it
   explicitly through the evaluator call or move to a per-scope table.
3. Split `Simulator` into:
   - `SimStatic` (Arc-shareable, read-only after elaboration):
     `signal_widths`, `signal_signed`, `signal_real`, `id_to_name`,
     `comb_entries`, `comb_dep_by_id`, `edge_blocks`,
     `compiled_edge_blocks`, `sdf_delays`, `module`.
   - `SimState` (mutable, owned by engine): `signal_table`, `prev_table`,
     `dirty_signals`, `dirty_list`, `nba_fast`, `nba_queue`, `heap`,
     `mailboxes`, `semaphores`, `event_queue`, `process_contexts`.
   - `SimIO` (owned by dump thread): VCD writer, file handles.
3. Introduce a `ThreadPool` abstraction (rayon is the obvious pick, but
   keep a trait so it can be swapped for a custom work-stealer).
4. Add a `--threads N` CLI flag that is still a no-op, so release notes
   and benchmarks can be wired early.

### Stage 1 — parallel VCD / AITRACE dumper
- Dedicated writer thread consuming `(time, Vec<(signal_id, Value)>)`
  records over an SPSC channel.
- Risk: low. Correctness unchanged, pure ordering preserved. Profiling
  targets: dump-heavy testbenches; should show immediate speedup.

### Stage 2 — parallel comb settle
- Precompute levelization of the acyclic part of `comb_dep_by_id` at
  elaboration time. Store `levels: Vec<Vec<usize>>`.
- Per worker: reusable scratch of `triggered` bitvec + write-delta
  buffer.
- For each settle iteration:
  1. Read `settle_dirty_ids` → mark triggered entries per level.
  2. For each level: `pool.scope(|s| level.par_chunks().for_each(...))`
     where each chunk reads `&SimStatic` + `&[Value] signal_table` and
     writes into its local delta.
  3. Barrier: serially merge deltas into `signal_table`, update
     `dirty_signals`, `dirty_list`. Deterministic tie-break by
     `(level_idx, entry_idx)`.
- Residual (unresolved-read entries, cyclic SCCs) evaluated
  sequentially after each parallel level, exactly as today.
- Validation: run the `tests/prtest` suite with `--threads 1..N` and
  diff VCDs byte-for-byte against the sequential baseline.

### Stage 3 — parallel edge-block / NBA execution
- Bucket edge blocks by triggering clock edge; run buckets concurrently.
- Per-thread NBA buffer; merge into `nba_queue` at the barrier in a
  deterministic order (sort by `(signal_id, pid)`).
- Apply NBA in parallel via per-signal ownership; detect multi-writer
  collisions and fall back to sequential resolution with a warning
  (matches SV last-write-wins semantics but flagged).

### Stage 4 — parallel process scheduler
- Make `ProcessContext` `Send`; turn `process_contexts` into a
  work-stealing deque of runnable PIDs.
- Serialize on shared mutable resources (`heap`, `mailboxes`,
  `semaphores`) behind fine-grained locks or a single actor thread.
- Highest complexity; defer until stages 1–3 have proven the refactor.

### Stage 5 — NUMA / design partitioning (optional, large-design only)
- Partition the design hierarchy so that each partition owns a disjoint
  slice of `signal_table`, with a boundary-signal exchange phase per
  delta cycle. Enables scaling past ~8 cores where shared signal_table
  cache traffic dominates.

---

## 5. Determinism strategy

A multicore simulator must produce bit-identical VCD output across runs
and across thread counts, or users will reject it. Rules:

- All parallel scatters (NBA merge, comb delta merge) sort by a stable
  key (`signal_id`, then source `entry_idx` / `pid`) before committing.
- Per-process RNG seeds are derived from `(global_seed, pid)`, not from
  a shared `StdRng`.
- Process dispatch order within a time step is `pid`-ordered, not
  thread-arrival-ordered.
- Add a CI job that runs the full test suite with `--threads 1`,
  `--threads 4`, `--threads 16` and diffs outputs.

---

## 6. Explicitly out of scope

- **Speculative time parallelism** (Time-Warp / Chandy–Misra / optimistic
  PDES). Massive complexity (rollback, anti-messages), only worthwhile
  for very loosely coupled multi-chip sims. Not a fit for a lightweight
  learning simulator today.
- **GPU offload of evaluators.** Requires a JIT to GPU IR; native_codegen
  is CPU-only. Revisit after stage 4.
- **DPI reentrancy.** Keep DPI serialized behind a mutex or a dedicated
  thread; trying to parallelize foreign code is not worth the blast
  radius.

---

## 7. Risks and costs

| Risk | Mitigation |
|---|---|
| Refactor churn in simulator.rs (6k+ lines) | Stage 0 split is mechanical; land before any threading |
| Loss of determinism | Stable tie-breaks + CI diff harness (§5) |
| HashMap mirror staleness bugs | Drop mirrors from hot path in stage 0 |
| Contention on small designs (parallel slower than serial) | Threshold-gate parallel paths; fall back to sequential below e.g. 1024 triggered entries |
| DPI / non-`Send` libraries | Dedicated DPI thread, serialize calls |
| Test flakiness | VCD byte-diff harness across `--threads` values |

Expected speedups (order-of-magnitude guesses, need benchmarks):

- Stage 1 (dumper): 1.1–1.5× on dump-heavy runs.
- Stage 2 (parallel settle): 2–4× on 8 cores for large combinational
  designs; near-zero on tiny tests.
- Stage 3 (edge blocks + NBA): additional 1.3–1.8× on flop-heavy RTL.
- Stage 4 (process scheduler): depends entirely on workload; most RTL
  testbenches won't benefit much.

---

## 8. Recommended next step

Start with **Stage 0** (state split + HashMap mirror cleanup) and
**Stage 1** (background dumper). Both are low-risk, independently
valuable, and unblock the harder stages. Only begin Stage 2 once a
deterministic VCD-diff CI harness is in place — without it, parallel
bugs will be impossible to triage.
