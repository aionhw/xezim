# xezim — SystemVerilog Simulator (Rust)

**xezim** is a **SystemVerilog simulator written in Rust** designed for experimentation, learning, and exploring AI-assisted chip design workflows.

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
* Waveform / trace dumps — VCD (`$dumpfile`/`$dumpvars`), XTrace v1.0 (`--xtrace`,
  optional zstd compression + scope filtering), and AITRACE-T (`--aitrace`)
* **UVM run-phase execution** (Accellera 1800.2-2017, with `-DUVM_NO_DPI`) — a real
  UVM testbench runs end-to-end: build → connect → topology → `run_phase` stimulus →
  sequencer↔driver TLM handshake → packet collection → objection-driven termination →
  report summary. The reference testbench (GettingVerilatorStartedWithUVM) reaches exact
  Verilator parity, and 32/35 UVM 1800.2-2017 example testbenches pass. Multiple top
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

---

# What's new in 0.8.1

* **Event-control `iff` guards** are now evaluated (`@(posedge clk iff cond)`),
  in both procedural waits and `always` blocks — previously the guard was
  parsed but silently dropped, so the block fired on every clock edge.
* **Wide-value `%0d` printing fixed** — `to_dec_string` no longer overflows for
  values wider than 128 bits (UVM prints the 4096-bit `uvm_bitstream_t`); it now
  uses an arbitrary-width decimal conversion.
* **Duplicate DPI import** of the same foreign function is tolerated when the C
  binding and property match (LRM §35.5.2), erroring only on a real conflict.
* **Duplicate same-scope declaration** is now a hard error under strict mode
  (the default), a warning under `--no-strict`.
* **`+seed=<n>`** plusarg is documented in `-h` — seeds the RNG for a
  reproducible run (same seed ⇒ byte-identical output).
* **Library-directory (`-I`) scan tightened** — `resolve_library_modules` now
  adopts only the library modules reachable from the compiled design (§23.3.2),
  instead of every definition in the directory. Fixes typedef/enum leakage from
  unrelated sibling files; lifted the sv-tests score from 52 % to 91 %.
* **sv-tests compliance measured** — see [Compliance](#compliance) below and
  `reports/`.

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

**Native compiler** (`xezim-b`) — AOT-lowers an elaborated design to Rust and links a standalone binary.

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

UVM 1800.2-2017 run-phase (see [docs/uvm-guide.md](docs/uvm-guide.md)):

| Testbench | Result |
|---|---|
| GettingVerilatorStartedWithUVM (`data0`/`data1`/`random`/`many_random`) | 4/4 — exact Verilator parity (monitors agree, `UVM_ERROR`/`UVM_FATAL` = 0) |
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
| `--max-time <N>` | Stop simulation at time `N` |
| `+trace`, `+<plusarg>` | Passed through to `$value$plusargs` / `$test$plusargs` |
| `+seed=<n>` | Seed the RNG for a reproducible run (same seed ⇒ byte-identical output; affects e.g. the number of packets a random UVM test collects) |
| `--sdf <file>` `--sdf-{min,typ,max}` | Annotate standard delays |
| `--sim_debug` | Print `[DEBUG]` / `[OPT]` diagnostics |
| `--log <file>` | Redirect stdout/stderr to a log file |
| `--xtrace <file>` | Emit an XTrace v1.0 dump (`.zst`/`.zstd` ⇒ zstd-compressed) |
| `--xtrace-scope <hier>` | Restrict the XTrace dump to signals under `<hier>` (repeatable) |
| `--aitrace` | Make `$dumpfile`/`$dumpvars` emit AITRACE-T text instead of VCD |

Selected env knobs (off by default unless noted):

| Env var | Effect |
|---|---|
| `XEZIM_EVENT_EDGE=1` | Skip gateable clocked flop fires whose data is unchanged (1.13-1.30× wall on c910/c906) |
| `XEZIM_INIT_ZERO=1` | Coerce X-initialized signals/arrays to 0 (required for some C910/C906 workloads, e.g. cmark) |
| `XEZIM_PROGRESS=N` | Emit a `[PROGRESS]` line every N wall-seconds (sim_time, iters, edges_fired, nba_q) |

Example — run the picorv32 testbench against a gate-level netlist:

```bash
./target/release/xezim testbench.v synth.v \
    +firmware=firmware/firmware.hex --max-time 50000000
```

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

