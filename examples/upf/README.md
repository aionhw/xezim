# Power intent (UPF) example

A small power-aware design driven by IEEE 1801 power intent:

- `pwr_demo.sv` — `pwr_demo_top` with an accumulator (`u_acc`) and a
  configuration register (`u_cfg`).
- `pwr_demo.upf` — an always-on domain `PD_TOP` (supplies `VDD`, `VSS`), a
  switchable domain `PD_ACC` fed through the header switch `acc_sw`
  (`VDD -> VDD_ACC`, controlled by `acc_on`), an isolation strategy `acc_iso`
  that clamps the domain outputs to 0 while `acc_iso_en` is high, and a
  retention strategy `cfg_ret` that keeps `u_cfg` through the power-down.
- `pwr_demo_tb.sv` — imports the `UPF` package, turns the supplies on with
  `supply_on`, runs the block, powers `PD_ACC` down cleanly (isolate, then
  switch off), and checks that the accumulator is corrupted to `x`, the
  outputs stay clamped, and the configuration register is retained.

Run it:

    ./examples/upf/run.sh

or directly:

    xezim --simulate -s pwr_demo_tb --upf examples/upf/pwr_demo.upf \
        examples/upf/pwr_demo.sv examples/upf/pwr_demo_tb.sv

The run prints the `[UPF]` elaboration report (scope, supply nets, switch,
domains, strategies), the supply and switch events as they happen, and ends
with `UPF_EXAMPLE_PASS`. Add `--upf-top /pwr_demo_tb/dut` to name the scope
instance explicitly. `tests/upf/example_flow.rs` runs this example in the
regression suite.
