Training designs for `scripts/build-pgo.sh` when it is run without a
training command. Together with the `tests/perf` shape designs and the
`xezim-bench` workloads they exercise the paths a profile-guided build
must see: the two-state executors (narrow, control-flow and wide), the
settle loop, edge dispatch and skipping, the interpreter, the NBA queue,
loop-heavy clocked blocks and the x-bail backoff. The workloads are bounded
so the training profile is cheap to regenerate.

`hier_memory.sv` adds a synchronous 64-bit SRAM with bank decoding and
read-before-write checks, behind eight wrapper levels. `MEMORY_FLAT` runs
the same operations without the wrappers. `banked_nba_memory.sv` adds
parallel bank writes, byte enables and checked read responses. Both are
synthetic memory workloads, rather than DRAM protocol models. Define
`MEMORY_SMALL` for short checks or `MEMORY_LARGE` for longer measurements.
