# UVM regression: GettingVerilatorStartedWithUVM

- **Date:** 2026-07-07
- **Simulator:** xezim 0.8.1 (debug build)
- **DUT:** `pipe` (2-stage pipeline), UVM testbench from
  [GettingVerilatorStartedWithUVM](https://github.com/MikeCovrado/GettingVerilatorStartedWithUVM)
- **UVM library:** Accellera IEEE 1800.2-2017 (`.../UVM/1800.2-2017/src`)

## Result summary

All four tests in the Makefile's `ALL_TESTNAMES` pass end-to-end with
**0 `UVM_ERROR` / 0 `UVM_FATAL`**. Each run builds the full UVM environment
(`uvm_test_top.env` → two agents with driver + monitor, scoreboard with
input/output analysis FIFOs, coverage), runs its sequence, and the
scoreboard's `compare_data()` checks every collected packet — so a 0-error
result reflects real matches, not a blocked/vacuous run.

| Test | Result | Packets collected & compared | Sim end (ns) |
|---|---|---|---|
| `data0_test`       | PASS (0 err / 0 fatal) | 77 | 1565 |
| `data1_test`       | PASS (0 err / 0 fatal) | 75 | 1535 |
| `random_test`      | PASS (0 err / 0 fatal) | 73 | 1500 |
| `many_random_test` | PASS (0 err / 0 fatal) | 73 | 1500 |

The differing packet counts are seed-dependent stimulus (see `+seed=<n>`).

## How it was run

There is no xezim target in `sim/Makefile`; the invocation was reconstructed
from its `SV_SOURCES`, `INCDIRS`, and `DEFINES`, substituting the local UVM
2017 library for the Makefile's `/opt/accellera/...` path. `+define+UVM_NO_DPI`
means no DPI shared library is required.

```sh
UVM=/home/bondan/repo/sv2023/UVM/1800.2-2017/src
cd GettingVerilatorStartedWithUVM/sim

for t in data0_test data1_test random_test many_random_test; do
  xezim --sv2017 \
    -D UVM_REPORT_DISABLE_FILE_LINE -D UVM_NO_DPI -D SVA_ON \
    +incdir+$UVM +incdir+../rtl +incdir+../sv +incdir+../tb \
    $UVM/uvm_pkg.sv ../sv/pipe_pkg.sv ../sv/pipe_if.sv ../rtl/pipe.v ../tb/top.sv \
    +UVM_TESTNAME=$t
done
```

Unlike the UVM 1.2 example testbenches, this one targets 1800.2-2017 directly
and does **not** require `-D UVM_ENABLE_DEPRECATED_API`.

## Non-fatal warnings observed (do not fail the tests)

- `run_test() invoked from a non process context` (×59) — emitted because
  `tb/top.sv` calls `run_test()` from an `initial` block; a known xezim quirk,
  no functional impact here.
- `delete: given index out of range for queue of size 0. Ignoring delete
  request` (×16) — a queue delete on an empty queue, warned and ignored.

## Notes

- Logs for each run are written to `GettingVerilatorStartedWithUVM/sim/logs/<test>.xezim.log`
  (that directory is gitignored in the tutorial repo).
- This exercises the fixes shipped in xezim 0.8.1 (wide-value `to_dec_string`
  overflow, event-control/`always` `iff` guards, DPI import tolerance).
