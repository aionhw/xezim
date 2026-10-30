# Building xezim

* [Requirements](#requirements)
* [Build](#build)
* [Profile-guided build](#profile-guided-build)
* [Native compilation](#native-compilation)
* [Repository layout](#repository-layout)
* [Working on xezim-core](#working-on-xezim-core)

## Requirements

* **Rust 1.99.1 or newer** (the minimum supported version is checked in CI).
  Install it from https://www.rust-lang.org/tools/install.
* **Git access to GitHub.** The parser and elaborator live in a second
  repository, [xezim-core](https://github.com/aionhw/xezim-core), which Cargo
  fetches automatically as a pinned git dependency. There is no submodule and
  nothing else to clone.
* **A C compiler** (`cc`) for the DPI and VPI tests, and for building DPI
  libraries of your own.

## Build

```bash
git clone https://github.com/aionhw/xezim.git
cd xezim
cargo build --release    # optimized
./scripts/build-pgo.sh   # optimized + profile-guided (recommended for release)
```

The release binary is `target/release/xezim`; the profile-guided one is
`pgo-target/release/xezim`. `cargo build` without `--release` gives a debug
build, which is much slower and meant only for development.

## Profile-guided build

`./scripts/build-pgo.sh` instruments the release build, trains it and
rebuilds with the profile. It is the build to ship or benchmark with.

Run **without arguments** it trains on a bundled set — the `tests/perf`
shape designs, the `scripts/pgo-train` designs and the `xezim-bench`
workloads — which takes a few minutes on top of two release builds. Measured
against a plain release build of the same sources (interleaved, same
machine, host instructions): a C906 SoC CoreMark −0.4% to −0.7%, a C910 SoC
CoreMark −0.3% to −0.7%, and a loop-heavy DRAM-model stress −12%. Output is
bit-exact. The bundled trainer covers the executors and the scheduler; what it
cannot know is *your* design's hot mix, so the SoC gain is modest.

Run **with a training command** (`./scripts/build-pgo.sh xezim --simulate …`,
the binary path is substituted) it trains on that run instead. A profile
trained on the workload itself is worth far more — the C906 memcpy
benchmark:

| | instructions | wall |
|---|---|---|
| release | 176.92 B | 51.5 s |
| **PGO, trained on the run** | **151.31 B (−14.5%)** | **44.0 s (−14.6%)** |

— and it generalizes: a C906-trained profile gave Ibex −11.1% instructions
against −10.6% for an Ibex-trained one. Budget for the instrumented run,
though: the instrumented binary is far slower than release, so train on a
short run (one benchmark iteration, a few hundred thousand cycles), not the
full-length one. An unrepresentative trainer can *deoptimize* the paths you
care about.

Two more measured facts: **the wall-clock gain depends on the design being
instruction-bound** (Ibex CoreMark loses ~11% of its instructions but its
wall time does not move — its host bottleneck is memory), and **BOLT on top
of a PGO build is net negative**; PGO alone wins.

## Native compilation

`cargo build --release --features jit` adds the in-process JIT and the AOT
backend. They are switched on at run time; see the
[user guide](user-guide.md#native-compilation) for when they pay.

## Repository layout

xezim is split across two repositories. This one holds the simulator and the
command-line tool; [xezim-core](https://github.com/aionhw/xezim-core) holds
the shared front end and is consumed as a git dependency:

```
xezim-core (git dependency) — preprocessor, parser, elaboration, values, SDF, VCD
xezim (this repository)     — bytecode compiler, simulator, CLI (binary: xezim)
```

```
.
├── src/
│   ├── main.rs, cli_compat.rs   — command line, including other simulators' spellings
│   ├── lib.rs                   — library API: parse, elaborate, simulate
│   └── compiler/
│       ├── bytecode.rs          — bytecode compiler for processes and continuous assigns
│       ├── simulator.rs         — event-driven kernel and bytecode executors
│       ├── simulator/           — VPI, code coverage, constraint solving, timing checks, names
│       ├── jit.rs, aot.rs       — native compilation (feature `jit`)
│       └── fst_sink.rs          — FST waveform writer
├── include/                     — svdpi.h, vpi_user.h, sv_vpi_user.h, the UVM DPI driver
├── contrib/cocotb/              — cocotb runner backend and its pytest suite
├── docs/                        — user guide and topic guides
├── scripts/                     — PGO build, UVM DPI build, local-core switch
└── tests/                       — integration test suites (see CONTRIBUTING.md)
```

## Working on xezim-core

`xezim-core` is pinned to the exact revision this xezim revision was tested
against (see `rev = ...` in `Cargo.toml`). A bare clone therefore always
builds the verified pair — never an untested newer core — and a release tag
of xezim pairs with the core revision it shipped with. The pin moves in the
same commit that starts depending on new core behaviour.

To work on core, clone it next to (or inside) this repository and switch the
build to it. After this, plain `cargo build` uses your checkout directly, with
**no network fetch**:

```bash
git clone https://github.com/aionhw/xezim-core.git ../xezim-core
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
