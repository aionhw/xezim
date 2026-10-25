# VHDL reference designs

Each `<name>.vhd` is checked against GHDL, the reference VHDL simulator:
`<name>.golden` is GHDL's standard output and `<name>.exit` its exit status,
both produced by `./regen.sh` (GHDL 4.1, `--std=08`); `simulation.rs` runs
each through xezim and compares.

The top unit of `<name>.vhd` is named `<name>`: an entity, or a
configuration declaration that configures one.

`generic_subprograms.golden` is written by hand from the LRM: GHDL 4.1
cannot call instantiated generic subprograms (it reports an internal
error), so `regen.sh` reports it as failed and leaves the golden alone.
