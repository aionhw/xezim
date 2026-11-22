# DDR3 and SRAM simulation workloads

The DDR workload uses Micron's DDR3 v1.74 timing model and exercises
initialization, calibration, BL8 writes and reads, precharge, and refresh. It
compares every read beat and verifies finish time against a separate simulator.
A deliberately early DQS must produce the same timing messages in both variants.

Use the three files from the pinned mirror's `testbench` directory. The harness
checks their SHA-256 digests and prepares the baseline and a local optimized copy
in a fresh output directory. Both copies use the bounded memory mode. The
optimized copy skips a 64-lane timing scan only when both the data timing and
expected-strobe conditions are known false. It retains the loop index's final
value and the nonblocking mask clears. The workload observes the DDR ports and
timing messages. [Pinned Micron model mirror](https://github.com/ericmoderbacher/micronDdr3Model/tree/36592b9a0e622b12e325c98da8a79f1a3bedf9e2/testbench)

```sh
python3 scripts/bench-memory/bench_ddr.py MODEL_DIR RUN_DIR /path/to/xezim --bursts 512
```

For a simulator comparison, add `--compare-xezim /path/to/candidate` to hold the
model fixed. The harness uses the Linux `/usr/bin/time` and accepts `--cpu` for
affinity. It expects the independent simulator's `iverilog` and `vvp` tools on
`PATH`. Each sample records a simulation-only phase and total process wall time.

The SRAM fixture checks synchronous read-before-write on 64-bit data with a
testbench scoreboard. Use `MEM_CELLS=0` for a word array or `MEM_CELLS=2` for
separate bit-lane arrays. `MEM_DEEP=1` adds hierarchical wrappers.

```sh
/path/to/xezim --simulate --verbose --no-cache --error-exit -s top \
  -D MEM_CELLS=2 scripts/bench-memory/sram_tb.sv --max-time 100ms +cycles=100000
```
