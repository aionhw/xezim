# Spike DPI shim for xezim

Tiny C++ shim that exposes Spike (riscv-isa-sim) state to SystemVerilog
testbenches as a `.so` loadable via xezim's `--dpi-lib`.

Purpose: provide an open-source Spike-backed reference model for UVM
testbenches (core-v-verif's cv32e40p, cva6, riscv-dv) that today expect
Imperas OVPsim. The shim's API is intentionally narrow — `init`,
`step`, `get_reg`, `get_pc`, `finish` — so dropping it into an existing
ISS-wrap module is a one-screen change.

## Two build modes

```bash
# Stub mode — returns canned values, builds with just g++
make
# real Spike — links libriscv.so + libfesvr.a + libsoftfloat.so
make real SPIKE_PREFIX=/path/to/spike/install
```

Stub mode is for exercising the SV-side integration (DPI loader,
testbench wiring) without depending on Spike. `make real` enables the
`XEZIM_SPIKE_REAL=1` compile path; the marked `TODO` blocks in
`xezim_spike_dpi.cpp` become real `processor_t::step()` calls.

## Smoke test

```bash
make
xezim -s tb --dpi-lib ./xezim_spike_dpi.so test_spike_dpi.sv
```

You should see three retire lines and the final `x1 = 3`, `pc =
0x8000000c` summary.

## Filling in the real Spike calls (Phase 2)

Each TODO in `xezim_spike_dpi.cpp` is paired with a hint of what the
real call looks like (e.g. `s->proc->step(1)`,
`s->proc->get_state()->log_reg_write`). The headers needed live under
`$SPIKE_PREFIX/include/riscv/` (processor.h, sim.h, mmu.h, cfg.h) and
`$SPIKE_PREFIX/include/fesvr/` (htif.h, elfloader.h, memif.h).

The minimum to drive cv32e40p:

1. Build a `cfg_t` with `--isa=rv32imc --priv=M`, a single hart.
2. Construct the memory map matching the cv32e40p TB (ROM and RAM
   regions; the OBI memory agent has the addresses).
3. Load the ELF via `htif_t::start()` so memory is populated.
4. Cache `proc = sim->get_core(0)` after `sim->run()` returns to the
   first `step` boundary, and from then on call `proc->step(1)` per
   `xezim_spike_step()` call.
5. After each step, read the retired PC and `log_reg_write` to surface
   the rd / rd_val pair.

## Integration with core-v-verif's `uvmc_rvfi_spike`

The OpenHW Spike RVFI reference model
(`lib/uvm_components/uvmc_rvfi_reference_model/uvmc_rvfi_spike.sv`)
already exists and is included by `uvmc_rvfi_reference_model_pkg.sv`.
That component expects a Spike-driving DPI surface very close to this
shim's API. Wiring it up amounts to:

1. Replace `imperas_iss.flist` references in `uvmt_cv32e40p.flist` with
   `uvmc_rvfi_reference_model_pkg.flist`.
2. Add `--dpi-lib /path/to/xezim_spike_dpi.so` to the xezim command
   line.
3. Wire the component's `step()` and `compare()` calls to the SV
   imports declared in `xezim_spike_dpi.svh`.

That's the next session.
