# xezim — SystemVerilog Simulator (Rust)

**xezim** is an **extensible, AI-native SystemVerilog simulator written in Rust** — built so new language features and analyses can be added one verified step at a time, with AI agents as first-class contributors to the codebase.

> `xezim` was previously developed under the name `sisSIM`. The binary, library, and compiled-artifact magic were renamed in place; behavior is unchanged.

This project explores whether modern tools and AI can dramatically reduce the complexity of building core EDA infrastructure such as simulators.

The simulator parses SystemVerilog source code, builds an internal representation, and executes simulations for combinational and sequential logic.

---

# Motivation

Traditional EDA tools require very large engineering teams and many years of development.

This project explores a key question:

> Can a small team — or even a single engineer with AI assistance — build core EDA tools such as a SystemVerilog simulator?

The simulator is being developed incrementally, starting from simple combinational logic and gradually adding more SystemVerilog features.

---

# Features

Current capabilities include:

* IEEE 1800-2023 grammar by default (`--sv2017` opts back to the earlier edition)
* SystemVerilog module parsing
* Signal and net representation
* Continuous assignments
* Basic expression evaluation
* Combinational logic simulation
* Sequential simulation infrastructure
* Test execution framework
* Waveform / trace dumps (**`--wave`**, off by default) — VCD
  (`$dumpfile`/`$dumpvars`; IEEE 1800-2017 §21.7, and matches Verilator/Icarus
  in GTKWave), **FST** (`--fst`, GTKWave's binary format, written on a
  dedicated writer thread with scope filtering), and XTrace v1.0 (`--xtrace`,
  optional zstd compression + scope filtering). All three are cross-checked
  against each other by decoding them, not by file size. Dumping is opt-in at
  model-compile time because it is not free — an active dump forces loops that
  would otherwise compile onto the AST path and builds a per-signal trace
  table — so `$dumpvars` needs `--wave` and warns once without it. `--fst` and
  `--xtrace` are explicit dump requests and imply `--wave`. `--stems` writes
  the RTLBrowse `.stems` sidecar (module source locations plus the instance
  tree) so `gtkwave -t out.stems out.fst` shows live values annotated on the
  design source. See [docs/source-annotation.md](docs/source-annotation.md).
* **UVM run-phase execution** (Accellera **1800.2-2017 and 1800.2-2020.3.1**, with
  `-DUVM_NO_DPI`) — a real UVM testbench runs end-to-end: build → connect → topology →
  `run_phase` stimulus → sequencer↔driver TLM handshake → packet collection →
  objection-driven termination → report summary. The reference testbench
  (GettingVerilatorStartedWithUVM) reaches exact Verilator parity on the 2017
  library and runs green on 2020.3.1, and 32/35 UVM 1800.2-2017 example
  testbenches pass. Multiple top
  modules (`-s hdl_top -s hvl_top`) and virtual-interface `config_db` are supported.
  See [docs/uvm-guide.md](docs/uvm-guide.md).
* UVM 1.2 runtime support, also demonstrated by running the `riscv-dv` instruction
  generator end-to-end (random RV32IMC programs that assemble cleanly with
  `riscv64-unknown-elf-as -march=rv32imc_zicsr_zifencei`)
* Event-driven edge gating (`XEZIM_EVENT_EDGE=1`) — opt-in skip of clocked
  flop fires whose data inputs haven't changed; 1.13-1.30× wall on the C910 /
  C906 hello / memcpy / cmark benchmarks, correct-by-construction
* **DPI-C loading** via `--dpi-lib <path>` — load shared libraries of
  `import "DPI-C"` implementations written in C or C++ (e.g. an ISS shim, a
  custom HDL-backdoor force/release layer, or your own UVM extensions). The
  repo ships minimal `svdpi.h` and `vpi_user.h` so DPI code compiles without a
  vendor install. See [docs/dpi-guide.md](docs/dpi-guide.md).
* **Event-control `iff` guards** (LRM §9.4.2.3) — `@(posedge clk iff rst_n)`
  is honored in both procedural `@` waits and edge-sensitive `always` blocks:
  the process resumes only on an edge where the guard holds.
* **User-defined nettypes with resolution functions** (LRM §6.6.7) —
  `nettype T wire_t with resolver;` including Z-skip and built-in resolution.
* **Per-module timescales** (LRM §3.14, §20.3, §21.3.5) — `$time`/`$realtime`
  scale to the calling module's time unit; `timeunit`/`timeprecision`
  declarations scale delays; `$timeformat`/`%t` and `$printtimescale` are
  honored; precision down to `fs`. Modules without a source-level timescale can
  be assigned one from the CLI (see
  [`--module-timescale`](#module-timescale-extension)).
* **VPI loading** via `--vpi-lib <path>` (`-m`) — classic VPI modules run their
  `vlog_startup_routines`: system-task/function registration (`vpi_register_systf`)
  and design iteration (`vpi_iterate`/`vpi_scan`, handle/property access).
* **cocotb** — Python testbenches run against xezim through a runner backend
  (`contrib/cocotb/xezim_runner.py`) on top of the VPI layer, including timed and
  synchronous callbacks.
* **Native compilation** (`--features jit`) — hot bytecode compiles to machine
  code, either through the in-process JIT (`XEZIM_JIT=1`) or the AOT backend
  (`XEZIM_JIT=1 XEZIM_AOT=1`), which emits Rust for eligible combinational
  entries, edge blocks, and process FSMs, builds it with `rustc`, and caches
  the resulting library across runs. See [below](#native-compilation).
* **`bind` by instance path** (§23.11) — `bind top.u_dut.u_sub target_tb u_tb();`
  and the colon form bind only the named instances, with upward references from
  the bound module resolving against the instance they were bound into.

### Non-standard extensions

These are **not** part of IEEE 1800 — they are de-facto vendor (Verilog-XL / VCS /
Questa / Xcelium) extensions supported for compatibility with existing gate-level
and testbench flows. Portable code should not rely on them.

* **`$deposit(target, value)`** — sets `target` to `value` immediately *without*
  installing a persistent driver: the value holds until the next driver
  transaction overwrites it (on an undriven net it simply sticks). This is a
  Verilog-XL/VCS system task, **not** in the LRM. xezim matches the vendor
  semantics — a variable keeps the deposited value, and a real driver on a net
  overrides a deposit on its next update.
* Gate-level-simulation CLI flags — `+nospecify`, `+notimingcheck`,
  `+delay_mode_zero`/`+delay_mode_unit`, `+mindelays`/`+typdelays`/`+maxdelays`,
  and the `-v`/`-y`/`+libext+` library flags — mirror the commercial spellings.

---

# Release notes

Per-release change lists, the verified-workload table and the compliance
results live in [NOTES.md](NOTES.md).

# Development workflow

How the repository is laid out, how to build it (including against a local
xezim-core checkout), and how to run the test suites. New contributors are
welcome; see [Contributors](#contributors) for the people behind the
project.

## Project Structure

xezim is split across two repos; this repo depends on `xezim-core` as a **git
dependency** (Cargo clones it automatically — no submodule, no manual checkout):

```
xezim-core (git dep) — shared library: parser, elaboration, value, SDF, VCD sink
./                   — bytecode interpreter + simulator (this repo, binary: xezim)
```

This repo:

```
.
├── src/
│   ├── compiler/
│   │   ├── simulator.rs   — event-driven simulator + bytecode VM
│   │   ├── bytecode.rs    — bytecode compiler for cont_assigns and always blocks
│   │   └── mod.rs         — re-exports value/elaborate/sdf from xezim-core
│   ├── lib.rs             — wraps xezim_core::parse_and_elaborate_multi + Simulator
│   └── main.rs            — CLI entry point (binary: xezim)
├── tests/                 — Rust integration tests + SV compliance suite
├── examples/
└── Cargo.toml             — depends on xezim-core (git dependency, fetched by cargo)
```

### Components

**Parser & elaboration** — live in `xezim-core`; consumed by both `xezim` and `xezim-b`.

**Simulator** — event-driven VM over a bytecode lowering of cont_assigns and always blocks.

---

## Build

Install Rust: https://www.rust-lang.org/tools/install

**If you only want to use xezim, there is nothing else to clone** — `xezim-core`
is a git dependency, and `cargo build` pulls it automatically:

```bash
git clone git@github.com:<you>/xezim.git
cd xezim
cargo build            # debug
cargo build --release  # optimized (recommended for large designs)
```

The release binary is produced at `target/release/xezim`.

### Profile-guided build (recommended for long runs)

`./scripts/build-pgo.sh <training-command>` instruments, trains on the command
you give it, and rebuilds with the profile. Measured on the C906 memcpy
benchmark (interleaved, same machine):

| | instructions | wall |
|---|---|---|
| release | 176.92 B | 51.5 s |
| **PGO** | **151.31 B (−14.5%)** | **44.0 s (−14.6%)** |

Output stays bit-exact (C906 gate, C910 hello, and the UVM AVIP suite all
unchanged).

Two things worth knowing before you reach for it. **The wall-clock gain
depends on the design being instruction-bound**: Ibex CoreMark also loses
~11% of its instructions but its wall time does not move, because its host
bottleneck is memory rather than instruction count — so measure, do not
assume. And the profile **generalizes better than expected**: a C906-trained
profile gave Ibex −11.1% instructions against −10.6% for an Ibex-trained one,
so a single representative trainer is usually enough. Do not stack BOLT on a
PGO build — measured net negative; PGO alone wins.

### Modifying xezim-core

`xezim-core` (parser + elaboration) is a separate repo, consumed as a git
dependency **pinned to the exact revision this xezim revision was tested
against** (see `rev = ...` in `Cargo.toml`). A bare clone therefore always
builds the verified pair — never an untested newer core — and a release tag
of xezim pairs with the core revision it shipped with. The pin is bumped in
the same commit that starts depending on new core behavior.

**Working on core?** Clone it next to (or inside) this repo and switch the
build to it — after this, plain `cargo build` uses your checkout directly,
with **no network fetch**:

```bash
git clone git@github.com:aionhw/xezim-core.git ../xezim-core
./scripts/use-local-core.sh        # detects ./xezim-core or ../xezim-core
cargo build --release              # builds against the local checkout
```

The script writes a git-ignored `.cargo/config.toml` with a `[patch]` that
overrides the pinned dependency; `./scripts/use-local-core.sh --remove`
returns to the pin. For a one-off invocation without persistent state,
`./scripts/cargo-local.sh build --release` applies the same patch for a
single command when `../xezim-core` exists.

`cargo tree -p xezim-core` shows which copy is in use (a path in parentheses
means your local checkout is active).


---

## Test Suite

~2,370 integration tests run in CI, each in **both** execution modes — the
bytecode interpreter (`cargo test`) and the JIT (`cargo test --features jit`).
A large share are differential tests whose expected values were measured on a
commercial reference simulator; their doc comments cite the LRM section and
the measured behavior.

**Credit:**
All `pr*.v` tests were taken from the **Icarus Verilog test suite**.

These tests help verify correctness against real-world Verilog/SystemVerilog edge cases.

### UVM tests

The UVM integration tests (`tests/classes/uvm_integration_tests.rs`) run against
the real Accellera UVM library from https://github.com/nitronis/UVM — one repo
carrying the 1.1d, 1.2, 1800.2-2017 and 1800.2-2020 releases as subdirectories.
No manual setup is needed: `cargo build` clones it into `target/uvm-checkout`
when no checkout is found (and the tests clone on demand as a fallback). To use
an existing checkout instead, set `XEZIM_UVM_DIR` to its root or clone it as a
`../UVM` sibling of this repo.

---

# Run

Run a simple example via cargo:

```bash
cargo run --release -- examples/test.sv
```

Or invoke the binary directly:

```bash
./target/release/xezim <source_files> [+plusargs] [options]
```

Common options:

| Option | Purpose |
|---|---|
| `-D<MACRO>[=val]` | Define a preprocessor macro |
| `-I<dir>` | Add an include directory |
| `--simulate` | Run the simulation (vs `--parse` / `--compile` / `--preprocess`) |
| `-s <module>` | Select a top-level module. Repeat for multiple roots (e.g. `-s hdl_top -s hvl_top`); xezim elaborates them all under a synthetic wrapper |
| `--dpi-lib <path>` | Load a DPI-C shared library (`.so`/`.dylib`/`.dll`). Repeatable. See [docs/dpi-guide.md](docs/dpi-guide.md). |
| `--vpi-lib <path>` (`-m`) | Load a VPI module and run its `vlog_startup_routines` (system-task registration, design walk). Repeatable. |
| `--module-timescale [mods=]<unit>/<prec>` | Assign a timescale to modules with no explicit source-level one. See [below](#module-timescale-extension). Repeatable. |
| `--dump-timescales` | Print every module's resolved timescale before the run (no source `$printtimescale` needed); modules with no `` `timescale `` are flagged. See [below](#module-timescale-extension). |
| `--max-time <N>[ps\|ns\|us\|ms\|s]` | Stop simulation after `N` of simulated time — **nanoseconds** when no unit is given. The cap is resolved to whole nanoseconds (a sub-ns value rounds to the nearest one; below half a nanosecond is rejected) and then converted to the design's tick, so the same `--max-time` covers the same simulated time whatever the precision |
| `+trace`, `+<plusarg>` | Passed through to `$value$plusargs` / `$test$plusargs` |
| `+seed=<n>` | Seed the RNG for a reproducible run (same seed ⇒ byte-identical output; affects e.g. the number of packets a random UVM test collects) |
| `--sdf <file>` `--sdf-{min,typ,max}` | Annotate standard delays |
| `--sim-debug` | Print `[DEBUG]` / `[OPT]` diagnostics (`--sim_debug` still accepted) |
| `--verbose` | Per-file compile progress: each file as it is parsed, and the modules/blocks it contributed to the working library |
| `--dump-files-list` | Print the fully resolved file list after `-f` expansion, then exit — confirms *which* sources a build actually reads |
| `--dump-merged-sv <file>` | Write the sources as one preprocessed, self-contained `.sv`. With `-s <top>`, keeps only the files that top needs. See [below](#reducing-a-multi-file-build) |
| `--artifact-compression <none\|1-22>` | Compression level for the `-o` compiled artifact (`none` writes it raw) |
| `--cache-dir <dir>` | Select the automatic elaborated-design cache directory |
| `--no-cache` | Disable the automatic elaborated-design cache |
| `-l`, `--log <file>` | Redirect all stdout/stderr — including DPI/VPI C output — to a log file |
| `-v <file>` | Library file: modules compiled only to resolve unresolved instantiations |
| `-y <dir>` | Library directory: `<module>.<ext>` loaded on demand |
| `+libext+<ext>+…` | Extension list for `-y` search (replaces the default `.v`/`.sv`/`.V`) |
| `+nospecify` | Suppress specify-block path delays — zero-delay gate simulation (`-nospecify` also accepted) |
| `+notimingcheck` | Accepted no-op: specify timing checks are not modeled (also `+notimingchecks`/`-notimingchecks`) |
| `--wave` | Compile the model with waveform support, enabling `$dumpfile`/`$dumpvars` (off by default; `--fst`/`--xtrace` imply it) |
| `--fst <file>` | Emit an FST (GTKWave binary) waveform dump |
| `--fst-scope <hier>` | Restrict the FST dump to signals under `<hier>` (repeatable) |
| `--stems <file>` | Write an RTLBrowse `.stems` sidecar (instance tree + each module's source file and header line) so `gtkwave -t <file> <dump>` shows live values annotated on design source |
| `--xtrace <file>` | Emit an XTrace v1.0 dump (`.zst`/`.zstd` ⇒ zstd-compressed) |
| `--xtrace-scope <hier>` | Restrict the XTrace dump to signals under `<hier>` (repeatable) |
| `--relax-implicit-static` | Accept `int x = ...;` inside a static task/function (§6.21) with a warning instead of an error — for vendor sources you cannot edit |
| `--error-exit` | Exit nonzero if any `$error` was reported (`$fatal` always does) |
| `--profile` | Print the `[PROF]` end-of-run profile report (edge-block, settle and timing counters). Same as `XEZIM_PROFILE_REPORT=1` |

Selected env knobs (off by default unless noted):

| Env var | Effect |
|---|---|
| `XEZIM_EVENT_EDGE=1` | Skip gateable clocked flop fires whose data is unchanged (1.13-1.30× wall on c910/c906) |
| `XEZIM_JIT=1` | Compile bytecode blocks to machine code in-process (needs a `--features jit` build) |
| `XEZIM_AOT=1` | Compile eligible blocks to native code via generated Rust + `rustc` instead of cranelift. **Requires `XEZIM_JIT=1` as well** — on its own it is a no-op. Needs `--features jit`. See [below](#native-compilation) |
| `XEZIM_AOT_OPT=0..3` | `rustc` optimization level for the generated crate (default 2) |
| `XEZIM_PROC_FSM=1` | Compile blocking `always` bodies into bytecode state machines with wait instructions |
| `XEZIM_NO_NATIVE_CACHE=1` | Disable the persistent native-library cache (`~/.cache/xezim/native`) |
| `XEZIM_REGIONS=1` | Fuse dependency-connected compiled combinational entries into region blocks (experimental; currently net-negative on the benchmark set) |
| `XEZIM_STUCK_CLOCK=1` | Flag a process parked on a clock/reset that never changes while the design keeps churning edges (`abort` variant for CI) |
| `XEZIM_INIT_ZERO=1` | Coerce X-initialized signals/arrays to 0 (required for some C910/C906 workloads, e.g. cmark) |
| `XEZIM_PROGRESS=N` | Emit a `[PROGRESS]` line every N wall-seconds (sim_time, iters, edges_fired, nba_q) |
| `XEZIM_CACHE_DIR=<dir>` | Override the elaborated-design cache directory |
| `XEZIM_NO_CACHE=1` | Disable the automatic elaborated-design cache |
| `XEZIM_COMPILE_PHASES=1` | Report detailed simulator compilation phase timings |
| `XEZIM_ALLOW_IMPLICIT_STATIC=1` | Same as `--relax-implicit-static` |
| `XEZIM_PROFILE_REPORT=1` | Same as `--profile` |
| `XEZIM_MAX_INST_DEPTH=N` | Instantiation-depth cap (default 200) — turns unbounded recursive instantiation into a clean error instead of memory exhaustion |
| `XEZIM_STACK_MB=N` | Stack size of the simulation worker thread (default 1024; `0` runs on the main thread) |
| `XEZIM_VALUE_TRACE=<substr>[,...]` | Print every committed change of signals whose hierarchical name contains a pattern: time, name, old→new value, dispatch phase, writing process origin (file:line). NBA commits are labeled `nba` |
| `XEZIM_VALUE_TRACE_LIMIT=N` | Cap value-trace output lines (default 20000) |

Example — run the picorv32 testbench against a gate-level netlist:

```bash
./target/release/xezim testbench.v synth.v \
    +firmware=firmware/firmware.hex --max-time 50000000
```

## Native compilation

Built with `--features jit`, xezim can turn hot bytecode into machine code.

```bash
cargo build --release --features jit

# in-process JIT
XEZIM_JIT=1 ./target/release/xezim <sources> -s <top>

# AOT: generate Rust, build it with rustc, load the result
# (XEZIM_JIT=1 is required — XEZIM_AOT selects the backend, it does not
#  enable native compilation on its own)
XEZIM_JIT=1 XEZIM_AOT=1 ./target/release/xezim <sources> -s <top>

# AOT plus compiled process state machines
XEZIM_JIT=1 XEZIM_AOT=1 XEZIM_PROC_FSM=1 ./target/release/xezim <sources> -s <top>
```

**Whether it pays depends on the design — measure before adopting it.** Same
binary, warm native cache, wall-clock:

| | interpreter | `XEZIM_JIT` | `+AOT` | `+AOT +PROC_FSM` |
|---|---|---|---|---|
| Ibex CoreMark | 50.8s | **39.0s** (−23%) | **38.8s** | 39.1s |
| C906 memcpy ×100 | 49.3s | 55.7s (**+13%**) | 49.5s | 47.8s (−3%) |

The C906 loss is entirely compile time, not slower simulation: JIT takes its
simulation phase from 43.6s to 42.9s but spends 7.0s more compiling, because
the design has 35,267 combinational entries to Ibex's 1,553 and the per-block
cost is amortized ~37× less. Compiling only the hot subset does not rescue it
— the eval distribution is steep enough (15% of entries carry 99.2% of
evaluations) that a threshold looked promising, but JIT is only worth 2.3% of
C906's simulation phase in the first place, and on Ibex the warmup needed to
measure hotness costs more than the compile it saves. Rule of thumb: native
compilation pays on designs with relatively few, very hot blocks.

The AOT backend covers combinational entries, edge-sensitive blocks, and — when
`XEZIM_PROC_FSM=1` is also set — process FSMs. Blocks it cannot lower (values
wider than 64 bits, unsupported opcodes, X/Z-carrying shapes) stay on the
interpreter, so coverage is partial by design; `XEZIM_JIT_VERBOSE=1` prints the
`[AOT] … compiled N/M` summary.

Generating and compiling that Rust is the dominant cost on a first run — minutes
on a large SoC — so the resulting library is cached under `$XEZIM_CACHE_DIR`,
`$XDG_CACHE_HOME/xezim/native`, or `~/.cache/xezim/native`, keyed on the
generated source, `XEZIM_AOT_OPT`, and the xezim build. Repeat runs load the
cached `.so` directly. Set `XEZIM_NO_NATIVE_CACHE=1` to force a rebuild, and
`XEZIM_AOT_OPT=0` to trade steady-state speed for a faster build.

## Warm design cache

Simulation mode stores a content-addressed elaborated design and compiled
combinational worklist after the first run, then reuses both on identical later
runs. A cache hit skips parsing, elaboration, and combinational dependency-index
construction, while simulator state, plusargs, time-zero initialization, and
event scheduling are rebuilt for every invocation. Timing-annotated and UDP
designs conservatively rebuild the worklist. The key covers source and library
contents, defines, include paths, top selection, language/strictness, timescale
and delay settings, and the xezim executable build.

The default directory is `$XEZIM_CACHE_DIR`, then
`$XDG_CACHE_HOME/xezim/designs`, then `$HOME/.cache/xezim/designs`. Use
`--cache-dir` for a workload-local cache or `--no-cache` for a cold run. Xezim
prints `[CACHE] miss`, `[CACHE] stored`, or `[CACHE] hit` on stderr.

## Reducing a multi-file build

Three flags answer the questions that come up when a large `-f` build does not
behave: *which* files were read, *what* each contributed, and *what does the
code look like after preprocessing*.

```bash
xezim -f build.args --dump-files-list          # the resolved file list, then exit
xezim -f build.args -s testbench --verbose     # each file as it is parsed, and what it defined
xezim --parse -f build.args -s testbench --dump-merged-sv repro.sv
```

`--dump-merged-sv` writes every source into one self-contained `.sv` with
`` `ifdef `` branches resolved, macros expanded and `` `include ``s inlined — a
125-file build becomes a single re-runnable file. Given `-s <top>` it keeps only
the files that top actually needs, which is what makes the result small enough
to hand to someone else.

Two properties are worth knowing before relying on it:

* **The reduction is per file, not per module.** A file defining both a module
  you need and one you do not drags the second one's dependencies in too.
* **The closure is lexical and runs before parsing**, so the dump still works on
  a design that does not elaborate — the case the flag exists for. It is
  conservative in the safe direction: it may keep a file more than strictly
  needed, never one fewer. Files that declare no design unit at all (a
  file-scope `typedef`/function, a top-level `bind`) are always kept, since
  nothing references them by name and dropping them would change behaviour.

Note `--parse` above: the dump is produced before elaboration, so a design whose
elaboration takes minutes still dumps in seconds. Only the step that appends
adopted `-v`/`-y` library files needs `--compile` or `--simulate`.

## Power intent (UPF)

xezim reads IEEE 1801 (Unified Power Format) files and simulates the power
intent alongside the RTL: supply nets carry a state and a voltage, power
switches gate them, powered-down logic corrupts to `x`, isolation cells clamp
domain outputs, and retained registers keep their values.

### Flags

| Flag | Meaning |
|---|---|
| `--upf <file>` | Load a UPF file. Repeat for several files; `load_upf` inside a file resolves relative to that file. |
| `--upf-top </path/to/instance>` | The design instance the UPF scope (`set_design_top`) refers to. Without it the first instance of the `set_design_top` module is used. |
| `XEZIM_UPF_DUMP=1` | Print the generated power-aware glue. |

    xezim --simulate -s tb --upf power.upf --upf-top /tb/dut/core rtl.v tb.sv

A complete runnable example (switched domain, header switch, isolation and
retention) lives in `examples/upf/`; `./examples/upf/run.sh` simulates it and
`tests/upf/` covers the flow in the regression suite.

### Driving supplies from the testbench

The testbench controls the supply ports through the standard `UPF` package
(IEEE 1801 §11.2.4), which xezim provides automatically when `--upf` is given:

```systemverilog
import UPF::*;
initial begin
  st = supply_on("/tb/dut/core/VDD", 1.0);   // state FULL_ON, 1.0 V
  st = supply_on("/tb/dut/core/VSS", 0.0);
  ...
  st = supply_off("/tb/dut/core/VDD");
end
```

| Function | Effect |
|---|---|
| `supply_on(path, volts = 1.0)` | Supply port goes FULL_ON at the given voltage. |
| `supply_off(path)` | Supply port goes OFF. |
| `supply_partial_on(path, volts)` | Reported as PARTIAL_ON and applied as FULL_ON at the given voltage. |
| `get_supply_on_state(path)` | 1 while the net is FULL_ON. |
| `get_supply_voltage(path)` | The net's voltage as a real. |

Paths are `/top/inst/.../NET`, the dotted form, or a net name relative to the
UPF scope.

### Commands

Simulated:

| Command | Behaviour |
|---|---|
| `create_supply_net`, `create_supply_port`, `connect_supply_net`, `set_domain_supply_net` | Each net is a state (FULL_ON, OFF, UNDETERMINED) plus a voltage; ports are the nets the testbench drives. |
| `create_power_switch` | The output supply follows the input while an `-on_state` boolean over the control ports holds; an `x` control yields UNDETERMINED. Multiple `-on_state`/`-off_state` clauses are honoured. |
| `create_power_domain -elements` | While a domain's primary power or ground is not FULL_ON, every variable, net and output inside its elements reads `x` and keeps `x` until written after power-up. `-elements {.}` names the scope instance itself. A domain without a primary supply is always on. |
| `set_isolation`, `set_isolation_control` | While the control is active (`-isolation_sense`), the domain's isolated outputs read their `-clamp_value` at the domain boundary; the drivers resume when the control releases. Element-specific strategies override `-applies_to outputs`; `-update` merges options into the named strategy. A domain that powers down with its isolation control inactive is reported. |
| `set_retention` (+ `set_retention_control`) | Retained elements are exempt from corruption and keep their values through the power-down. |
| `load_upf [-scope inst]` | Nested files load relative to the loading file; with `-scope` their commands apply below that instance, and a `set_design_top` inside them names that instance's module. |
| `set_scope`, `set`, `$var`, `puts` | Tcl subset: braces, quotes, `\` continuation, `#` comments, `;` separators, variable substitution. |

Parsed and reported only (no runtime effect): `set_level_shifter`,
`add_port_state`, `create_pst`, `add_pst_state`, `create_supply_set`,
`associate_supply_set`, `add_power_state`, `create_logic_net`,
`create_logic_port`, `connect_logic_net`, `set_port_attributes`,
`set_design_attributes`, `set_simstate_behavior`, `upf_version`. Any other
command is skipped with a warning, so a full-flow UPF set (constraints,
configuration and implementation files chained by `load_upf -scope`) loads and
the simulated subset applies.

### Reporting

Elaboration prints a `[UPF]` summary (scope, supply nets, switches, domains
with their corruptible-signal count and retained elements, isolation
strategies, PSTs) followed by warnings for anything unresolved. During
simulation every power event is logged with the `[UPF] Time: ...` prefix:
supply changes, switch state, domain power-up/down, isolation enable/disable,
and isolation-control checks.

### Not modelled

PST legality checks at run time, supply-set functions (`PD.primary.power`),
`add_power_state` evaluation, level shifters (transparent), the `latch` clamp
value, `-applies_to inputs`, retention save/restore timing, and elements
inside instance arrays or generate blocks.

## Module-timescale extension

`--module-timescale` is an xezim-specific command-line extension. It assigns a
time unit and precision to module *definitions* that have **no explicit
source-level timescale**, without changing the semantics of the source. It is
handy for retrofitting a timescale onto legacy RTL that omits one, or onto a
mix of files where only some carry `` `timescale ``.

```bash
# Every module without an explicit timescale gets 1ns/1ps:
xezim --module-timescale 1ns/1ps design.sv

# Only the listed definitions (comma-separated), 10ns/1ns:
xezim --module-timescale cpu,cache=10ns/1ns design.sv

# Repeatable; the named form wins over the global one:
xezim --module-timescale 1ns/1ps --module-timescale mem_ctrl=1ps/1fs design.sv
```

**A module has an explicit source-level timescale** — which the option never
overrides — when it has a `timeunit`/`timeprecision` declaration, **or** a
`` `timescale `` directive is active where it is declared (`` `resetall ``
clears that). Effective precedence, highest first:

1. module-local `timeunit` / `timeprecision`
2. an active `` `timescale `` directive
3. a named `--module-timescale mods=<unit>/<prec>`
4. a global `--module-timescale <unit>/<prec>`
5. the 1ns / 1ns default

The precision must be equal to or finer than the unit (`1ns/1ps` is legal,
`1ps/1ns` is an error). Two *different* named assignments for the same module
are an error; an unmatched name, or one that lands on a module that already has
an explicit timescale, is a warning (the assignment is ignored). Assignments
apply to a definition, so every instance of it shares the timescale.

Sub-nanosecond precision is honoured — the simulation tick is the finest
precision declared anywhere in the design, down to `fs`. `--max-time` is
independent of that: it is given in nanoseconds and converted to the tick, so
`--max-time 100` stops at 100 ns whether the design runs at `1ns` or `1fs`
precision. What a finer precision does change is the *number of ticks* covered,
and hence the wall-clock cost of reaching the same simulated time. Reported
times (`$time`, the closing `Simulation finished at time …`) are in ticks, so
the same run prints `100` at `1ns/1ns` and `100000` at `1ns/1ps`.

Because the cap is held in whole nanoseconds, a sub-nanosecond `--max-time`
(`--max-time 1ps`) is rejected rather than silently rounded to zero. To stop a
run as early as possible, prefer `--parse` or `--compile`, which never start a
simulation at all.

### Inspecting resolved timescales

`--dump-timescales` prints the resolved timescale of every module *before* the
run — no source `$printtimescale` calls required. It reports each definition's
`` `timescale `` semantics (an explicit/`--module-timescale` value, or the
`1ns/1ns` default when a module has none) and flags the modules that carry no
`` `timescale ``. Combine it with `--module-timescale` to confirm an assignment
landed where you intended.

```bash
$ xezim --dump-timescales design.sv
=== module timescales (3 modules) ===
  cache                        10ns / 1ns
  cpu                          1ns / 1ps
  glue                         1ns / 1ns   (no `timescale — 1ns/1ns default)
======================================
```

A flagged module also emits the `has no timescale directive` warning in a
mixed-timescale design; give it a source `` `timescale `` or a
`--module-timescale` assignment to resolve it. (The default is tool-defined by
IEEE 1800 §3.14.2.2; xezim uses `1ns/1ns` for both delays and `$realtime`, so
an untimed module's `#1` is one nanosecond — declare a timescale explicitly when
you mean something else.)

---

# Long-Term Vision

This project explores several long-term ideas:

* **AI-assisted EDA development**
* **Rapid simulator prototyping**
* **Cloud-scale simulation**
* **Distributed multi-CPU simulation**

The goal is to investigate whether modern software and AI tools can dramatically accelerate the creation of chip design infrastructure.

---

# License

Apache License 2.0

See the `LICENSE` file for details.

---

# Contributors

xezim is developed in the open, and a number of people have improved it through
pull requests. Thank you to everyone who has contributed — bug fixes, features,
tests, and tooling all move the project forward:

* **Thomas Burg** — class-system and UVM fixes: static-property chains through
  object handles (§8.25), associative-array method dispatch and ref-writeback,
  `ClassName::static_prop` access, parser-gap self-tests, test-harness
  hardening, per-process bookkeeping for methods that park mid-body, the
  condition-waiter drain de-duplication, and the NBA-region lane in the
  `--max-time` hang report.
* **Vrajesh Prakhya** — real-number modelling coverage: Verilog-AMS `wreal`
  nets resolved by summing, user-defined nettypes across the hierarchy and in
  packages (§6.6.7, §6.6.8), real-ness of members projected from call results,
  negative-test registrations, and the diagnosis that `cover property` sites
  were tallied as failing assertions.
* **Oscar Gustafsson** — expanded VPI functionality (`vpi_get_value`,
  `ObjectValType`), CI setup, and clippy cleanups.
* **Chen Ben Haroosh** — submodule-inline generate-for elaboration: genvar-
  dependent declarations and `parameter type` default resolution, plus the
  accompanying SystemVerilog compliance cases.
* **Jayaraman RP** — cross-platform installation scripts, including the macOS
  installer with UVM setup.

New contributors are welcome — see [Development workflow](#development-workflow).

---

# Acknowledgements

* Icarus Verilog project for the public test suite
* The Rust community
* Open-source EDA projects
