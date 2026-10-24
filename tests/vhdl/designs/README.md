# VHDL reference designs

Each `<name>.vhd` is checked against GHDL, the reference VHDL simulator:
`<name>.golden` is GHDL's standard output and `<name>.exit` its exit status,
both produced by `./regen.sh` (GHDL 4.1, `--std=08`). They pin GHDL's
message formats — report/assert lines, `@<time>` and `'image` formats,
range/index/overflow check failures, `std.env` stop/finish — and become
simulation tests once VHDL designs simulate.
