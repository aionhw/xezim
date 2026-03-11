# SystemVerilog Simulator (Rust)

A lightweight **SystemVerilog simulator written in Rust** designed for experimentation, learning, and exploring AI-assisted chip design workflows.

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
│   │   ├── parser
│   │   ├── simulator
│   │   └── elaboration
│   │
│   └── main.rs
│
├── tests/
│   └── prtest/
│       ├── pr*.v
│       └── program*.v
│
└── Cargo.toml
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

Install Rust:

```
https://www.rust-lang.org/tools/install
```

Build the simulator:

```bash
cargo build
```

Release build:

```bash
cargo build --release
```

---

# Run

Example simulation:

```bash
cargo run -- examples/test.sv
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

