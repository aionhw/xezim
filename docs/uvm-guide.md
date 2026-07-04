# Running UVM testbenches on xezim

xezim runs **UVM 1800.2-2017** testbenches end-to-end on its event-driven 4-state core:
build → connect → topology → `run_phase` stimulus → sequencer↔driver TLM handshake →
packet collection → objection-driven termination → report summary.

This guide covers how to invoke xezim on a UVM testbench, what is supported, and the known
limitations. (For the internal design/implementation history, see
[`uvm-run-phase-plan.md`](uvm-run-phase-plan.md).)

---

## Quick start (single top)

Point xezim at the UVM library, the include dirs, and the source files, and select the top
module with `-s`:

```bash
xezim --simulate -s top \
  -I <UVM>/src -I <rtl> -I <sv> -I <tb> \
  -D UVM_NO_DPI -D UVM_REPORT_DISABLE_FILE_LINE \
  <UVM>/src/uvm_pkg.sv \
  <design and testbench files...> \
  +UVM_TESTNAME=<test_name>
```

- `-I <UVM>/src` makes `` `include "uvm_macros.svh" `` resolve.
- `-D UVM_NO_DPI` — xezim services UVM reporting/cmdline directly instead of via DPI.
- `+UVM_TESTNAME=<name>` selects the test; it overrides the `run_test("...")` argument.

### Worked example — GettingVerilatorStartedWithUVM

```bash
xezim --simulate -s top \
  -I $UVM/src -I rtl -I sv -I tb \
  -D UVM_REPORT_DISABLE_FILE_LINE -D UVM_NO_DPI -D SVA_ON \
  $UVM/src/uvm_pkg.sv sv/pipe_pkg.sv sv/pipe_if.sv rtl/pipe.v tb/top.sv \
  +UVM_TESTNAME=data0_test
```

Expected: the test topology table, both monitors reporting `COLLECTED PACKETS = 76`, a
`--- UVM Report Summary ---` with `UVM_ERROR : 0` / `UVM_FATAL : 0`, and a clean `$finish`.

---

## Multiple top modules (hdl_top + hvl_top)

Many UVM testbenches declare **two unconnected top modules** — e.g. a BFM `hdl_top` holding
the interfaces, clock, and `uvm_config_db::set` calls, and an `hvl_top` running `run_test`.
Pass each with its own `-s`; xezim elaborates them all under a synthetic wrapper root:

```bash
xezim --simulate -s hdl_top -s hvl_top \
  -I <UVM>/src -I <agent> -I <tb> \
  -D UVM_NO_DPI -D UVM_REPORT_DISABLE_FILE_LINE \
  <UVM>/src/uvm_pkg.sv <agent files...> <rtl files...> \
  <tb>/hdl_top.sv <tb>/hvl_top.sv
```

If you give only one `-s`, behavior is exactly as before (no wrapper synthesized).

---

## What you get

- **Topology** — `this.sprint(printer)` / `print()` renders the component + port tree in
  `uvm_table_printer` format (Name / Type / Size / Value).
- **Stimulus** — the sequencer↔driver `get_next_item` / `item_done` /
  `start_item` / `finish_item` rendezvous runs; sequences drive items into the DUT.
- **Termination** — the run phase ends when the phase objection count returns to zero
  (drain time honored), then extract/check/report/final run, then `$finish`.
- **Report summary** — `--- UVM Report Summary ---` with counts by severity and by id.

For UVM extensions that need their own DPI-C (e.g. a custom HDL backdoor, a cocotb
bridge, or an ISS shim), see [`dpi-guide.md`](dpi-guide.md) — the same `--dpi-lib`
mechanism works for UVM-side code as for plain SV testbenches.

---

## Supported

- `uvm_test` / `uvm_env` / `uvm_agent` / `uvm_driver` / `uvm_monitor` / `uvm_sequencer` /
  `uvm_scoreboard` and the standard phase methods.
- Sequences: `body`, `start`, `start_item`/`finish_item`, ``uvm_do``/`uvm_do_with`,
  `randomize() with {...}`.
- TLM: analysis ports (broadcast), `uvm_*_imp`/export, `put`/`get`, TLM fifos, via the
  connect-phase connection graph.
- Virtual interfaces (LRM §25.8/§25.9): member reads, `@(posedge vif.clk)` event
  sensitivity, vif assignment (null clears; vif-to-vif copy), task-arg aliasing.
- `uvm_config_db#(T)::set/get/exists` — scope-aware with wildcard matching, including
  virtual-interface values and the BFM `set(null,"uvm_test_top",...)` pattern.
- Objection model: `raise_objection` / `drop_objection` / `set_drain_time`.
- The factory (`type_id::create`), overrides, and parameterized components.

## Known limitations / out of scope

- **Deprecated UVM-1.0 API** — `` `uvm_sequencer_utils ``, `` `uvm_sequence_utils ``,
  sequence libraries. These macros are undefined in 1800.2-2017 and will produce a parse
  error.
- **DPI backdoor access** — `uvm_hdl_*` (force/deposit/read) and DPI-based C stimulus.
- **RAL** (register abstraction layer) and sequence lock/grab arbitration beyond the
  common path.
- **Cosmetic differences vs a reference run:** topology handle ids (`@N`) are xezim heap
  handles; Report-Summary `UVM_INFO`/`UVM_WARNING` totals are higher (xezim emits more
  verbose informational reports). The correctness-bearing `UVM_ERROR : 0` /
  `UVM_FATAL : 0` lines match exactly.

---

## Troubleshooting

| Symptom | Likely cause / fix |
|---|---|
| `config_db ... ::get(...) failed` (NOVIF) at build_phase, sim ends at t=0 | A second top (e.g. BFM `hdl_top`) that holds the `config_db::set` calls wasn't elaborated. Pass every top with its own `-s` (see *Multiple top modules*). |
| `No test specified` (UVM_FATAL NOTEST) | Add `+UVM_TESTNAME=<name>` or ensure `run_test("<name>")` has an argument. |
| `Requested test "X" not found` | The test class name doesn't match a compiled class; check spelling and that the file is in the source list. |
| `unexpected token in class: "("` near a sequencer/sequence | Deprecated UVM-1.0 `` `uvm_*_utils `` macro — out of scope (see limitations). |
| Run never terminates (hits `--max-time`) | The test raises no objection (some examples run open-ended). Set `--max-time <N>` to bound it. |
| Stimulus never flows / monitor collects 0 | Confirm the driver's `seq_item_port` connects to the sequencer in `connect_phase`, and the test starts a sequence on that sequencer. |
