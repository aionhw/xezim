# Supported features

What xezim supports, in more detail than the [README](../README.md). Changes
per release are in the [release notes](../NOTES.md).

## Language

* IEEE 1800-2023 grammar by default; `--sv2017` selects the 1800-2017 edition.
* Event-driven simulation of RTL and gate-level netlists — continuous
  assignments, procedural blocks and the IEEE 1800 scheduling regions, UDPs and
  drive strengths, specify-block delays and timing checks, and SDF
  back-annotation (`--sdf`).
* Classes, constrained randomization, covergroups and concurrent assertions
  (SVA).
* **Event-control `iff` guards** (§9.4.2.3) — `@(posedge clk iff rst_n)` is
  honoured in both procedural `@` waits and edge-sensitive `always` blocks: the
  process resumes only on an edge where the guard holds.
* **Deferred immediate assertions** (§16.4) — `assert #0` / `assert final`
  evaluate where they run, but their action block runs only when the report
  matures at the end of the time slot; a report is dropped if its process
  resumes first, and one still pending at `$finish` prints a note instead of
  running its action.
* **Sampled values in concurrent assertions** (§16.5.1) — a property reads
  every variable as sampled in the Preponed region, elements of unpacked
  arrays included: multidimensional arrays, arrays of structs, arrays in
  interfaces and array ports. A select samples only the element it names; one
  whose index reads variables finds that element from the index's sampled
  value in each time slot, so a large memory is never copied. An index known
  only when the property is evaluated (a sequence local variable, a
  procedure's loop variable, a sampled-value function or a call) samples the
  whole array. In time slot 0 a sampled value is the default sampled value:
  a variable's declaration initializer, else its type's default, so an
  attempt at a time-0 clock edge does not see what the slot's continuous
  assignments and processes write. The `disable iff` condition reads current
  values (§16.12).
  Dynamic-array and queue elements past index 63 and associative-array
  entries are still read unsampled.
* **Assertion control** (§20.11, §20.12) — `$asserton`, `$assertoff`,
  `$assertkill`, the pass/fail/vacuous action-control tasks and
  `$assertcontrol` with Lock/Unlock, the assertion-type and directive-type
  masks, and `levels` plus a list of scopes or assertion names. They apply to
  concurrent, immediate and deferred assertions and covers; `expect`
  statements ignore them, as in the reference simulator.
* **User-defined nettypes with resolution functions** (§6.6.7) —
  `nettype T wire_t with resolver;` including Z-skip and built-in resolution.
* **Per-module timescales** (§3.14, §20.3, §21.3.5) — `$time`/`$realtime`
  scale to the calling module's time unit; `timeunit`/`timeprecision`
  declarations scale delays; `$timeformat`/`%t` and `$printtimescale` are
  honoured; precision down to `fs`. Modules without a source-level timescale
  can be given one from the command line (see the
  [module-timescale extension](user-guide.md#module-timescale-extension)).
* **`bind` by instance path** (§23.11) — `bind top.u_dut.u_sub target_tb u_tb();`
  and the colon form bind only the named instances, with upward references from
  the bound module resolving against the instance they were bound into.
* **Power intent (IEEE 1801 UPF)** via `--upf <file>` and `--upf-top
  <instance>` — supply nets and power switches, powered-down corruption to `x`,
  isolation and retention, with the supplies driven from the testbench through
  the standard `UPF` package. See the [UPF guide](upf-guide.md).

## VHDL

* **VHDL-2008** (IEEE 1076-2008): entities, architectures, configurations,
  packages and generic packages, processes, signals with resolution,
  drivers and delta cycles as the LRM's simulation cycle, generics and
  generate, records, arrays, access and protected types, files and TEXTIO,
  the IEEE packages (`std_logic_1164`, `numeric_std`, `fixed_pkg`,
  `float_pkg`, ...), external names, force and release; results and messages
  as GHDL's.
* **VHDL-2019** (IEEE 1076-2019, `--std=19`): conditional analysis, mode
  views, conditional expressions and returns, sequential blocks, generic
  type classes, generic protected types and private variables, `'image` and
  `'value` of composites, STD.ENV (time, environment, directories,
  assertion state), the file API and STD.REFLECTION.
* **Mixed designs**: SystemVerilog instantiating VHDL entities and VHDL
  instantiating SystemVerilog modules.

See the [VHDL guide](vhdl-guide.md).

## UVM

* **Accellera 1800.2-2017 and 1800.2-2020.3.1** run end to end: build →
  connect → topology → `run_phase` stimulus → sequencer↔driver TLM handshake
  → packet collection → objection-driven termination → report summary.
  Multiple top modules (`-s hdl_top -s hvl_top`) and virtual-interface
  `config_db` are supported.
* The GettingVerilatorStartedWithUVM reference testbench reaches exact parity
  with Verilator on the 2017 library and runs green on 2020.3.1, and 32 of 35
  UVM 1800.2-2017 example testbenches pass. The mbits-mirafra AVIP base tests
  for AXI4, APB, I3C, SPI and AXI4-Lite print the same UVM messages as a
  commercial reference simulator.
* **UVM's DPI-C library is built in.** Compile UVM without `-DUVM_NO_DPI` and
  its regex matching (config_db / resource_db wildcards and `/regex/` scopes,
  `+uvm_set_*` plusargs, factory overrides by path), command-line processing
  and `uvm_hdl_*` backdoor access (read / deposit / force / release by full
  path, including bit- and part-selects, memory words and packed-struct
  members) run natively, with the C code's semantics and `UVM/DPI/*` error
  reports. A `--dpi-lib` library defining the same symbols takes precedence.
* UVM 1.2 runs too, demonstrated by the `riscv-dv` instruction generator end
  to end (random RV32IMC programs that assemble cleanly with
  `riscv64-unknown-elf-as -march=rv32imc_zicsr_zifencei`).

See the [UVM guide](uvm-guide.md).

## Coverage

* **Functional and assertion coverage**, always on — covergroups (explicit,
  automatic, array, wildcard and transition bins, `ignore_bins`/`illegal_bins`,
  crosses, `iff` guards, `at_least`/`weight`/`auto_bin_max`/`merge_instances`),
  the `get_coverage()`/`get_inst_coverage()`/`$get_coverage()` queries, and
  pass/fail counts for `cover property` and the assertions. A run with any of
  them writes the results to `xezim_cov.json` (`XEZIM_COV_DB=<path>` to move
  it).
* **Code coverage**, off by default — statement, branch and toggle counts with
  `--code-coverage` (or `+cover`), per instance and per design unit, in the
  same `xezim_cov.json`. Instrumented at compile time, so a run without it pays
  nothing.

See the [coverage guide](coverage-guide.md).

## Debug and integration

* **Waveforms** (`--wave`, off by default) — VCD, FST (`--fst`) and XTrace
  (`--xtrace`). See the [user guide](user-guide.md#waveforms).
* **DPI-C** via `--dpi-lib <path>` — load shared libraries of
  `import "DPI-C"` implementations written in C or C++ (an ISS shim, a custom
  HDL-backdoor layer, your own UVM extensions). The repository ships
  `svdpi.h`, `vpi_user.h` and `sv_vpi_user.h`, so DPI code compiles without
  any vendor installation.
* **VPI** via `--vpi-lib <path>` (`-m`) — the IEEE 1800 VPI routines and value
  formats, simulation and value-change callbacks, and the design object model
  a VPI tool can walk. See the [DPI and VPI guide](dpi-guide.md).
* **cocotb** — Python testbenches run against xezim through a runner backend
  (`contrib/cocotb/xezim_runner.py`) on top of the VPI layer, including timed
  and synchronous callbacks. `get_runner("xezim")` takes the options of
  cocotb's other runners: `parameters` become `-g` overrides, and since xezim
  has no compile step, `build_args` apply when `test()` runs.

## Performance features

* **Edge gating**, on by default — clocked flop fires whose data inputs have
  not changed are skipped (1.13–1.30× wall on the C910/C906 benchmarks;
  `XEZIM_EVENT_EDGE=0` turns it off).
* **Packed memory arena**, on by default — large integral memories are stored
  compactly (`XEZIM_PACKED_MEM=0` turns it off).
* **Warm design cache** — repeated runs skip parsing and elaboration. See the
  [user guide](user-guide.md#warm-design-cache).
* **Native compilation** (`--features jit`) — hot bytecode compiles to machine
  code through the in-process JIT (`XEZIM_JIT=1`) or the AOT backend
  (`XEZIM_JIT=1 XEZIM_AOT=1`). See the
  [user guide](user-guide.md#native-compilation).
* **Profile-guided builds** — see [building](building.md#profile-guided-build).
