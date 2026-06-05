# xezim performance improvement suggestions

Companion to [MULTIKERNEL-NOTES.md](MULTIKERNEL-NOTES.md). That file
documents what was shipped/tried this session. This file is a forward-
looking menu of additional improvements beyond that list — ideas that
were considered but not implemented, ranked by expected ROI and effort
for c910-scale workloads.

Reference baselines (post-NBA-elision):
- c910 hello sim wall: **77.3 s** (was 86.8 s pre-elision)
- c910 memcpy sim wall: **233.4 s**
- c910 cmark iter=1 sim wall: **5 934 s (≈ 1h 39m)**

Phase distribution on hello after elision:
```
edges      54.9 s  71%    ← #1 target
└ edge_detect   10.8 s
└ edge_exec     44.1 s
settle     16.0 s  21%    ← #2 target
nba         2.2 s   3%    (halved by NBA-elision)
snap        3.0 s   4%
process     1.2 s   2%
```

---

## A. Interpreter cost (edge_exec = 44 s / 57% of hello wall)

The biggest target. Goes without saying.

### A1. ~~`cargo build --release --features jit` rollout~~ ❌ VALIDATED: REGRESSED

**Status: tried this session, REGRESSED on c910. See MULTIKERNEL-NOTES.md §"What was tried and failed" §D for full data.**

- **Measured hello: +11% (Cranelift) / +5% (LLVM) sim wall regression.**
- 72% block coverage achieved (15 040/20 779 blocks JIT'd) — coverage is
  not the bottleneck.
- **Root cause:** JIT uses FFI bridge per signal access (~10–20 ns each);
  c910 RTL blocks are load-heavy, so FFI cost exceeds dispatch savings.
- **Fix would require:** inlining signal_table access into JIT-generated
  code (no FFI per load/store). Multi-week refactor — Phase 1 MVP likely
  stalled here for this exact reason.
- **Do NOT enable on c910 with the current design.** Keep gated and off.

### A2. LLVM PGO build of xezim itself
- **Hello est: −5 to −15 s**
- **Effort: low**
- **Where:** `Cargo.toml` + a one-time profiling build.
- **How:**
  ```bash
  RUSTFLAGS="-Cprofile-generate=/tmp/pgo" cargo build --release
  ./target/release/xezim --simulate ...   # warm-up run with hello
  /usr/local/bin/llvm-profdata merge -o /tmp/pgo/merged.profdata /tmp/pgo
  RUSTFLAGS="-Cprofile-use=/tmp/pgo/merged.profdata" cargo build --release
  ```
- **Notes:** Bytecode dispatch loops are textbook PGO wins (5–20%).
  Stacks with JIT (independent optimization axes).

### A3. 1-bit fast-path Insn variants
- **Hello est: −3 to −5 s**
- **Effort: medium**
- **Where:** Add `Insn::NbaAssign1Bit(sig_id, val_reg)` and
  `Insn::LoadSignal1Bit(reg, sig_id)` to [`src/compiler/bytecode.rs:96+`](src/compiler/bytecode.rs).
  Bytecode compiler emits the 1-bit variant when `signal_widths[sig_id] == 1`.
  Interpreter paths use `u8` instead of full Value.
- **Notes:** ~80% of c910 NBAs target 1-bit signals (single-flop Q outputs).
  16-byte Value alloc/compare/store dropped to 1-byte path.

### A4. Insn fusion (peephole) at compile time
- **Hello est: −2 to −5 s**
- **Effort: medium**
- **Where:** New pass in [`src/compiler/bytecode.rs`](src/compiler/bytecode.rs)
  before emitting final Insn stream.
- **Notes:** Common patterns to fuse:
  - `LoadSignal + Compare + Branch` → `LoadSignalCompareBranch`
  - `LoadSignal + Add + Store` → `LoadSignalAddStore`
  - `LoadConst + Store` → `StoreImm`
  Reduces dispatch overhead per "logical op."

### A5. Unchecked indexing in the hot loop
- **Hello est: −1 to −3 s**
- **Effort: low**
- **Where:** [`src/compiler/simulator.rs:6210`](src/compiler/simulator.rs)
  and [:5794](src/compiler/simulator.rs).
- **Notes:** Bounds checks on `vm_regs[r as usize]`, `signal_table[sig_id]`
  are provably safe (regs allocated by compiler, sig_ids validated at
  elaboration). `unsafe { *x.get_unchecked(i) }` saves a bounds check
  + branch per access.
- **Risk:** Any future Insn crafting an invalid reg/sig_id → UB. Gate behind
  a `debug_assert!` variant so debug builds still catch it.

---

## B. Settle phase (16 s / 21% of hello wall)

Settle is iterative comb-propagation. Currently ~2 iters per call, max 6.

### B1. SCC-based settle scheduling
- **Hello est: −3 to −6 s**
- **Effort: medium-hard**
- **Where:** [`src/compiler/simulator.rs:10730`](src/compiler/simulator.rs)
  (`settle_combinatorial`). Pre-compute SCCs of the comb dependency graph
  at elaboration time (use the existing Tarjan SCC code in
  [`src/multikernel.rs`](src/multikernel.rs) for the DDG).
- **Notes:** Run within-SCC iter until fixed, then move to next SCC in
  topological order. Avoids re-evaluating already-settled comb exprs
  while a downstream SCC is iterating.

### B2. Reorder `comb_entries` by topological depth
- **Hello est: −1 to −2 s**
- **Effort: low**
- **Where:** After `build_comb_entries`
  ([`src/compiler/simulator.rs`](src/compiler/simulator.rs)).
- **Notes:** Sort by SCC depth (or simple topological rank). First settle
  pass becomes a single sweep on acyclic regions; only cyclic regions
  need iteration.

### B3. Settle convergence early-out
- **Hello est: −0.5 to −1 s**
- **Effort: low**
- **Where:** Inside `settle_combinatorial`'s main iter loop.
- **Notes:** If a settle iter dirties no signal that wasn't already in
  this call's worklist, terminate one iter earlier. Today max_iters=6
  fires often (see `[PROF] settle_calls`/`max_iters`); converging at 4
  instead of 5 cuts ~17% off settle.

### B4. Defer settle when only clocks are dirty
- **Hello est: −2 to −3 s**
- **Effort: medium**
- **Where:** Top of event_loop iter.
- **Notes:** When `fire_clock_generators` dirties only clk signals AND
  nothing else has fired yet, defer settle until something else dirties
  a non-clock signal.
- **Risk:** Comb dependents of clock would lag by one iter — same fail
  mode as the write_sig! macro tracker. Needs careful audit. **High
  correctness risk**; consider only after gaining confidence with B1/B2.

---

## C. Memory layout / cache locality

c910 has 36M signals but ~200k actively touched. signal_table sees
gigantic strided access patterns.

### C1. Hot-signal arena
- **Hello est: −3 to −8 s**
- **Effort: medium-hard**
- **Where:** At elaboration, identify the ~200k signals appearing in
  `edge_signal_ids` ∪ `signal_lp_writer` writes ∪ comb_entries reads.
  Pack them into a `Vec<Value>` of ~4 MB and reroute access via a
  `signal_id → hot_offset` map.
- **Notes:** Cold signals (memory arrays, unused wires) stay in the big
  table. Hot arena fits in L3 → 3–10× memory bandwidth win on the hot
  path. The harder bit is rerouting every read/write site; consider
  doing this as a Vec-of-pointers redirect at runtime.

### C2. Value representation slim-down
- **Hello est: −2 to −5 s + huge RAM win**
- **Effort: hard**
- **Where:** `Value` type (in `xezim-core`).
- **Notes:** Today `Value` is ~16 bytes. ~90% of c910 signals are
  ≤32-bit and X/Z-free. A discriminated enum `Value { Small(u32),
  SmallSigned(i32), Wide(Box<...>) }` would halve the hot signal_table
  memory footprint.
- **Risk:** Touches every Value operation in the codebase. Significant
  refactor. Best done with comprehensive Value test coverage in place.

### C3. SoA verification for signal_widths/signed/has_xz
- **Hello est: −0.5 to −1 s**
- **Effort: low**
- **Where:** Field layout on `Simulator`.
- **Notes:** Already laid out as separate parallel `Vec`s. Verify
  cache-line alignment; consider packing `(width, signed, has_xz)`
  into a single byte each → one u32 per signal.

---

## D. Edge detection (10.8 s / 14% of hello wall)

### D1. Write-driven dispatch
- **Hello est: −5 to −8 s**
- **Effort: medium**
- **Where:** Replaces the scan in
  [`src/compiler/simulator.rs:9890`](src/compiler/simulator.rs)
  (the position-iter loop inside `check_edges_inner`).
- **Notes:** Today: scan all ~10k edge_signal_ids each iter, compare
  cur vs prev. Replace with: maintain `signals_written_this_iter: bitmap`
  (or sparse list), walk only the bitmap ∩ edge_signal_ids subset.
- **Risk:** Every signal write path must set the bitmap bit — **same
  hazard as the write_sig! macro failure documented in
  MULTIKERNEL-NOTES.md**. Requires a full audit of every direct
  `signal_table[id] = val` write site before shipping.

### D2. Static clock-derivation analysis
- **Hello est: −2 to −3 s**
- **Effort: hard**
- **Where:** Elaboration pass + extends the clocks-only scaffold already
  in [`src/compiler/simulator.rs`](src/compiler/simulator.rs) (`check_edges_clocks_only`,
  `is_edge_signal_non_clock`).
- **Notes:** Detect which non-clock edge signals are purely clock-derived
  (clk → buf → div_clk chain). Treat them as clock-equivalent in the
  fast-path predicate. Settle's writes to clock-derived signals stop
  disqualifying the fast path. The 25% zero-edge iters on c910 become
  real wins.

### D3. Per-direction fanout dispatch
- **Hello est: −1 to −2 s**
- **Effort: medium**
- **Where:** `edge_blocks_by_sig` in [`src/compiler/simulator.rs`](src/compiler/simulator.rs).
- **Notes:** `EdgeFanout` already separates posedge/negedge/anyedge
  lists. On a negedge iter where most posedge-fanout has no fire, skip
  the posedge subscan entirely. Today the inner loop walks all three
  lists regardless. Track per-tick clock direction in
  `toggled_clock_positions` (add a `bool fires_pos` field).

---

## E. Profile-guided (cross-run, persistent)

Rely on a profile file generated once and reused.

### E1. Block-frequency-ordered dispatch
- **Hello est: −1 to −3 s**
- **Effort: low-medium**
- **Where:** `edge_block_exec_counts` already collected
  ([`src/compiler/simulator.rs`](src/compiler/simulator.rs)).
- **Notes:** Persist counts to disk after a warm-up run. On next run,
  sort `compiled_edge_blocks` to keep hot blocks at lower indices →
  better i-cache locality. Pure layout change, zero correctness risk.

### E2. ~~Selective JIT compilation~~ — UNHELPFUL given A1 result

**Status: superseded by A1 finding.** Selectively JIT'ing the top-N
hottest blocks would magnify the per-block JIT regression, not reduce
it — the issue is per-signal-access FFI cost which scales with how
HOT a block is, not whether it has full or partial coverage. A
hot-block-selective JIT would deliver MORE FFI overhead per
unit-time, not less.

Defer until the underlying FFI-inline-access redesign lands.

### E3. Persistent compile cache
- **Hello est: 0 sim wall, −29 s compile-phase wall per repeat run**
- **Effort: medium-hard**
- **Where:** Extend `xezim-core::write_compiled`/`read_compiled` to
  cover bytecode IR. Currently those serialize only elaboration output.
- **Notes:** Covered briefly in MULTIKERNEL-NOTES.md. Highest dev-loop
  ergonomics win. Blocker: `Insn::StmtFallback` holds `Arc<Statement>`,
  which is `xezim-core`'s recursive AST type — deriving serde across
  it is a serde refactor in xezim-core.

---

## F. Build / toolchain

### F1. LTO release profile ✅ VALIDATED: SHIPPED

**Status: tried this session, SHIPPED. See MULTIKERNEL-NOTES.md §"What's shipped" §5.**

- **Measured: hello −2.7%, memcpy −5.3%, sim_time bit-identical.**
- Cargo.toml gained `[profile.release-lto]` inheriting from release
  with `lto = "fat"`, `codegen-units = 1`.
- Build invocation:
  ```bash
  RUSTFLAGS="-C target-cpu=native" cargo build --profile release-lto --bin xezim
  # binary at target/release-lto/xezim
  ```
- Slow build (~5×), but xezim's interpreter benefits noticeably from
  cross-crate inlining. Stacks with PGO (untried).
- **Existing `[profile.release]` left as-is** to preserve the JIT-LLVM
  safety note (LTO miscompiles inkwell/llvm-sys). Two profiles
  coexist; use `release-lto` for non-JIT builds.

### F2. `+native` CPU target
- **Hello est: −1 to −3 s**
- **Effort: trivial**
- **Where:** `~/.cargo/config.toml` or env.
- **How:**
  ```toml
  [build]
  rustflags = ["-C", "target-cpu=native"]
  ```
- **Notes:** AVX2/BMI2 helps Value math. Verify it's not already on for
  release builds.

---

## G. Workload-specific (RTL memory model)

### G1. Memory-array bypass
- **Hello est: modest wall + 4× RAM cut**
- **Effort: medium**
- **Where:** RTL memory arrays (`RTL_MEM.ramN.mem[]`, `mem_inst_temp[]`,
  `mem_data_temp[]`) currently stored as `Vec<Value>` (16 bytes each).
- **Notes:** Treating these as plain `Vec<u8>` (no X/Z support, no
  parallel structures) cuts RAM by 4× and speeds the load loop.
- **Risk:** Any code reading those arrays as `Value` breaks. Constrain
  to memory-pattern arrays only (identified by name/size heuristic).

### G2. `$readmemh` direct-to-storage
- **Hello est: minor wall, big startup**
- **Effort: low**
- **Where:** `$readmemh` handling in
  [`src/compiler/simulator.rs`](src/compiler/simulator.rs).
- **Notes:** Currently `$readmemh` populates `mem_inst_temp`, then a
  testbench loop copies into `RTL_MEM.ramN.mem`. Could load
  `$readmemh` DIRECTLY into the RAM target if names line up.

---

## H. Algorithm-level (PDES alternatives to the scope-based plan)

### H1. Signal-domain PDES
- **Hello est: unknown**
- **Effort: very hard**
- **Where:** Replaces the `--multikernel-scope` partition logic in
  [`src/multikernel.rs`](src/multikernel.rs).
- **Notes:** Current `--multikernel-scope` partitions by RTL hierarchy.
  An alternative partitions by SIGNAL SET (each LP owns a disjoint set
  of signals it can write). Boundary classifier already produces the
  write-set partition (`signal_lp_writer`). Could enable finer-grained
  k > 2 partitions without scope alignment.

### H2. Speculative event_loop
- **Hello est: unknown**
- **Effort: research**
- **Where:** New layer over `run_one_tick`.
- **Notes:** Run iter N+1 optimistically while N's NBAs apply on another
  thread. Rollback if N's apply changes input state to N+1. Classical
  optimistic PDES territory (Time Warp / rollback).

### H3. Activity-driven simulation
- **Hello est: unknown**
- **Effort: research**
- **Where:** Per-block read-set hashing in
  [`src/compiler/simulator.rs`](src/compiler/simulator.rs).
- **Notes:** Skip evaluation of blocks whose read-set hash hasn't changed
  since last firing. Similar to Synopsys VCS's "edge-driven optimization."
  Hash-cache one slot per block — cheap to test, no LRU eviction. Pays
  especially well for clock-gated regions where the gating signal is 0
  for many cycles.

---

## Recommended action ladder (what to try next, in order)

Updated post-session-2 with PGO + partial-NBA-elision validation
results. Items marked ✅ are shipped; ❌ are validated-and-rejected.
Each remaining step is independent enough to ship before starting
the next.

1. ✅ **F1 (LTO + native CPU)** — DONE. Modest stand-alone (−2.7% /
   −5.3%) but stacks well with PGO.
2. ❌ **A1 (`--features jit`)** — REGRESSED. +5% to +11% on hello.
   See A1 entry for root cause (FFI bridge per signal access).
3. ✅ **F2 (PGO with `-Cprofile-generate`)** — DONE. Standalone PGO
   −6.5% / −6.7%. With matched-LTO profile: **−20% / −19% cumulative**
   from baseline. See MULTIKERNEL-NOTES.md §6 for the build steps.
   **PROFILE MUST BE MATCHED TO LTO** — mismatched profile regresses.
4. **E1 (block-frequency-ordered dispatch)** — ~40 LOC, no risk, uses
   existing `edge_block_exec_counts`. Improves i-cache locality
   without touching correctness. **Note:** within-run reordering is
   complex (block_idx referenced everywhere); really needs persistent
   cross-run profile file. Defer until compile cache lands.
5. ✅ **Partial-range NBA-elision** — DONE. **−10% / −11% sim wall**
   on hello/memcpy; biggest single code change of the session.
   Original estimate (−1 to −3 s) was way off — turns out partial-
   range NBAs (especially NbaAssignBitDyn for flop bit writes)
   dominate c910's NBA traffic.
6. **B1 + B2 (SCC-based settle ordering)** — biggest remaining
   non-JIT structural win. Settle is now ~20% of hello (post-elision).
7. **D2 (static clock-derivation)** — unlocks the existing clocks-only
   scaffold to actually fire on c910.
8. **C1 (hot-signal arena)** — biggest cache-locality lever; requires
   the most thinking. Save for after the lower-effort items.
9. **JIT FFI-inline redesign** — if JIT is to be salvaged, this is
   the path: inline signal_table access into generated code, drop
   per-access FFI. Multi-week project. Defer until the lower-effort
   items above have shipped — at that point the single-thread
   baseline will be tighter and the JIT redesign ROI clearer.
10. **Per-LP event_loop (PDES Phase 4)** — covered in
    [PERLP-EVENTLOOP-PLAN.md](PERLP-EVENTLOOP-PLAN.md). The
    architectural ceiling fix. Defer until items 3–8 land; the
    Amdahl ceiling for 2-LP PDES depends on the single-thread
    baseline, so tightening it via the items above raises what
    the per-LP event_loop must deliver to justify ~3000 LOC.

**Baseline after items 1+3+5 (the shipped wins) is hello 66.0 s,
memcpy 189.6 s.** Use these as the new reference for items 6+.

## Validation reminders

Every change must preserve the validation criteria from
MULTIKERNEL-NOTES.md:
- Bit-for-bit sim_time match (hello 44 695 ns, memcpy 101 965 ns,
  cmark iter=1 2 007 365 ns)
- TEST PASSED in testbench output
- All 20 PDES unit tests pass (`cargo test --release --lib multikernel::`)
- No regression beyond noise on c910 hello/memcpy walls vs the current
  baseline (hello 77.3 s, memcpy 233.4 s)

## Anti-recommendations

Same as MULTIKERNEL-NOTES.md but worth repeating:

- **DO NOT** instrument `write_sig!` macro for fast-path predicates —
  several hot paths bypass it. Caused a TEST FAILED this session.
- **DO NOT** prune compiled edge blocks based on within-run exec_counts
  alone — error-handling paths may not fire until late. Use cross-run
  profile data (E3 compile cache) for safe persistent pruning.
- **DO NOT** try snap-skip — every clock toggle requires a snap.
- **DO NOT** enable any `XEZIM_PDES_*` flag besides `DISPATCHER=pdes`
  by default on c910. PAR_APPLY in particular is a measurable regression
  (NBA work too small to amortize std::thread::scope spawn cost).
