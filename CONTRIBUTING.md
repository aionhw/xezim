# Contributing to xezim

Bug reports and pull requests are welcome. This page explains what makes them
quick to land and how the test suites are organised. To build from source,
see [docs/building.md](docs/building.md).

## Reporting bugs

Open an issue on [GitHub](https://github.com/aionhw/xezim/issues) with:

* **A small, self-checking testcase** — a module that prints `PASS`/`FAIL` per
  check — and the command line you ran.
* **The output you expected and where it comes from:** the IEEE 1800 section,
  or another simulator's result. When you checked against a commercial
  simulator, say "the reference simulator" rather than naming a product.
* **The xezim version or commit.** `--verbose` prints the version banner
  (`=== xezim 0.11.4 ===`).

## Pull requests

* **Base the pull request on current `main`.** The code moves quickly; a
  branch several releases behind usually conflicts. Rebase before opening or
  updating it.
* **Keep formatting separate.** Run `cargo fmt` on your change, but don't mix
  a reformat of untouched code into a fix.
* **Add a regression test for each fix** under `tests/<group>/`, registered
  in `tests/<group>.rs`, that fails without the fix. Expected values should
  follow the LRM; cite the section in a comment. When you checked them against
  another simulator, say "the reference simulator" rather than naming a
  product.
* **Run the full suite** (`cargo test --release`) and include the result.
* **Mind the hot paths.** The simulator, bytecode and `Value` code run billions
  of times per simulation; for changes there, include an instruction count
  (`perf stat -e instructions`) before and after on a representative design,
  with identical output.
* **Fix the root cause**, not the one testcase, and keep each pull request to
  one topic.

## Test suites

Each suite is one test binary, `tests/<suite>.rs`, with its cases in
`tests/<suite>/`. In 0.11.4, the default suite passes 5,838 tests: 5,728
integration tests in eleven suites, 109 unit tests and one documentation test.
One additional scheduling test is marked `#[ignore]`:

| Suite | Tests | Covers |
|---|---:|---|
| `classes` | 1,287 | classes, UVM, randomization, covergroups |
| `collections` | 374 | queues, dynamic and associative arrays, array methods |
| `gates` | 195 | gate primitives, UDPs, drive strengths, waveform dumps |
| `hierarchy` | 614 | instances, ports, interfaces, binds, hierarchical references |
| `lrm` | 177 | IEEE 1800-2023 conformance cases |
| `misc` | 1,260 | CLI, lint, elaboration, assignments and other cases |
| `perf` | 48 | deterministic work counters that catch performance regressions |
| `scheduling` | 710 | event regions, sensitivity, timing, assertions |
| `strings` | 261 | strings, formatting, DPI and VPI |
| `types` | 798 | data types, widths, selects, operators |
| `upf` | 4 | power intent (`--upf`) |

The xezim-core repository has its own tests for the parser and elaboration.

The `lrm` suite, added after 0.11, is the IEEE 1800-2023 conformance group:
one module per LRM clause, each test running a small probe design and
comparing its output with the reference simulator's. Lines where xezim still
differs are left out and listed as `Known gap` comments at the test.

CI runs the full default suite (`cargo test`) and two JIT smoke tests selected
in `.github/workflows/test.yml`. The full JIT regression remains available
manually with `cargo test --features jit`; JIT execution itself is switched
on at run time with `XEZIM_JIT=1`. A large share are differential
tests whose expected values were measured on a commercial reference simulator;
their doc comments cite the LRM section and the measured behaviour. Tests
marked `#[ignore = "known gap: ..."]` pin a known divergence from the
reference until it is fixed.

The `pr*.v` tests come from the **Icarus Verilog test suite**.

### UVM tests

The UVM integration tests (`tests/classes/uvm_integration_tests.rs`) run against
the real Accellera UVM library from https://github.com/nitronis/UVM — one repo
carrying the 1.1d, 1.2, 1800.2-2017 and 1800.2-2020 releases as subdirectories.
No manual setup is needed: `cargo build` clones it into `target/uvm-checkout`
when no checkout is found (and the tests clone on demand as a fallback). To use
an existing checkout instead, set `XEZIM_UVM_DIR` to its root or clone it as a
`../UVM` sibling of this repository.

The `classes` target registers the 67 UVM cases as eight grouped tests. Each
case runs on a fresh thread, preserving isolation of thread-local simulator state;
the group continues after a panic and reports every failed case by its
original module/function name. The case bodies and their assertions remain
in the original `tests/classes/uvm*.rs` files. The complete case-to-group
mapping is in [`uvm_groups.rs`](tests/classes/uvm_groups.rs).

| Group selector (`uvm_groups::` prefix) | Cases | Coverage |
| --- | ---: | --- |
| `uvm_foundation_config_factory` | 11 | Configuration/resource databases, factory overrides and linkage |
| `uvm_phasing_versions` | 10 | Library versions, phase completion, objections and timeout |
| `uvm_sequences_traffic` | 8 | Sequencers, responses, locks, traffic and method policies |
| `uvm_tlm` | 8 | TLM-1/TLM-2 ports, channels, sockets and connection errors |
| `uvm_ral` | 10 | Register/memory policies, prediction and frontdoor/backdoor access |
| `uvm_dpi` | 6 | Regex, command line, HDL backdoor and assertion sampling |
| `uvm_printing_reporting` | 10 | Printers, strings, package calls and report controls |
| `uvm_objects_features` | 4 | Object automation, callbacks, events and barriers |

Run all groups or one group with the existing test binary:

```bash
cargo test --release --test classes uvm_groups::
cargo test --release --test classes uvm_groups::uvm_tlm
```

To select one original case, use its full name from the mapping:

```bash
XEZIM_UVM_CASE=uvm_integration_tests::uvm_2020_traffic_is_independent_of_method_policy \
  cargo test --release --test classes uvm_groups:: -- --nocapture
```

Unknown case names fail explicitly. Existing optional-library skips retain
their original messages. The method-policy case still runs its three
isolated process policies; its child selects the group and that one case.

### cocotb runner tests

The cocotb runner (`contrib/cocotb/xezim_runner.py`) has a pytest suite in
`contrib/cocotb/tests`, which `cargo test` does not run. It needs Python with
cocotb 2 and pytest. From the repository root, after a release build:

```bash
XEZIM_BIN=$PWD/target/release/xezim python -m pytest contrib/cocotb/tests
```

## License

By contributing, you agree that your contributions are licensed under the
[Apache License 2.0](LICENSE), the license of the project.
