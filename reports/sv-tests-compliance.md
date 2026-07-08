# sv-tests compliance run (runner = xezim)

- **Date:** 2026-07-07
- **Simulator:** xezim 0.8.1 (release build)
- **Suite:** [chipsalliance/sv-tests](https://github.com/chipsalliance/sv-tests) @ `4d3772a6`
- **Command:** `make report RUNNERS=Xezim -j5` (with `XEZIM_BIN` pointing at the
  release binary)
- **Full HTML report:** `sv-tests/out/report/index.html` (+ `report.csv`)

## Headline

| Category | Pass / Total | Rate |
|---|---|---|
| **Native sv-tests (LRM chapters + UVM)** | 1621 / 1647 | **98.4 %** |
| Imported third-party suites | 860 / 3121 | 27.6 % |
| **All tests** | **2481 / 4768** | **52.0 %** |

xezim passes **98.4 %** of the native SystemVerilog LRM compliance and UVM
tests. The overall 52 % is pulled down by the imported third-party suites —
above all the Icarus Verilog `ivtest` regression set, which is ~53 % of all
tests and targets simulator-specific Verilog behaviors rather than SV-LRM
conformance.

## Imported-suite detail

| Suite (tag) | Pass / Total | Rate | Notes |
|---|---|---|---|
| `ivtest`   | 352 / 2531 | 13.9 % | Icarus Verilog regression suite (simulation behaviors, delays, PLI) |
| `basejump` | 335 / 360  | 93.1 % | BaseJump STL cores |
| `yosys`    | 169 / 216  | 78.2 % | Yosys front-end tests |

## Selected native-tag rates (all ≈100 %)

`uvm-req` 294/294, `uvm` 107/110, `uvm-classes` 36/36, keyword-library
(`5.6.2`) 248/248, data-types (`5.7.1`) 63/64, `11.4.*` operators 100 %,
`22.5.1` 29/29.

## How it was run

There was no manual wiring needed — sv-tests already ships a `Xezim.py`
runner (registers as `xezim`, resolves the binary from `$XEZIM_BIN` first,
then `xezim` on `PATH`). Modes map to `--preprocess` / `--parse` /
`--compile` / `--simulate`; all invocations pass `--sv2017`.

```sh
export XEZIM_BIN=/home/bondan/repo/fix/jul7/xezim/target/release/xezim
cd sv-tests
make report RUNNERS=Xezim -j5
# → out/report/index.html
```

Per-test timeouts are honored by the runner (from each test's metadata), so
a pathological case can't stall the batch. A handful of `WARNING | Error when
opening file third_party/...` messages come from tests referencing
third-party submodules that aren't checked out; they don't affect the xezim
results above.
