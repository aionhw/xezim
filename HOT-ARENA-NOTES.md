# Hot-signal arena — design notes

Design doc for item C1 in
[IMPROVEMENT-SUGGESTIONS.md](IMPROVEMENT-SUGGESTIONS.md). The
single-thread interpreter on c910 is partly bottlenecked by L3 cache
misses against a scattered ~1.1 GB `signal_table`. Packing the ~1%
of signals that are actually hot into a contiguous arena would
massively improve cache locality. This file documents the design and
the audit hazards. Implementation deferred to a dedicated session.

---

## The opportunity (measured this session)

c910 hello, post-elaboration:

```
[HOT_STATS] total_signals=35955017 hot_signals=388833 (1.08%)
            sizeof(Value)=32B
            total_table=1097.3MB
            hot_arena=11.9MB
            cold=1085.4MB
```

- **1.08% of signals are hot** (touched by edge sensitivities, comb
  reads, comb writes, or are clock outputs).
- **11.9 MB hot arena fits in L3** on modern CPUs (typical L3 is 16–32 MB).
- Cold signals (33.5M mostly memory-array elements) account for 99% of
  the table but are rarely accessed during normal CPU simulation.

Expected impact on hello (sim wall 66–67 s post-LTO+PGO):
- ~1.7 B signal accesses across 571 M insns.
- Current LLC-miss rate: estimated 30–50% on random access patterns
  against 1.1 GB.
- After arena: <10% LLC-miss rate (hot working set in L3).
- Per-access savings: ~5–10 ns × ~0.3 miss-rate-reduction × 1.7 B
  accesses ≈ **2–5 s on hello**, proportional on memcpy and cmark.

Memory-pattern math is approximate; could be larger if the cold
region is currently warm in DRAM but evicting hot cache lines.

## Design — sid renumbering at end of elaboration

The cleanest implementation renumbers signal IDs so hot signals occupy
sid 0..hot_count. The CPU's natural Vec indexing (`signal_table[id]`)
then lands hot accesses contiguously in memory.

### Scope: named region only

The current sid layout already has named signals at sid 0..named_count
(~2.4 M on c910) and unnamed array elements at sid named_count..total
(~33.5 M). The remap is constrained to the **named region** only —
array elements stay at their current sids.

Memory: named region is 2.4 M × 32 B = ~77 MB, larger than L3.
Reordering to put hot named signals first makes the hot 11.9 MB subset
contiguous at the start.

### The array-contiguity constraint

`array_first_id[name] = (first_sid, lo, hi)` semantics require that
all elements of an array occupy consecutive sids. If we remap
individual element sids, the array breaks.

**Constraint:** treat each named array (where elements have explicit
names like `regfile[0]`, `regfile[1]`, etc.) as an atomic block. The
whole block moves together. If any element of an array is hot, move
the whole array to the hot region; else keep it in the cold region.

Detection: walk `array_first_id` at remap time, mark sids in each
array's range as "array-member of array X". The remap algorithm
processes:
1. Standalone sids (not in any array): individually classified as
   hot/cold.
2. Array blocks: classified as a unit (hot if any member is hot).
3. Output order: hot standalone + hot arrays first, then cold
   standalone + cold arrays.

### Audit checklist — every sid reference

This is the audit hazard that requires care. Each of these must be
remapped or proven not to need remapping. Missing one = silent
correctness bug (sim_time will drift, like the failed `write_sig!`
tracker incident documented in MULTIKERNEL-NOTES.md).

**Parallel arrays indexed by sid:**
- `signal_table: Vec<Value>` ✱
- `signal_widths: Vec<u32>` ✱
- `signal_signed: Vec<bool>` ✱
- `signal_real: Vec<bool>` ✱
- `signal_has_xz: Vec<u8>` ✱
- `prev_val: Vec<u64>` (sized to named_count — safe if remap stays
  within named region)
- `prev_xz: Vec<u64>` (ditto)
- `sdf_delays: Vec<u64>` ✱
- `signal_lp_writer: Vec<Option<u32>>` ✱
- `is_edge_signal_non_clock: Vec<bool>` ✱
- `dirty_signals: Vec<bool>` (should be all false at remap time — clear
  + permute if not)
- `id_to_name: Vec<Arc<str>>` ✱

**HashMaps keyed by sid (or by Arc<str> with sid values):**
- `signal_name_to_id: HashMap<Arc<str>, usize>` — rebuild after
  id_to_name permutation
- `array_first_id: HashMap<Arc<str>, (usize, i64, i64)>` — remap the
  usize per the array-block rule above

**Vec<usize> containing sids:**
- `edge_signal_ids` — remap values, then re-sort + dedup
- `comb_unresolved_idx` — entry indices, not sids; UNCHANGED
- `comb_time0_idx` — entry indices; UNCHANGED
- `dirty_list: Vec<usize>` — should be empty at remap time
- `settle_dirty_ids: Vec<usize>` — should be empty at remap time

**Vec/struct fields with embedded sids:**
- `edge_blocks[i].resolved_sensitivities[j].signal_id` — remap each
- `edge_blocks_by_sig: Vec<EdgeFanout>` — parallel to edge_signal_ids;
  rebuild from scratch since edge_signal_ids changes order
- `comb_entries[i].read_signal_ids: Vec<usize>` — remap values
- `comb_entries[i].write_signal_ids: Vec<usize>` — remap values
- `comb_dep_offsets: Vec<u32>` — indexed by sid → REBUILD after
  permutation (resort signals + recompute CSR)
- `comb_dep_entries: Vec<u32>` — entry indices, UNCHANGED
- `clock_generators[i].signal_id` — remap
- `clock_generators[i].edge_signal_position` — rebuild (depends on
  new edge_signal_ids order)
- `event_waiters[i].resolved_sensitivities` — at remap time should be
  empty/initial; remap if not
- `delayed_updates` — runtime state, empty at remap

**Bytecode Insns embedded in compiled_edge_blocks:**

Each `compiled_edge_blocks[i]` is `Option<CompiledBlock>` containing
`Vec<Insn>`. Every Insn variant with a sid argument must be remapped:

- `Insn::LoadSignal(reg, sid)`
- `Insn::LoadSignalSigned(reg, sid)`
- `Insn::BlockingAssign(sid, val_reg, width)`
- `Insn::BlockingAssignRange(sid, hi, lo, val_reg)`
- `Insn::BlockingAssignRangeDyn(sid, hi_reg, lo_reg, val_reg)`
- `Insn::BlockingAssignBitDyn(sid, idx_reg, val_reg)`
- `Insn::NbaAssign(sid, val_reg, width)`
- `Insn::NbaAssignRange(sid, hi, lo, val_reg)`
- `Insn::NbaAssignRangeDyn(sid, hi_reg, lo_reg, val_reg)`
- `Insn::NbaAssignBitDyn(sid, idx_reg, val_reg)`
- `Insn::LoadArrayElem(reg, array_name, idx_reg)` — uses array_name
  (text), resolves via array_first_id. Remapped indirectly through
  array_first_id update.
- `Insn::NbaAssignArray(array_name, idx_reg, val_reg, width)` —
  similar, indirect via array_first_id.
- `Insn::NbaAssignArrayRange(array_name, idx_reg, hi_reg, lo_reg,
  val_reg)` — same.
- `Insn::BlockingAssignArray(array_name, ...)`, `Insn::BlockingAssignArrayRange(array_name, ...)`
  — same.

Walking all compiled blocks at remap time is O(N_blocks × avg_insns)
= O(20 779 × ~30) = ~600 K Insn updates on c910. Fast.

**JIT-compiled functions:**

If `--features jit` is on, JIT functions have sids baked into native
code. The remap would invalidate them. Solution: refuse to apply
remap when JIT is active (`--features jit` + `XEZIM_JIT=1`), or
clear `jit_fns` to force recompilation. Recommend: gate remap behind
"JIT disabled" check.

### Self-consistency check

To catch missing-remap bugs early, the remap should:

1. **Before remap:** record `pre_remap_values[old_sid] =
   signal_table[old_sid].clone()` for sid in 0..named_count.
2. **Apply remap.**
3. **After remap:** for each old_sid, verify
   `signal_table[perm[old_sid]] == pre_remap_values[old_sid]`. If
   not, the data movement is broken — abort before time-0 settle.

This catches the parallel-array movement correctness. Bytecode Insn
correctness is harder to validate at remap time (requires actually
running the simulation). Hello sim_time match is the integration
test.

Add a debug-mode invariant: at the top of `event_loop`, for a small
random sample of sids, assert
`signal_widths[sid].is_consistent_with(signal_table[sid].width)`.
Catches sid remap mismatches across parallel arrays.

## Implementation cost estimate

- Hot-set computation + array-block detection: ~80 LOC
- Permutation building (atomic groups + standalone): ~100 LOC
- Apply permutation to all parallel arrays: ~100 LOC (mechanical)
- Apply permutation to Vec<usize> containing sids: ~50 LOC
- Apply permutation to Insns in compiled_edge_blocks: ~150 LOC
  (one match arm per variant)
- Rebuild comb_dep_offsets/dep_entries: ~50 LOC
- Self-consistency check: ~30 LOC
- Tests: ~50 LOC (unit test on synthetic small example)
- Validation runs on c910 hello/memcpy: half a day

**Total: 600–700 LOC + 1.5 days of careful work + validation.**

The audit hazard means this needs uninterrupted focus. Any missed
sid reference = silent sim_time drift = TEST FAILED on c910 like the
JIT FFI-tracker incident.

## Falsification criteria

Before investing the days, validate the hypothesis:

1. **Measure actual LLC miss rate** on hello (perf stat -e
   LLC-loads,LLC-load-misses) with current code. If LLC-miss is low
   (<5%), the entire hypothesis is wrong and arena buys little.
2. **Check array-element hotness**: extend the HOT_STATS diagnostic
   to count how many of the 388 833 hot signals are inside named
   arrays. If <10% are array elements, the array-contiguity
   complication can be ignored (move only standalone hot sids, skip
   hot array elements — they stay where they are, missing a small
   fraction of the win).

If either of these comes back unfavorable, the arena project is
uneconomical and should be deferred / dropped.

## Falsification checks — DONE (session-3), both PASS

### Check 1: cache miss rate (perf stat)

```
$ perf stat -e cache-references,cache-misses ./target/release-lto/xezim --simulate ... (c910 hello)
        6534469425      cache-references:u
        2686982400      cache-misses:u    #  41.12% of all cache refs
```

**41.12% cache-miss rate** — well above the >20% threshold. The
scattered 1.1 GB signal_table access pattern is confirmed as a real
bottleneck. (LLC-specific counters were `<not supported>` under
perf_event_paranoid=2, but the overall cache-miss rate is decisive.)

### Check 2: array-member hotness (XEZIM_HOT_STATS=1)

```
[HOT_STATS] total=35955017 hot=388833 (1.08%) hot_in_array=316
            hot_standalone=388517 named_count=2400585 arrays=863
```

**Only 316 of 388 833 hot signals are array members (0.08%).** The
array-atomicity constraint — the hardest part of this design — is
essentially a non-issue on c910.

## De-risked implementation plan (post-falsification)

The array-atomicity complexity that dominated the original design is
**eliminated**: the remap can simply SKIP the 316 hot array-member
signals (leave them at their current sids in the cold tail, losing
the locality benefit for 0.08% of hot signals) and renumber only the
388 517 standalone hot signals to the front.

Revised algorithm (simpler than the original atomic-group version):

1. Build `is_hot[sid]` (edge sigs + comb r/w + clocks) — restricted
   to the named region (sid < named_count = 2.4M).
2. Build `is_array_member[sid]` from array_first_id ranges.
3. Permutation: hot-AND-not-array-member signals get sids
   0..hot_standalone_count. Everything else keeps relative order
   after. Array members and cold signals are NOT moved relative to
   each other (preserves array contiguity for free — we never split
   an array because we never move array members).
4. Apply permutation to all sid references per the audit checklist
   above. Bytecode Insn remap is still the biggest piece (~150 LOC).

Revised effort: ~400-500 LOC (down from 600-700; the atomic-group
permutation logic is gone). Still a dedicated-session project because
the sid-reference audit (every parallel array + Vec<usize> + Insn +
ClockGen + array_first_id) must be complete — a missed reference is a
silent sim_time drift.

Estimated win: 2-5 s on hello (from cutting the 41% cache-miss rate
on the hot working set, which fits in 11.9 MB / L3).

The `XEZIM_HOT_STATS=1` diagnostic is now in the codebase
(simulator.rs, in the elaborate-init block) — zero default-path cost
(gated by env var), useful for re-validating on other designs.

## CRITICAL audit addendum — lazy sid caches (found session-3)

Beyond the static sid references in the audit checklist above, xezim
has **lazily-populated sid caches** that a remap MUST also handle.
Missing any of these = silent sim_time drift (the same failure class
as the JIT write-tracker incidents).

1. **`hier.cached_signal_id`** — a per-`HierName` (AST identifier)
   cache populated on first resolution during execution. 10+ read/
   write sites in simulator.rs (lines ~9996, 10003, 10285, 11898,
   11926, 12233, 12238, 16300, 16315, …). Lives inside every
   `StmtFallback` Insn's `Arc<Statement>` AST and in continuous-
   assign / always-block ASTs. After a remap, any populated cache
   holds a STALE sid.

2. **`array_elem_ids`** — lazy per-array element-ID Vec cache
   (simulator.rs:~1043). Maps array index → resolved element sid.
   Stale after remap.

**Are these populated at remap time?** Some continuous-assign /
always-block evaluation happens during compile + time-0 settle.
If the remap runs AFTER any such evaluation, caches may be populated
and would need clearing.

**Mitigation options:**
- (a) Run the remap BEFORE any expression evaluation (earliest
  possible point — right after signal_table is built, before
  classify_always_blocks). But then comb_entries / edge_signal_ids
  don't exist yet to remap. Chicken-and-egg.
- (b) Run remap late (after all structures built) AND explicitly
  clear every lazy cache: walk all StmtFallback ASTs + cont-assign
  ASTs + always-block ASTs resetting `cached_signal_id`, clear
  `array_elem_ids`. ~50 LOC of cache-walking + a recursive AST
  visitor. This is the additional audit surface.
- (c) Gate remap behind "JIT off AND no caches populated yet" —
  requires proving caches are empty at the chosen remap point.

This addendum is why C1 is firmly a **dedicated-session** task: the
lazy-cache audit (option b's AST visitor) is itself ~50-100 LOC and
must be exhaustive. A single missed cache is a silent correctness
bug only caught by a full sim_time comparison.

## Implementation attempt (session-3) — early-remap, reverted with a bug

Implemented `remap_for_hot_arena()` using the **early-remap approach**:
run the renumbering at the very START of `compile()`, before any
bytecode Insn, comb_entry, edge_signal_id is built.  This was meant
to sidestep the Insn-remap AND lazy-cache audit entirely — everything
built afterward resolves names → new sids natively.

### What the implementation did
- Hot set via lightweight AST pre-scan (`collect_stmt_reads` on
  always-blocks + `collect_expr_reads/lhs_writes` on continuous
  assigns).  Approximate (241 704 hot vs the precise 388 833) but
  correctness-transparent — renumbering doesn't change semantics, so
  hot-set precision affects only the locality benefit.
- Permutation: hot-standalone signals → sids 0..hot_count; everything
  else (cold named + array members) → hot_count..named_count in
  ascending old-sid order (preserves array contiguity for free);
  cold region (>= named_count) identity.
- Remapped foundational arrays only: signal_table, signal_widths,
  signal_signed, signal_real, signal_has_xz, prev_val, prev_xz,
  sdf_delays, id_to_name (+ rebuild signal_name_to_id),
  array_first_id.first.
- Gated by `XEZIM_HOT_ARENA=1`; default path untouched (verified
  bit-identical 44 695 ns / TEST PASSED with the env unset).

### The bug (unresolved — reverted)
With `XEZIM_HOT_ARENA=1`, c910 hello **runs forever** (19+ min CPU
time, never reaches $finish vs ~63 s normal).  The CPU executes but
makes no correct progress → some signal_table access lands on the
wrong sid.

### Candidates RULED OUT by static reasoning
- **Bijection validity**: new_id is a verified bijection over 0..total
  (named region → [0, named_count), cold identity).  No duplicate/gap.
- **Array contiguity**: array members are all non-hot (excluded via
  is_array_member), processed in ascending old-sid order in the
  second pass → stay consecutive.  array_first_id.first remapped to
  match.
- **cached_signal_id**: the elaborator inits it to `Cell::new(None)`
  (xezim-core/elaborate.rs:4108, 5427) — NOT populated during
  elaboration.  Empty at compile start; populated during sim using
  post-remap name resolution.  So this is NOT the bug (contrary to
  the addendum's worry — early remap genuinely avoids it).
- **id_to_name sid-indexing**: `name_for_id` does `id_to_name.get(id)`
  → id_to_name IS sid-indexed; named region is exactly 0..named_count.
  Permutation correct.
- **prev_val/prev_xz sizing**: sized to named_count; permutation keeps
  named sids < named_count, so in range.

### What to try next (interactive debugging required)
The bug needs a VALIDATION HARNESS, not more static reasoning:
1. **Re-resolve-and-compare**: after remap, for every name in
   signal_name_to_id, assert `signal_table[new_sid]` equals the
   value that was at the pre-remap sid.  Catches data-movement bugs.
2. **Find the un-remapped sid source.**  Some structure reads
   signal_table by a sid NOT obtained via the remapped
   signal_name_to_id.  Suspects not yet checked:
   - `prev_wide: HashMap<usize, Value>` — keyed by sid; if non-empty
     at compile start, stale.  (Likely empty, but verify.)
   - The elaborated module's initial signal_table VALUES — are any
     cross-referenced by absolute sid during Simulator::new setup?
   - VCD/XTrace trace tables (sid lists) — built when?  This run had
     no trace flags, so probably not it, but confirm.
   - `--multikernel-scope` partition: `signal_lp_writer` (Vec by sid)
     + `edge_block_partition` — built post-compile, should use new
     sids, but the scope→sid resolution path needs checking.
   - The bytecode compiler's `signal_name_to_id` lookups: confirm it
     ALWAYS goes through the (remapped) map and never a stale
     `array_first_id` snapshot or a `&self.signal_widths` captured
     before remap.
3. **Bisect**: enable remap but make the permutation IDENTITY
   (new_id[old]=old).  If hello passes, the data-movement plumbing
   is correct and the bug is in the permutation/hot-set.  If it
   still hangs, the remap *infrastructure* (the timing/ordering of
   the call in compile()) is the issue.

The 188-LOC implementation is reverted (not left gated-but-buggy in
the tree — that's a trap).  The approach is sound; a focused session
with the validation harness above should land it.

## Recommended next step

The falsification checks are DONE and both favorable. Begin the
implementation in a dedicated session, WITH the lazy-cache audit
addendum above factored into scope:

1. Build the permutation (steps 1-3 above) — ~100 LOC.
2. Apply to parallel arrays + Vec<usize> sid lists + HashMaps +
   ClockGen — ~150 LOC. Add the self-consistency check
   (pre/post-remap Value comparison) before touching bytecode.
3. Apply to compiled_edge_blocks Insns — ~150 LOC, one match arm
   per sid-bearing variant.
4. Validate: c910 hello/memcpy bit-identical sim_time + TEST PASSED.
   Re-run perf stat to confirm the cache-miss rate dropped.
5. Guard: refuse the remap when JIT is active (baked pointers would
   be invalidated) or re-bake after.

## Why not in this session

Honest assessment: this is a 1.5–2 day project with a high
correctness-hazard surface. Cumulative session wins are already
substantial (hello −18%, memcpy −22%, cmark −10% measured). The C1
work belongs in a dedicated focused session where the implementer
can give the audit the attention it requires. The HOT_STATS
diagnostic this session is in `MULTIKERNEL-NOTES.md` as a future
reference; the actual remap stays unimplemented until that focused
session.

## What's NOT recommended

- **DO NOT** implement a partial remap (e.g. skip array sids entirely)
  without the audit. Even ignoring arrays, all the parallel arrays
  + Insns must be remapped consistently — there's no "small safe
  fraction" of the work.
- **DO NOT** use a translation-table approach (Vec<u32>
  id → location). The extra indirected load per access cancels the
  cache-locality win.
- **DO NOT** combine C1 with concurrent per-LP work. Per-LP threads
  each have a `PerLpSignalTable` with its own local→global sid map.
  Adding a third remap layer compounds the audit hazard. Land C1
  first, then per-LP, or vice versa.
