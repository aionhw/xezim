# xezim — Development Guide for AI Coding Agents

This file is read by coding agents (Claude Code, OpenAI Codex, Cursor, Copilot) at
the start of every session. It is **operational**: what to build, how to build it,
what must never break. Deep design notes stay in `docs/` — this file points to them
and stays general. If a fact here conflicts with the code, the code (and the test
suite) is the source of truth; fix this file.

## What this is

`xezim` is a **SystemVerilog bytecode interpreter** written in Rust (~100k LOC in
`src/`). It parses IEEE 1800-2023 (opt back to 2017 with `--sv2017`), elaborates,
and simulates combinational + sequential logic, classes/UVM, constraints, DPI/VPI,
SDF, and VCD/XTrace/FST dumps. It is **not** a compiler — a separate crate
(`xezim-b`) does ahead-of-time native compilation and is out of scope here.

Parsing, elaboration, and the 4-state value model live in the **sibling repo
`xezim-core`** (a path dependency at `../xezim-core`, not a submodule — clone both
repos into the same parent). The simulator runs **elaborated** designs: most bug
fixes land either in `xezim-core` (parser/elaboration) or in `src/compiler/simulator.rs`
(the VM). Read `../xezim-core/AGENTS.md` before editing that repo.

## Repository map

```
src/main.rs                     CLI: arg parsing, --parse/--compile/--simulate/--preprocess,
                                memory watchdog, design-cache dir, -l log redirect
src/lib.rs                      Public API; test entry points xezim::simulate / simulate_multi;
                                content-addressed design cache
src/compiler/simulator.rs       Event-driven VM — the repo's largest file; most fixes touch it
src/compiler/bytecode.rs        Bytecode IR + compiler (flat Insn[] array, no pointer-chasing)
src/compiler/arena.rs           Bump allocator for per-tick/per-block allocations
src/compiler/dispatch.rs        Direct-threaded dispatch table (POC; partial integration)
src/compiler/soa.rs             Structure-of-Arrays signal-table layout (perf work)
src/compiler/jit.rs             Optional cranelift JIT (--features jit), falls back to interpreter
src/compiler/fst_sink.rs        FST waveform sink
src/intra_delay.rs              Pre-parse rewrite of `lhs = #d rhs` (see Gotchas)
src/should_fail_lint.rs         Additive second-pass error lint for sv-tests negative cases
src/multikernel.rs              EXPERIMENTAL per-LP PDES skeleton — not production
tests/                          8 integration groups + subprocess runners + compliance suites
bench/                          Cross-platform benchmarks (B1/B2/B3/B5, run_bench.sh)
simtest/                        XuanTie C910/C906 real-RTL workloads (need external setup)
dpi/                            Spike DPI shim example
include/                        Minimal svdpi.h / vpi_user.h / veriuser.h + UVM DPI driver
examples/                       Small .sv / .v demos (start here to try the tool)
scripts/                        UVM DPI build (Makefile), installers
docs/                           uvm-guide.md, dpi-guide.md, perf notes, design notes
reports/                        sv-tests compliance HTML/CSV and UVM reference reports
```

## Build

Prerequisite: `xezim-core` checked out at `../xezim-core` (Cargo references it via
`path = "../xezim-core"`). Toolchain: Rust **1.92** MSRV (`rust-version` in
`Cargo.toml`), edition 2024.

```bash
cargo build                        # debug
cargo build --release              # optimized; binary at target/release/xezim
cargo build --release --features jit    # optional cranelift JIT (CI tests this config)
cargo build --release --profile release-lto --bin xezim   # interpreter-only fat-LTO
```

`.cargo/config.toml` sets `RUST_MIN_STACK = 33554432` for cargo-invoked processes:
deeply recursive SystemVerilog expressions (concat chains, nested member access,
recursive functions, parameterized class construction) overflow the Rust default
2 MiB stack in debug builds. **Never lower this.** Release builds use smaller frames.

## Run

```bash
cargo run --release -- examples/full_adder.sv          # simulate (default mode)
./target/release/xezim <sources...> [plusargs] [options]
```

Modes: `--parse` (lex+parse only), `--compile` (parse+elaborate), `--simulate`
(default), `--preprocess` (emit expanded text). Common flags:

| Flag | Purpose |
|---|---|
| `-s <module>` | Top module; repeat for multiple roots (`-s hdl_top -s hvl_top`) |
| `-I<dir>` / `-D<MACRO>[=val]` | Include dir / preprocessor define |
| `--dpi-lib <path>` | Load a DPI-C shared library (repeatable) |
| `--vpi-lib <path>` (`-m`) | Load a VPI module; run its `vlog_startup_routines` |
| `--sdf <file>` + `--sdf-{min,typ,max}` | Annotate standard delays |
| `--max-time <N>[ps\|ns\|us\|ms\|s]` | Stop after N ns (unit default ns); cap in whole ns |
| `+<plusarg>` | Passed to `$value$plusargs` / `$test$plusargs` |
| `+seed=<n>` | RNG seed (default 1 ⇒ reproducible; `+seed=random` opts into entropy) |
| `--module-timescale [mods=]<u>/<p>` | Timescale for modules with none (xezim extension) |
| `--dump-timescales` | Print every module's resolved timescale, then run |
| `--dump-files-list` | Print resolved file list after `-f` expansion, then exit |
| `--dump-merged-sv <file>` | Write all sources as one self-contained `.sv` |
| `-l` / `--log <file>` | Redirect stdout+stderr (incl. DPI/VPI C output) to a log |
| `-v <file>` / `-y <dir>` / `+libext+<ext>+…` | Library files/dirs (IEEE §23.3.2) |
| `+nospecify` / `+notimingcheck` | Gate-level flags (commercial-compatible) |
| `--xtrace <file>` / `--xtrace-scope <hier>` | XTrace v1.0 dump (`.zst` ⇒ zstd) |
| `--sim-debug` / `--verbose` | `[DEBUG]`/`[OPT]` output / per-file compile progress |
| `--cache-dir <dir>` / `--no-cache` | Select / disable the design cache |

Env knobs (off unless noted): `XEZIM_EVENT_EDGE=1` (skip unchanged flop fires),
`XEZIM_INIT_ZERO=1` (coerce X-init to 0), `XEZIM_PROGRESS=N` (`[PROGRESS]` every N s),
`XEZIM_STUCK_CLOCK` (dead-clock watchdog: `warn` default, `abort` for CI),
`XEZIM_NO_MEM_WATCHDOG=1`, `XEZIM_CACHE_DIR`, `XEZIM_NO_CACHE=1`,
`XEZIM_COMPILE_PHASES=1`.

## Testing

The suite is the source of truth — the README reports 91.3 % of sv-tests pass
(4354/4768) and this suite must stay green. **CI runs `cargo test` in two feature
configs** (`test.yml`: `cargo test --no-fail-fast` then
`cargo test --features jit --no-fail-fast`); `msrv.yml` runs `cargo msrv verify`.

```bash
cargo test                       # everything (the full run takes a while)
cargo test --test classes        # one integration group
cargo test --test classes class_property   # name filter within a group
cargo test --features jit        # JIT config (CI requirement)
```

Two harness styles — use the one that fits:

1. **In-process groups** (the norm for new features/bug fixes). `tests/` has 8
   group roots — `classes.rs`, `collections.rs`, `gates.rs`, `hierarchy.rs`,
   `misc.rs`, `scheduling.rs`, `strings.rs`, `types.rs` — each a `#[path = "…"]`
   module list over one topic directory (~1680 `#[test]` fns total). A test
   simulates an inline SV source and asserts on final signal values:

   ```rust
   //! §8.25 — doc comment cites the LRM section and the bug/fix.
   use xezim::simulate;
   #[test]
   fn range_mixing_class_and_scope_parameters_keeps_elaborated_width() {
       let sim = simulate(SV_SOURCE, 100).expect("simulate failed");
       assert_eq!(u(&sim, "w_both"), 12, "W+MW-1:0 with W=8, MW=4");
   }
   ```
   Signals are read with `sim.get_signal("tb.sig")` (try both bare and `tb.`
   prefixed names) → `.to_u64()`. **To add a test: drop the file in the group's
   directory AND add one `#[path = "group/name.rs"] mod name;` line to the group
   root** — forgetting the mod line is the classic silent no-op.

2. **Subprocess runners** (`tests/misc/*_runner.rs`): spawn
   `env!("CARGO_BIN_EXE_xezim")` against `.sv`/`.v` files and assert on output.
   Markers that matter: positive tests must print `TEST_PASS` (not `TEST_FAIL`) or
   `PASSED` (not `FAILED`); no output may contain `Parse errors` or
   `Simulation error`; negative tests must exit non-zero and print `: error:` /
   `Parse errors` / `Simulation error`. Suites: `prtest` (Icarus `pr*.v`, ~132 of
   762 wired), `ivtest_*_cluster` (`reject`/`accept` — illegal forms must be
   REJECTED, legal neighbors must still compile), `lrm_audit*`, `issue30`,
   `issue_cases`, `sv_compliance` (manifest-driven positive + `tests_negative/`).

Test code conventions: each test's `//!` doc states the LRM § and what it guards;
assertion messages name the expected value; use the small `u(sim, name)` helper
pattern where appropriate.

## Architecture & where a symptom lives

Pipeline: source → preprocessor → parser/AST (`xezim-core/xezim-parser`) →
elaboration (`xezim-core/src/elaborate.rs`) → bytecode compile (`bytecode.rs`) →
event-driven execution (`simulator.rs`). Key types: `Value`/`LogicBit` (4-state
0/1/X/Z, width + signedness), `ElaboratedModule`, `Insn`, `Simulator`.

| Symptom | Owner |
|---|---|
| Parse/preprocessor errors, AST shape | `xezim-core/xezim-parser` (lexer, preprocessor, parse/, ast/) |
| Type/width/signedness, parameters, classes, port binding, `should_fail`-style legality | `xezim-core/src/elaborate.rs` |
| Wrong simulated value, race/timing, event ordering, class/UVM runtime, `randomize`, DPI/VPI, dumping | `src/compiler/simulator.rs` |
| Compile-time codegen / VM perf | `bytecode.rs`, `dispatch.rs`, `soa.rs`, `jit.rs` |
| `$display`/formatting output | `xezim-core/src/stdout_sink.rs`, `simulator.rs` |

Subsystems inside `simulator.rs`: Active/NBA/Reactive scheduling regions, comb
settle iteration, edge dispatch (always_ff/always_latch/@posedge, `iff` guards),
event/mailbox/semaphore waiters, process control (§9.7), constraint solver,
coverage/covergroups, SDF delays, VCD/XTrace/FST sinks, plus `pdes_*` (experimental
parallel, not production). Read `README.md` §Features and the `docs/*` design notes
before deep work.

## Critical invariants — do not break these

- **Determinism is a hard requirement.** Two runs with identical inputs must be
  byte-identical. The RNG defaults to seed 1 (`+seed=<n>` for a different stream;
  `+seed=random` opts into entropy) and hash iteration is deterministic
  (`xezim_core::hasher::HashMap/HashSet`, fixed ahash seeds). **Never** use
  `std::collections::HashMap/HashSet` where iteration order can affect observable
  behavior, and never seed an RNG from entropy by default.
- **Global allocator**: `mimalloc` is installed in `xezim-core` (its default
  feature). Rust allows exactly one `#[global_allocator]` per binary — never add
  another. A downstream crate opting out uses
  `xezim-core = { path = "../xezim-core", default-features = false }`.
- **Design cache**: `--simulate` stores a content-addressed elaborated design and
  reuses it on identical runs; it prints `[CACHE] hit/miss/stored` on stderr. The
  key covers sources, defines, top, and the executable's own mtime/size, so a local
  rebuild invalidates it — but use `--no-cache` whenever you are debugging a
  behavior change you expect to see immediately.
- **Memory watchdog**: `main.rs` spawns a thread that **SIGKILLs the process** when
  RSS exceeds 3/4 of system memory (protects against OOM on huge designs). For
  C910-scale runs set `XEZIM_NO_MEM_WATCHDOG=1`. This is expected behavior, not a
  crash bug.
- **intra-assignment delays**: the parser discards `lhs = #d rhs` (§9.4.5), so
  `src/intra_delay.rs` rewrites the source text into a `$__xz_intra_delay(...)`
  marker call **before parsing**. Any parser/lexer work must preserve this
  interplay (the rewrite is applied in `simulate_multi` and the CLI).
- **`should_fail` lint**: `src/should_fail_lint.rs` is an *additive* second pass
  that rejects illegal constructs in `--compile` mode for sv-tests negative cases.
  It must never change clean-design behavior and must stay conservative (validated
  against a 1005-case static baseline). Add checks there only for definite LRM
  violations.
- **Logging**: route output through `log_eprintln`/`log_println` (they feed the
  `-l` fd-level redirect that captures DPI/VPI C output). Bare `println!`/`eprintln!`
  bypasses `--log`.
- **Timescales**: per-module; the simulation tick is the finest declared precision
  anywhere (down to `fs`); `--max-time` is in ns (independent of tick); `$time`
  reports ticks. `--module-timescale` only applies to modules with no source-level
  timescale.
- **Sibling coordination**: parser/elaboration changes in `../xezim-core` are shared
  with `xezim-b`. Keep the `xezim::compiler::*` re-export surface stable.

## Coding conventions

snake_case; Rust 2024 edition; MSRV 1.92 (`rust-version` in `Cargo.toml`);
clippy-clean. Modules carry `//!`
doc comments. Hard fixes get explanatory comments with **LRM § citations**
(e.g. `§8.25`) and regression tests referencing the same section. Keep the public
API minimal — re-exports from `xezim-core` exist so existing `xezim::compiler::…`
paths keep working; do not remove them. User-facing errors are `Result<_, String>` /
diagnostic strings; do not panic on user input. Prefer the smallest correct change
and match the surrounding style.

## Workflows

**Fix a bug**
1. Write the minimal SV repro and confirm the wrong behavior (`./target/release/xezim repro.sv`).
2. Add a failing regression test (in-process group preferred; correct group root + `#[path]` mod line).
3. Fix it in `simulator.rs` or `xezim-core`; keep the fix localized.
4. `cargo test --test <group> <name>` until green, then the whole group.
5. Run `cargo test --features jit` for the config CI covers, and re-verify
   determinism (`+seed=1` twice, diff output) for RNG/solver changes.

**Debug a wrong sim result**: `--sim-debug` and `--verbose` for diagnostics;
`--dump-files-list`/`--dump-merged-sv` to understand a multi-file build;
`XEZIM_PROGRESS=5` to watch a long run; the stall report (names the parked
processes and the signal that should have woken them) for hangs; `XEZIM_STUCK_CLOCK`
for frozen-clock churn; `--max-time` to bound a runaway; (or use `scripts/dev/quick-repro.sh '…snippet…'` to iterate without writing a repo file) `--threads` to toggle the
parallel edge path (also `XEZIM_NO_PARALLEL=1` / `XEZIM_FORCE_PARALLEL=1`).

**Add a feature**: parse it in `xezim-parser` → elaborate/type it in `elaborate.rs`
→ execute it in `simulator.rs` (bytecode only if it must be fast) → add tests →
document any new flag/env knob in the README tables.

## Git identity

Every commit must be **authored and committed as the same single identity**:
`opensource-elearning <159253500+opensource-elearning@users.noreply.github.com>`
(the only verified email for the GitHub account). Never commit under the legacy
`opensource-elearning@users.noreply.github.com`, the typo variant
`opensource.elearning@users.noreply.github.com`, or any new username/email. A commit
whose author and committer emails differ makes GitHub render two identities and
notify the wrong account, so before committing run
`git config user.name opensource-elearning` and
`git config user.email 159253500+opensource-elearning@users.noreply.github.com`
and keep author == committer on every commit.

## Before you open a PR

- Regression test added **with an LRM § citation** in its doc comment, wired into
  the correct group root.
- `cargo test --no-fail-fast` **and** `cargo test --features jit --no-fail-fast`
  both green (this is exactly what CI runs — a PR that fails CI here will be
  rejected).
- New CLI flag or env var documented in the README tables (and `print_usage`).
- Determinism preserved; no `std` hash/RNG order leaks.
- Minimal diff: no unrelated refactors, no dead code added.
- Run the full suite on one machine before submitting — a few local tests are not
  enough.
- If this guide becomes wrong (paths, commands, conventions), fix it in the same PR.

## Resources

- `docs/dev/README.md` — developer documentation: architecture, debugging, testing, gotchas
- `README.md` — features, CLI reference, compliance, build/run, module-timescale extension
- `docs/uvm-guide.md` — running UVM 1800.2-2017 / 2020.3.1 testbenches
- `docs/dpi-guide.md` — compiling + loading DPI-C libraries
- `docs/ast_shared_stmt_lists_scope.md`, `docs/perf_dump_offload_2026-07-28.md` — design/perf notes
- `bench/README.md` — benchmark methodology (fix the work, not the time)
- `reports/sv-tests-compliance.md` — sv-tests pass rates by category
- `simtest/xuantie_c910/README.md` — real-RTL C910/C906 workloads
