# JIT redesign — FFI-inline signal access

Design notes for item #9 in
[IMPROVEMENT-SUGGESTIONS.md](IMPROVEMENT-SUGGESTIONS.md). The current
JIT regresses c910 (Cranelift +11%, LLVM +5%) because every signal
load/store crosses a Rust FFI boundary (`xezim_jit_load_signal` etc.)
that costs ~10-20 ns per call. c910 RTL blocks are load-heavy (many
LoadSignal between few arithmetic ops), so FFI overhead exceeds the
dispatch savings.

This file documents the redesign: inline signal_table access into
JIT-generated code without crossing FFI. Estimated savings ~3 s on
hello LoadSignal alone, ~5-8 s with stores added, ~15-20 s with NBA
fast-path inlined.

---

## Problem statement

Current per-call cost (measured indirectly from JIT regression on hello):

| Insn | Current path | Cost | Interpreter equivalent |
|---|---|---:|---:|
| LoadSignal | FFI → `xezim_jit_load_signal` → `signal_table[id].to_u64()` | ~10-20 ns | ~2-3 ns (direct Vec index + Value field load) |
| BlockingAssign | FFI → `jit_store_signal` → dirty_id + write_sig! | ~20-30 ns | ~5-10 ns |
| NbaAssign | FFI → `jit_schedule_nba` → Vec push | ~20-30 ns | ~5-10 ns |

On hello: 571M total insns. If 30-40% are LoadSignal (≈200M calls),
the FFI overhead is 200M × 15 ns ≈ 3 s of pure overhead the
interpreter doesn't pay. Stores + NBAs add another 50M × 25 ns ≈ 1.3 s.

Total FFI overhead on hello: roughly **4-5 s of the +9 s JIT
regression**. The remaining gap is Cranelift's slightly heavier
generated code per arithmetic op vs the hand-tuned interpreter loop.

## Root cause: Value layout opacity

`xezim_core::value::Value`:

```rust
pub struct Value {
    storage: ValueStorage,   // enum { Inline {val,xz}, Wide(Vec<LogicBit>) }
    pub width: u32,
    pub is_signed: bool,
    pub is_real: bool,
}
```

To read `val_bits` from a Value, the JIT must:
1. Read the `ValueStorage` discriminant
2. Branch on it (Inline vs Wide)
3. If Inline: read `val_bits` from a known offset
4. If Wide: walk a Vec<LogicBit>

The discriminant + branch in JIT-generated code is awkward. The
current implementation punts to FFI (Rust does the discriminant
match), which is correct but slow.

For ≤64-bit signals (already gated by `block_signals_fit_u64`), Value
is always Inline. But the JIT still can't avoid the discriminant load
without committing to a specific Value memory layout.

## Proposed solution: parallel inline-bits storage

Add a new field on Simulator that holds the (val_bits, xz_bits) pair
for each signal in a layout the JIT can address directly:

```rust
/// Parallel storage to `signal_table`, packed for JIT-friendly
/// access: each entry is (val_bits as u128 low, xz_bits as u128 high).
/// Only valid for signals with width <= 64; wide signals have
/// undefined contents (the JIT refuses to compile blocks touching
/// wide signals via `block_signals_fit_u64`).
///
/// Maintenance: every write to `signal_table[id]` must also update
/// `signal_inline_bits[id]`.  The `write_sig!` macro is the canonical
/// write path; the parallel apply_nba path (which bypasses the
/// macro) needs explicit handling.  Same hazard as the prior
/// `nba_touched_edge_non_clock` tracker — but here the consequence
/// of missed writes is correctness, so the audit MUST be complete.
///
/// 16 bytes/signal × 36M signals = ~600 MB additional memory on
/// c910.  Justifiable if it saves 3-5 s of sim wall; alternative
/// is sparse per-block (hot signals only).
pub(crate) signal_inline_bits: Vec<u128>,

/// Cached pointer to signal_inline_bits.as_ptr().  JIT code reads
/// this from a known offset on `*sim` to get the base pointer
/// without an FFI call.  Refreshed whenever signal_inline_bits is
/// resized (only during elaboration on c910).
pub(crate) signal_inline_bits_ptr: *const u128,
```

### Codegen change for LoadSignal

Current (FFI):
```
call xezim_jit_load_signal(sim_ptr, id)  ; cost: ~15 ns
mov reg, return_value
```

Proposed (inline):
```
mov base, [sim_ptr + offset_of(signal_inline_bits_ptr)]   ; load cached pointer (1 cycle, cached after first use)
mov reg, [base + id * 16]                                  ; load val_bits (cost: ~2-3 ns)
```

The high u64 (xz_bits) is loaded only when needed (for X/Z checks),
saving an instruction in the common case.

### Width / sign extension

Currently `to_u64()` returns the val_bits already mask-truncated.
Inlined version emits Cranelift `band` with the width mask:

```
load r1, [base + id*16]
band r1, r1, mask(width)            ; (1u64 << width) - 1
```

For signed signals (rare on c910 for individual flop reads), emit
`sshl + sshr` for sign extension.

### Discriminant safety

The block_signals_fit_u64 gate ensures every signal touched by the
block has width ≤ 64. ValueStorage's Inline variant is the only
populated form for such signals. Reading the (val_bits, xz_bits) pair
without checking the discriminant is safe **IF the Value's memory
layout is stable** — meaning ValueStorage's Inline variant must be
laid out predictably across builds.

Two options:

**Option A — commit to a layout via `#[repr]`:**
```rust
#[repr(u8)]
enum ValueStorage {
    Inline { val_bits: u64, xz_bits: u64 } = 0,
    Wide(Vec<LogicBit>),
}
```
This makes the discriminant 1 byte at offset 0, val_bits at offset 8,
xz_bits at offset 16. Stable across builds. Requires touching
xezim-core.

**Option B — use the parallel storage approach (recommended).**
Don't touch Value's layout at all. Maintain a separate
`signal_inline_bits: Vec<u128>` that's by definition a flat layout
the JIT can index. Decouples the JIT from Value's internals
completely.

**Option B is the recommended path** because:
- Value's enum nature is intentional (Wide variant for >64-bit
  signals is a non-trivial space optimization)
- xezim-core is a separate crate with its own consumers; changing
  Value's layout for JIT's benefit is a long-distance coupling
- The 600 MB memory cost is acceptable on c910 (already running with
  ~5 GB sim footprint)
- Future read-by-pointer optimizations (SIMD diff, async trace
  writer) all benefit from a flat parallel array

### Maintenance contract

Every write to `signal_table[id]` MUST also update
`signal_inline_bits[id]` (or be guaranteed to touch only wide
signals, which the JIT doesn't read). Write sites in
src/compiler/simulator.rs:

1. `write_sig!` macro — extend to write both arrays atomically
2. Parallel apply_nba path (~line 9612) — already bypasses the macro
   for performance; manual update needed
3. JIT bridge `jit_store_signal` — calls write_sig! → automatic
4. `apply_delayed_updates` — calls write_sig! → automatic
5. Any other direct `signal_table[id] = val` site — audit required

**Same audit hazard that killed the `nba_touched_edge_non_clock`
write-tracker.** Difference here: missing a write causes a
correctness bug, not a missed optimization. Mitigation: build a
debug-only invariant check that re-derives inline_bits from
signal_table every iter and compares — fails fast on missed update.

### Estimated savings (hello)

Conservative model:
- 200M LoadSignal calls × 12 ns saved = 2.4 s
- 50M LoadSignalSigned similar = 0.6 s
- = 3.0 s on LoadSignal alone

With store inlining (BlockingAssign + NbaAssign):
- 50M store calls × 18 ns saved = 0.9 s
- = 3.9 s total

Combined with PGO + LTO + partial-NBA baseline of 66.0 s, target is
roughly 60 s on hello = additional −9% from current best.

Memcpy proportional (~3× the insn count): target 175 s vs current
189.6 s = −7%.

### Implementation cost

- Add `signal_inline_bits` field + maintenance: ~50 LOC across 8
  write sites + write_sig! macro + apply_nba parallel path
- Add Cranelift codegen for inline LoadSignal: ~80 LOC in jit.rs
- Add debug-mode invariant check: ~30 LOC
- Tests + benchmark: ~1 hour
- **Total: ~1-2 days for the first prototype (LoadSignal only)**

Full rollout (stores + NBAs + array indexing): 1-2 weeks.

## Why not LLVM JIT specifically

The LLVM backend (--features jit-llvm) regressed less (+5%) than
Cranelift (+11%), suggesting LLVM produces tighter code but pays
similar FFI cost. The redesign applies identically to both backends
— the FFI is the bottleneck, not the codegen quality.

If both backends are kept, the inline-bits redesign should be in
shared code (probably in `Simulator::signal_inline_bits` plumbing +
a `BridgeKind::Inline` vs `BridgeKind::Ffi` enum the codegen
respects).

## Stage 1 + 2 implementation status (post session-3)

Stage 1 (write-site maintenance) — **SHIPPED**, validated clean:
- `signal_inline_bits: Vec<[u64; 2]>` field on Simulator.
- `after_signal_write(id)` canonical helper called at all 13 direct-write sites.
- `write_sig!` macro maintains inline_bits inline.
- Parallel `apply_nba` path refreshes inline_bits on main thread post-join.
- `verify_inline_bits_invariant` debug check via `XEZIM_VERIFY_INLINE_BITS=1`.
- Validation on c910 hello: **invariant CLEAN across all 8 938 iters / 35 955 017 signals.**
- Default-path overhead: zero (signal_inline_bits stays empty when env unset).
- Maintenance-path overhead: ~1.8 s on hello (89M writes × 20 ns; 576 MB extra RAM).
- See commit `d33dee6 pdes: write-site refactor — canonical after_signal_write helper`.

Stage 2 (JIT inline LoadSignal codegen) — **PROTOTYPE SHIPPED**, validates the hypothesis:
- `JitModule.inline_bits_ptr: Option<(u64, u32)>` set via `set_inline_bits_storage`.
- Cranelift codegen for `Insn::LoadSignal` / `Insn::LoadSignalSigned`
  branches on `inline_bits_ptr`: when set + sid < len, emits a direct
  `load i64, MemFlags::trusted(), base, sid*16` instead of the FFI
  call to `xezim_jit_load_signal`. Falls back to FFI otherwise.
- `[JIT] backend=cranelift compiled N/M blocks (inline_bits=on)` line confirms.

**Measured on c910 hello (`XEZIM_JIT=1 XEZIM_INLINE_BITS=1`):**
| build | sim wall | TEST | Δ vs FFI JIT |
|---|---:|---|---:|
| Interpreter (no JIT) | 74.1 s | PASSED | — (interp baseline) |
| JIT FFI (Stage 0) | 91.7 s | PASSED | reference |
| **JIT inline LoadSignal (Stage 2)** | **89.3 s** | PASSED | **−2.4 s (−2.6%)** |

The −2.4 s win validates the design. Per-load FFI overhead removed for
the ~200M LoadSignal/LoadSignalSigned calls. The interpreter still wins
overall because the store paths (BlockingAssign, NbaAssign,
BlockingAssignRange, NbaAssignBitDyn, etc.) all remain FFI-mediated —
that's the next-stage work.

**Two integration bugs surfaced and fixed this session (notes for the
next implementer):**

1. **Allocation-order bug.** `signal_inline_bits` was originally
   allocated inside `build_comb_entries`, AFTER `compile_edge_blocks`
   ran. The JIT module saw an empty Vec and disabled inline codegen
   (logged `inline_bits=off`). Fix: allocate BEFORE
   `compile_edge_blocks`. Moved to the `simulator.rs` elaborate-init
   block right after `classify_always_blocks`.

2. **Duplicate-allocation pointer invalidation.** The same allocation
   block existed in two places (one before `compile_edge_blocks`, one
   inside `build_comb_entries`). The second one reallocated the Vec
   AFTER the JIT had baked the first pointer — SIGSEGV on the first
   inline LoadSignal. Fix: remove the duplicate.

3. **JIT signal_has_xz semantics.** Initial `after_signal_write`
   updated both `signal_inline_bits` AND `signal_has_xz`. Updating
   `signal_has_xz` accurately let the JIT execute MORE blocks (since
   the prelude's "may have X/Z" hint stopped being stale-conservative),
   which exposed latent JIT codegen bugs and broke sim_time. Fix:
   restrict `after_signal_write` to inline_bits only; leave
   `signal_has_xz` updates to `write_sig!` (its prior behavior).
   This is correctness-by-conservation — the JIT's existing safe
   regime depends on signal_has_xz staying stale on partial-bit
   writes.

## Stage 3 attempt — bridge NBA-elision (NEGATIVE result, reverted)

Tried adding eval-time NBA elision to `xezim_jit_schedule_nba` (and
range / bit_dyn variants) — same `if self.signal_table[id] == val { return; }`
pattern the interpreter Insn::NbaAssign uses, shipped this session.

**Result on c910 hello (`XEZIM_JIT=1 XEZIM_INLINE_BITS=1`):**
- Stage 2 baseline (no bridge elision): 89.3 s sim wall, nba_elided=84 349 412
- Stage 3 (with bridge elision): 90.4 s sim wall, **same** nba_elided=84 349 412

The bridge elision counter (`prof_nba_elided`) did not budge between
Stage 2 and Stage 3. Yet the bridge IS called for JIT'd-block NBAs.
Hypothesis: the JIT-path NBAs that hit the bridge have a high real-
change rate (the no-op fraction concentrates in interpreter-handled
blocks where the existing Insn::NbaAssign elision already fires).
The added per-call Value comparison cost (~5 ns × ~100M calls = 500 ms)
isn't offset by any new elisions, producing the +1.1 s regression.

**Lesson:** Bridge-level elision is the wrong layer for c910's NBA
pattern. The interpreter's elision in `Insn::NbaAssign` captures the
elision-eligible NBAs already (they live in blocks the JIT rejected).
To improve JIT NBA further, the next attempt should be **inlining the
NBA queue push entirely** in JIT codegen (eliminate the FFI call cost
for the real-change NBAs that DO need queuing). That's substantially
more work — Vec<NbaFast> mutation across the JIT boundary requires
either a thread-safe queue protocol or a more elaborate bridge that
batches.

Reverted. The bridge code is back to the Stage 2 state.

## Stage 4 — design + cost breakdown for next session

Remaining JIT vs interpreter gap on hello: 89.3 s − 74.1 s = **15.2 s**.
Stage 2 captured 2.4 s; Stage 3a-attempt confirmed bridge-level NBA
elision is the wrong layer (the elision-eligible NBAs concentrate in
interpreter-handled blocks).

**Cost breakdown of `xezim_jit_schedule_nba` (~70-100M JIT NBA calls
on hello):**
| Component | Per-call cost | Notes |
|---|---:|---|
| FFI bridge call overhead | ~10-20 ns | crossing JIT/Rust boundary |
| Bounds check + width compare + resize | ~3-5 ns | usually no-op (JIT pre-resizes) |
| `Value::from_u64` | ~2-3 ns | inline storage init |
| `val.is_signed = ...` | ~1 ns | byte write |
| `nba_fast_index.insert` | ~30-50 ns | **HashMap insert — dominant** |
| `nba_fast.push(NbaFast)` | ~5-10 ns | Vec realloc amortized |
| **Total** | **~50-90 ns** | per JIT NBA call |

The HashMap insert (`nba_fast_index`) is the dominant per-call cost,
not the FFI bridge. ~3 s of the JIT regression on hello sits there.

**Three implementation tiers for Stage 4 (in order of complexity):**

### Tier A — Leaner FFI bridge (3-day project)

Add `xezim_jit_schedule_nba_fast(sim, id, val_bits)` that assumes:
- id is in range (JIT validates at codegen time)
- val_bits is already width-masked (no resize)
- is_signed defaults to `signal_signed[id]` (precomputed)

Saves: ~5-10 ns per call × 100M = 500 ms - 1 s on hello.

JIT codegen for `Insn::NbaAssign`: when `width == signal_widths[sig_id]`
at compile time, emit call to `_fast` variant; else fall back to
existing bridge.

This is the easiest, lowest-risk Stage 4 increment. Real but modest win.

### Tier B — Replace `nba_fast_index` HashMap with dense Vec (1-week project)

Independent of JIT — also benefits the interpreter. The HashMap is
used to merge partial-bit NBAs into existing whole-value entries
(NbaAssignRange / BitDyn). Replace with `nba_fast_index: Vec<u32>`
sized to `num_signals`, where `u32::MAX` means "no current entry".

Memory cost: 4 bytes × num_signals = 144 MB on c910. Possibly too
much; sparse alternative needed.

Or: lazy/sparse — keep HashMap but use it only at apply time, build
on the fly when needed. Defer the cost from per-NBA to per-apply.

Saves: ~30 ns × 100M = ~3 s on hello (both interpreter and JIT).

### Tier C — Inline NBA queue push entirely (multi-week project)

**Revised cost model after Tier A+B shipped (session-3):**

Tier B (`nba_fast_index` HashMap → dense Vec) already captured the
30-50 ns/call HashMap insert cost that was the original Tier C
motivator. The current Tier 4 Tier A+B path has per-call cost:

| Component | Per-call cost | Saved by Tier C? |
|---|---:|---|
| FFI bridge call overhead | ~10-20 ns | yes (inline) |
| `Value::from_u64` | ~3-5 ns | yes (write val_bits direct) |
| `signal_signed` load + set | ~2 ns | partial (bake constant) |
| `nba_fast_index.insert` (now dense Vec) | ~5-10 ns | partial (inline write) |
| `nba_fast.push(NbaFast)` | ~5-10 ns | partial (manual atomic len) |
| **Total** | **~25-50 ns/call** | revised Tier C win |

**Revised Tier C expected win:** ~3-5 s on hello (was ~5-7 s in the
original estimate; Tier B captured ~3 s of that).

#### Implementation paths (in order of layout safety)

**Path C1 — Side queue + batch transfer (safest, 2-3 days):**

JIT writes to a parallel `Vec<(u32, u64)>` (sig_id, val_bits) using
raw pointer + atomic length counter. Vec pre-allocated to large
capacity (e.g. 1M entries × 12 bytes = 12 MB) and never resized.

Before `apply_nba`, an FFI bulk-transfer drains this side queue into
`nba_fast` (converting (u32, u64) → NbaFast). One FFI call per iter
instead of per NBA write.

Pros: NbaFast layout fragility avoided. Code change isolated.
Cons: Batch transfer is itself O(N) per iter. Net win = ~80% of
per-call FFI saving = ~2-3 s on hello.

**Path C2 — Direct NbaFast field write (faster, layout-fragile,
1-2 weeks):**

Original Tier C design. Pre-allocate `nba_fast` to fixed capacity.
JIT writes NbaFast fields directly:
- offset 0: signal_id (u64)
- offset 8: Value::storage discriminant + Inline { val_bits, xz_bits }
- offset 32: width (u32)
- offset 36: is_signed, is_real (u8 each)
- offset 40: block_index (u32)

Saves the FFI call AND batch-transfer overhead. ~3-5 s on hello.

Risk: any change to `Value` or `NbaFast` layout in xezim-core silently
breaks JIT correctness. Mitigations:
- `#[repr(C)]` on NbaFast and ValueStorage::Inline
- Compile-time `assert_eq!` of field offsets via const fn
- Debug invariant check that re-reads a sampled slot via interpreter
  path + compares with what JIT wrote

#### Recommended sequencing

1. **Path C1 first** (side queue + batch transfer). 2-3 day project,
   ~2-3 s win, low correctness risk. Validates the inline-write
   architecture without committing to NbaFast layout assumptions.
2. **Path C2 only if C1's batch-transfer overhead is too high** —
   measured by profiling the batch step. If <500 ms per iter, C1 is
   sufficient. If 500ms+, the inline write is worth the layout audit.
3. **Either way, expect the JIT-vs-interpreter gap to close to ~5-8 s**
   on hello after Tier C. Further closure needs LLVM JIT or
   codegen-level improvements (LLVM's tighter codegen + PGO on the
   JIT path itself).

## Codegen-level finding (session-3 end): LLVM > Cranelift even without Stage 2-4 ports

Built with `--features jit,jit-llvm` and ran hello with
`XEZIM_JIT_BACKEND=llvm`:

| Build (XEZIM_JIT=1)               | hello sim wall | compile | TEST |
|-----------------------------------|---------------:|--------:|------|
| Cranelift Stage 0 (FFI)           |         91.7 s |     5 s | PASSED |
| Cranelift Stage 2 (inline LoadSig)|         89.3 s |     5 s | PASSED |
| Cranelift Stage 4 Tier C          |         84.3 s |     5 s | PASSED |
| **LLVM (FFI baseline)**           |     **79.3 s** |    44 s | PASSED |
| Interpreter (no JIT)              |         69.7 s |       — | PASSED |

**LLVM with FFI-only bridges beats Cranelift+all-Stage-4 by 5 s on
hello.** The codegen-quality difference is bigger than any of the
FFI-elimination tiers shipped.

Per-phase comparison (hello):
- Cranelift Tier C: edge_detect 11.1 s, edge_exec 50.1 s, ns/insn 87.7
- LLVM FFI:         edge_detect 11.1 s, edge_exec 45.8 s, ns/insn 80.2

LLVM saves 4.3 s on edge_exec (bytecode arithmetic + dispatch).
edge_detect identical (not in JIT path).

### Compile-time tradeoff

LLVM JIT compilation: 44 s vs Cranelift's 5 s.

| Workload     | Cranelift + Stage 4 total | LLVM FFI total       |
|--------------|--------------------------:|---------------------:|
| hello        | 35 s elab + 84 s sim = 119 s | 35 s elab + 44 s LLVM + 79 s sim = 158 s |
| memcpy (est) | 35 s + 234 s = 269 s         | 35 s + 50 s LLVM + ~210 s sim = ~295 s |
| cmark (est)  | 30 s + ~4500 s = ~4530 s     | 30 s + ~60 s LLVM + ~3900 s sim = ~3990 s |

**LLVM only pays off on cmark-class workloads.** On hello/memcpy the
Cranelift compile-time advantage outweighs LLVM's sim-wall savings.

### Recommended sequencing (revised)

1. **Default JIT backend stays Cranelift** for fast iteration on
   short tests (hello, memcpy). Already opt-in via `XEZIM_JIT=1`.
2. **Document `XEZIM_JIT_BACKEND=llvm` as the long-run optimization**
   — explicitly opt in for cmark-class workloads where the 44 s
   JIT compile is amortized.
3. **If JIT-codegen-quality is the goal, port Stage 2-4 to LLVM**
   (~1 week). Would put LLVM at an estimated 73-74 s on hello,
   essentially matching interpreter's 69.7 s. Tier B's
   `nba_fast_index` HashMap→Vec ports trivially (same simulator
   data structure). Tier A's lean bridge ports (LLVM's emit_insn
   doesn't currently have Stage 4 branching logic).  Stage 2
   inline LoadSignal needs LLVM-side baked pointer support.
4. **PGO on the JIT path itself** (LLVM has native PGO support
   for JIT'd code via `profile-instr-generate`) — never tried for
   xezim. Could give another 5-10% on the JIT'd codepath.

### Recommended sequencing

1. **Tier A first.** Bounded, low-risk, real win. Use as the next
   session's first piece.
2. **Tier B next.** Independent of JIT; benefits both paths. May
   require sparse HashMap-vs-Vec hybrid to avoid 144 MB cost.
3. **Tier C only if A+B don't close enough of the gap.** The
   layout-fragility makes it expensive to maintain across xezim-core
   updates.

### What NOT to do

- **DO NOT** retry Stage 3a (bridge NBA elision). Negative result
  documented above — wrong layer.
- **DO NOT** combine with hot-signal-arena (C1) in the same session.
  The arena's sid renumbering invalidates baked JIT pointers (same
  as the Vec-realloc bug surfaced this session). Land them
  separately and re-bake the JIT.
- **DO NOT** enable `--features jit` in default builds without
  Stage 2+ improvements. Current default-build performance (interp
  at 74.1 s on hello, post LTO+PGO 66 s) beats anything JIT can
  deliver until Stage 4 closes the gap.

## Stage 1 partial implementation — what we learned (session-2)

Attempted the maintenance-only step: add `signal_inline_bits:
Vec<[u64; 2]>`, instrument `write_sig!` macro + 5 known direct-write
sites (BlockingAssign inline paths, settle FastDirectCopy/DirectCopy,
parallel apply_nba). Add `XEZIM_VERIFY_INLINE_BITS=1` invariant check
that walks all signals each iter and reports mismatches.

**Result on c910 hello:**
```
[INLINE_BITS_MISMATCH] iter=0 sid=38 table=(v=0x1, x=0x0) inline=(v=0x0, x=0x1)
[INLINE_BITS_MISMATCH] iter=0 total_mismatches=16291/35955017
[INLINE_BITS_MISMATCH] iter=1000 total_mismatches=147510/35955017
```

16 291 mismatches at iter=0 (time-0 settle bypasses some instrumented
paths), growing to 147 510 by iter 1000. ~0.4% of signals diverge per
1000 iters. Confirms the audit hazard.

**Inventory of write sites the partial instrumentation missed:**

```bash
$ grep -n "signal_table\[.*\]\.set_bit\|signal_table\[.*\] = " src/compiler/simulator.rs
# 17 hits across 12 distinct write contexts
```

Missing instrumentation includes:
- `Insn::BlockingAssignBitDyn` (single-bit write via set_bit, line ~6574)
- `Insn::BlockingAssignRange` wide path (set_bit loop, line ~6633)
- `Insn::BlockingAssignRangeDyn` wide path (set_bit loop, line ~6699)
- `Insn::BlockingAssignArrayRange` wide path (line ~6850)
- A handful of NBA paths writing via set_bit (lines 11932, 12035)
- JIT bridge `jit_blocking_assign_range` (line 16496) — uses set_bit
- `set_bit_code` mutator at line 16189
- (Probably more in less-trafficked code paths)

**Lesson:** The audit cannot be done piecemeal across the existing
codebase. The correct path is **refactoring all signal_table writes
to go through ONE canonical helper** (analogous to `write_sig!`
macro but covering set_bit / set_inline_bits paths too). That
refactor is itself a 1-day project before the inline-bits
maintenance can layer on safely.

**Alternative considered: lazy refresh.** Instead of maintaining
`signal_inline_bits` synchronously with every write, refresh it
lazily before each settle / check_edges call by walking
`dirty_list`. Eliminates write-site audit at the cost of a per-iter
walk. Maybe a useful intermediate step — needs benchmarking.

## Recommended next steps

1. **First: refactor all signal_table writes to go through a single
   `Simulator::write_signal(id, val)` helper.** This is the
   prerequisite the JIT-redesign Stage 1 attempt revealed. Estimated
   1 day. Validation: run hello with current code unchanged, plus
   the helper in place — must produce bit-identical sim_time.
2. **Then: land the parallel `signal_inline_bits: Vec<u128>` storage**
   (no JIT change yet). Verify maintenance via debug invariant
   check on hello. Memory cost: 576 MB on c910. If acceptable, ship
   it as a no-op preparation step.
2. **Implement inline LoadSignal codegen** in Cranelift first.
   Re-measure JIT-on hello. Target: at least neutral (not regression).
   If still regresses, the LoadSignal cost wasn't the real bottleneck
   and the project is uneconomical.
3. **If step 2 wins**, extend to LoadSignalSigned (trivial),
   BlockingAssign + NbaAssign (more involved — need to handle dirty
   tracking and queue management).
4. **If step 2 loses**, the regression is elsewhere (Cranelift
   register allocation, hot-path branches, etc.). Profile with perf
   to find the real bottleneck before further investment.

## Why I'm not implementing this in this session

Time budget. The session has already shipped:
- NBA-elision (simple + partial): −10% / −11%
- LTO + matched PGO: cumulative −20% / −19%

Item #9 requires:
- A 50-LOC write-tracker that's correctness-critical (worse hazard
  than the failed `write_sig!` tracker)
- A debug invariant check to catch silent bypass
- Cranelift codegen knowledge for the inline path
- Multiple build+measure cycles to validate

That's ≥1 day. The shipped wins this session are larger, faster, and
correctness-safer. Punting #9 to a follow-up session that can
dedicate proper time to the audit is the right call.

If a future session wants to start #9, the first concrete step is:
add the `signal_inline_bits` field and a debug-only invariant check.
Run hello + memcpy + cmark with `XEZIM_VERIFY_INLINE_BITS=1` to
prove the maintenance is complete BEFORE touching the JIT codegen.

## Falsifying this design

Two things would invalidate the analysis:

1. **LoadSignal isn't actually the FFI bottleneck.** If profiling
   shows the FFI cost is elsewhere (e.g. dirty tracking on stores),
   the read inlining wins little. Test: instrument xezim_jit_load_signal
   with a per-call wall-time counter and compare to total JIT block
   exec time.

2. **Cranelift's generated arithmetic is the bottleneck.** If
   inlining LoadSignal halves load cost but block-total exec time
   barely moves, the arithmetic between loads is what's slow. Then
   the right path is either (a) LLVM JIT with PGO data, or (b)
   keeping the bytecode interpreter and just driving more PGO.

Either falsification kills the redesign cleanly and saves the
follow-up weeks of work. Recommended to do both measurements before
committing.