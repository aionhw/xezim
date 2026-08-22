---
name: xezim-development
description: Use when developing, fixing, or adding tests to the xezim SystemVerilog interpreter — wrong simulation results, elaboration or compile errors, RNG or determinism work, or adding a regression test.
---

# xezim-development

## Overview
xezim is a SystemVerilog **bytecode interpreter** (Rust). Behavior is validated through its simulator, not library unit tests. The pipeline: source → parser (`../xezim-core/xezim-parser`) → elaboration (`../xezim-core/src/elaborate.rs`) → bytecode (`src/compiler/bytecode.rs`) → VM (`src/compiler/simulator.rs`).

Core principle: reproduce a wrong behavior as a **minimal SV case in the simulator first**, then fix the file that owns the symptom, then add a regression test that cites its LRM §.

## When to Use
- Any bug fix or feature in xezim (or the `xezim-core` path dependency)
- A simulation result looks wrong, or a compile/elaboration error appears
- Adding a regression test
- RNG, solver, or any determinism-affecting change

When NOT: parser/elaborator work with no observable sim difference still needs the `xezim` suite to verify — the library's tests run through this repo.

## Workflow: Fix a bug
1. **Reproduce.** Write the minimal `.sv` and run `cargo run --release -- repro.sv --no-cache`. Confirm the wrong value.
2. **Add a failing regression test** (next section).
3. **Fix the file that owns the symptom** (Quick Reference).
4. **Verify:** the test group, then `cargo test --no-fail-fast` and `cargo test --features jit --no-fail-fast` (what CI runs).

## Workflow: Add a regression test
A test only runs if it is registered. **Both steps are required:**

1. **Create** `tests/<group>/<name>.rs` (topic dir under `tests/{classes,collections,gates,hierarchy,misc,scheduling,strings,types}/`).
2. **Register** it in the group root `tests/<group>.rs` with one line: `#[path = "group/name.rs"] mod name;`

Test body:

```rust
//! §8.25 — doc comment cites the LRM section and the bug/fix.
#[test]
fn whole_struct_write_reaches_nested_interface_member() {
    let sim = xezim::simulate(SV_SOURCE, 100).expect("simulate failed");
    assert_eq!(u(&sim, "tb.sig"), 42, "expected value from the bug report");
}
```

Run just this test: `cargo test --test <group> <name>`.

## Quick Reference
| Need | Do |
|---|---|
| Wrong sim value / race | fix `src/compiler/simulator.rs` |
| Parse or elaboration error | fix `../xezim-core` (`xezim-parser` / `elaborate.rs`) |
| `$display` formatting | fix `xezim-core/src/stdout_sink.rs` |
| Debug a behavior change | always pass `--no-cache` (stale cached artifact) |
| Watch a long run | `XEZIM_PROGRESS=N` |
| RNG stream | `+seed=<n>`; `+seed=random` opts into entropy |

## Common Mistakes
| Mistake | Fix |
|---|---|
| Test file added but never runs | missing `#[path]` mod line in the group root — silent no-op |
| Sim result unchanged after your fix | stale design cache — rerun with `--no-cache` |
| Edited the VM for a parse error | symptom lives in xezim-core, not `simulator.rs` |
| No LRM § in the test doc comment | every regression test cites its section |
| `std::collections::HashMap` or a 2nd allocator | use `xezim_core::hasher::{HashMap,HashSet}`; never another `#[global_allocator]` |
| Bare `println!` in sim code | use `log_println`/`log_eprintln` (feeds `--log`) |

## Red Flags — STOP
- You added a test file but no group-root `#[path]` line → the test never runs
- A result changed and you didn't re-run with `--no-cache`
- You changed serialized fields in `xezim-core` without bumping `XEZIM_BYTECODE_MAGIC`
- An RNG/solver change you didn't re-verify for determinism (`+seed=1` twice, diff output)

## Resources
- `AGENTS.md` — canonical repo guide (all conventions, CLI flags, invariants)
- `../xezim-core/AGENTS.md` — shared library (parser/elaboration) guide
- `docs/dev/*`, `README.md` — deeper design and feature notes
