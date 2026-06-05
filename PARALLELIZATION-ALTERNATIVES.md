# Parallelization alternatives for the c910 sim — evaluation

Context: the per-LP (core0/core1) PDES cut is now functionally correct but
does NOT speed up (IN-PLACE-SETTLE-SCOPE.md). Root reason: the loop's
fine-grained per-tick structure — each tick's edge exec (~55%) and comb settle
(~40%) are split into MANY small fired-block / cone batches, so any per-tick or
per-batch sync/spawn cost dominates (measured: scoped 65.4s ≈ persistent pool
65.3s ≈ sequential 67.0s). So the key question for ANY alternative is: does it
AVOID per-tick fine-grained sync?

Profile (memory): edge-block exec 55.6%, comb settle 39.9%, serial 4.5%.
~585k signals, ~438k comb entries, ~20k edge blocks, ~4500 clocked ticks (hello).

---

## A. Reduce the work (single-thread) — sidesteps the granularity wall entirely

These attack the PER-ENTRY interpreter cost, which multiplies through BOTH the
edge 55% and settle 40%. No threading, no sync, no granularity issue. For an
interpreter this is usually the highest ROI.

### A1. JIT to native code
UPDATE (2026-05-29, CURRENT measurement, build --features jit + XEZIM_JIT=1):
the existing cranelift JIT is SLOWER than the current interpreter, not faster.
  interpreter: edge_exec 34.1s, 59.7 ns/insn, sim 64.7s
  JIT cranelift: edge_exec 46.8s, 81.9 ns/insn, sim 81.4s (15040/20779 blocks
                 compiled, TEST PASSED) — +37% ns/insn, +26% wall.
ROOT CAUSE: the JIT bridges EVERY signal read/write through FFI into Rust
(~10-20ns/call; see jit.rs header). c910 blocks are signal-access-dense, so
the FFI overhead swamps the dispatch savings. The JIT's older logs looked
break-even only because the interpreter was ~80 ns/insn THEN; since, the
interpreter was optimized to ~60 (inline-bits, prefetch, fast paths) and the
JIT (still FFI-bound ~80-89) fell behind. The `inline_bits=on` direct-read
attempt was WORSE (88.8 ns/insn), so partial direct-read didn't fix it.
POTENTIAL: native code with FULL direct signal-table access (no FFI, no
per-op Value construction) could plausibly reach ~20-30 ns/insn (~2-3x over
the interpreter) — the dispatch + Value churn the interpreter still pays would
vanish. But realizing it is a SIGNIFICANT redesign: generate native code that
(a) reads/writes signal_table memory directly with the Value layout
(val+xz/width/inline-vs-wide), (b) does dirty tracking in native code, (c)
handles wide/X cases inline — i.e. reimplement the interpreter's signal-access
fast paths in codegen. VERDICT: highest POTENTIAL single-thread lever, but the
current JIT must be redesigned (eliminate FFI-per-signal-access) to beat the
now-fast interpreter; that's a focused multi-week effort with real but
unproven payoff. Do NOT enable the current JIT (it regresses ~26%).

PROFILING (2026-05-29, perf on the JIT run): the #1 hot function IN THE JIT
RUN is the INTERPRETER `exec_insns` (~11%), because the JIT compiles only EDGE
blocks (~72%) — ALL comb settle (~40% of the loop) + 28% of edge stay
interpreted. So even a perfect edge-JIT caps at the edge fraction, and the
measurement shows the JIT'd edge isn't even beating the interp. Two further
obstacles to optimizing the JIT here: (1) runtime-generated JIT code is
UNSYMBOLIZED (perf shows it as scattered anonymous addresses, not a hot
function) so it can't be cleanly profiled; (2) `perf` attach-mode is
restricted in this env (paranoid), so steady-state isolation is hard. NET: to
make A1 pay off needs (a) tight native codegen that beats 60 ns/insn (no FFI,
no per-op Value construct), (b) JIT the comb settle too (not just edge), and
(c) a profilable dev environment. This is a real project, not an incremental
tweak; making blind codegen changes here (unprofilable + 2min build + 80s run
per iteration) would be thrashing.

ENGINEERING ATTEMPT (2026-05-29) — measured 4 JIT configs on hello + a fix:
  interpreter                       59.7 ns/insn / 64.7s
  JIT FFI-reads (inline_bits off)   81.9 / 81.4s
  JIT direct-reads (inline_bits on) 79.5 / 79.6s   (inline_bits also activates
                                    nba_side_queue, so NBA WRITES skip FFI too)
  JIT direct-reads + opt=speed      80.7 / 80.9s
FIX LANDED: cranelift was built at DEFAULT opt_level=none; set it to "speed"
(jit.rs). Did NOT help c910 (codegen quality wasn't the bottleneck) but is the
correct JIT default + latent win for large-block designs.
AIRTIGHT CONCLUSION: with reads inline, writes inline, AND opt=speed, the JIT
is STILL ~79-81 ns/insn vs the interpreter's 59.7 (~33% slower). The only
remaining per-fire FFI is the X-precheck (~5 ns/insn max). So the JIT'd native
body + per-fire function-call dispatch is simply NOT faster than this
interpreter on c910's TINY edge blocks (~6.4 insns/fire): the interp is already
extremely tight, and at ~6 insns/block the per-fire dispatch + precheck negate
any native-body savings. A1 does not pay off on c910 without a rearchitecture
that amortizes per-fire overhead over MUCH larger units (JIT whole ticks /
block groups) — major + speculative. Do not enable the JIT for c910.

### A2. 2-state fast path (post-reset X-free)
UPDATE (2026-05-29, verified in xezim-core/src/value.rs): the op-level 2-state
fast path is ALREADY IMPLEMENTED. add/sub/mul/div (l.525+), bitwise_and/or/xor
(l.657+), is_equal (l.900+) all branch on `xz==0` (or has_xz()) and take a
plain u64 path with no X-mask work. So "skip xz_bits in the ops" has NO
headroom left — it's done. The remaining per-op cost is NOT X-handling; it is
(1) VM dispatch (exec_insns match per insn) and (2) Value object churn — each
op constructs/returns a ~32-byte Value (storage enum + width + flags) and
LoadSignal clones it. Those are removed by A1 (JIT), not by a 2-state op path.
The ONLY remaining 2-state angle is STORAGE footprint: signal_table is
Vec<Value> (~32 B x 585k = ~18 MB; profile says it misses L3). A packed
read path (the SoA `signal_inline_bits` [val,xz] = 16 B/signal, already
maintained for the JIT under XEZIM_INLINE_BITS) halves the hot footprint —
but wiring the interpreter's LoadSignal to read from it is JIT-adjacent infra,
not a standalone op tweak. VERDICT: A2-as-scoped is already done; the real
remaining single-thread win is A1 (JIT removes dispatch + Value churn) with the
SoA packed-read as a shared sub-component.

### A3. Less settle work — event-driven / NBA elision
Already partly done (nba_elide, dirty-cone incremental settle). Further:
coalesce redundant re-evaluations, skip entries whose inputs are bit-stable,
better topo ordering to reduce chaotic-iteration passes. VERDICT: incremental
single-thread wins; lower ceiling than A1/A2 but free of parallel risk.

---

## B. Coarse-grained parallelism — beats the granularity wall

### B1. Test-suite / multi-program parallelism — TRIVIAL near-linear
If the goal is a regression SUITE (hello, memcpy, cmark, ...), run each test in
its own process on its own core: embarrassingly parallel, ~Nx on N cores, zero
shared state, zero risk. Does NOT speed a single sim, but for CI/regression
throughput it is the best ROI by far. VERDICT: do this for suite throughput;
it's the answer most "make the sims faster" asks actually want.

### B2. Decouple IO/trace from compute (producer-consumer)
With VCD/xtrace ENABLED (the debugging use case), trace formatting + zstd
compression can be 20-40% of wall time and is currently inline/serial. Move it
to a dedicated thread: the compute loop pushes per-cycle signal-delta buffers
to a channel; an IO thread formats/compresses/writes. ONE coarse boundary, big
batched chunks → no granularity wall. VERDICT: real, low-risk win WHEN tracing
is on (the common debug scenario); ~no benefit for untraced runs.

### B3. K>1 lookahead at a REGISTERED seam — the only viable spatial-parallel
The per-LP cut failed partly because the core0/core1 boundary is comb-consumed
(zero lookahead → per-tick sync → granularity wall). A cut at a REGISTERED
seam (e.g. the L2/BIU AXI master register stage, or a clock-domain-crossing
synchronizer) has >=1 cycle lookahead, so each LP can run K ticks between
syncs — amortizing the sync over K ticks instead of paying it every tick. This
is the original conservative-PDES vision. VERDICT: the only spatial parallelism
that can beat the granularity wall, but requires (a) finding a registered seam
that actually splits the COMPUTE (the L2/mem interface may not — most compute
is in the two cores sharing one clock), and (b) lookahead-correct boundary
protocol. High effort, uncertain payoff (the two cores still share the core
clock → the registered seam is between cores+L1 and the shared L2, which may
not balance the load). Worth a bounded feasibility probe (locate the BIU
register stage, measure compute on each side) before building.

---

## C. Fine-grained parallelism — DEFEATED by granularity (do not pursue)

- C1. Per-LP (core0/core1) per-tick edge+settle — MEASURED dead (this effort).
- C2. Per-block / per-cone within a tick — same wall; the existing scoped +
  persistent-pool edge dispatch already measured ~0.
- C3. Per-tick wavefront/level-parallel settle — steady-state cones are ~160
  signals; splitting across threads with a per-level barrier loses to sync.

These all pay sync per small per-tick batch; the per-tick work is too small and
too uneven. No amount of harness engineering changes the granularity.

---

## D. Other / not applicable

- D1. SIMD/vectorization of wide-Value ops + batched narrow-signal ops:
  intra-op data parallelism, no sync — a modest single-thread win on
  wide-bus-heavy code; stacks with JIT. Worth it only after JIT.
- D2. GPU / gate-level mapping: c910 is RTL with complex always-blocks +
  4-state + memory + DPI/testbench — not amenable to gate-per-thread GPU
  mapping without a full gate-level re-elaboration. Not viable here.

---

## Ranked recommendation (revised after measuring A1 + A2)

Reality check: the interpreter is MATURE (60 ns/insn). Every "easy" lever is
already captured or defeated. Remaining options, by realistic value:

1. **B1 test-suite parallelism** — TRIVIAL near-linear for regression
   throughput, zero risk. The one clear, immediate win (for the suite, not a
   single sim).
2. **A1 JIT REDESIGN (FFI-free direct signal access)** — highest single-sim
   POTENTIAL (~2-3x) but the current JIT is SLOWER than the interpreter (+26%
   wall, FFI-bound); needs a significant codegen redesign. Multi-week, real
   but unproven payoff.
3. **B2 IO/trace decoupling** — real win only when VCD/xtrace is enabled.
4. **B3 K>1 lookahead at a registered seam** — only viable in-sim spatial
   parallelism; high effort + uncertain load balance.
- (~done) **A2 2-state op fast path** — already implemented at op level.
- (avoid) **A1 current JIT as-is** — measured SLOWER; do not enable.
- (avoid) **C fine-grained spatial** — measured dead.

META-FINDING: this simulator is already heavily single-thread-optimized;
there is no easy 2x available. The single-sim speedup requires the JIT
redesign (major); broad throughput is best won by B1 (run tests in parallel).

Bottom line: for a SINGLE c910 sim, the speedup is in REDUCING per-entry work
(JIT, 2-state) — not in threading the fine-grained per-tick loop. Threading
pays off only at coarse boundaries (whole tests, IO, or K-tick lookahead at a
registered seam).

================================================================================
## OTHER METHODS — work-reduction (NOT parallelism / NOT JIT)

Parallelism (per-LP, B3) and JIT (A1) both LOSE to the fine-grained per-tick
structure + the already-tight interpreter. The real headroom is REDUCING the
work. Grounded in measured data (hello, current build):
  edges_fired=89.5M, insns=571M, edge_exec=34.1s (55%)
  nba_elided=84.3M  <-- 94% of edge fires compute a value that DOESN'T CHANGE
  settle=15.3s (incremental already), quiescent_skips=0 (prior attempt inert)

### O1 — IMPLEMENTATION SPEC (worked out 2026-05-29; correctness-critical)
Conservative gate (SAFE): on the main clock's posedge, skip a flop block iff
NONE of its non-clock data inputs changed since that clock's previous posedge
(then Q is provably unchanged). Pieces:
1. Per-edge-block DATA-READ set: extract LoadSignal/LoadSignalSigned sids from
   compiled_edge_blocks[bi].instructions (as try_compile_with_xz does), MINUS
   the block's sensitivity signals (clk/rst). Mark a block NON-gateable (always
   fire) if it has any DYNAMIC read (LoadArrayElem) — can't know its reads
   statically. Store edge_block_data_reads[bi] + edge_block_gateable[bi].
2. CHANGE ACCUMULATOR `recent_change: Vec<bool>` + list. MUST be COMPLETE —
   set on EVERY signal write. NOTE the gotcha: write_sig! does NOT call
   after_signal_write; writes happen via (a) write_sig! macro, (b)
   set_inline_bits fast paths, (c) after_signal_write, (d) direct assigns.
   Hook ALL of them (or, cleaner: funnel writes through ONE primitive first).
   An incomplete hook => false skip => SILENT WRONG RESULTS. (signal_has_xz is
   already set per write_sig! — a hint that a per-write hook is reachable.)
3. MAIN CLOCK: the posedge edge-signal with the largest flop fanout. Gate ONLY
   its posedge fanout; clear recent_change right after gating it (so the
   accumulator spans posedge-to-posedge). Other clocks' flops always fire
   (single global accumulator can't track multiple clock intervals — safe).
4. INIT: recent_change all-true for the first posedge (everything fires once).
5. VALIDATE: hello+memcpy TEST PASSED + edge_exec drop + nba_elided drop.
   Build the SAFE measurement-only version FIRST (count would-skip, don't
   skip) to confirm would-skip ≈ 94% before enabling the actual skip.
Gated behind an env flag; default path one branch in the write hook.

### O1. EVENT-DRIVEN EDGE / activity gating  *** HIGHEST untapped potential ***
94% of clocked-flop fires produce no change (D==Q): the block runs all its
insns, computes D, then the NBA is elided because D==Q. The COMPUTATION is
wasted. Cause: the edge layer is CLOCK-driven (every posedge-clk flop fires
every clock edge) while the comb layer is already EVENT-driven (dirty cone).
Fix: make the flop layer DATA-event-driven — fire a flop only when its data
cone changed since its last clock. Mechanism: accumulate the "changed since
last clk edge" signal set (the comb settle already produces dirtied signals),
and on a clk edge fire only flops whose data-input signals are in that set;
skip the rest (their Q is unchanged). Needs a flop -> data-input-signal map +
careful reset/enable handling. POTENTIAL: up to ~10x on the edge 55% if the
94%-idle holds (a CPU running a small program leaves most of the chip — FPU,
vector, L2, debug, unused paths — idle every cycle). This is the single
biggest lever and it's grounded in hard data, not speculation. (quiescent_skips
=0 shows the existing coarse attempt doesn't catch it; the per-flop
data-activity granularity is what's needed.) Effort: substantial (a real
event-driven flop scheduler) but bounded, single-thread, no granularity wall.

### O2. Signal-table cache-footprint reduction
signal_table is Vec<Value> (~32 B x 585k = ~18 MB) and the profile shows it
misses L3 — this is the LIKELY reason even the JIT loses (the interp prefetches
signal reads; the JIT'd code doesn't, and both fight an 18 MB working set).
Shrink the hot path: store 2-state narrow signals in a dense u64/u32 array
(8/4 B) with wide/X signals in a side table, and/or renumber signal ids by
block-access affinity so co-read signals share cache lines. Smaller working set
-> fewer L3 misses -> faster interp AND any future JIT. Medium effort, helps
everything. (This is the storage half of "A2 2-state" — the op half is done.)

### O3. Bytecode optimization (fewer insns per block)
571M edge insns x ~60 ns. CSE / DCE / constant-folding / peephole on the Insn
streams + superinstructions (fuse LoadSignal+op+Store, like the comb FusedGate)
to cut insn count and dispatch. Direct multiplier on both interp and JIT; the
~6.4 insns/fire average may have removable redundancy. Medium effort.

### O4. (lower) whole-design 2-state post-reset; SIMD batching of narrow ops
- After reset (X resolved), switch to an 8-byte 2-state signal store + simpler
  ops (no xz). Big/invasive; overlaps O2. Boot stays 4-state (the X window).
- SIMD/batch homogeneous narrow-signal ops (e.g. many 1-bit gates) — modest,
  after O2/O3.

### Ranked (work-reduction): O1 event-driven edge >> O2 cache footprint ~
O3 bytecode opt > O4. O1 alone could beat every parallel/JIT lever combined,
because it attacks the 94% measured waste directly and single-threaded.
