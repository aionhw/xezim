# xezim performance work — August 2026

Reference workload: **XuanTie C906**, memcpy firmware, `+iterations=50`
(`simtest`-style run reconstructed from `rtlmeter`). Cross-checked on **XuanTie C910**
and on CoreMark, which is ~23x heavier in simulated time.

---

## 1. Headline result

Clean interleaved A/B on an idle machine, 2 reps, `XEZIM_NO_PARALLEL=1`:

| | before | after |
|---|---|---|
| wall, c906 memcpy x50 | 55.4 s | **32.2 s — 1.72x** |
| compile phase | 14.6 s | **3.0 s — 4.8x** |
| simulation loop | 40.4 s | 29.1 s — 1.39x |
| retired instructions | 387.7 G | **284.0 G — −26.7%** |
| executed bytecode instructions | 1,707 M | **1,276 M — −25.2%** |

Cross-design, interleaved (machine under load, so these understate):

| workload | before | after | notes |
|---|---|---|---|
| c906 cmark x2 | 1855 s | 1267 s (1.46x) | 152 M ticks |
| c910 cmark x2 | 3866 s | 2428 s (1.59x) | different design |
| c910 memcpy x50 | 274.2 s | 206.8 s (1.33x) | never tuned against |

**Every result is bit-identical to the pre-optimization binary** — same cycle counts,
same simulated finish times, byte-identical stdout.

Test suite went 1739 passed / 3 failed → **1770 passed / 0 failed / 13 ignored**
(+22 tests added by this work; the 3 prior failures were environmental and were fixed
upstream in 0.9.8).

---

## 2. What landed, and why each worked

Ordered by size of contribution.

### 2.1 Compile phase: 15.2 s → 3.3 s

`classify_one_always_block` computed `comb_sensitivity_is_faithful` **eagerly for every
always block** — two `HashSet<String>` built from `format!`-ed hierarchical names, plus
a scope inference and per-name id resolution — although it is only consulted when four
cheaper predicates have already admitted the comb path. On a CPU core almost every
block is `always @(posedge clk)`, where `all_level` is false and the result is thrown
away. Folding it into the `&&` chain as the last operand lets it short-circuit.

**11.58 s → 46.6 ms** for that predicate. 76% of the entire compile phase.

### 2.2 Redundant `Resize` deleted at compile time: −14.1% of executed bytecode

Instrumenting the VM handler showed **99.7% of `Resize` executions were no-ops** — the
register already had the target width (242.4 M of 243.1 M). `Resize` was the second most
frequent opcode.

A static width-inference pass over the emitted stream now deletes them. Unknown width
keeps the instruction; branch targets invalidate the table. `Concat` (sum of operand
widths) and `Replicate` were the decisive rules — without them only 87.6% of dynamic
resizes went, with them 98.5%. The conservative control-flow barrier blocked **zero**
eliminations, so a real dataflow merge is provably unnecessary.

Static `Resize` 89,071 → 11,799. Validated by a debug mode that keeps each deleted
instruction and asserts the width already matched: **zero assertions over 56 M
instances** across the full test suite.

### 2.3 Bytecode VM: in-place results, −9.7% instructions

Every hot arm was `vm_regs[d] = vm_regs[l].op(&vm_regs[r])` — constructing a fresh
32-byte `Value`, moving it, and dropping the destination's previous contents (a branch
on the storage discriminant plus `free` when it was `Wide`). The inline (≤64-bit) case
now writes the destination's two words in place. Each `vm_*` helper returns `None` for
any shape it does not reproduce, so the original `Value` method still handles it.

`Replicate` built a `Vec` of N clones and turned out to be the hot `concat_refs` caller.

### 2.4 `Value` hot paths: −5.3%, then −4.6%

`lto = false` means every un-annotated `pub fn` in `xezim-core` is a real cross-crate
call from the VM loop. `range_select`, `bit_select` and `resize` were split into
`#[inline]` heads with `#[inline(never)]` cold tails.

Then byte-parallel `Wide` paths: `LogicBit`'s `#[repr(u8)]` discriminant *is* the
`(xz<<1)|val` code, so `Wide` storage converts to/from packed planes 8 bits per
multiply. `concat_refs`' wide arm was measured at 1.33 M calls x ~153 operands with
**99.2% of operands one bit wide** — the cost was per-operand, not per-bit.

### 2.5 Instruction fusions

Driven by a dynamic opcode census (`--features opcode-census`, `XEZIM_OPCODE_CENSUS=1`),
not by guessing.

| fusion | occurrences | effect |
|---|---|---|
| `LoadSignalBit ; BranchIfFalse` → `BranchIfSignalFalse(.., bit)` | 25.4 M | −4.8% of stream |
| `LoadSignal ; LoadArrayElem ; NbaAssign` → `NbaAssignArrayRead` | 16.5 M | −5.4% bytecode |
| `LoadConst ; Add\|Eq\|CaseEq` → `BinOpConst` | 32.3 M | −8.0% bytecode, −2.6% wall |

The array-read triple is an RTL memory read feeding a flop — the dominant shape in a
CPU's register file and caches. Both constituent pairs reported *identical* census
counts, which is what identified it as one idiom rather than two.

### 2.6 Edge detection: −4.4%

`after_signal_write` called `raw_bits()` — whose `Wide` arm repacks a byte per bit —
*before* the guard deciding whether the result was wanted. `snap_one` never inlined
despite `#[inline]` (six parameters, one a `&mut HashMap`). The detect loop
re-materialized every table base pointer from `self` each iteration because its stores
could alias.

### 2.7 Settle: −3.9%

110 M writes from the fused arms took a `mark_dirty_id` → `dirty_list` push → rescan →
flag-clear → dep-walk round trip that is net-zero within a settle. They now trigger
dependents directly.

### 2.8 Smaller items

- **Opcode census compiled out** (−1.3%): the census flag test sat on the VM's dispatch
  critical path. Now behind `--features opcode-census`.
- **`forced_signals.is_empty()` guard**: `contains_key` hashed the id on every signal
  write even when nothing was forced.
- **`%m` scope interning** (see §4): recovered 85% of a regression that arrived with
  upstream 0.9.8.
- **`Insn` signal ids `usize` → `u32`**: instruction-neutral; landed as a prerequisite
  and for a checked narrowing choke point.

---

## 3. Correctness fixes found while profiling

- **JIT dropped the X/Z plane.** The inline-bits `LoadSignal` fast path loaded only
  `val_bits` and never wrote `xz_slots[dest]`. Registers are 4-state, so **every X/Z
  signal read back as a determinate value** — silently wrong results, not a crash. It
  wedged c906 under `XEZIM_JIT=1 XEZIM_INLINE_BITS=1`. Same trap that had already taken
  the NBA fast paths out of service; the read path was missed at the time.
- **JIT width table missing entries.** `jit.rs` keeps its *own* register-width table
  driving post-op masking, and it lacked `RangeSelectConst` and `Select`. Deleting a
  `Resize` whose width came from those would have left the JIT masking with width 0.
- **`%m` inside `always_comb` reported the wrong instance** — found during the Stage 1
  experiment, caused by comparing declared vs actual width before the wide check.

---

## 4. An upstream regression, isolated and fixed

Upstream 0.9.8's `5387b14 fix: %m names the instance in sensitivity-driven blocks`
added to `exec_bytecode`:

```rust
if self.m_block_scope != self.edge_blocks[block_idx].scope { ... }
```

A `String` compare plus an `Arc` deref plus a scattered load into a 3607-entry struct
vector, on **~189 M block fires**. Cost **+9.8 G retired instructions (+3.5%) and +9.5%
simulation time** — with every work counter bit-identical (entry_evals, edges_fired,
bytecode instructions all unchanged). Same work, more time, which is the signature of
per-operation overhead rather than a behaviour change.

Fixed by interning each edge block's scope to a dense `u32` at compile time and
comparing ids; blocks sharing an instance share an id, so consecutive fires in one scope
skip the buffer copy. **85% of the regression recovered.** This fix is a candidate to
send upstream rather than carry locally.

---

## 5. Measured and rejected

Eleven candidates. Each was plausible; each lost. Recorded so none is retried blind.

| candidate | result | mechanism |
|---|---|---|
| **SoA signal store (mirror)** | −9.4% | see below |
| **SoA signal store (authoritative)** | **+11.3%** | header tax — see below |
| **Cranelift JIT** | +33–55% wall | VM registers in stack slots; IPC 2.30 → 1.78; 28% coverage |
| **LTO** | thin +4.27%, fat +1.22% wall | `value.rs` already has ~90 `#[inline]` added *because* `lto = false` |
| **`target-cpu=native`** | +2.36% wall | retires 1.94% *fewer* instructions and is slower |
| **PGO** | −2.9% wall | only build-config winner; dropped by direction (staleness risk) |
| **Dense NBA index** | +3.3% | 140 MB array; a compact hash caches better than sparse probes |
| **serde replacement** | n/a | zero serde symbols in a default run's profile |
| **`prev_val` by edge position** | <1% | needs a 5-structure sync; failure mode is a silently missed edge |
| **Buffer-chain collapsing** | <1% ceiling | see below |
| **Two more branch fusions** | parity | codegen tax — see §6 |

### 5.1 The SoA result, and the correction that matters

The mirror form (`signal_inline_bits` beside `signal_table`) measured 9.4% slower, and
the natural reading was *"the cost is maintaining two live representations; one
authoritative store would be fine."* **That reading was wrong.**

A full migration was built: authoritative 16 B/signal store, mirror deleted, `soa.rs`
deleted, wide signals in a side table, X/Z exactness proved by an instrumented probe
over 35.1 M + 36.0 M signals and all 1770 tests with zero violations. Result:

| | c906 | c910 |
|---|---|---|
| instructions | 283.95 G → **316.15 G (+11.3%)** | +5.0% |
| wall | +9.4% | slower |
| peak RSS | **−212 MB (−8.0%)** | **−444 MB (−11.5%)** |

**The real mechanism is separating the scalar header from the bits.** A 32-byte `Value`
carries `width`/`is_signed`/`is_real` in the *same cache line* as the bits, for free. A
16-byte val/xz cell forces every access to re-fetch them from three separate 35–140 MB
arrays — 3 extra loads on `LoadSignal`, which is 16.3% of the executed stream. The
accessor layer alone, before any storage changed, already cost +21.2%: a bounds-checked
index into `signal_widths[]` can panic, so LLVM cannot elide the load.

Confirming evidence: the **small benches got ~11% faster**. With few signals the header
arrays stay cache-resident and the halved footprint wins. The loss is specific to
designs with large scattered header arrays — i.e. real ones.

Patch preserved at `STAGE1.patch` (6,647 lines, all gates green) if the memory win is
ever wanted on its own.

### 5.2 Buffer-chain collapsing

Chains exist — 5,358 buffer-like comb entries, 49.4% chained, overwhelmingly 1:1 copies
— but the ceiling is under 1%: buffer-like entries are only 12.3% of settle evaluations,
`settle_iters/settle_calls` is 1.36 (so chains cost no extra passes), and collapsing
removes dependency *depth*, not work — the intermediate signal is observable, so both
writes still happen. Settle is dominated by `FusedGate` at **57.8%** of evaluations,
95% of which genuinely change their output.

---

## 6. The fusion ceiling

Two further fusions were built and validated — `BinOpConstCaseEq;BranchIfFalse` (9.1 M,
100% of that opcode's executions) and `LoadSignal;BranchUnlessZero` (13.9 M) — capturing
**23,029,102 pairs exactly** and cutting bytecode **5.07%**.

They were **reverted**, because:

| config | retired instructions |
|---|---|
| before the patch | 284.03 G |
| variants added, **fusions disabled** | **287.98 G (+1.39%)** |
| variants added, fusions enabled | 284.08 G |

**Merely adding two arms to the 65-variant `match` costs +1.39% — for code that never
runs.** The bytecode win is exactly cancelled. Defaulting them off would be strictly
worse. Patch preserved at `branch-fusions-DEFERRED.patch`; it becomes worth landing once
dispatch is a dense table rather than a large enum match.

Per-instruction cost is now hostage to how LLVM lays out that match, and it moves by
more than an entire fusion's worth when perturbed. **Further fusion work is
self-defeating until the instruction representation changes.**

---

## 7. Remaining opportunities, ranked

Profile after all of the above (idle machine, c906 memcpy x50). IPC 2.24,
branch-miss 1.14%, cache-miss 6.4% — **instruction-bound, not memory-stalled**.

```
42.80%  exec_insns                     2.86%  after_signal_write
18.19%  settle_combinatorial_inner     2.56%  snapshot_edge_signals
12.03%  check_edges_inner              2.13%  allocator
```
Top three are 73%.

1. **Pack the header into `Insn`.** `LoadSignal(RegId, SigId)` uses 12 of the enum's 24
   bytes. Packing `width | signed<<30` removes 2 of the 3 header loads on 16.3% of
   executed opcodes; both are immutable at run time. Small change, no storage
   migration. *(`is_real` must still be loaded — the `signal_real` classifier
   under-reports for real array cells and parameter-port reals.)*
2. **Flat `ExecInsn` representation.** `u8` opcode into a dense table, pool indices
   replacing the 11 `Box`-carrying variants (6 exceed a 15-byte payload and are the
   binding constraint on 24 B). Removes the codegen coupling in §6 and makes existing
   fusions actually show up. Expected 2–5%, plus codegen stability.
3. **Typed VM registers.** 586 M register reads measured **99.826% inline**; `is_real`
   and `is_fill` occur **zero** times in a whole run. Note `vm_regs` is a single shared
   scratch `Vec` reused by every block, so 29% of stores change width — `RegMeta` must
   live on `CompiledBlock`, not a global parallel array. Expected 2–3%.
4. **24-byte signal cell** `{val, xz, width, flags}` — 842 MB vs 1124 MB (−25%) with
   *zero* extra loads. The layout that gets a footprint win without the header tax.
5. **Re-land `branch-fusions-DEFERRED.patch`** after (2).

Not promising: settle and edge detect have each had two passes and are dominated by
necessary work; the JIT needs codegen quality before coverage (raising coverage first
makes it slower).

---

## 8. How to measure this correctly

Getting this wrong produced several false conclusions before the rules were established.

- **Always `XEZIM_NO_PARALLEL=1`.** The parallel-dispatch path self-calibrates on *wall
  clock*, so two runs of an identical binary can differ ~10% in retired instructions.
- **Retired instructions for source changes** (contention-independent, ~0.1%
  reproducible); **wall clock decides codegen/layout changes**. Two cases proved this:
  the JIT retired 7.2% *fewer* instructions and was 33–55% slower; `target-cpu=native`
  retired 1.94% fewer and was 2.36% slower.
- **Same-binary A/B via an env escape hatch** (`XEZIM_FUSE_CONST=0` etc.) removes
  build-to-build noise. This is what caught the +1.39% codegen tax in §6.
- **≥4 interleaved reps**, and reverse the order on a second pass. One engineer
  concluded the JIT won on wall time; order-reversed at matched load, it lost.
- **`cargo test --release` fail-fasts** — always `--no-fail-fast`, and run each crate
  from its own directory.
- **Profile with `perf record --call-graph lbr`** — `panic = "abort"` strips unwind
  tables, so DWARF call graphs do not work.
- **The machine must be genuinely idle** for wall numbers. Contended runs understated
  the headline by ~45%.

---

## 9. Regression gates

Any change must reproduce all of these.

| gate | expected |
|---|---|
| c906 memcpy x50 | `cost 727`, finish `6477650` |
| c906 cmark x2 | `714196` cycles, finish `152197450`, CoreMark `1.400176` |
| c910 memcpy x50 | `216` cycles, finish `2282050` |
| c910 cmark x2 | `158034` cycles, finish `34985250`, CoreMark `6.327752` |
| `b2_vm_dispatch` | `cycles=200000` |
| `b3_mem_sweep_20` | `cycles=100000` |
| `b2b_vm_branchy` | `cycles=50000` |
| tests (`--no-fail-fast`, both crates) | **1770 passed, 0 failed, 13 ignored** |
| feature builds | `--features jit`, `--features opcode-census` |

`checksum=x` on the benches is the expected value, not a failure — `%0d` renders as `x`
when the value has unknown bits.

**The strongest and cheapest check is a full stdout `diff` against the previous binary.
Byte-identical is the bar**, and it caught more than any golden number alone.

Also run at least one gate *without* `XEZIM_NO_PARALLEL=1` — that is the only way
`exec_comb_block_isolated` and the partitioning analyses get exercised.

---

## 10. Hazards specific to this codebase

1. **~25 analysis sites match `Insn` with a catch-all `_ =>`** to extract signal ids. A
   new variant compiles fine while being silently ignored there, producing wrong event
   gating rather than an error. This is how the JIT's `is_supported` list rotted to 28%
   coverage with no build error. **Mitigation: change the ARITY of an existing variant
   instead of adding one** — every pattern then fails to compile and the compiler hands
   you the complete site list. Used successfully three times.
2. **`build_event_measure_state`** drives event-edge gating. A missed read means a flop
   is skipped when its input changed — silent wrong answer.
3. **Branch polarity.** `BranchUnlessZero` jumps on **true or X**;
   `BranchIfSignalFalse` jumps on **false or X**. Opposite, agreeing only on X.
   Conflating them inverts a branch and still terminates with plausible output.
4. **Width truncation.** `Value::MAX_WIDTH` is `1<<20`, so width does not fit a `u16`.
5. **`jit.rs` keeps its own register-width table**, separate from the bytecode
   compiler's. Width-inference changes must update both.
6. **`exec_comb_block_isolated`'s match is now exhaustive** — do not reintroduce a `_`
   arm. It is the safety net for any future port of the exec loops.
7. **`$root.`-prefixed hierarchical references in an event control are mis-simulated.**
   rtlmeter's `always @(posedge $root.<clk>) ++cycles` counts 1 instead of 39,567. Use
   the firmware's own cycle count, not `_rtlmeter_cycles.txt`. Not fixed.

---

## 11. Reproducing the workloads

RTL is not in these repos. `simtest/xuantie_c906/work/c906.fl` points at a path that
does not exist; clone `https://github.com/verilator/rtlmeter` instead. Build the file
list from `descriptor.yaml`'s `compile.verilogSourceFiles` and append
`rtlmeter/rtl/__rtlmeter_utils.sv`. Each run needs its own directory — the testbench
`$readmemh`s `inst.pat`/`data.pat` from the CWD.

```bash
XEZIM_NO_PARALLEL=1 XEZIM_INIT_ZERO=1 xezim --simulate --max-time 100000000 \
  -s tb "-D__RTLMETER_MAIN_CLOCK=tb.clk" \
  -I <design>/src -I rtlmeter/rtl -f <design>.fl +iterations=50
```

`__RTLMETER_MAIN_CLOCK` must be defined or `__rtlmeter_utils.sv` is a syntax error.
`XEZIM_INIT_ZERO=1` is required for cmark.

**Timescale is `1ns/100ps`, so reported "finished at time" is in 100 ps ticks** —
divide by 10 for ns. Getting this wrong makes xezim look 16x slower than it is.

### Comparison with Verilator

Verilator runs the same c906 memcpy in 0.89 s. But the two are **not simulating the same
work**: Verilator finishes at 396 us / 39,567 clock cycles, xezim at 647.8 us
(~64,776 cycles), and the firmware reports 727 vs 368 CPU cycles for the copy. xezim
runs **~1.64x more clock cycles for the same firmware**, so any speed ratio must be
normalized for that — roughly 27x per simulated cycle rather than the ~44x the raw wall
times suggest. The cause of that divergence was not investigated; `XEZIM_INIT_ZERO=1`
forcing X→0 everywhere is a plausible suspect.

---

## Combinational cone merging — measured, rejected as a large win (Aug 2026)

The proposal: compile chains of `CombEntry` (`c = a&b; d = c^x; y = d|z`) into one
kernel invocation, removing the per-entry scheduler work, dirty-queue traffic, entry
lookup and dispatch at every hop. Estimated beforehand at 5–15% (small RTL) and
15–40% (large DUT-heavy).

**The tree already contains the analyzer for this.** `simulator.rs:19146`, opt-in via
`XEZIM_CONE=1`, whose header states the design verbatim: "entry A writes signal s, and
B is the ONLY reader of s, and s has no other writer — then A+B can become one entry".
It was extended here to report *merge economics*, because chain-shape alone does not
tell you whether merging saves work.

### Why chain-shape alone is not the answer

A merged kernel fires when **any** member's inputs change and then recomputes **every**
member. So its best-case work is `max(member evals) * len`, against `sum(member evals)`
unmerged. Members that fire equally often give a ratio of 1.0 (merging is pure win);
skewed members give >1.0, and that extra compute has to be paid for out of the saved
dispatch.

| | entries | chain-members (static) | eval-weighted | blanket-merge compute |
|---|---|---|---|---|
| c906 memcpy | 52,124 | 39.8% | **43.0%** | **+32.8%** |
| c910 memcpy | 367,320 | 29.8% | **25.0%** | **+44.2%** |

Blanket merging is a wash or a loss: on c906 it removes 50.4% of in-chain dispatches
(37.06M → 18.38M) but adds 32.8% compute (37.06M → 49.22M evaluations).

**The size effect is the opposite of the intuition.** The larger, more DUT-heavy design
has less than half the opportunity, and its hot entries are *less* chain-shaped than its
static share (25.0% vs 29.8%) while c906's are *more* (43.0% vs 39.8%).

### The viable form is selective, and its ceiling is ~1%

77% of c906 chains (6,406 of 8,313) recompute at <1.1x — those are nearly free. Folding
only those:

| threshold | c906 dispatches removed | c906 extra compute | c910 removed | c910 extra |
|---|---|---|---|---|
| **<1.1x** | **14.9% of all evals** | **+0.1%** | **5.7%** | **+0.0%** |
| <1.5x | 17.5% | +1.9% | 6.8% | +0.7% |
| <2x | 18.6% | +6.4% | 7.3% | +4.8% |
| all | 19.0% | +12.3% | 7.7% | +7.9% |

Past <1.1x the trade turns bad fast — c906 <2x buys +3.7pp of dispatch for +6.3pp of
compute.

`settle_combinatorial_inner` is 18.19% of the run at 222 retired instructions per entry
evaluation. So even if dispatch were *100%* of that cost, the selective merge ceiling is
`14.9% x 18.19%` = **2.7% on c906 and 1.0% on c910**. Dispatch is realistically ~60 of
those 222 instructions (~50–70 of overhead against ~20–25 of actual gate algebra for a
`Bin2`), which puts the expected win at **~0.7% / ~0.3%** — an order of magnitude below
the estimate, for a transform that must also keep interior nets observable to VCD,
`force` and VPI, preserve ordering, and exclude NBA-bearing and unresolved-read entries.

### Independent corroboration

`XEZIM_BSP_SETTLE=1` already implements the coarse form of the same idea — evaluate by
static topological level instead of by dirty queue. Measured on c906 memcpy x50:
**354.4G retired vs 284.2G, +24.7%**, with `cost=727` (correct). Over-evaluation is
precisely what makes group evaluation lose here, and it is the same force that caps
cone merging.

### What the analysis *did* produce

The per-dispatch constant is real and large. Cone merging just happens to remove it from
only 15% (c906) / 6% (c910) of evaluations. Two changes attack the identical cost across
**100%** of them:

1. ~~**Re-land the `entries[]` prefetch.**~~ **Tried, measured, reverted — see below.**
2. **`settle_triggered: Vec<bool>` → bitset.** One byte per entry = 367 KB on c910,
   randomly probed once per dispatch and once per dependent inside `trigger_deps!`
   (119.1M dirty-propagation round trips per c906 run, 93% of them from the fused arms).
   A bitset is 8x denser at 46 KB — L2-resident — and is touched by every evaluation.

The `XEZIM_CONE` economics reporting added for this investigation is diagnostic-only and
opt-in; default-path instructions and stdout are unchanged (284.35G vs 284.20/284.22G
baseline, byte-identical output, 1742 tests passing).


## Settle prefetch — re-tried, measured, reverted (Aug 2026)

`simulator.rs:32905` claimed a single-stage `_mm_prefetch` on
`entries[cur_list[cur_pos+8]]` gave **c906 cmark a 31% wall-time win**, reverted because
c910 t=1 k=0 hung at iters=200040 "despite the prefetch being a semantic no-op — likely a
downstream cache-state interaction". Both halves of that note turned out to be
questionable.

### It works, but it does not pay

Re-implemented behind `XEZIM_SETTLE_PREFETCH=1` with **both** indices bounds-checked
(`cur_list.get(..)` for the worklist read, `nidx < entries.len()` for the pointer), so the
address is provably in-bounds and the prefetch is a genuine no-op. c906 memcpy x50,
3 interleaved reps, same binary:

| metric | off | on | delta |
|---|---|---|---|
| instructions | 284.91 G | 287.66 G | **+0.96%** |
| cycles | 133.13 G | 132.47 G | **-0.50%** |
| IPC | 2.140 | 2.172 | +1.5% |
| wall | 33.90 s | 33.68 s | -0.67% (noise: off-arm spread 1.80 s) |

`cost=727` and stdout byte-identical in all three reps. The prefetch does what it is
supposed to — IPC rises — but the lookahead costs more instructions than the cycles it
saves, and on an instruction-bound workload (IPC 2.14, cache-miss 6.4%) that is a wash.

Two side findings worth keeping:
- Moving the prefetch **below** the `!triggered[eidx]` guard changed the instruction cost
  *not at all*. That disproves the theory that many worklist pops are stale duplicates —
  nearly every position is a real evaluation. The +2.84 G is intrinsic to doing a
  lookahead inside that very large loop body.
- The `pf_on` branch cost **~0.25% even when disabled** (clean baseline 284.09 G vs
  284.87–285.02 G with the flag merely compiled in). Worth remembering for any future
  env-gated hot-loop A/B: the escape hatch is not free, and it biases the "off" arm.

### The c910 hang is not about prefetching

The "semantic no-op" framing does not survive three checks:
1. The revert commit `dba9289` itself says the hang had the **"same signature as the
   original NBA-order bug"**.
2. `simulator.rs:30819-30827` documents that c910 **already** hangs at the identical
   `iters=200040` point if you merely **sort** `parallel_blocks`: "c910 sequential
   dispatch has block-order dependencies that break under index-sorted order."
3. That loop is *combinational settle* and the run was single-threaded, so it cannot
   reorder NBAs. A true no-op cannot change termination.

The most likely mechanism for the original failure is that the lookahead was indexed
**unchecked**: `cur_pos + 8` past the end of `cur_list` is UB, which permits LLVM to
assume it in range and transform the `cur_pos < cur_list.len()` loop itself. That would
make the original "prefetch" not a no-op at all.

**So the real open bug is c910's block-order dependency in edge-block dispatch** — a
design whose result depends on the order blocks were collected in is a latent
nondeterminism, and it is worth hunting on correctness grounds independently of any
performance work. Prefetching was only ever the messenger.

Reverted at user direction; tree restored to `7ccf55a` with simulation output
byte-identical to baseline.

### cmark A/B — instruction cost confirmed, timing self-contaminated

The c906 cmark x2 A/B finished after the revert. All four runs produced the golden
`714196` cycles/iteration.

| run (in order) | instructions | cycles | wall |
|---|---|---|---|
| off1 | 6001.8 G | 3522.3 G | 1125.6 s |
| on1 | 6067.7 G | 3363.5 G | 1091.6 s |
| off2 | 6004.7 G | 3118.2 G | 949.7 s |
| on2 | 6066.9 G | 2628.0 G | 682.4 s |

**Instructions: +1.07%**, matching the clean memcpy figure of +0.96% — architecturally
exact, so contention-independent, and the one trustworthy column here.

**The cycles and wall columns are invalid.** A c910 run was executing concurrently and was
killed partway through this sequence, so contention fell monotonically across
`off1 -> on1 -> off2 -> on2`; the fastest run is simply the last one. Read naively the
table shows -14.5% wall for the prefetch, which is an artifact of that gradient. Recorded
here as a worked example of the failure mode the measurement protocol exists to prevent:
**never interleave an A/B with an unrelated long job, and never let machine load drift
monotonically across the run order.**

This neither confirms nor refutes the original 31%. But it does show the instruction cost
is the same on cmark as on memcpy, so there is no mechanism by which cmark would respond
60x more strongly than the clean memcpy measurement (-0.50% cycles) did. A contaminated
baseline of exactly this shape remains the most economical explanation for the original
figure.


## `settle_triggered` bitset — measured, rejected (Aug 2026)

`settle_triggered` is a `Vec<bool>` — one **byte** per comb entry, probed once per settle
dispatch and once per dependent inside `trigger_deps!` (119.1 M propagation round trips
per c906 run). At 52 KB on c906 and 367 KB on c910 it sits past the 32 KiB L1d, so packing
it to one bit per entry (6.5 KB / 46 KB) looked like a clean locality win — and unlike
cone merging it touches **100%** of evaluations.

Converted to a packed `EntryFlags` with `test_and_set` fusing the probe-then-set that
`trigger_deps!` performs. Correct: `cost=727`, output byte-identical. c906 memcpy x50,
3 interleaved reps:

| | baseline | bitset (checked) | bitset (unchecked) |
|---|---|---|---|
| instructions | 284.20 G | +1.96% | **+1.42%** |
| cycles | 134.22 G | +1.34% | **+1.58%** |
| wall | 35.20 s | +3.4% | +1.8% |

Rust bounds checks on `words[i >> 6]` cannot be elided (indexing can panic), so the
unchecked variant used `get_unchecked` with a `debug_assert!`. It recovered only 0.54 pp —
**bounds checking was not the problem.**

### Why it lost, and it is not the obvious reason

The decisive number is that **cycles rose more than instructions** (+1.58% vs +1.42%). Had
the smaller footprint helped even slightly, cycles would have risen *less* than
instructions. They did not, so there is no cache benefit hiding behind the instruction
cost — the packed form is worse for the memory system as well.

The likely mechanism is false serialization. With one byte per entry, two entries in the
same cache line still have **independent addresses**, so a store to one never blocks a
load of another. Packed 64-to-a-word, every `set`/`clear` creates a store-to-load
dependency for all 64 neighbours — and `trigger_deps!` walks CSR runs of clustered entry
indices, so it hits that pattern constantly. Density bought L1 residency and paid for it
in a dependency chain.

**Corollary.** This is now the third footprint-reduction attempt to lose on this workload,
after the 16-byte signal planes (header tax, +11.3%) and this. At IPC ~2.14 with a 6.4%
cache-miss rate, xezim is instruction-bound, and shrinking a hot array reliably costs more
in added instructions and dependencies than it recovers in misses. **Do not propose
another "shrink a hot array" optimization without first showing that array is actually
missing.**

Reverted; tree restored with output byte-identical to baseline.

## Clock-domain kernel batching for `always_ff` — the literal form fails, the inverted form is the best remaining lead

Proposal: on a posedge, instead of waking hundreds of independent event blocks, run one
clock-domain kernel over the blocks, split into pure compiled FF / dynamic-fallback /
VPI-visible, executing the pure subset directly.

### Most of the machinery already exists

- **The pure/dynamic split is already computed.** `edge_block_parallel`
  (`simulator.rs:14853`) is exactly "pure compiled FF": no `StmtFallback`, no blocking
  writes, no dynamic/array writes, sub-range writes only when single-writer. On c906
  `XEZIM_PURITY_STATS=1` reports **pure=2859 blocks / 36,946 insns vs non-pure=748 /
  9,560 — 79.3% of blocks and 79.4% of edge instructions**. Non-pure is dominated by
  `Nba(array)` (633 blocks, 56% of non-pure insns).
- **The clock-domain index already exists.** `edge_blocks_by_sig[pos].posedge`
  (`simulator.rs:10505`) is literally "every block on posedge S". Detection is
  signal-major (`simulator.rs:30198`), so **`edge_triggered_list` is already segmented by
  clock signal** — a kernel could record segment boundaries rather than build an index.
- Class (c) VPI-visible has **no** per-block vector; VPI callbacks are keyed by signal
  (`dpi_value_change_cbs`) and mutate at run time via `vpi_register_cb`, so block-level
  VPI visibility would have to be derived dynamically.

### Why the literal form fails: the gating is doing enormous work

`[EVENT-EDGE] would-skip 110,213,593/118,741,930 gateable main-clk flop-fires (**92.8%**)
had NO data-input change since last posedge`. Only 7.2% of flop fires need to execute at
all. Running a clock domain without that per-block gating is directly measurable —
`XEZIM_EVENT_EDGE=0`, same binary, c906 memcpy x50:

| | instructions | cycles |
|---|---|---|
| gating on (default) | 284.19 G | 133.43 G |
| gating off | 338.20 G | 159.88 G |
| | **+19.0%** | **+19.8%** |

`cost=727` in both. This is the same over-evaluation wall that made `XEZIM_BSP_SETTLE`
+24.7% and capped cone merging. **A clock-domain kernel must keep per-block gating**, which
leaves it only the per-fire dispatch overhead on the 7.2% that execute.

### The inverted form is where the value is

`[PROF] edge_detect=3111.4ms edge_exec=5140.4ms edges_fired=188,888,084` in a ~33 s run
(`settle=19479.8ms`). Of the 188.9 M fires, **109.6 M are armed-fast-skips** — so ~79 M
actually execute and **110 M are rejected one at a time**. `edge_detect` is 3111 ms =
**~9.4% of simulation time**, much of it spent proving blocks should NOT run.

The `armed` bit is already maintained incrementally: `write_sig!` (`simulator.rs:540`) sets
`edge_block_armed[bi] = 1` when a data input is written. So the set of blocks that need to
run is **already known before the clock edge arrives**. Inverting that bitmap into a
per-domain worklist turns "posedge → walk all fanout blocks → test each → skip 92.8%" into
"posedge → run exactly the armed set". That is this proposal in the form the data
supports, and it targets a measured 9.4% rather than fighting the gating.

### Two unconditional per-fire inefficiencies found while measuring

1. `compiled_edge_blocks[bi]` is dereferenced **twice per fire** — once at
   `simulator.rs:30756` for `instructions.len()` (partition split) and again at
   `simulator.rs:16291` for ptr/len/num_regs. Both are pointer chases into a cold vector.
2. The gating makes **two linear passes over the same read span** per surviving fire: a
   snapshot compare (`simulator.rs:30565`) then a full snapshot refresh
   (`simulator.rs:30677`).
3. Minor: `std::env::var("XEZIM_FORCE_PARALLEL")` is called **per dispatch pass**
   (`simulator.rs:30787`) — uncached, unlike the neighbouring `cached_env_flag` helpers.
   Only reached when the parallel path qualifies, so it does not affect `XEZIM_NO_PARALLEL`
   benchmarks, but it is a real cost in default runs.

### The c910 order-dependency bug — this hypothesis was DISPROVEN (see below)

`is_pure` (`simulator.rs:14853`) applies the `nba_writer_count > 1` multi-writer test only
to `NbaAssignRange`/`NbaAssignBitDyn` — **not to whole-signal `NbaAssign`**. Meanwhile the
sequential NBA path stamps `block_index: 0` and resolves collisions by queue position
(`simulator.rs:16891`), with `nba_fast_index` overwriting in place. So two blocks that
`is_pure` calls order-independent, both NBA-writing the same signal, are resolved by
**dispatch order** — which is exactly why sorting `parallel_blocks` hangs c910 at
iters=200040. The machinery to detect this (`nba_writer_count`, `simulator.rs:14803`)
already exists and simply is not applied to the whole-signal case.


## `is_pure` multi-writer hole — closed, but it is NOT the c910 bug

**Correction to the previous section.** I proposed that c910's block-order dependency was
caused by `is_pure` (`simulator.rs:14853`) applying its `nba_writer_count > 1` multi-writer
test only to `NbaAssignRange`/`NbaAssignBitDyn` and not to whole-signal `NbaAssign` — so two
blocks NBA-writing the same signal would be labelled order-independent while actually being
resolved by dispatch order. **The measurement disproves it.**

Extending the test to the whole-signal variants (`NbaAssign`, `NbaAssignConst`,
`NbaAssignArrayRead`) and adding a `[PURITY]` category for them:

| design | pure before | pure after | whole-sig multi-writer blocks |
|---|---|---|---|
| c906 | 2859 / 36,946 insns | **2859 / 36,946** | **0** |
| c910 | 19,181 / 362,419 insns | **19,181 / 362,419** | **0** |

Zero blocks on either design. Confirmed a true negative rather than dead code by
temporarily lowering the threshold to `> 0`, which catches **616 blocks on c906** — the arm
is live and simply never fires at `> 1`.

The reason is now clear and it vindicates the original author's scoping: in these designs
the multi-writer pattern is confined to **sub-range** writes — yosys-style per-bit flops
(`cpu_state[0] <= _00014_;` in one block, `cpu_state[1] <= _00015_;` in another) — which
the existing test already covered. Whole-signal NBA targets have exactly one writing block.

### What was kept, and why

The fix ships anyway (`d8bff6f`): a block that whole-signal-NBA-writes a multi-writer target
genuinely is not order-independent, and `is_pure` claiming otherwise is a real hole that a
different design could hit. It is correct by construction and costs nothing measured —
**zero blocks demoted on both designs**, c906 stdout byte-identical, c910 `cost=216
finish=2282050` exact, 1742 + 44 tests passing. `nba_writer_count` is now built
unconditionally, since the whole-signal arm needs it even under `XEZIM_NO_PARALLEL_RANGE`.

But it is **defensive, not a fix for the c910 hang**, and that bug remains unexplained.
Remaining suspects, none yet checked: `write_sig!` side effects coupling blocks within one
edge (`simulator.rs:540` re-arms `edge_block_armed`; `:559-565` records `edge_exec_wrote`,
driving `drain_edge_exec_rescan`, which can re-fire blocks at the same timestamp);
`apply_nba` ordering changing the dirty-list order into settle; or the hang being specific
to the partition / k-way-merge path and misattributed to `t=1 k=0`. It should not be
described as understood until there is a repro.

## Guard-specialized dynamic sensitivity — the phenomenon is real and large, but the mechanism that pays is BIT-GRANULAR, not guard-based

Proposal: after a block runs and its guard selects a path, narrow its active sensitivity to
the signals on that path. `if (mode) y=a+b; else y=c+d;` with `mode=1` should stop waking on
`c`/`d`.

### The phenomenon is confirmed, and it is big

Instrumented `settle_combinatorial_inner` to count evaluations that changed NOTHING — the
entry was woken, ran, and every output compared equal. A wakeup that changes something was
necessary work regardless of guards, so this is the ceiling on the whole idea.

```
[NOCHANGE] wasted wakeups 63,280,356/233,348,958 (27.1%)
gate=6%  contassign_c=64%  alwaysblk_c=69%  fastcopy/dircopy/fanout=~0%
```

**27.1% of all comb-entry evaluations are wasted**, and the split lands exactly where the
proposal predicts: branch-free gates waste 6% (independently corroborating the earlier "95%
of fused-gate evals change their output"), while the VM-executing entries — the expensive
ones — waste **64-69%**.

### But guard specialization addresses only 14.3% of it

Partitioning the 56.0M wasted VM-entry evaluations by what kind of guard the block actually
contains (priority: branch > Select > bit/range read > none). Note a ternary compiles to
`Insn::Select`, a data mux evaluating BOTH sides — a semantic guard with no branch — so it
must be counted separately or the classification misses the proposal's own example.

| guard kind | entries | evals | wasted | share of VM waste |
|---|---|---|---|---|
| branch (`if`/`else`) | 723 | 7.6 M | 68.6% | **9.3%** |
| `Select` (ternary mux) | 1,337 | 6.3 M | 44.2% | **5.0%** |
| **bit/range read only** | 20,383 | 58.1 M | 69.7% | **72.4%** |
| no guard at all | 6,321 | 14.2 M | 52.4% | 13.3% |

Guard specialization — branch and ternary together — reaches **14.3%** of the waste, worth
about 8.0M of 233.3M evaluations (3.4%). Real, but a fifth of the opportunity.

### Where the value actually is

**72.4% of the waste is blocks whose only guard is that they read a BIT or RANGE of a
signal.** They are woken because the signal changed and the dirty flag is per-signal, but
the bits they read did not change. That is **40.5M wasted evaluations = 17.4% of all comb
entry evaluations**, on VM-executing entries.

This is the **GSIM bit-granular** mechanism, not the ERASER-style path specialization —
so the citation that turns out to matter is the first one.

**It is tractable, and there is precedent in-tree.** `check_edges_inner` already carries
`bitsel_sid_bits`, a per-signal bitmap prefilter that does exactly this for edge blocks with
bit sensitivities. The comb side would need: a changed-bit mask per write
(`(old_val ^ new_val) | (old_xz ^ new_xz)`, an XOR on a path that already compares
old vs new), a per-(entry,signal) read mask harvested statically from `LoadSignalBit` /
`LoadSignalRange` / `BitSelectConst` / `RangeSelectConst` operands, and an intersection test
in `trigger_deps!` before enqueuing. Unlike cone merging, this REMOVES work rather than
rearranging it, and unlike the footprint attempts it does not trade instructions for cache.

### Measurement cost note

The instrumentation itself cost **+0.77%** on the default path (286.41G vs 284.2G) because
`entry_changed = true` fires unconditionally inside `trigger_deps!` (~119M times). Reverted
rather than shipped; patch preserved at `scratchpad/NOCHANGE-instrumentation.patch`. This is
the second time an always-present hot-loop flag cost ~0.25-0.8% — see the settle-prefetch
note.


## c910 `t=1 k=0` sort constraint — does not reproduce on current code

`simulator.rs:30819` forbids sorting `parallel_blocks` outside the partition path: "c910
sequential dispatch has block-order dependencies that break under index-sorted order (c910
t=1 k=0 hangs at iters=200040 if we sort here)." Ran that exact experiment — sort forced on
via a temporary flag, no partition, `--threads=1`, c910 **cmark** — against an identical
unsorted control:

| | sort forced on | control |
|---|---|---|
| result | **TEST PASSED** | TEST PASSED |
| cycles/iteration | **158034** (golden) | 158034 |
| finish time | **34985250** (golden) | 34985250 |
| iters reached | **698,067** | 696,234 |

The sorted run went **3.5x past** the documented `iters=200040` failure point and produced
byte-exact golden output. c906 memcpy also passed but only reaches `iters=41,295`, so it
never approaches the failure point and is not evidence either way — worth noting because it
was the first thing tried.

**Caveats, which matter.** This shows the constraint does not hold on *current* code, not
that it was wrong when written in May 2026: 17 upstream commits plus this session's work
have landed since `dba9289`, so the underlying defect may simply have been fixed elsewhere.
The second sanity the comment demands (`t=4 k=4`) was **not** run, and the partition path
independently needs the sort for its k-way merge — so `use_partition`'s sort must stay.
The temporary repro flag was removed; nothing shipped.

One structural fact worth recording: this sort sits inside `if use_parallel`, which
`XEZIM_NO_PARALLEL=1` skips entirely. **Every performance measurement in this document is
therefore orthogonal to this constraint** — an earlier framing of mine implied otherwise.

## Bit-granular comb sensitivity — prototyped and measured: 9.8% of evaluations avoidable

Built the measurement using the SAME data structure the real optimization needs — a
per-dependency-edge read mask parallel to `comb_dep_entries` — but counting suppressions
instead of performing them, so a wrong mask skews the estimate rather than dropping a
wakeup. `XEZIM_BITGRAN=1`, c906 memcpy x50, `cost=727` throughout.

### Results

```
[BITGRAN] dep edges: 26239 narrow, 75422 all-bits (25.8% narrowable)
[BITGRAN] dependent-wakeups tested=510,731,563 suppressible=121,355,954 (23.8%)
[BITGRAN] ENTRY EVALUATIONS avoided=22,933,862/233,348,958 (9.8%)
```

The three numbers measure different things and only the last one matters. 23.8% of
*trigger edges* are suppressible, but an entry woken by three inputs is still evaluated
once — suppression only saves work when EVERY input that woke it was bit-irrelevant. That
is **9.8% of all comb entry evaluations**, i.e. **22.9M of the 63.3M wasted evaluations —
36.2% of all waste recovered**.

### Cost

The disabled path measured **+0.63%** (285.89G vs 284.1G baseline) purely from the
`if bitgran_on` branch in `trigger_deps!`. A real implementation makes the test
unconditional, so that same ~0.6% IS the mechanism's cost: one mask load, one AND, one
branch per dependent across 510.7M tests. Net expectation therefore ~9% of evaluations
saved against ~0.6% added — and because the avoided evaluations are bit-reading VM entries
(above average cost, they enter the interpreter), the time saving should exceed the
evaluation-count share.

### The classifier, and why it is safe

Of 66 `Insn` variants exactly **15 carry a SigId** and 9 carry a `Box` payload; the other
42 are pure register/constant ops that cannot name a signal. Reads narrow only for
`LoadSignalBit` (one bit), `LoadSignalRange` (a range); `LoadSignal`/`LoadSignalSigned`/
`BranchIfSignalFalse` take all bits; sub-range writes take all bits because they preserve
the untouched bits (an implicit read of the destination); and `StmtFallback` plus every
`ArrayOperand`-carrying opcode **bail the whole entry to all-bits**, since they can name
signals the scan cannot see. Only compiled VM entries are analysed — `FusedGate` and the
copy/fanout arms stay at all-bits, so the 9.8% is a floor, not a ceiling.

### Design note for the real implementation — do NOT use the prototype's shadow arrays

The prototype derives the changed mask by diffing against `bitgran_shadow_val/xz`, two
`Vec<u64>` sized to the signal count = **561 MB on c906**. That is fine for a diagnostic and
wrong for production. The write sites already hold both old and new values, so the changed
mask should be computed there and passed into `trigger_deps!`: the fused arms know the
exact bit (`set_bit_code` -> `1 << bit`), `FastDirectCopy` has `(sv^dv)|(sx^dx)` in hand,
and the compiled path would carry a mask alongside `dirty_list`. No per-signal array needed.

Correctness argument for the shadow/broadcast scheme, which carries over: a bit change is
always contained in exactly one broadcast's changed mask, and every dependent is tested at
that broadcast, so a dependent whose mask includes the bit is never skipped.

### Bug found while building it

`self.comb_dep_entries` is `mem::take`n into a settle-local for the whole settle, so the
field reads EMPTY inside the loop. Indexing `self.comb_dep_entries[k]` there panics with
"len is 0". Anything added to `trigger_deps!` must use the locals, not the fields.

Prototype preserved at `scratchpad/BITGRAN-prototype.patch` (251 lines). Reverted; tree
restored byte-identical.

## Bit-granular sensitivity, REAL implementation — correct, suppresses as predicted, does not pay

Built the full mechanism (not just the counter): per-dependency-edge read masks, a changed-bit
mask derived per propagation, and actual suppression in `trigger_deps!`. `XEZIM_NO_BITGRAN=1`
disables it for same-binary A/B.

### Correctness: exact

**Byte-identical output with suppression ON vs OFF and vs the pristine baseline**, on BOTH
designs — c906 `cost=727`, c910 `cost=216`. The mask classifier is sound.

### It suppresses, roughly as predicted

| design | entry_evals OFF | entry_evals ON | avoided |
|---|---|---|---|
| c906 | 233,348,958 | 220,033,704 | **-5.7%** |
| c910 | 601,191,165 | 582,283,560 | **-3.1%** |

(The prototype predicted 9.8% on c906; the real figure is 5.7% because the settle SEEDING
path drains `dirty_list` and walks the CSR directly, bypassing `trigger_deps!` entirely.
From the earlier dirty-propagation census that path is ~69% of all wakeups.)

### But it costs more than it saves

c906 memcpy x50, 2 interleaved reps, against the 284.1 G pristine baseline:

| | instructions | cycles | wall |
|---|---|---|---|
| pristine | 284.1 G | ~134 G | ~34 s |
| machinery present, OFF | 289.5 G (+1.9%) | 136.1 G | 34.8 s |
| suppression ON | **285.2 G (+0.4%)** | 136.2 G (+1.6%) | 35.4 s |

Suppression recovers 4.3 G instructions but the machinery costs 5.4 G. **Net +0.4%
instructions and +1.6% cycles — a loss.** c910 agrees: -0.17% instructions from suppression,
nowhere near the machinery cost.

### Why: the test/benefit ratio

**510 M edge tests to avoid 13.3 M evaluations — 38 tests per avoided evaluation.** The mask
test is paid on every dependency edge traversal, but the saving only materialises when EVERY
trigger for an entry is suppressed. And the three extra arrays (mask + two shadow planes) are
randomly accessed, so cycles degrade more than instructions — the same memory-traffic
mechanism that sank the `settle_triggered` bitset.

### Two implementation lessons worth keeping

1. **Indexed access in `trigger_deps!` cost +4.3% by itself.** Rewriting the dependent loop
   from a slice iterator to `for k in lo..hi` adds a bounds check on BOTH arrays across ~510 M
   iterations. Zipping the two slices instead recovered 2.4 points of that. Anything touching
   this loop must avoid indexed access.
2. **`self.comb_entries` is assigned ~260 lines AFTER the dependency CSR** inside
   `build_comb_entries`. Building masks right after the CSR silently produced an all-`u64::MAX`
   table (no suppression at all, 25.9 K evals avoided instead of 13.3 M) with no error. The
   build must follow `self.comb_entries = entries;`.

### The one unexplored lever

The seeding path bypasses the masks and is ~69% of wakeups. Masking it too would roughly
double the suppression (5.7% -> ~9.8% of evals, ~+3 G instructions saved) — but that is still
short of the 5.4 G machinery cost, and it would add test cost on the seeding path as well. So
the expected best case remains around parity, and cycles would likely stay worse. Not pursued.

Working implementation preserved at `scratchpad/BITGRAN-real-implementation.patch` (257 lines,
byte-identical on both designs). Reverted; tree restored.


## cranelift 0.109 -> 0.134 (Aug 2026)

Updated the optional JIT's cranelift dependency across 25 minor versions. `Cargo.lock` is
gitignored here, so the change is `Cargo.toml` + `src/compiler/jit.rs` (143 insertions,
133 deletions).

### The five breaking API changes

| change | fix |
|---|---|
| `stack_store(x, ss, off)` gained a leading `pointer_type` | 8 sites |
| `stack_load(ty, ss, off)` gained a leading `pointer_type` | 25 sites |
| `MemFlags` became an interned u16 handle; `trusted()` moved to `MemFlagsData` | 7 sites, import swapped |
| `jump`/`brif` block arguments are now `BlockArg`, not `Value` | 2 sites, `BlockArg::Value(..)` |
| `FunctionBuilder::finalize()` takes a `TargetFrontendConfig` | 1 site |

The `pointer_type` threading was the bulk of it: the free helper functions (`ld2`, `st2`,
`emit_cmp`, `emit_binop`, `emit_shift`, `emit_binop_arith`) had no access to it, so the
parameter had to be pushed through transitively — a fixed-point iteration, since adding it
to `ld2`/`st2` made *their* callers need it too.

### Verification

- default (non-JIT) build unaffected: c906 `cost=727`, **stdout byte-identical** to
  pre-update, 284.38 G
- **`XEZIM_JIT=1` output byte-identical to the interpreter on BOTH designs** — c906
  `cost=727` (154/3607 blocks compiled), c910 `cost=216` (58/21305). This matters: the JIT
  has a documented history of silent wrong answers (the inline-bits `LoadSignal` X/Z bug),
  so "it compiles" is not evidence.
- **1758 passed / 0 failed** with `--features jit`

### The JIT is still slower, and by more than before

c906 memcpy x50, 2 interleaved reps, same binary:

| | interpreter | JIT | delta |
|---|---|---|---|
| instructions | 284.7 G | 294.4 G | **+3.4%** |
| cycles | 135.0 G | 164.5 G | **+21.8%** |
| wall | 35.08 s | 41.81 s | **+19.2%** |
| IPC | 2.11 | 1.79 | -15% |

So 25 versions of cranelift codegen work did **not** change the verdict — the JIT remains
net negative, and the IPC collapse to 1.79 reproduces the 0.109 measurement almost exactly
(1.78 then). That is consistent with the original diagnosis: the loss is architectural, not
a codegen-quality problem. VM registers live in cranelift stack slots, so native code
reproduces the interpreter's memory traffic minus dispatch, then pays it back in FFI bridge
calls. Raising coverage first would make it slower still.

## Cache-fit structure (Aug 2026) — two diagnostic defects, and two pathological arrays

### The diagnostic was lying about the hardware

`XEZIM_CACHE_FIT=1` hardcoded `L1d=32KiB L2=1MiB L3=16.5MiB`. The actual machine is a Tiger
Lake i7-1165G7: **L1d=48 KiB, L2=1.25 MiB, L3=12 MiB**. Every "x L3" ratio was therefore
~37% understated — the headline "79x L3" is really **109x L3**. Now read from
`/sys/devices/system/cpu/cpu0/cache/`.

Second defect: `one signal's hot bytes across arrays = 39 B (spans 1 lines if scattered)`
divided 39 by the 64-byte line size — which answers the *contiguous* question, the opposite
of what "if scattered" means. Those 39 bytes live in **five separate arrays**, so touching
all of them costs **up to five distinct cache lines**, not one. A layout report that makes a
5-line access look like 1 is worse than no report.

### The layout, corrected

| array | c906 | c910 | indexed by |
|---|---|---|---|
| `signal_table` | 1071.6 MiB | 1097.3 MiB | signal (32 B/sig, 2 per line) |
| `signal_widths` (u32) | 133.9 MiB | 137.2 MiB | signal |
| `dep_offsets` (u32) | 133.9 MiB | 137.2 MiB | signal |
| `signal_real` / `signal_two_state` / `dirty_signals` | 33.5 MiB each | 34.3 MiB each | signal |
| `comb_entries` | 3.2 MiB | 22.4 MiB | comb entry |
| `settle_triggered` | 0.05 MiB | 0.35 MiB | comb entry |

Hot per-signal working set: **1306 MiB (c906) / 1337 MiB (c910) = 109x / 111x L3.**

### Finding 1 — `signal_widths` (134 MiB) is provably redundant

`Value` already carries `width`, in the same cache line as the bits. Added a full-table
comparison to the report: **identical for all 35,114,136 signals on c906 and all 35,955,898
on c910**, after complete 50-iteration runs. They are seeded together
(`signal_table[id] = Value::zero(signal_widths_vec[id])`) and never diverge.

BUT the value is memory, not speed: the hot loops barely read it — **0 reads in
`exec_insns`, 0 in `check_edges_inner`, 2 in `settle_combinatorial_inner`** out of 100 sites
repo-wide. So deleting it is a ~134 MiB (10% of the signal working set) RSS win that should
be roughly perf-neutral — pure deletion of redundant data, adding no instructions, unlike
every re-encoding attempt this session. Note the report itself counts it in the "hot"
working set, which overstates that figure by 134 MiB.

### Finding 2 — `dep_offsets` (134 MiB) is 0.094% useful and IS hot-path read

`[DEP_STATS] dep_edges=101661 dep_signals=32986 avg_dep_fanout=3.08 max_dep_fanout=3073`

**Only 32,986 of 35,114,136 signals have any comb dependent — 0.094%.** Yet `dep_offsets` is
a dense `Vec<u32>` over every signal, 133.9 MiB, and `trigger_deps!` reads
`dep_offsets[tid]` / `[tid+1]` on **every** propagation. So the overwhelmingly common case —
a signal write with no comb reader at all, e.g. the ~33 M array/memory elements — pays a
random access into a 134 MiB array to learn "nothing depends on me".

A "has any comb dependent" **bitset** is `num_signals/8` = **4.2 MiB**, 32x smaller and
L3-resident, gating access to the big array. Two reasons this is not the `settle_triggered`
bitset that failed: it is **built once and read-only at runtime**, so there is no
store-to-load false serialisation (the mechanism that sank that one); and it *avoids* a
larger access rather than re-encoding an existing one. Unmeasured — the honest next step is
to count what fraction of `trigger_deps!` calls hit signals with zero dependents before
building anything.

### Why both designs have ~35 M signals

c906 and c910 report 35.1 M and 36.0 M signals despite very different core sizes, and only
1.56 M are named on c906 (4.4%). The count is dominated by array elements — the testbench
memory — not by design logic. Any per-signal array is therefore sized by the testbench RAM,
which is why these arrays are 134 MiB while the entries that use them number ~52 K.

## The cache-fit framing is misleading, and that explains the whole session

Acting on the `dep_offsets` finding: **66.5% of `trigger_deps!` calls hit signals with ZERO
comb dependents** (228,588,579 of 343,587,176 on c906) — every one a random probe into a
134 MiB array only to learn "nothing depends on me". Built the gate: a read-only
`comb_dep_any` bitset, `num_signals/8` = 4.2 MiB, consulted before `dep_offsets`.

Correct (`cost=727`, stdout byte-identical) and **slower**: c906 memcpy x50, 3 interleaved
reps, two binaries — **instructions +0.46%, cycles +1.19%, wall +0.9%**. Reverted.

### Why it lost, and why it matters more than the optimization

Measuring the machine instead of the allocation:

| counter | c906 memcpy x50 |
|---|---|
| instructions | 284.5 G |
| cache-references | 6.67 G |
| cache-misses | 484 M (7.3% of references) |
| **LLC-load-misses** | **54.8 M** |
| **dTLB-load-misses** | **170 M** |

54.8 M LLC misses is ~3.5 GB of DRAM traffic across a 35 s run — roughly 100 MB/s, which is
nothing. **The arrays allocate 1.3 GB but the touched working set is small**, because the
signals actually written and read repeatedly are a tiny concentrated subset (the memcpy
buffer, the active flops) while the ~33 M testbench-memory elements are mostly cold or
re-touched in place.

So `[CACHE-FIT] hot per-signal working set = 1306 MiB => 109x L3` measures **allocation, not
locality**. There is no cache bottleneck to fix. That retroactively explains four separate
failures this session, all of which optimized a problem that is not there:

| attempt | footprint change | result |
|---|---|---|
| SoA 16-byte signal cells | -212 MB | +11.3% |
| `settle_triggered` bitset | -321 KB | +1.4% insn, +1.6% cycles |
| bit-granular sensitivity | +3 arrays | +0.4% insn, +1.6% cycles |
| `comb_dep_any` gate | -0% (adds 4.2 MiB) | +0.46% insn, +1.19% cycles |

**Rule going forward: do not propose a memory-layout optimization for this workload.** At
0.019% of instructions causing an LLC miss, xezim is instruction-bound with no meaningful
locality headroom. Only instruction-count reductions can pay.

### One live lead the counters did surface

**dTLB misses (170 M) are 3x the LLC misses.** The ~1 GB arrays span ~340 K 4 KiB pages,
far past TLB reach, so page walks — not cache misses — are the memory-side cost. At ~10-20
cycles a walk that is ~1.3-2.5% of the run, and it costs *no instructions* to fix.
System THP is `madvise`-only, and mimalloc's `MIMALLOC_ALLOW_LARGE_OS_PAGES=1` /
`MIMALLOC_LARGE_OS_PAGES=1` changed nothing (dTLB 161.3 M -> 165.7 M / 159.9 M, cycles flat)
because the big `Vec`s are single large allocations served directly by `mmap`. Testing this
properly needs an explicit `madvise(MADV_HUGEPAGE)` on the large arrays — a small code
change, not a config flag. Untested; the only memory-side lever left with a plausible
mechanism.

## Huge pages: the feature already exists, and it is INERT on this machine

`Simulator::advise_hugepages()` (`simulator.rs:13480`, called from `simulate()` at 13539)
already exists, is **default-on** (`XEZIM_HUGEPAGE=0` disables), and covers exactly the
arrays the TLB analysis pointed at: `signal_table`, `signal_widths`, `signal_signed`,
`signal_real`, `signal_two_state`, `dirty_signals`, `comb_dep_offsets`, `comb_dep_entries`,
`comb_entries`. It issues `MADV_HUGEPAGE` then `MADV_COLLAPSE`.

**Correction to the previous section:** the 170 M dTLB misses reported there were measured
*with this already enabled*. Huge pages were never an untried lever.

### Both madvise results were discarded, hiding a total failure

Added `XEZIM_HUGEPAGE_STATS=1`:

```
[HUGEPAGE] signal_table    1071.60 MiB  HUGEPAGE=ok COLLAPSE=errno 22 (2MiB-aligned)
[HUGEPAGE] signal_widths    133.95 MiB  HUGEPAGE=ok COLLAPSE=errno 22 (2MiB-aligned)
...  (every array identical; comb_dep_entries 0.39 MiB SKIPPED, < 2 MiB)
```

**`MADV_COLLAPSE` fails with EINVAL on every array.** And per the function's own comment,
`MADV_HUGEPAGE` alone cannot help here — the arrays are fully populated by `compile()`
before the advice is issued, so there are no future faults left to steer.

### It is the environment, not xezim

A standalone C probe: a fresh 64 MiB anonymous `mmap`, 2 MiB-aligned, fully faulted —
`MADV_COLLAPSE` still returns EINVAL. And checking `/proc/self/smaps_rollup`:

| case | AnonHugePages |
|---|---|
| advise **then** fault (the ideal case) | **0 kB** |
| fault then advise (what xezim does) | **0 kB** |

`transparent_hugepage/enabled` reports `always [madvise] never`, but **no huge pages are
obtainable at all** — a container/VM restriction below the sysfs knob.

Consequences:
1. The hp-on vs hp-off A/B (LLC -4.8%, cycles -0.5%) is **noise, not a huge-page effect** —
   nothing was ever backed by a huge page in either arm.
2. The "dTLB misses are 3x LLC misses, worth 1.3-2.5%" lead **cannot be tested on this
   machine**. It remains plausible and unverified.

### The design fix worth making on a machine where THP works

`advise_hugepages()` runs from `simulate()`, i.e. *after* `compile()` has populated every
array — which is precisely the case `MADV_HUGEPAGE` cannot serve, leaving the whole feature
dependent on `MADV_COLLAPSE`. Issuing the advice at **allocation time**, before the arrays
are filled, is the case THP is designed for and would not need `MADV_COLLAPSE` at all.
Deliberately NOT implemented here: it cannot be measured in this environment, and shipping
an unmeasurable change is exactly what the rest of this document argues against.

## Huge pages: FOUND AND FIXED — the blocker was an inherited `PR_SET_THP_DISABLE`

**This supersedes the previous section, which concluded huge pages were unobtainable here.
That was wrong.**

`/proc/<pid>/status` exposes a `THP_enabled` field. Walking the process ancestry:

```
pid=2505879 name=bash    THP_enabled=0     <- our shell
pid=2202357 name=claude  THP_enabled=0     <- the launcher
pid=1511303 name=bash    THP_enabled=1
pid=1511302 name=sshd    THP_enabled=1
```

The launching process had `prctl(PR_SET_THP_DISABLE, 1)` set, and **the flag is inherited
across fork+exec**, so every xezim run inherited it. While set, the kernel accepts
`madvise(MADV_HUGEPAGE)` — returns 0, and `VM_HUGEPAGE` genuinely appears in the VMA's
`VmFlags` as `hg` — but the fault handler never attempts a huge page and `MADV_COLLAPSE`
fails EINVAL. Every symptom I had attributed to the kernel:

- `AnonHugePages: 0 kB` even for a fresh 2 MiB-aligned, fully-faulted anonymous mapping
- `thp_fault_alloc` never incrementing (not a compaction failure — the path was never taken)
- `MADV_COLLAPSE` -> EINVAL on every array

Clearing it requires **no privilege** and affects only the calling process.

### Placement matters as much as the call

Clearing it inside `advise_hugepages()` (which runs from `simulate()`) recovers only part of
the win, because `compile()` has already faulted the ~1.3 GB of per-signal arrays in as
4 KiB pages, leaving a partial `MADV_COLLAPSE` to clean up:

| where cleared | dTLB-load-misses | cycles |
|---|---|---|
| not at all (previous behaviour) | 163.8 M | 133.60 G |
| in `advise_hugepages()` (late) | 93.7 M | no win |
| **first line of `main()`** | **64.9 M** | **127.95 G** |

So it is done at the top of `main()`, before any large allocation. `XEZIM_HUGEPAGE=0` opts
out and is checked in both places.

### Result — c906 memcpy x50, 3 interleaved reps, same binary

| | THP off | THP on | delta |
|---|---|---|---|
| **dTLB-load-misses** | 163.8 M | **64.9 M** | **-60.4%** |
| **cycles** | 133.60 G | **127.95 G** | **-4.23%** |
| instructions | 284.47 G | 280.28 G | -1.47% |
| wall | 34.72 s | 33.99 s | -2.11% |
| IPC | 2.129 | 2.191 | +2.9% |

The instruction drop is real, not measurement error: ~260 K fewer minor page faults means
less kernel fault-handling code executed (perf counts user+kernel).

Gates: `cost=727` and stdout **byte-identical** with THP on vs off; c910 `cost=216`
`finish=2282050`; **1758 tests passed / 0 failed**; benches 200000 / 100000 / 50000;
`--features jit` builds.

### Why this took so long to find, and the lesson

I twice concluded from strong-looking evidence that the environment could not provide huge
pages — the second time after a standalone C probe showed `AnonHugePages: 0 kB` on a fresh
aligned mapping, which felt conclusive. It was not: every check I ran confirmed the
*request* was well-formed while none checked whether the *process* was permitted to receive
one. `THP_enabled` in `/proc/<pid>/status` is the one field that answers that, and a
per-process, inherited policy bit is invisible to every system-wide knob
(`transparent_hugepage/enabled`, `defrag`, the per-size mTHP controls) that I did check.

**This is also the first genuine performance win of the session, and it is not an
instruction-count reduction** — it is the one memory-side effect that survived, precisely
because it costs no instructions to obtain. That is consistent with the earlier finding that
xezim is instruction-bound: page-walk cost was the one memory-side overhead not already
absorbed by the small touched working set.

## Stage 2 (flat ExecInsn interpreter) — built, measured, rejected

The exec-representation migration's Stage 2 was piloted end-to-end: a 16-byte fixed-width
`FlatInsn` (u8 opcode, u16 regs, u64 imm, side pools for constants/arrays), a 52-op flat
interpreter transliterated arm-for-arm from `exec_insns` (same `vm_*` helpers, same NBA and
dirty-list mechanics), a one-way `lower_flat()` in `finish()` with an EXHAUSTIVE `Insn`
match (new variants fail to compile — the anti-`is_supported`-rot design), wired into
`exec_bytecode` and both settle arms, `XEZIM_FLAT=0` escape hatch.

Correct everywhere: `cost=727`, stdout byte-identical, across every configuration.
**Dynamic coverage: 38.7%** of VM instructions executed flat — a real sample.

### Verdict 1 — flat dispatch does not beat the enum match

3 interleaved reps, same binary: **+0.14% instructions, +0.76% cycles.** The 24-byte
66-variant `Insn` match was hypothesized to pay a dispatch tax that a dense u8 jump table
would remove; measured, the two dispatch forms are equivalent. LLVM already compiles the
enum match to a good jump table. (The +1.39% "fusion ceiling" from adding enum variants is
a layout-INSTABILITY effect at compile time, not a steady-state dispatch cost.)

### Verdict 2 — the fusion substrate works mechanically, and still loses

Added a flat-only `BranchSignalUnlessZero` (the 17.3M-per-8-iters `LoadSignal;
BranchUnlessZero` pair whose enum-side version measured net-zero), with dead-register and
branch-target safety checks. Result: **instructions -0.49%** (-1.36G, exactly the fused
pair count x the elided LoadSignal cost — the mechanism is confirmed) but **cycles +1.2%,
worse in every run**. Same lesson as the array-read triple, amplified: removed instructions
are not on the critical path, and at 38.7% coverage BOTH interpreters are hot, so the VM's
I-cache/BTB footprint roughly doubles — a structural cost of any partial-coverage tiering.

### What this kills, and what it does not

- Kills Stage 2's rationale outright: no dispatch win exists to collect, and fusion gains
  on the flat side are eaten by the dual-interpreter overhead. Full coverage would remove
  the dual-loop cost but chase a dispatch delta measured at zero.
- Also retro-explains the JIT verdict: the JIT removes dispatch entirely and still loses —
  because dispatch was never the cost. The VM's ~35 retired insns per bytecode op are in
  the ARM BODIES (Value semantics), not the dispatch.
- Does NOT test Stage 3 (typed registers): that changes the arm bodies themselves —
  `val:u64/xz:u64` planes instead of 32-byte `Value` registers — which is now the only
  untested part of the migration, and the only one aimed at where the cost actually is.

Patch preserved at `scratchpad/STAGE2-flat-interpreter.patch` (1,028 lines, all gates
green). Reverted; tree byte-identical to baseline.

## Stage 3 (typed registers) — built, measured, rejected. The representation axis is closed.

On top of the revived Stage-2 substrate: VM registers as four parallel planes
(`treg_v`/`treg_x` bits + DYNAMIC `treg_w`/`treg_s` header, set per store exactly as
`vm_store` does — deliberately no static width inference, avoiding the JIT's
`update_reg_width_only` hazard class). Every `vm_*` helper was refactored into a shared
bit-level `vmb_*` core called by BOTH loops, so the typed arms cannot drift semantically.
`lower_flat` marks a block `typed` only when no real/fill/wide value can enter a register
(signals checked by width+realness, pool constants by inspection; the op set cannot widen
past 64). Assign/queue mechanics materialise a `Value` from the planes on demand.

Correct: `cost=727`, stdout byte-identical. **Typed coverage: 36.2% of all VM instructions
(96% of flat-eligible ones)** — a real sample. Three-way A/B, same binary, 3 interleaved
reps, c906 memcpy x50:

| interpreter | instructions | cycles |
|---|---|---|
| `Insn` enum loop (`XEZIM_FLAT=0`) | 281.25 G | **131.92 G** |
| Stage-2 value-flat (`XEZIM_TYPED=0`) | **279.88 G** | 132.71 G |
| Stage-3 typed planes (default) | 280.50 G | 134.29 G |

**Typed registers are worse than Value registers on both counters** (+0.62 G insn,
+1.58 G cyc vs value-flat; +1.8% cycles vs the plain enum loop). The reason, measured
rather than assumed: the `Value` fast paths were already near-minimal — `set_inline_bits`
+ header stores on one 32-byte line — while four parallel planes cost four address
computations and up to four cache lines per register touched. The "32-byte Value overhead"
that Stage 3 was meant to remove does not exist on the fast paths that dominate.

### The migration triad is now fully measured

| stage | form | result |
|---|---|---|
| 1 | SoA signal planes | **+11.3%** (header tax) |
| 2 | flat wordcode dispatch | **±0** (dispatch was never the cost) |
| 3 | typed register planes | **worse than Stage 2** on insn AND cycles |

Combined with the JIT (dispatch removed entirely: still +19% wall on cranelift 0.134),
every representation-level hypothesis about the VM is now dead by direct measurement. The
~35 retired instructions per bytecode op are the intrinsic cost of 4-state semantics plus
necessary memory traffic — **this interpreter is already near the efficiency frontier for
its semantics.** Future effort should go to executing FEWER bytecode ops (algorithmic /
scheduling work, e.g. the armed-worklist inversion) — not to executing the same ops
through yet another representation.

Patch preserved at `scratchpad/STAGE3-typed-registers.patch` (1,847 lines, includes the
revived Stage 2; all gates green). Reverted; tree byte-identical to baseline.

## Armed-worklist inversion — priced by annotation, deprioritized

`perf annotate` on `check_edges_inner` (11.6% self) breaks its time down: per-position
detection scan (clock-tree memo, fanout-empty gate) ~14% of the function; the
`dispatch_block!` fanout walk + gating tests (`fanout.posedge` iteration, `snap_valid`/
`bitsel`/armed loads) ~18-20%; `fired_snap` prev-writeback ~3%. The inversion eliminates
only the walk+gating share: **ceiling ~2.2% of runtime, realistically ~1-1.5% net** after
paying the arm-side push — far below the earlier 3-7% estimate, and it carries the
`woke_any`/`snap_valid` semantic coupling risks. Parked.

## Technique survey (Aug 2026) — what the literature offers vs what is measured dead here

The session's measurements close every *within-paradigm* lever: representation (S1/S2/S3,
JIT), layout/locality (no cache bottleneck: 54.8M LLC misses), activity suppression at
interpreter granularity (bit-granular: machinery > win), batching (cone/BSP/clock-domain:
over-evaluation). The literature's big wins are all **cross-paradigm**:

| technique | source | fit for xezim |
|---|---|---|
| **AOT compilation to C/C++/Rust source** (not a template JIT) | Verilator, ESSENT, Cuttlesim, GSIM — all compiled | **#1 candidate.** The cost is proven to live in arm bodies; LLVM-compiled per-block source gets real register allocation, inlining, and specialization — everything cranelift's stack-slot codegen (+19% wall) could not do. Pilot: emit Rust for the 2,859 pure edge blocks + compiled cont-assigns, build a cached `.so` (the prepared-comb cache precedent). High effort, only untested compilation form. |
| **Two-state execution with X-elision** ("two simulation functions per gate"; Verilator's new 4-state work splits vars into two 2-state planes — exactly xezim's val/xz) | Antmicro 4-state Verilator (Apr 2026), classic event-driven X-prop literature | **#2.** The measured ~35 insn/op floor is 4-state semantics; after reset X mostly vanishes but every op still carries the xz plane. Block-level 2-state variants (selected when operands are X-free, like the JIT's X/Z pre-check bail) halve data movement on the dominant path. Untested here. |
| **Essential-signal / conditional eval** | ESSENT, GSIM (7-20x over Verilator on XiangShan/Rocket, arXiv 2508.02236) | Interpreted version measured dead (bit-granular, 38 tests/avoided eval); only pays fused into COMPILED code where the check inlines to ~2 insns — i.e. it rides on #1, not standalone. |
| **Replication-aided partitioning** | RepCut | Machinery exists in-tree (multikernel/PDES, hypergraph partitioner). Parallel self-calibration currently picks sequential on c906-scale; c910-scale re-tuning is the realistic angle. |
| Circuit deduplication | DRY (2025) | Compile-time/I-cache win for compiled simulators; an interpreter already shares one loop. Not applicable. |
| GPU batch stimulus | RTL-to-CUDA (2022) | Different use case (many parallel stimuli), not single-run latency. |

**Conclusion:** xezim's interpreter is at its paradigm's frontier (measured, not asserted).
The remaining order-of-magnitude lives where Verilator/GSIM get theirs: ahead-of-time
compilation with 2-state specialization and inlined activity checks. Everything smaller
than that paradigm jump is now measured at ≤ ~1.5%.

## Questa techniques #2 (vopt) and #3 (two-state) explored — and a benchmark-fidelity discovery

### The headline is not an optimization: XEZIM_INIT_ZERO was distorting the Verilator comparison

Measuring X rates required a no-`INIT_ZERO` control run, which resolved the year-old
"fidelity caveat" in the workload notes ("xezim runs ~1.64x more clock cycles for the same
firmware; INIT_ZERO is a plausible suspect" — never investigated). Confirmed, c906 memcpy
x50, same binary:

| | cost | finish | instructions | wall |
|---|---|---|---|---|
| `XEZIM_INIT_ZERO=1` (the golden config) | 727 | 6,477,650 ticks | 282.4 G | 32.6 s |
| default true 4-state | **368** | **3,956,650 ticks = 395.7 us** | **189.0 G (-33%)** | **22.9 s (-30%)** |

`cost=368` and `finish=395.7 us` are **Verilator's exact numbers** (368 cycles, 396 us).
In its default 4-state mode xezim simulates the identical firmware execution; `INIT_ZERO`
(required only by cmark) makes the memcpy firmware take a 1.64x longer path, and every
Verilator comparison has carried that inflation. Honest ratio: **22.9 s vs 0.89 s ~ 26x**,
directly, no normalization. The `cost=727` fingerprint remains the regression gate for the
golden config; Verilator comparisons should use the default-mode run.

### #3 — two-state opportunity, measured

- **Declared** two-state (`bit`/`int`): 131,072 of 35.1 M signals — **0.37%**. Questa-style
  demotion must PROVE X-freedom (reset analysis); declarations provide nothing here.
- **Dynamic** X rate (new counters, patch preserved): LoadSignal operands with X/Z
  **0.55%** under INIT_ZERO, **0.87%** in true 4-state; NBA writes with X/Z **0.00%** /
  0.11%. So 2-state specialized bodies would run **>99% of dynamic executions**.
- **State vs flow**: in true 4-state mode **95.2% of all signals hold X at end** (never-
  written testbench memory) while the executed path is >99% X-free — X is everywhere in
  STATE, nowhere in FLOW. Sparse-X storage is a memory play (and layout plays are dead here).
- Value bound: removing xz-plane computation from X-free bodies is worth perhaps 20-30% of
  arm work => **ceiling ~3-5% of runtime** — but as a THIRD interpreter loop it pays the
  dual-loop I-cache tax measured at ~+1.2% cycles, so interpreted it is marginal. It
  becomes free-riding only inside AOT-compiled bodies (Questa does exactly this: compiled
  2-state code behind 4-state boundaries). The JIT's X/Z pre-check-bail (`xz_bail`) is the
  in-tree precedent for the guard pattern.

### #2 — vopt-style passes, priced individually for an interpreter

- **Process merging**: measured dead (clock-domain batching: dropping per-block gating
  costs +19%; the gating IS xezim's activity optimization). Only viable inside compiled
  bodies with inlined checks.
- **Aliasing / hierarchy flattening**: port-copy entries are 12.3% of settle evals; chain
  collapse was measured <1%; true single-id aliasing also removes writes+triggers but
  breaks independent force/observability — requires an `+acc`-style visibility contract.
  Ceiling ~1-2%.
- **Constant propagation**: the 92.8% armed-skip already harvests runtime stability
  dynamically; static folding would remove residual prefilter cost, priced ~1-1.5%.
- **Dead logic**: the never-written 95% of signals are a footprint play — perf-neutral by
  the LLC evidence.

**Conclusion:** each vopt pass, interpreted, is worth ~1-2%; Questa's stack pays because
the passes multiply INTO compiled code and the visibility trade. Both threads point at the
same door as the technique survey: AOT compilation first; 2-state specialization and
vopt-style passes as multipliers on top of it, not standalone.

Dynamic-census instrumentation preserved at `scratchpad/XZ-DYNAMIC-CENSUS.patch`
(+0.77% hot-path tax when compiled in — reverted per protocol).

## AOT compilation — piloted end-to-end, and it closes the loop on WHY compilation needs 2-state

Built the full pipeline (patch: `scratchpad/AOT-pilot.patch`, 533 lines): generate Rust
source for pure edge blocks (registers as `Value` locals, ops as inlined `xezim-core`
methods — layout identical BY CONSTRUCTION via a path dependency), basic-block state
machine for control flow, ONE FFI callback per NBA write plus a "host executes this one
Insn via `exec_insns` on a 1-insn slice" escape (zero duplicated semantics), cargo-built
cdylib cached by source hash (81.5 s first build, then cache hit), dlopen + per-block
symbols. `XEZIM_AOT=1` opt-in.

**It worked immediately and exactly**: 1,566/3,607 blocks compiled, `cost=727`, stdout
byte-identical to the interpreter across all runs.

### Verdict — the JIT's failure shape, amplified, and now fully explained

| | interpreter | AOT | |
|---|---|---|---|
| instructions | 280.60 G | 277.34 G | **-1.16%** (real) |
| cycles | 128.77 G | 147.69 G | **+14.7%** |
| IPC | 2.18 | 1.88 | |
| L1i-load-misses (x25) | 637 M | 1,369 M | **2.15x** |
| ifetch stall cycles (x25) | 1.90 G | 5.04 G | **2.65x** |

The generated `.so` is **4.1 MB of native code for 43% of blocks**. Inlining full 4-state
`Value` semantics turns every bytecode op into ~KB of machine code; 1,566 functions cycle
through the I-cache where the interpreter runs one resident loop. Fewer instructions
retired, catastrophically worse fetch behaviour — the cranelift JIT's signature (IPC
2.11→1.79), reproduced with GOOD codegen, which acquits cranelift: the problem was never
codegen quality.

### The terminal insight of the whole performance arc

- Stages 1-3: the interpreter's per-op cost is intrinsic 4-state semantics, not
  representation.
- JIT + AOT: compiling that 4-state work VERBATIM explodes code size and dies on I-cache,
  regardless of compiler quality.
- Therefore compilation only pays TOGETHER WITH 2-state specialization (which shrinks
  `a<=b&c` to a few instructions) — they are not independent techniques but one paired
  move. This is exactly the Verilator/GSIM/Questa formula, now derived from xezim's own
  measurements: **compile AND demote, or do neither.**

The measured assets for that future combined attempt: >99% of dynamic executions are
X-free (the 2-state guard fires rarely), the JIT's X/Z pre-check-bail is the guard
pattern, and this pilot's pipeline (codegen/cache/dlopen/bridges) is reusable as-is.

## Combined 2-state AOT pilot — the mechanism works; the population is wrong

Rebuilt the AOT pipeline with 2-state codegen (patch:
`scratchpad/AOT-2STATE-pilot.patch`, 665 lines): registers are `(u64, u32, bool)` locals,
**signal widths/signedness baked as constants** (provably immutable), a `has_xz()` entry
guard over the block's read set, X-producing corner cases (out-of-range selects) bail with
`return 1` and the host re-runs the block on the interpreter (value-idempotent: NBA queue
pushes are last-write-wins). Mul/Div/Mod/shifts/dynamic-range excluded pending width-rule
proofs.

**Everything about the mechanism worked**: 1,046 blocks compiled, `cost=727`/`1113`,
stdout byte-identical, **bail rate 1.1–2.6%** (matching the measured dynamic X rate),
`.so` 2.16 MB (vs 4.05 MB 4-state), and the cycle regression collapsed **+14.7% → +1.7%**
with L1i misses down from 2.15x to 1.07x. The 2-state thesis is confirmed: X-elision is
what makes compiled 4-state simulation viable.

### But: instructions only -0.17%, because the addressable population is 1.3% of fires

`native=2,514,523` of ~189 M block fires. The reason is the session's own earlier finding
turned around: **pure blocks are exactly the ones the armed-skip already suppresses 92.8%
of the time.** The gate-level flops AOT can compile are cheap AND rarely execute; the
blocks that actually burn interpreter time are the IMPURE ones — `Nba(array)` (633 blocks,
56% of non-pure insns, every register-file port), `BlockingAssign` state machines — which
need array semantics and dirty propagation in native code to compile.

### The AOT arc, complete

| pilot | insn | cycles | verdict |
|---|---|---|---|
| 4-state AOT (1,566 blocks) | -1.16% | **+14.7%** | code-size death |
| 2-state AOT (1,046 blocks) | -0.17% | +1.7% | mechanism proven, population disjoint from heat |

To make AOT pay, the compiled set must include the impure hot blocks: array NBA writes,
blocking assigns with dirty-list/`after_signal_write` effects, and the settle-side
`CompiledContAssign` bodies (26.9% of settle evals). That is a full compiler backend, not
a pilot — with the 2-state guard/bail architecture validated here as its foundation, the
codegen/cache/dlopen pipeline reusable, and one measured warning to carry: per-block
native functions must stay small or I-cache eats the win.

## Full-stack measurement in true-4-state mode (INIT_ZERO off), with the Questa checklist

c906 memcpy x50, default mode (the Verilator-equivalent execution), 3 interleaved reps:

| config | cost | instructions | cycles | wall |
|---|---|---|---|---|
| interpreter, all default opts | **368** | 187.91 G | 87.06 G | **22.8 s** |
| + 2-state AOT (`XEZIM_AOT=1`) | 368 | 187.60 G (-0.17%) | 88.56 G (+1.7%) | 23.3 s |
| (golden INIT_ZERO config, for reference) | 727 | 280.3 G | ~128 G | ~34 s |

Byte-identical between arms; AOT bail rate 2.6% even in true 4-state — X stays out of the
executed flow. AOT remains net-negative for the same population reason; the right setting
is OFF.

**Verilator comparison, fully honest**: 22.8 s vs 0.89 s = **25.6x**, identical simulated
execution, no normalization.

### The Questa optimization checklist, resolved against measurement

| Questa technique | xezim status |
|---|---|
| 1. Native compiled processes | Piloted both ways (4-state, 2-state guard). Mechanism proven; net negative until the IMPURE hot blocks compile. OFF. |
| 2. vopt global optimization | Partially in-tree (peephole fusion, resize elision, clock-tree dedup). Process merging / aliasing / const-prop each measured ~1-2% and blocked by the always-on visibility contract — an `+acc`-style mode is the enabler xezim lacks. |
| 3. Two-state solving | Guard architecture proven (1-2.6% bail). As interpreter specialization: marginal. Pays only inside compiled bodies. |
| 4. Event-count reduction | IN-TREE AND DEFAULT-ON — the armed-skip (92.8%), clock-tree dedup, NBA elision. This is where xezim's existing -26.7% lives, and it is exactly the Questa layer xezim already matches. |
| 5. Workflow (incremental, PDUs) | Prepared-comb cache + AOT source-hash cache are the analogues. |

So "all Questa optimizations that measure positive" = the default binary: the event-layer
is already Questa-grade, huge pages are on, and the compile/2-state pair waits on a
backend for the impure hot blocks plus a visibility contract.

## c910 in both init modes — INIT_ZERO is irrelevant on c910

| run | INIT_ZERO=1 | INIT_ZERO=0 |
|---|---|---|
| memcpy x50 | cost=216, finish=2282050, 789.0 G, 230.8 s | **identical fingerprints**, 783.4 G, 231.8 s |
| cmark x2 | 158034 cyc, finish=34985250 | **identical fingerprints**, CoreMark 1.0 PASSED, 2626.6 s |

Both c910 workloads simulate the exact same execution with X-initialization on or off —
the 1.64x firmware-path divergence was c906-memcpy-specific. Consequences: c910's golden
fingerprints were Verilator-comparable all along, and the long-standing "INIT_ZERO=1 is
required for cmark" rule is disproven for c910 (c906 cmark remains untested without it).

## 2-state AOT on c910 cmark — the first measured AOT win

c910 cmark x2, `INIT_ZERO=0`, single run per arm:

| | interpreter | 2-state AOT (`XEZIM_AOT=1`) |
|---|---|---|
| cycles / finish | 158034 / 34985250 | **exact match**, TEST PASSED |
| eligible blocks | — | 9,705/21,305 (45.6%) |
| native block fires | — | **156,592,110** (bail 3.09%) |
| one-time crate build | — | 313.5 s (source-hash cached) |
| wall net of build | 2626.6 s | **2532.3 s (-3.6%)** |

Phase split from the `[PHASE]` timers (the AOT build lands inside the "simulation" phase —
`aot_init()` runs from `simulate()`):

| phase | interpreter | AOT |
|---|---|---|
| simulator construction | 2.51 s | 2.47 s |
| SV -> bytecode compilation | 9.52 s | 9.34 s |
| AOT crate build | — | 313.5 s (first run only) |
| **pure simulation** | **2616.6 s** | **2522.5 s (-3.6%)** |

Startup is identical between arms (~12 s), so the whole -94.1 s is in the simulation loop
where the 156.6 M native fires happen — not a startup artifact. Note xezim's front-end
remains ~12 s for all of c910; the AOT build is the new dominant startup cost, hence the
cache (and, for a shippable version, a fire-count heuristic before building).

The c906-memcpy verdict ("compilable and hot are disjoint") was a WORKLOAD property, not
a technique property: on cmark the c910 core computes continuously, the pure gate-level
blocks actually fire (156.6 M native executions vs c906 memcpy's 2.5 M), and with real
dynamic coverage the 2-state compiled bodies measure **-3.6% wall** — the first positive
AOT result across four pilots. Caveats: single run per arm (44-min runs; ±1-2%
uncertainty), and the 313.5 s first-build cost is real though cached thereafter.

Where this leaves the AOT ledger: correct everywhere (byte-exact on four
workload/mode combinations), negative on memcpy-class workloads (population), positive on
cmark-class (coverage). A shippable version needs: per-design opt-in or a fire-count
heuristic before building, plus confirmation reps — but the paradigm door is now
measurably open, on exactly the terms the session's analysis predicted (compile + 2-state
as one move).

## The 9x QuestaSim gap, decomposed from the post-AOT steady-state profile

QuestaSim reportedly runs these workloads ~9x faster (~280 s where xezim spends 2,522 s on
c910 cmark). Profiled the steady state (200 s sampling delay to skip elaboration) of c910
cmark WITH the 2-state AOT active:

```
37.3%  exec_insns              <- interpreter, STILL — see below
21.4%  settle self             16.6%  check_edges        9.7%  snapshot_edge_signals
 3.7%  memmove                  2.2%  after_signal_write  1.1%  allocator
  <1%  each: all 9,705 AOT native fns (aggregate small) + exec_bytecode 0.9%
```

Two facts jump out:

1. **The 156.6 M native fires cost almost nothing** — no AOT function reaches 1%. Native
   execution of pure flops is nearly free; that is why the win was only -3.6%: the pilot
   compiled the cheap work.
2. **`exec_insns` is still 37.3%** because the pilot hooked only the EDGE path. Everything
   settle-side — `CompiledContAssign`/`CompiledAlwaysBlock` bodies (27%+ of settle evals)
   — plus the 11.6 K ineligible edge blocks still interprets.

### The gap, bucketed

| bucket | share | what Questa does instead |
|---|---|---|
| interpretation remaining (`exec_insns`) | ~37% | compiles every process |
| scheduling machinery (settle self + check_edges + snapshot) | ~48% | compiled sensitivity, merged processes, no per-tick snapshot gather |
| memory/allocator/misc | ~7% | — |
| actual compiled compute | ~8% | this is the part that scales |

2,522 s x ~0.10-0.15 residual ≈ **250-380 s ≈ the reported 9x**. The gap is fully
accounted for — no magic, just: (a) compile ALL bodies, not the pure-flop subset, and
(b) collapse per-event scheduling into the compiled code.

### The roadmap this fixes in place

1. **AOT the settle comb bodies** (largest single brick, attacks the 37%): a `blk_assign`
   bridge (BlockingAssign semantics host-side, one call per write — the `nba_push`
   pattern), a second fn table for `comb_entries`, dispatch in the settle arms (the
   Stage-2 wiring points, already mapped). Estimated 10-20% total.
2. **`snapshot_edge_signals` at 9.7%** (vs 2.9% on c906) — a per-tick gather over 2.4 M
   named signals that dwarfs c906's share on tick-light workloads. Untouched by
   everything so far; deserves its own investigation.
3. **Per-fire scheduling overhead** — the ideas measured dead at interpreter granularity
   (batching, merged dispatch) change economics once bodies are native, because the
   per-fire constant becomes the dominant cost of a fire. Re-price AFTER (1).

The pilots' verdicts stand: each increment must re-measure. But unlike the start of this
session, the remaining 9x is now an itemized bill, not a mystery.

## Settle-comb AOT — built with a 3-agent debug cycle, measured, rejected

Extended the 2-state AOT to settle comb bodies: a `blk_assign` bridge (the interpreter's
BlockingAssign arm verbatim), a second fn table, dispatch in both settle arms. Three
parallel audit/test agents caught, pre-measurement: a silent no-op patch replace (dropped
eligibility arms), **two live mixed-signedness comparison bugs** (`ts_eq` extended both
operands with `asg&&bsg` where the interpreter uses each operand's own signedness;
casez/casex were mapped to sign-extending equality when they NEVER sign-extend), and two
unbounded-width holes (`Resize`, `NbaAssign`). All fixed; gates byte-identical in both init
modes afterward. NOTE: the two comparison bugs also exist in the superseded
`AOT-2STATE-pilot.patch` — use `AOT-2STATE-COMB.patch` (which contains the fixes) as the
base for any future work.

c910 cmark x2, INIT_ZERO=0: **72,645 blocks native (10,139 edge + 62,506 comb),
1.62 BILLION native fires, bail 0.546%, byte-exact golden — and pure sim
2616.6 s -> 3090.1 s (+18.1%)**, worse than both the interpreter and edge-only AOT
(2522.5 s). One-time crate build: 22 min.

**Why:** comb bodies average ~3-6 bytecode ops. Each native call pays ctx construction,
an indirect call, the entry X-guard, and a Value materialization + bridge call per write —
per-entry glue now exceeds the body it replaces. Edge blocks won because they amortize the
boundary; settle entries cannot. This is the settle-side twin of the fusion-ceiling
lesson, and it sharpens the Questa formula once more: compiled processes only pay when
the SCHEDULING is compiled with them (merged cones with inlined queueing) — one native
function per tiny entry behind a bridge is structurally the wrong shape.

Final AOT ledger: edge-AOT on compute-heavy workloads = -3.6% (real, kept as the
recommended opt-in shape); everything else measured negative. The next genuine step
remains a backend that compiles ENTIRE settle cones (scheduling included) — a compiler
project, not an increment.

## Merge of upstream (25 + 14 commits, Aug 2026) — and the death of the cost=727 fingerprint

Merged cleanly except one dispatch.rs conflict (upstream added `ClearSigned`/`Pow` opcodes
next to our fused ones; both kept, NUM_OPCODES 68 -> 70, pin-assert order preserved).

**Post-merge gates:** tests **1798/0** (upstream added 40), c906 default-mode
`cost=368` PASSED, c910 memcpy `cost=216/finish=2282050` PASSED — but **c906 with
`XEZIM_INIT_ZERO=1` now wedges** (testbench watchdog: "no instructions retired", t=5.0M).
A pristine `origin/main` worktree build fails identically, so this is an upstream
behavioral change (the §9.2.2.2/§9.4.2/NBA-region compliance commits), not a merge
artifact — and it lands precisely on the configuration this session already proved
artificial (X->0 coercion that made c906 diverge 1.64x from Verilator).

**Consequence: the golden fingerprints move.** `cost=727` is no longer reachable on
current code; the regression gate is now the default-mode set: c906 `cost=368`, c910
`cost=216/finish=2282050`, c910 cmark `158034/34985250` (all verified passing post-merge),
plus the 1798-test suite. cmark on c906 under INIT_ZERO is untested post-merge; c910 needs
no INIT_ZERO at all.

## INIT_ZERO refactor design (research, Aug 2026)

Current `XEZIM_INIT_ZERO=1` (`simulator.rs:5499`, `:5630`) coerces EVERY X-valued signal
(nets included, plus all array elements) to 0 at CONSTRUCTION time — invisible to the
event system: no write, no dirty marking, no X->0 transition. Three measured consequences:
the 1.64x c906 firmware-path distortion (nets forced into unreachable states), the
post-merge wedge (upstream's stricter event semantics need the initial transitions that
coercion suppresses), and c910 needing the flag for nothing.

Design (VCS `+vcs+initreg` model): (1) select REGISTER-class storage only — edge-block NBA
targets (flop set, derivable via the existing nba_writer_count scan) + procedural
variables; exclude cont_driven/comb-driven nets; memories under a separate
`XEZIM_INIT_MEM` keeping cheap construction-time zeroing. (2) initialize by REAL writes at
simulation start through the standard write path (set_inline_bits + dirty + after_signal_
write) — semantically "initial q = 0;" per flop: time-0 settle derives nets, X->0 fires
negedge (not posedge), armed bits set correctly. (3) knob `XEZIM_INIT_REG=0|random`
(random: per-id LCG — reset-bug hunting), `XEZIM_INIT_ZERO` as deprecated alias.
(4) gates: c906/c910 complete post-merge; on properly-reset designs the flag must be a
behavioral no-op after the first reset (byte-diff from reset onward).

## AOT coverage extension — measured, and it INVERTS the win

Extended eligibility with `NbaAssignRange` (shared-method bridge — the interpreter arm and
bridge call the same `nba_assign_range_val`, no drift possible) and native `Replicate`:
c910 edge eligibility 9,705 -> **17,400/21,305 (82%)**, native fires 156.6M -> **332.1M**.
Byte-exact golden output, bail 4.37%. Pure sim: **2616.6s interp / 2522.5s original AOT
(-3.6%) / 2993.8s extended (+14.4%)**.

**Coverage is not the objective function.** The 175M added fires are 1-3-op bodies
(per-bit nba-range flops, replicates) where per-call glue (ctx + indirect call + X-guard +
Value materialization + bridge) exceeds the body — the settle-comb lesson repeating at the
edge level. The original set was accidentally self-selected for amortizable bodies.
**A shippable AOT needs a fire-weighted body-size heuristic (e.g. compile only blocks with
>= ~8-10 ops), not maximal coverage.** The -3.6% at the 9,705-block set stands as the
best measured configuration. WIP preserved at `scratchpad/AOT-COVERAGE.patch` (includes
the NbaAssignRange shared-method refactor, worth keeping for its own sake).

## INIT_REG implemented — the wedge fixed, and the coercion tax quantified at CoreMark scale

`XEZIM_INIT_REG=0|random` shipped (`60643dd`): register-class signals (NBA targets of
compiled edge blocks minus cont_driven nets) initialized by REAL t=0 writes through
`write_sig!` + dirty marking — "initial q = 0;" semantics. Construction-time coercion
retired; `XEZIM_INIT_ZERO=1` aliases to the new path; arrays split to `XEZIM_INIT_MEM`.

Gates: flag-off byte-identical; c906 memcpy `cost=368` PASSED under `=0`, `=random`, AND
the legacy alias (which wedged before); c910 `216/2282050` (34,997 flops); tests 1804/0.

**c906 cmark x2 under INIT_REG=0: TEST PASSED at `cycles=286469` — vs 714,196 under the
old coercion.** Register-only init runs the same firmware in 2.5x fewer simulated cycles;
combined with memcpy's 727->368, the old flag was distorting every INIT_ZERO benchmark by
1.6-2.5x. New cmark fingerprint: 286469 cycles/iteration under `XEZIM_INIT_REG=0`.
