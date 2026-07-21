# xezim — SystemVerilog Simulator (Rust)

**xezim** is a **SystemVerilog simulator written in Rust** designed for experimentation, learning, and exploring AI-assisted chip design workflows.

> `xezim` was previously developed under the name `sisSIM`. The binary, library, and compiled-artifact magic were renamed in place; behavior is unchanged.

This project explores whether modern tools and AI can dramatically reduce the complexity of building core EDA infrastructure such as simulators.

The simulator parses SystemVerilog source code, builds an internal representation, and executes simulations for combinational and sequential logic.

---

# Quick Install

## Linux & macOS

Install the latest release:
```bash
curl -fsSL https://raw.githubusercontent.com/aionhw/xezim/main/scripts/install.sh | sh
```

Install a specific version (one-time):
```bash
XEZIM_TAG=v0.9.6 curl -fsSL https://raw.githubusercontent.com/aionhw/xezim/main/scripts/install.sh | sh
```

## Windows (PowerShell)

Install the latest release:
```powershell
irm https://raw.githubusercontent.com/aionhw/xezim/main/scripts/install.ps1 | iex
```

Install a specific version (one-time):
```powershell
$env:XEZIM_TAG='v0.9.6'; irm https://raw.githubusercontent.com/aionhw/xezim/main/scripts/install.ps1 | iex
```

> The installers auto-detect the latest stable release from git tags. Set `XEZIM_TAG` to pin any specific version. See the [Build](#build) section below for manual installation steps.

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
* Waveform / trace dumps — VCD (`$dumpfile`/`$dumpvars`; IEEE 1800-2017 §21.7, and
  matches Verilator/Icarus in GTKWave) and XTrace v1.0 (`--xtrace`, optional zstd
  compression + scope filtering)
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

# What's new in 0.9

* **UVM 1800.2-2020.3.1 runs green** — the reference testbench passes against the
  2020.3.1 library (`UVM_ERROR : 0` / `UVM_FATAL : 0`, in/out monitors agree).
  Closing this required a general preprocessor fix (inline
  `` `ifdef ``/`` `endif `` mid-line, §22.6), class-body `localparam` constants,
  and sequencer-path fixes (`process::self()`, fork/join_none automatic-variable
  sharing).
* **User-defined nettypes** (LRM §6.6.7) — `nettype` declarations with
  user resolution functions, Z-skip, and built-in resolution.
* **Per-module timescales** — `$time`/`$realtime` scale to the calling module's
  unit; `timeunit`/`timeprecision` declarations scale delays; `$timeformat`/`%t`
  and `$printtimescale` honored; sub-ns precision down to `fs`; new
  [`--module-timescale`](#module-timescale-extension) CLI extension for
  legacy RTL with no source-level timescale.
* **String & aggregate conformance fixes** — `s[i]` read/write on string
  variables (§11.4.13), `ref`/`output` queue arguments copy back on return
  (§13.5.2), `%p` renders function-local queues/associative arrays (§21.2.1.7),
  `foreach` over a string iterates its content length, `q = {}` clears string
  queues, and a never-touched module-scope queue reports `size() == 0`.
* **Free functions no longer see the caller's class context** (§13.4) — a bare
  name in a package/module function that collided with a caller class property
  used to silently alias the property; queue-property access from outside the
  class (`obj.q.push_back(x)`, `%p` of `obj.q`) now resolves correctly.

---

# Project Structure

xezim is split across two repos that live **side by side**; this repo depends on
`xezim-core` via a relative path (`../xezim-core`) — it is a sibling directory, not a
submodule:

```
../xezim-core/  — shared library: parser, elaboration, value, SDF, VCD sink (sibling repo)
./              — bytecode interpreter + simulator (this repo, binary: xezim)
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
└── Cargo.toml             — depends on xezim-core (path = ../xezim-core, a sibling repo)
```

### Components

**Parser & elaboration** — live in `xezim-core`; consumed by both `xezim` and `xezim-b`.

**Simulator** — event-driven VM over a bytecode lowering of cont_assigns and always blocks.

---

# Verified Workloads

End-to-end TEST PASSED with bit-identical results vs the workloads' own
golden expectations:

| Design | Test | sim_time / cycles | baseline wall | +O1 wall |
|---|---|---|---|---|
| XuanTie C910 (dual-core) | hello | sim_time 44695 | 95s | **73s** (1.30×) |
| XuanTie C910 | memcpy ×7000 | sim_time 101965 | 216s | **166s** (1.30×) |
| XuanTie C910 | cmark ×1 (`+iterations=1`, INIT_ZERO=1) | 167124 cycles | 87 min | **73 min** (1.19×) |
| XuanTie C906 (single-core) | memcpy ×50 | — | 99s | **88s** (1.13×) |
| XuanTie C906 | cmark ×1 (INIT_ZERO=1) | 295294 cycles | 714s | **587s** (1.22×) |
| riscv-dv (UVM 1.2) | `+num_of_tests=10` random RV32IMC | — | — | 10/10 assemble clean |

UVM run-phase (see [docs/uvm-guide.md](docs/uvm-guide.md)):

| Testbench | Result |
|---|---|
| GettingVerilatorStartedWithUVM vs **1800.2-2017** (`data0`/`data1`/`random`/`many_random`) | 4/4 — exact Verilator parity (monitors agree, `UVM_ERROR`/`UVM_FATAL` = 0) |
| GettingVerilatorStartedWithUVM vs **1800.2-2020.3.1** | green — in/out monitors agree (77/77 packets), `UVM_ERROR`/`UVM_FATAL` = 0 |
| sv-tests UVM 1800.2-2017 example suite | 32/35 pass (3 out of scope: deprecated UVM-1.0 macros, DPI backdoor) |

---

# Compliance

Full [sv-tests](https://github.com/chipsalliance/sv-tests) run with the
suite's own `xezim` runner (`make report RUNNERS=Xezim`), xezim 0.8.1. The
generated HTML report and per-test CSV are checked in under `reports/`
(`svtests_index.html`, `svtests_report.csv`, and `sv-tests-compliance.md`).

| Category | Pass / Total | Rate |
|---|---|---|
| **All tests** | **4354 / 4768** | **91.3 %** |
| &nbsp;&nbsp;UVM (1800.2-2017) | 484 / 487 | 99.4 % |
| &nbsp;&nbsp;non-`ivtest` | 2153 / 2237 | 96.2 % |
| &nbsp;&nbsp;Icarus `ivtest` suite | 2201 / 2531 | 87.0 % |

An earlier run scored only 52 % because a `-I` library directory
(`ivtest/ivltests/`, ~1000 mutually independent single-file tests) was scanned
too eagerly: xezim honors IEEE §23.3.2 library semantics — an `-I` dir supplies
module definitions to satisfy unresolved instantiations — but it was adopting
*every* definition in the directory, so typedefs/enums from unrelated sibling
files leaked into the primary design and failed a spurious §6.18 base-type
check. `resolve_library_modules` now pulls in only the library modules actually
reachable from the compiled design (transitively), which reclaimed ~1870
`ivtest` cases with no change to the native LRM/UVM results.

---

# Test Suite

Many test cases are included to validate functionality.

**Credit:**
All `pr*.v` tests were taken from the **Icarus Verilog test suite**.

These tests help verify correctness against real-world Verilog/SystemVerilog edge cases.

---

# Build

Install Rust: https://www.rust-lang.org/tools/install

This repo depends on `xezim-core` as a **sibling directory** (Cargo references it via
`path = "../xezim-core"`), so clone both repos side by side into the same parent:

```bash
git clone git@github.com:<you>/xezim-core.git
git clone git@github.com:<you>/xezim.git
cd xezim
```

Expected layout:

```
<parent>/
├── xezim-core/   — shared library (parser, elaboration, value, SDF, VCD)
└── xezim/        — this repo (binary: xezim)
```

Build the simulator:

```bash
cargo build            # debug
cargo build --release  # optimized (recommended for large designs)
```

The release binary is produced at `target/release/xezim`.

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
| `--max-time <N>` | Stop simulation at time `N` (counted in the design's finest time precision) |
| `+trace`, `+<plusarg>` | Passed through to `$value$plusargs` / `$test$plusargs` |
| `+seed=<n>` | Seed the RNG for a reproducible run (same seed ⇒ byte-identical output; affects e.g. the number of packets a random UVM test collects) |
| `--sdf <file>` `--sdf-{min,typ,max}` | Annotate standard delays |
| `--sim_debug` | Print `[DEBUG]` / `[OPT]` diagnostics |
| `--cache-dir <dir>` | Select the automatic elaborated-design cache directory |
| `--no-cache` | Disable the automatic elaborated-design cache |
| `-l`, `--log <file>` | Redirect all stdout/stderr — including DPI/VPI C output — to a log file |
| `-v <file>` | Library file: modules compiled only to resolve unresolved instantiations |
| `-y <dir>` | Library directory: `<module>.<ext>` loaded on demand |
| `+libext+<ext>+…` | Extension list for `-y` search (replaces the default `.v`/`.sv`/`.V`) |
| `+nospecify` | Suppress specify-block path delays — zero-delay gate simulation (`-nospecify` also accepted) |
| `+notimingcheck` | Accepted no-op: specify timing checks are not modeled (also `+notimingchecks`/`-notimingchecks`) |
| `--xtrace <file>` | Emit an XTrace v1.0 dump (`.zst`/`.zstd` ⇒ zstd-compressed) |
| `--xtrace-scope <hier>` | Restrict the XTrace dump to signals under `<hier>` (repeatable) |

Selected env knobs (off by default unless noted):

| Env var | Effect |
|---|---|
| `XEZIM_EVENT_EDGE=1` | Skip gateable clocked flop fires whose data is unchanged (1.13-1.30× wall on c910/c906) |
| `XEZIM_INIT_ZERO=1` | Coerce X-initialized signals/arrays to 0 (required for some C910/C906 workloads, e.g. cmark) |
| `XEZIM_PROGRESS=N` | Emit a `[PROGRESS]` line every N wall-seconds (sim_time, iters, edges_fired, nba_q) |
| `XEZIM_CACHE_DIR=<dir>` | Override the elaborated-design cache directory |
| `XEZIM_NO_CACHE=1` | Disable the automatic elaborated-design cache |
| `XEZIM_COMPILE_PHASES=1` | Report detailed simulator compilation phase timings |

Example — run the picorv32 testbench against a gate-level netlist:

```bash
./target/release/xezim testbench.v synth.v \
    +firmware=firmware/firmware.hex --max-time 50000000
```

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
xezim --module-timescale 1ns/1ps --module-timescale dram=1ps/1fs design.sv
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
precision declared anywhere in the design, down to `fs`. (`--max-time` is
counted in that tick, so a `1ps`-precision design covers proportionally less
wall-clock time for the same `--max-time`.)

### Inspecting resolved timescales

`--dump-timescales` prints the resolved timescale of every module *before* the
run — no source `$printtimescale` calls required. It reports each definition's
`` `timescale `` semantics (an explicit/`--module-timescale` value, or the
`1s/1s` default when a module has none) and flags the modules that carry no
`` `timescale ``. Combine it with `--module-timescale` to confirm an assignment
landed where you intended.

```bash
$ xezim --dump-timescales design.sv
=== module timescales (3 modules) ===
  cache                        10ns / 1ns
  cpu                          1ns / 1ps
  glue                         1s / 1s   (no `timescale — 1s/1s default)
======================================
```

A flagged module also emits the `has no timescale directive` warning in a
mixed-timescale design; give it a source `` `timescale `` or a
`--module-timescale` assignment to resolve it. (The reported `1s/1s` is the
IEEE-default *display* value; such a module's effective delay unit is the
design's global tick — a further reason to declare one explicitly.)

---

# Development Workflow

Typical development loop:

```
edit code
↓
cargo build
↓
run tests
↓
add new SystemVerilog features
```

Rust provides strong guarantees for memory safety and concurrency, making it well suited for building large-scale EDA infrastructure.

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

# Acknowledgements

* Icarus Verilog project for the public test suite
* The Rust community
* Open-source EDA projects
