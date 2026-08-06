# Testing xezim

How the suite is organized, how to run it, and how to add a test.

## 1. The two harness styles

**In-process groups** (the norm for new features/bug fixes). Eight integration
group roots in `tests/` — `classes.rs`, `collections.rs`, `gates.rs`,
`hierarchy.rs`, `misc.rs`, `scheduling.rs`, `strings.rs`, `types.rs` — each a
flat list of `#[path = "<group>/<name>.rs"] mod <name>;` declarations (no `{}`
wrapper) over one topic directory. ~1680 `#[test]` fns total. A test simulates
an inline SV source and asserts on final signal values.

**Subprocess runners** (in `tests/misc/`). They spawn
`env!("CARGO_BIN_EXE_xezim")` against `.sv`/`.v` files and assert on the output.
Suites: `prtest_runner.rs` (Icarus `pr*.v`, ~132 of 762 wired),
`ivtest_{ce,misc,tail,tail2}_cluster.rs` (CE cluster has `reject(name, src)` /
`accept(name, src)` helpers — illegal forms must be REJECTED, legal neighbors
must still compile), `lrm_audit{,2,3}_runner.rs`, `issue30_runner.rs`,
`issue_cases_runner.rs`, `sv_compliance_runner.rs`, `sv2023_compliance_runner.rs`.

## 2. Running tests

```bash
cargo test                          # everything (the full run takes a while)
cargo test --test classes           # one integration group
cargo test --test classes class_property   # name filter within a group
cargo test --features jit           # the JIT config CI also covers
```

CI (`test.yml`) runs `cargo test --no-fail-fast` **and**
`cargo test --features jit --no-fail-fast` — both must stay green.

## 3. Adding an in-process test

Manually, or with `scripts/dev/new-test.sh <group> <snake_case_name>`.

**Both of these are required:**

1. Create `tests/<group>/<name>.rs`.
2. Add `#[path = "<group>/<name>.rs"] mod <name>;` to the group root
   `tests/<group>.rs`.

The test body looks like this:

```rust
//! §8.25 — doc comment cites the LRM section and the bug/fix.
use xezim::simulate;

#[test]
fn the_bug_under_test() {
    let sim = simulate(SV_SOURCE, 100).expect("simulate failed");
    assert_eq!(u(&sim, "tb.sig"), 42, "expected value");
}
```

`u` is a small local helper that tries `sim.get_signal(n)` then the
`tb.`-prefixed form — see the real `fn u` in
`tests/classes/issue4_coupled_constraints.rs:21` and
`tests/classes/class_property_param_width.rs:145`.

## 4. Conventions for a good test

- The `//!` doc comment cites the LRM § and the bug/fix (e.g.
  `tests/classes/class_property_param_width.rs` cites `§8.25 / §6.9.1`).
- The assertion message names the expected value.
- Use `xezim::simulate_multi` when you need multiple sources / include dirs /
  defines (see `tests/classes/uvm_config_db_tests.rs`).
- Signals are read with `sim.get_signal("tb.sig")` (try both bare and `tb.`
  prefixed names) → `.to_u64()`.

## 5. The subprocess runner suites and their stdout markers

Markers that matter:

- Positive tests must print `TEST_PASS` (not `TEST_FAIL`) or `PASSED` (not
  `FAILED`).
- No output may contain `Parse errors` or `Simulation error`.
- Negative tests must exit non-zero and print `: error:` / `Parse errors` /
  `Simulation error`.

## 6. Common pitfalls

- **Missing mod line**: a test file dropped in a topic directory but never
  registered in the group root compiles silently and never runs. This is the
  classic silent no-op.
- **Wrong group**: pick the topic directory that matches the feature.
- **Signal path**: read signals by their hierarchical name (`tb.` prefix) — a
  bare name may not resolve.
- **4-state X where 2-state is expected**: X-init can leak into asserted values;
  use `XEZIM_INIT_ZERO=1` only when X-init coercion is the point.
- **Random data without a fixed seed**: a test that collects random packets
  (`$urandom`) must not assert an exact count unless it fixes `+seed` — the
  default seed is 1, but an explicit `+seed` makes the intent obvious.
