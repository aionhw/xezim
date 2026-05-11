# c910 memcpy investigation — full analysis

17 rounds of root-cause narrowing on the c910 RISC-V memcpy hang in xezim.
Bug remains unfixed at session end but is characterized to extreme depth.

## Symptom

- xezim runs c910 hello and cmark tests successfully.
- xezim's c910 memcpy test fails: simulation runs to its 1,000,195 ns
  watchdog (`*** Error: There is no instructions retired in the last
  50000 cycles!`), iverilog of the same RTL/program passes at sim
  1,019,650 ns.
- Last retire in xezim varies between rebuilds (T=44605, T=47395,
  T=59315) — "heisenbug" caused by bytecode-binary-layout shifts.
- Last common retire with iverilog is at PC 0x1B92 (T=59295 in one
  capture). After that, iverilog continues but xezim stops.

## What was ruled out (with refs)

| # | Hypothesis | Refuted by |
|---|---|---|
| 1 | NBA merge order (`block_index` vs `eval_order`) | Switching to `eval_order` regresses cmark; default `block_index` already works for cmark. Same memcpy failure in both modes. |
| 2 | `expr_max_width::Index` returning 1 for unpacked array elements | Codex hypothesis tested. Regresses cmark even though theoretically correct. Reverted. |
| 3 | `expr_max_width::Inside` → 1, `SystemCall::$signed` → arg width | Theoretically correct widening. Regresses hello. Reverted. |
| 4 | Case-stmt default-arm not firing for sel=3'b000 | Synthetic `tests/case_default_arm.rs` test passes — local logic correct. |
| 5 | IFU byte-reversal of memory at PC=0 | Misread on author's part. tb.v `f_spsram_large` testbench deliberately byte-reverses inst.pat literals into 4 byte-banks; this is the c910 test fixture's intentional layout, not a xezim bug. xezim correctly reads the byte-distributed form. |
| 6 | Vector pipeline (split_long, VIQ0, VFPU) | All stalled because nothing reaches them; cascading symptom only. |
| 7 | DIVUW dispatch (PC 0x1BA4 REMUW) | Both sims dispatch and retire DIVUW correctly at T=59165. AIQ0 entry-create matches. |
| 8 | AIQ0 entry 2 src0/src1 wb-bit at allocation | All 5 chain stages (`rt_dp_inst0_src0_data[1]`, `is_aiq0_create0_data[37]`, `aiq0_create0_data[59]`, `dp_aiq0_create0_data[59]`, `aiq0_entry2_create_data[59]`) bit-identical between iv and xz. dep_reg_entry's local rdy/wb/rdy_for_issue logic verified correct via standalone synthetic test (`tests/dep_reg_entry_synth.rs`). |
| 9 | Pipedown 227-bit `{N{en}} & data` replicate-AND | Earlier session's RTL ternary substitution failed identically. Synthetic `tests/replicate_and_pattern.rs` covers the pattern in isolation; passes. |
| 10 | Parameter-arithmetic `[P:P-8]` RangeSelect width | Synthetic `tests/range_select_param_arith.rs` covers it; passes. |
| 11 | `XEZIM_CASCADE_LIMIT=64` (settle convergence depth) | Tested. No change. `max_iters=6` already in default — cascade limit never the bottleneck. |
| 12 | `ident_lookup` AST fallback (100,010 per memcpy run) | Same 100,010 in cmark which passes. Only 4 unique idents fail compile, all cross-hierarchy refs starting with `x_ct_core`. AST fallback path works correctly for them. |

## Verified at high precision

The cascade of dependencies that the symptom builds on:

```
xezim memcpy fails at sim 1M watchdog
  ↑
AIQ0 entry 2 src0.wb flop stays 0 (verified via tb.v probe — flop never sets)
  ↑
LSU pipe3 writeback never fires (1 event in xz vs 250 in iv)
  ↑
RB entry stuck in WAIT_RDY (FSM never advances to REQ_BIU)
  ↑
rb_entry_not_sync_fence_ready = 0 (vs iv: 1)
  ↑
wmb_rb_so_pending = 1 (vs iv: 0)
  ↑
WMB SO FIFO not empty (create_ptr=2, pop_ptr=0)
  ↑
biu_lsu_b_vld never asserts AGAIN after T=45785 (4 successful AXI write
responses match iverilog exactly, then xezim stops)
  ↑
biu_pad_awvalid never asserts AGAIN after T=45935 (4 writes match iverilog
exactly, then xezim's CPU pipeline stops sending writes)
  ↑
lbuf_inst_vld / inst0 / inst0_pc / ifu_idu_ib_inst0_data CLEAR TO 0 at
T=45515 ← ROOT-LEVEL SYMPTOM
```

## What the IFU is doing at T=45515

When the pipeline stops at T=45515:
- xezim's IFU clears `lbuf_inst_vld`, `inst0`, `inst0_pc`, the IB output.
- iverilog continues delivering instructions at the same time, including
  inst at PC 0x388 with the T-Head custom-0 opcode `0x0B` (a vector
  load/store at offset 8 within its 16-byte cacheline).
- The 4 already-dispatched stores in xezim drain by T=45935.
- Retires can continue for a while as residual ROB entries commit.
- All AXI traffic ceases by T=45935.
- Watchdog fires at T=1,000,195.

## Unverified hypotheses for next session

Ranked by likelihood and ease of testing:

1. **xezim's IFU byte-bank assembly for non-zero offset within 16-byte
   cacheline.** The 4 stores that complete are all at one cacheline. The
   5th instruction needs to come from a different cacheline OR a
   non-zero offset, and xezim mis-assembles bytes.
   Test: Probe `ibuf_ibdp_inst0`'s per-byte-bank source signals at
   T=45505 in both sims. Find the bank that disagrees.

2. **`apply_nba` order: `nba_fast` drains before `nba_queue`.** AST
   fallback writes via `assign_value` go through `nba_queue`. If both
   compiled (`nba_fast`) and AST-fallback (`nba_queue`) writes target
   the same signal in the same tick, AST writes clobber compiled
   writes. cmark also has 100K fallbacks and passes, but maybe the
   specific signal pattern differs.
   Test: Search for any c910 RTL pattern where a signal is written by
   both a compiled `NbaAssign` and an AST-fallback statement in the
   same block.

3. **`NbaAssignArray` bypass of `nba_fast_index`.** Compiled
   `NbaAssignArray` uses `nba_fast_index` for merge. AST-fallback path
   in `assign_value` writes `signal_table` directly. Mismatched array
   element updates between the two paths could lose writes.
   Test: Add a debug assertion that warns if both paths target the
   same element in the same tick.

4. **`infer_lhs_width` `_ => 32` fallback for unsupported ExprKinds.**
   The function returns 32 for any ExprKind not in
   {Ident, Index, RangeSelect, Concat}. If c910 uses MemberAccess or
   other patterns in an LHS, the inferred width is wrong, causing the
   compiled write to use the wrong width.

5. **Case statement compilation with `ctx_width=0` selector.** Lower
   priority — the synthetic test passed in isolation, but the c910
   instantiation context may expose a corner the synthetic misses.

## Tooling and artifacts

- **Iverilog reference vvp** (built this session, verified TEST PASSED):
  - `/tmp/c910_iv_new.vvp` — basic probe set
  - `/tmp/c910_iv_aiq.vvp` — AIQ0 probes
  - `/tmp/c910_iv_state.vvp` — RB FSM probes
  - `/tmp/c910_iv_fence.vvp` — fence/idfifo probes
  - `/tmp/c910_iv_fifo.vvp` — idfifo internals
  - `/tmp/c910_iv_axi.vvp` — AXI signals
  - `/tmp/c910_iv_lsu.vvp` — LSU writeback chain
  - `/tmp/c910_iv_lsu2.vvp` — LSU 4-source OR
  - `/tmp/c910_iv_rb.vvp` — RB entry internals
  - `/tmp/c910_iv_chain.vvp` — pipedown chain
- **Iverilog rebuild flow** (one critical fix discovered this session):
  Defines wrapper MUST go inside the `-f` filelist, not before it on
  the command line. iverilog does not propagate macros from
  command-line files to filelist entries.
  Command: `iverilog -g2012 -I .../src -I /tmp/c910_iv_inc -o OUT.vvp -f
  /tmp/c910_iv_files_combined.list`
- **xezim VCDs** (memcpy runs at `--max-time 70000`):
  All in `/tmp/fix_memcpy/memcpy/dump_*.vcd`.
- **Timescale conversion**: iverilog writes VCD in 100ps; xezim writes
  in 1ns. Divide iverilog timestamps by 10 to match xezim.

## Definitive bug characterization (Round 22 — Questa cross-reference)

A QuestaSim 2021.1 VCD at `/home/bondan/agent/repo/memcpy_30k_70k.vcd`
covering sim 30K-70K with retire and AXI signals provides
ground-truth retire stream for the memcpy loop region.

**Questa retire stream around T=45005**:
- Cycle T=45005: PCs 0x710, 0x712, 0x714, 0x716 ALL retire across the 3 slots
- Loop body has 4 instructions

**xezim retire stream same cycle**:
- Cycle T=45005: PCs 0x710, 0x714, 0x716 retire (PC 0x712 MISSING)
- Searched all 3 retire slots across the entire run: **PC 0x712 NEVER appears**

PC 0x712 corresponds to the halfword at byte offset 2 within the
16-byte cacheline 0x710-0x71F. The original handoff diagnosed
"vector op at PC 0x712 stuck" — this Questa cross-reference proves
the diagnosis was correct all along; the 22 rounds of downstream
probing chased cascading symptoms while heisenbug probe-set shifts
made the downstream picture inconsistent.

**Bug location**: xezim's IFU never delivers PC 0x712 to the IDU's
dispatch unit. Pre-decode (`ct_ifu_precode.v`) or instruction-buffer
pop (`ct_ifu_ibuf.v` pop_h0/h1 selection) drops it.

The c910 testbench byte-distribution (tb.v:436-454) distributes each
inst.pat 32-bit literal across 16 byte-banks; `f_spsram_large.v:176-190`
reassembles them via `Q[N*8+7:N*8] = ramN_dout`. So for the cacheline
holding PC 0x710, byte 0x710 = ram0[i] = literal[31:24]. Whether this
makes PC 0x710 a 16-bit RVC or 32-bit RV instruction depends on the
exact halfword value — Questa shows it as a 16-bit RVC (since PC 0x712
retires separately, the inst at 0x710 must be 16-bit).

**Three remaining hypotheses for the next-session fix**:

1. **xezim mis-evaluates `ct_ifu_precode.v`** for the specific halfword
   data at this cacheline. The boolean expressions are straightforward
   (lines 240-296) but one could mis-compile.
2. **xezim's pop_h0/pop_h1 selection logic** (`ct_ifu_ibuf.v` lines
   5687-5694 and the 8000-line case-tree at 7920-8362) drops PC 0x712.
3. **xezim's pre-decode flag propagation** from the icache to the
   ibuf entries loses the bry0/bry1 bit for h2 (offset 2 halfword).

## Test files added (committed)

- `tests/dep_reg_entry_synth.rs` — c910 dep_reg_entry synth, passes
- `tests/range_select_param_arith.rs` — `[P:P-8]` slice, passes
- `tests/case_default_arm.rs` — case-stmt with default, passes
- `tests/replicate_and_pattern.rs` — `{N{en}} & data`, passes
- `tests/c910_settle_miri.rs` — AIQ0 dep_reg miri shape, passes

## Code changes committed this overall investigation

- `7379b85` bytecode: CaseNeq is self-determined like CaseEq
- `88dd1ea` tests: AIQ0 dep_reg miri shape
- `e629e5c` tests: replicate-AND synthetic regression
- `52b9da5` JIT: refuse blocks that touch >64-bit signals + `docs/u64_audit.md`
- `84332c7` tests: case-stmt default arm for c910 IFU/IB shape
- `86343f0` tests: synthetic c910 dep_reg_entry + param-arith RangeSelect
- xezim-core `75b2adf` value: document `to_u64` silently truncates wide values

## Round 23 (2026-05-10) — IFU code audit, no new fix candidate

Read both `ct_ifu_precode.v` (320 lines) and the IBUF pop case-tree
sections in `ct_ifu_ibuf.v` end-to-end. Confirmed nothing exotic in
either:

- precode logic is straightforward `==` and `&&`/`!` reductions; the
  expr_max_width relational fix (f127254/01ca2b1) already covers this
  pattern. No remaining width-inference hazard found.
- IBUF entry-output mux uses a standard 32-way one-hot `case` keyed by
  `ibuf_retire_pointer[31:0]` — no `casez` here, just plain `case`.
  CaseEq compilation already verified by `case_default_arm` synth test.
- IBUF dispatch arm picker at lines 7920-8362 uses `casez({h0..h4
  _32_start})` with `?` wildcards. xezim's casez compilation has been
  verified (CasezEq op in bytecode.rs:606, Value::casez_eq treats Z bits
  as don't-care; `?` lex-maps to LogicBit::Z at value.rs:25).

Audited Value comparison helpers in xezim-core/src/value.rs:
- `is_equal` / `case_eq` / `less_than` / etc. use `to_u64().unwrap_or(0)`
  on no-X paths (value.rs:930, 999) — same wide-truncation hazard as
  add/sub/mul before commit 710a793, but only matters for width > 64.
  Case selector here is 32-bit, so not the immediate culprit.

Audited expr_max_width and compile_expr alignment:
- Unary self-determined operand_ctx=0 fix is present (bytecode.rs:915).
- Conditional handler correctly self-determines condition (bytecode.rs:1019).
- expr_max_width::Index still returns 1 unconditionally (line 1877).
  `infer_lhs_width::Index` correctly distinguishes unpacked-array element
  width vs bit-select width=1 (lines 1668-1681). Memory notes that
  "fixing expr_max_width::Index to return element width regresses cmark"
  — that fix exposed a downstream truncation that wasn't paired-fixed.
  This remains a known but uncorrected divergence between the two
  inference functions.

**Why no fix this round**: After full code audit of the IFU/IBUF
critical path and the bytecode width-inference helpers, no single
change has high probability of fixing memcpy without regressing
cmark/hello. The 22-min test cycle and heisenbug-shifted symptoms
preclude speculative one-shot fixes.

**Required for next round** (must iterate, not one-shot):

1. ~~**Add a single targeted probe** in xezim's tb.v that captures
   `pre_code[31:0]` for the precode of the cacheline containing PC
   0x710~~ **RULED OUT** via cone-of-influence synth test
   `tests/ifu_precode_c910_pc710.rs` (commit 9e14b02). xezim's bytecode
   compile of `ct_ifu_precode.v`'s boolean evaluation produces the
   correct pre_code for the 0x710 cacheline inst_data for both
   candidate byte-orderings. Hypothesis #1 (precode mis-compile)
   eliminated without needing the full 22-min run.

2. ~~If precode matches Questa: probe `entry_inst_data_N`~~ **RULED OUT**
   via `tests/ifu_ibuf_entry_pop_c910.rs` (commit 166aaef) and
   `tests/ifu_ibuf_32_instances_c910.rs` (commit 6aab719). xezim correctly
   compiles the per-entry write-enable replicate-AND + 32-way one-hot
   pop mux in both single-module and 32-instance-sub-module structural
   forms. Hypothesis #2 eliminated.

3. ~~If pop-mux output is wrong: bug is in case-tree selection.~~ Also
   **RULED OUT** via `tests/ifu_ibuf_casez_dispatch_c910.rs`
   (commit 62d769c). The 5-bit `casez({h0..h4_32_start})` dispatch tree
   produces correct half_num arm selection for all 32 input combos.
   Hypothesis #3 eliminated.

## Round 26 (2026-05-10) — SECOND REORIENT: same bug as cmark canary

Widening the retire+writeback tracer to capture all retire events
during sim 46000-47000 (not just the 0x700-0x720 loop region) caught
the actual termination event: **TEST FAILED at t=46665**.

The full retire sequence at the failure:
```
t=46025 pc=0x0150 wb0=0x0       wb1=0x0
t=46055 pc=0x0158 wb0=0x10      wb1=0x2
t=46055 pc=0x015c wb0=0x10      wb1=0x2
t=46075 pc=0x0160 wb0=0x400     wb1=0x410
t=46115 pc=0x0166 wb0=0x400     wb1=0x410
t=46115 pc=0x0168 wb0=0x400     wb1=0x410
t=46115 pc=0x016c wb0=0x400     wb1=0x410
t=46535 pc=0x0170 wb0=0x400     wb1=0xee
t=46555 pc=0x0172 wb0=0xe6      wb1=0xee
t=46655 pc=0x00ee wb0=0x2c      wb1=0x0
t=46665 pc=0x00fa wb0=0x3b      wb1=0x0
TEST FAILED
```

So the memcpy loop at 0x710 DOES terminate (jumps to 0x150 around
t=46000), then falls into post-loop code 0x158→0x172, then takes a
branch/jump to 0x0ee→0x0fa where the failure sentinel `64'h2382348720`
is observed in `value0`/`value1`/`value2` per tb.v:562, triggering
`$display("TEST FAILED")`.

**`0x2382348720` is the EXACT same failure sentinel** documented in
the existing memory `feedback_c910_cmark_test_failed_canary.md` for
the c910 cmark test:

> TEST FAILED is tb.v sentinel hitting 0x2382348720 from corrupt
> `idu_iu_rf_pipex_src0/src1` at sim 49405. Bug is in xezim's IDU
> mux/PRF/forwarding path. Heisenbug hypothesis falsified — both
> ae1e88f and 7d4aede fail identically.

**Conclusion**: c910 memcpy and c910 cmark fail with the same bug —
**xezim's IDU register-file-pipe source-operand mux delivers corrupt
data**. The IFU/IBUF cone-of-influence tests (rounds 23-24) and the
PC 0x712 / retire-log-presentation analysis (rounds 22, 25) were all
investigating the wrong layer. The bug has been characterized in
prior sessions but not fixed.

**Concrete next step** (follows previous-session investigation
trail): probe `idu_iu_rf_pipex_src0` / `idu_iu_rf_pipex_src1` at
t=46555 in xezim vs iverilog reference; the divergence cycle pinpoints
which specific PRF read or forward path delivers the wrong value.
Once that pipe/cycle is known, the fix target in
`xezim/src/compiler/` is a specific compile_expr or NBA-ordering
bug; the cmark canary memory entry has the relevant pointer.

## Round 25 (2026-05-10) — RETIRE-STREAM REORIENT: loop iterates forever

**Major reorient via live retire tracer (added to tb.v, not under
xezim git).** The "PC 0x712 missing" diagnosis was a **logging
artifact**, NOT the actual bug:

- The 32-bit inst at PC 0x710 = `0x5847d70b` (T-Head custom-0 opcode
  0x0b, rd=x14, funct3=5) occupies bytes 0x710-0x713. PC 0x712 is the
  upper halfword of that inst.
- Questa's retire log emits BOTH PC 0x710 AND PC 0x712 for the same
  32-bit inst (lower + upper halfword PC). xezim only emits PC 0x710.
- This is a retire-log presentation difference, not a missed execution.

**The actual bug**: the memcpy loop body `0x710 → 0x714 → 0x716`
iterates forever in xezim. Retire trace from sim 46000 to 50000 shows
continuous loop iteration with no halt; the watchdog eventually
fires at sim 1M because no sentinel-write event ever occurs (the
TEST PASSED check at tb.v:553 requires writing `64'h444333222` to
the success address).

Loop body decoded:
- PC 0x710: T-Head custom-0 inst (likely vector memcpy primitive), rd=x14
- PC 0x714: c.addiw x14, 1 (RVC: x14 += 1)
- PC 0x716: bne ?, ?, -6 (32-bit branch back to 0x710 if condition true)

Inst.pat @000001C4 = `0bd74758 0527e39d d7feeff0 7ff79567`. Per
tb.v's big-endian byte distribution into 16 byte-banks:
- byte 0x710 = 0x0b, byte 0x711 = 0xd7, byte 0x712 = 0x47, byte 0x713 = 0x58
- inst at 0x710 (little-endian within halfword): 0x5847d70b → custom-0, rd=x14
- inst at 0x714: 0x2705 (c.addiw x14, 1) — increments x14
- inst at 0x716: 0xfed79de3 (BNE) — branches back to 0x710

**New hypothesis**: xezim's execution of the custom-0 vector inst at
PC 0x710 has a wrong semantic that either fails to terminate the
loop, fails to actually copy memory, or sets x14 incorrectly so the
BNE comparison never goes false. cmark/hello don't exercise this
specific custom-0 funct3=5 inst.

**Next concrete step**: probe x14 (architectural register, not PRF
preg) at t=41145 (loop start) and at t=49995 (still iterating). If
x14 monotonically increases as expected, the bug is in the BNE
comparison source register OR the vector inst's memory side effect
(not producing the data the BNE compares against). If x14 stays at
a small value despite the c.addiw retiring, the bug is in xezim's
retired-but-not-effective-on-x14 forwarding for the custom-0 inst's
rd write.

The five committed cone-of-influence synth tests
(ifu_precode_c910_pc710, ifu_ibuf_entry_pop_c910,
ifu_ibuf_casez_dispatch_c910, ifu_ibuf_32_instances_c910,
ifu_ibuf_create_ptr_rotate) remain useful as regression guards but
they were investigating the wrong layer — IFU is not the bug location.

## Round 24 (2026-05-10) — All four IBUF isolated patterns work

Round-23's three hypotheses plus the structural 32-instance pattern
all verified correct in xezim's bytecode compile via cone-of-influence
synth tests:

- `tests/ifu_precode_c910_pc710.rs` — precode boolean eval ✓
- `tests/ifu_ibuf_entry_pop_c910.rs` — per-entry write + one-hot pop ✓
- `tests/ifu_ibuf_casez_dispatch_c910.rs` — 5-bit casez dispatch tree ✓
- `tests/ifu_ibuf_32_instances_c910.rs` — 32 cross-module instances ✓

The c910 memcpy still fails at sim 1M watchdog. So the bug must be:

(a) **An interaction across patterns** — some combination of the four
    that none of the isolated tests exercise (e.g., simultaneous writes
    to two entries while pop-mux reads a third, or casez-dispatch
    selection feeding into the pop-mux entry selection simultaneously
    with an entry-write).

(b) **In the IFU pipeline above the IBUF** — icache fetch
    (`ct_ifu_icache.v`), LBUF (`ct_ifu_lbuf.v`), or IBDP
    (`ct_ifu_ibdp.v`) delivering wrong inst_data or half_vld bits to
    the IBUF input. The IBDP halfword-vld-num logic was probed in a
    prior session (`case_default_arm` synth test passed) but the
    surrounding pipeline (precode → ibdp → ibuf) hasn't been exercised
    as a whole.

(c) **A scheduling/NBA-ordering bug** specific to the c910's exact
    block layout — possibly triggered only when both a write and a
    read of the same cross-instance signal occur in the same tick
    across different always-blocks compiled to different
    `CompiledBlock`s with different partition assignments.

**Concrete next step**: combine all four patterns into ONE synth test
where the dispatch-tree output gates the pop-mux selection AND a
simultaneous entry-write fires for an adjacent entry. If that passes
too, the bug is likely above the IBUF (option b) or in NBA scheduling
(option c). Then the full-test cycle becomes unavoidable.
