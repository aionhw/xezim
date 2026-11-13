# xezim — Development Guide for AI Coding Agents

This file is read by coding agents at the start of every session. It is
**operational**: how to build and test, where things live, and what must not
break. It covers this repository: the bytecode compiler, the event-driven
simulator and the `xezim` command line. The preprocessor, parser, elaborator
and 4-state value model live in the sibling
[xezim-core](https://github.com/aionhw/xezim-core) repository; read
`../xezim-core/AGENTS.md` before changing anything there. If a fact here
conflicts with the code or the test suite, they win; fix this file in the
same change.

## What lives where

| Symptom | Owner |
|---|---|
| Parse or preprocessor error, AST shape | xezim-core (`xezim-parser/`) |
| Types, widths, parameters, port binding, legality at elaboration | xezim-core (`src/elaborate.rs`) |
| Value arithmetic, X/Z propagation, `$bits` | xezim-core (`src/value.rs`) |
| Wrong value at run time, scheduling, processes, classes, randomization | here (`src/compiler/`) |
| Command line, design cache, second-pass lint errors | here (`src/main.rs`, `src/*_lint.rs`) |

The simulator runs the **elaborated** design, so most wrong-result bugs are
here even when the construct looks like a front-end feature. Reproduce first,
then decide which repository owns the fix.

## Repository map

```
src/main.rs                    command line, design cache, run driver
src/cli_compat.rs              other simulators' command-line spellings
src/cli_files.rs               options that read their values from a file
src/lib.rs                     library API: simulate(), simulate_multi(), design-cache key
src/env_vars.rs                registry of every XEZIM_* variable (--show-env-avail)
src/should_fail_lint.rs        second-pass lint for illegal constructs (--compile)
src/type_lint.rs, hier_ref_lint.rs   assignment-compatibility and hierarchical-name checks
src/compiler/bytecode.rs       bytecode compiler for processes and continuous assigns
src/compiler/simulator.rs      simulator state and shared types
src/compiler/simulator/        execution by responsibility: event_loop, processes, edges,
                               classes, collections, constraint_solver/rand_csp, assertions,
                               coverage, DPI/VPI, checkpoint, timing_checks, ...
src/compiler/jit.rs, aot.rs    native compilation (feature `jit`)
src/compiler/fst_sink.rs       FST waveform writer
include/                       svdpi.h, vpi_user.h, sv_vpi_user.h, the UVM DPI driver
tests/<suite>.rs + tests/<suite>/   integration suites (one test binary per suite)
docs/                          user guide and topic guides
scripts/                       PGO build, UVM DPI build, local-core switch
```

## Build and test

Rust 1.100 MSRV, edition 2024. See `docs/building.md` for requirements and
the profile-guided build.

Native C/C++ builds use Clang and LLVM archive tools. Install `clang`,
`clang++`, `llvm-ar` and `llvm-ranlib` on `PATH`. Linux x86_64 Rust links
use Clang with the Rust toolchain's bundled LLD.

```bash
cargo build --release                          # target/release/xezim (thin LTO)
cargo test --release --no-fail-fast            # the full default suite, as CI runs it
cargo test --release --test <suite> <name>     # one suite, filtered by test name
cargo test --features jit --test <suite> <name>  # with the native backend
./target/release/xezim repro.sv --no-cache     # run a design; --no-cache while bisecting
```

- The suites are `classes`, `collections`, `gates`, `hierarchy`, `lrm`,
  `misc`, `perf`, `scheduling`, `strings`, `types` and `upf`; see
  `CONTRIBUTING.md` for what each covers. A new test goes in
  `tests/<suite>/<name>.rs` with one `#[path = "<suite>/<name>.rs"] mod <name>;`
  entry in `tests/<suite>.rs`. Most tests call `xezim::simulate(src, max_time)`
  and check the printed lines.
- Some tests spawn the built `xezim` binary. Do not rebuild the same target
  directory while a suite runs from it; use a separate `CARGO_TARGET_DIR` for
  experiments.
- CI runs the full default suite, two JIT smoke tests and the DPI tests on the
  nested-scheduler fallback and on macOS (`.github/workflows/test.yml`), plus
  an MSRV build (`msrv.yml`). The full JIT regression is manual
  (`cargo test --features jit`); JIT execution is switched on at run time
  with `XEZIM_JIT=1`.
- Use the release build for anything timing-related; a debug build is
  10–30× slower.
- For a quicker optimized build, set `CARGO_PROFILE_RELEASE_LTO=false` and
  `CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16`.

## Critical invariants — do not break these

- **Determinism.** The same design, seed and flags give the same output on
  every run. Use `xezim_core::hasher::{HashMap, HashSet}` wherever iteration
  order can reach output or scheduling. Check randomization, hashing and
  scheduler changes with `+seed=1` twice and diff the outputs.
- **Cache keys move with the code they cache.**
  - `METHOD_CACHE_COMPILER_SALT` (`src/compiler/simulator.rs`): bump it on
    any change that can alter compiled-method instructions or the
    admit/decline decision for the same source, and add a line to the comment
    above it.
  - The checkpoint `MAGIC` (`src/compiler/simulator/checkpoint.rs`): bump
    its last byte when a saved state field changes type.
  - The design cache is keyed by xezim-core's `XEZIM_BYTECODE_MAGIC`, the
    crate version and the semantic salt (`src/lib.rs`); a change to a
    serialized elaboration type bumps the magic in core.
- **Every `XEZIM_*` environment variable is registered** in
  `src/env_vars.rs`. `tests/misc/env_var_registry.rs` scans both repositories
  and fails on an unregistered or stale entry.
- **Command-line flags are documented.** A new flag goes in the `--help` text,
  `docs/command-line.md` and, for a `--relax-*` switch, the relaxation table
  in `docs/user-guide.md`.
- **Hot paths.** The bytecode VM, the event loop and `Value` code run billions
  of times per simulation. For a change there, give an instruction count
  (`perf stat -e instructions`) before and after on a representative design,
  with identical output.
- **The JIT matches the interpreter.** A change to compiled or native
  execution keeps output identical with `XEZIM_JIT=1`.
- **No commercial tool names.** Expected values measured on another simulator
  are cited as "the reference simulator" in code, tests, commits, notes and
  issues; never name the product.

## Coding conventions

- snake_case, rustfmt-clean, clippy-clean.
- Explain non-obvious fixes in a comment citing the IEEE 1800-2023 section
  (`§9.3.2`), and cite the same section in the regression test's doc comment.
- User-facing errors are diagnostics or `Result<_, String>`; do not panic on
  user input.
- Prefer the smallest correct change that fixes the root cause, not the one
  testcase, and match the surrounding style.
- **Commits:** a one-line imperative subject, such as
  `Read a dynamic-array or queue element in compiled code from the live collection (§7.5, §7.10)`,
  with code and its tests in separate commits.
- **User-visible changes:** a bullet under `### Unreleased` in `NOTES.md`
  (**Correctness**, **Performance** or **Command line**).

## Workflows

**Fix a wrong simulation result**
1. Cut the design to a small self-checking `.sv` (`PASS`/`FAIL` per check)
   and confirm the wrong output with `--no-cache`. `docs/DEBUGGING.md` lists
   the tracing switches.
2. Settle the expected behavior from the LRM, or from the reference
   simulator when the LRM leaves room.
3. Add a regression test that fails without the fix.
4. Fix the root cause. Probe the neighbouring forms too: other types and
   widths, compiled and interpreted paths, `XEZIM_JIT=1`.
5. Run the focused test, then the full suite.

**The fix belongs in xezim-core**: follow "Keeping xezim and xezim-core in
sync" below; the regression test usually still lives here.

## Keeping xezim and xezim-core in sync

The same section is in `../xezim-core/AGENTS.md`; change both together.

- **The pin is the two `rev =` lines** in `Cargo.toml` (`xezim-core` and
  `sv-parser`). Both name the same full 40-character core commit.
  `Cargo.lock` is git-ignored here, so nothing else pins core.
- **xezim CI builds the pinned core, not core `main`.** A core change is
  untested by xezim CI, and invisible to xezim users, until the pin moves to
  it. Core's own CI runs only the core and parser tests.
- **Develop against the local checkout.** Run `scripts/use-local-core.sh`
  once; plain `cargo build`/`cargo test` then uses `../xezim-core` (or
  `./xezim-core`) through a git-ignored `.cargo/local-core.toml` `[patch]`, included by
  the tracked `.cargo/config.toml` compiler configuration.
  `cargo tree -p xezim-core` shows a path when the patch is active.
- **Validate the exact commit you will pin.** Commit everything in core first
  (`git -C ../xezim-core status` clean), then run core's tests (`cargo test`
  and `cargo test --manifest-path xezim-parser/Cargo.toml` there) and the
  full suite here (`cargo test --no-fail-fast`) against it. Uncommitted core
  edits make the tested tree differ from the pinned one.
- **Push order:**
  1. Push core to `main`.
  2. Here, add a commit `Sync core revision` that changes only both `rev =`
     lines to the pushed core `HEAD` (`git -C ../xezim-core rev-parse HEAD`).
  3. Check that the pin resolves without the local patch: move
     `.cargo/local-core.toml` aside, run `cargo fetch`, put it back.
  4. Push xezim, with the commits that need the new core in the same push.
- **Never pin a core commit that is not on core's `origin/main`**
  (`git -C ../xezim-core merge-base --is-ancestor <rev> origin/main`). A
  local-only or force-pushed-away commit breaks every bare clone and CI run.
  Never rewrite pushed core history for the same reason: old xezim pins
  point into it.
- **A core push that xezim does not need yet** (docs, a core-only test)
  still gets a `Sync core revision` in the next xezim push, so the pin does
  not fall behind `main`.
- **Releases:** `xezim`, `xezim-core` and `sv-parser` always carry the same
  `version`. Push the core version bump, then pin xezim to that commit in the
  release push.

A quick check that the pin is current:

```bash
grep -o 'rev = "[0-9a-f]*"' Cargo.toml | sort -u        # exactly one line
git ls-remote https://github.com/aionhw/xezim-core.git refs/heads/main
```

## Before you open a PR

- A regression test for each fix, citing the LRM section, that fails without
  the fix.
- `cargo test --release --no-fail-fast` green, with the result in the PR.
- Cache keys bumped where the invariants above require it; determinism kept.
- No unrelated refactors or reformatting of untouched code.
- A core change: core pushed first and the pin moved, as below.
- If this guide is wrong (paths, commands, conventions), fix it in the same PR.

## Resources

- `../xezim-core/AGENTS.md`: the library this repository builds on.
- `docs/building.md`: build, profile-guided build, "Working on xezim-core".
- `CONTRIBUTING.md`: test suites, bug reports, pull requests.
- `docs/DEBUGGING.md`: tracing, hangs, comparing with another simulator.
- `docs/command-line.md`, `docs/user-guide.md`: user-facing flags and behavior.
- `docs/xezim-skill.md`: using xezim (as opposed to developing it).
