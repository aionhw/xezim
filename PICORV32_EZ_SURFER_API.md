# PicoRV32 EZ Through Surfer API

This note records the current xezim-to-Surfer API smoke path using the PicoRV32
`testbench_ez.v` design.

## What Runs

The flow starts `xezim-surfer-plugin`, connects with `synthetic-surfer`, requests
simulator info and hierarchy, tracks the first eight hierarchy variables, and
then sends `RunSimulation { time: None }` so xezim runs until `$finish`.

The verified PicoRV32 EZ result reaches simulation time `11000` and returns final
tracked values through `SimulatorToSurferMessage::ValueChanges`.

## Why A Temporary Testbench Is Used

The upstream PicoRV32 EZ testbench uses:

```verilog
$dumpvars(0, testbench);
```

xezim currently treats that scoped dump request differently from Icarus for this
design, producing an empty VCD hierarchy. The script creates a temporary copy
that changes only:

```verilog
$dumpfile("/tmp/picorv32_test_ez_xezim_surfer.vcd");
$dumpvars;
```

The PicoRV32 checkout is left untouched.

## Run

From the xezim repo:

```bash
scripts/run_picorv32_ez_surfer_api.sh
```

Optional environment variables:

```bash
PICORV32_DIR=/path/to/picorv32 PORT=6972 scripts/run_picorv32_ez_surfer_api.sh
```

Default PicoRV32 path:

```text
/home/bondan/agent/claude/repo/picorv32
```

The script prints the synthetic Surfer output and the final plugin log tail. It
also writes the full plugin log to:

```text
/tmp/xezim_surfer_plugin_picorv32_ez_${PORT}.log
```

