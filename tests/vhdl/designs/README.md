# VHDL reference designs

Each `<name>.vhd` is checked against GHDL, the reference VHDL simulator:
`<name>.golden` is GHDL's standard output and `<name>.exit` its exit status,
both produced by `./regen.sh` (GHDL 4.1, `--std=08`); `simulation.rs` runs
each through xezim and compares.

The top unit of `<name>.vhd` is named `<name>`: an entity, or a
configuration declaration that configures one.
