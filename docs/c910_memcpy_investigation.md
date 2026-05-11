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
