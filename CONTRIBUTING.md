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
