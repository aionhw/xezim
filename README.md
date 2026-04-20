# xezim — SystemVerilog Simulator (Rust)

**xezim** is a lightweight **SystemVerilog simulator written in Rust** designed for experimentation, learning, and exploring AI-assisted chip design workflows.

> `xezim` was previously developed under the name `sisvsim`. The binary, library, and compiled-artifact magic were renamed in place; behavior is unchanged.

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

* SystemVerilog module parsing
* Signal and net representation
* Continuous assignments
* Basic expression evaluation
* Combinational logic simulation
* Sequential simulation infrastructure
* Test execution framework

---

# Project Structure

```
.
├── src/
│   ├── compiler/
│   │   ├── simulator.rs   — event-driven simulator + bytecode VM
│   │   ├── bytecode.rs    — bytecode compiler for cont_assigns and always blocks
│   │   ├── elaborate.rs   — module elaboration & hierarchy
│   │   └── value.rs       — 4-state (0/1/X/Z) Value type
│   ├── lib.rs
│   └── main.rs            — CLI entry point (binary: xezim)
│
├── xezim-parser/          — SystemVerilog parser (git submodule)
├── tests/                 — Rust integration tests + SV compliance suite
├── examples/
└── Cargo.toml             — package: xezim
```

### Components

**Parser**

Reads SystemVerilog source and builds the AST.

**Elaboration**

Resolves module hierarchy, signals, and connections.

**Simulator**

Evaluates expressions and propagates signal changes across the design.

---

# Test Suite

Many test cases are included to validate functionality.

**Credit:**
All `pr*.v` tests were taken from the **Icarus Verilog test suite**.

These tests help verify correctness against real-world Verilog/SystemVerilog edge cases.

---

# Build

Install Rust: https://www.rust-lang.org/tools/install

Clone with submodules (parser lives in `xezim-parser`):

```bash
git clone --recursive <repo-url> xezim
cd xezim
# or, if already cloned:
git submodule update --init
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
| `--top <module>` | Select the top-level module |
| `--max-time <N>` | Stop simulation at time `N` |
| `+trace`, `+<plusarg>` | Passed through to `$value$plusargs` / `$test$plusargs` |
| `--sdf <file>` `--sdf-{min,typ,max}` | Annotate standard delays |
| `--sim_debug` | Print `[DEBUG]` / `[OPT]` diagnostics |
| `--log <file>` | Redirect stdout/stderr to a log file |

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

