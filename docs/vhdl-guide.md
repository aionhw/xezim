# VHDL (IEEE 1076-2008)

xezim simulates VHDL-2008 designs. Give it the `.vhd`/`.vhdl` files like
any other source:

    xezim tb.vhd
    xezim --work=mylib pkg.vhd --work=work top.vhd   # design libraries

Each file is analysed into the library named by the last `--work=<lib>`
before it (default `work`). The order of the files does not matter: design
units are analysed when something refers to them. `--std=08` is accepted
(VHDL-2008 is the only revision). The top is the last entity no other unit
instantiates. Output, messages and exit status follow GHDL:

    tb.vhd:12:5:@10ns:(report note): hello
    ghdl:error: bound check failure at tb.vhd:20

Without `--max-time`, a VHDL simulation runs until no events are left.

## How it works

xezim-core parses VHDL with the `vhdl_syntax` crate and does the rest
itself: design libraries, visibility, types, overload resolution, static
evaluation, and lowering of the elaborated design to SystemVerilog, which
the simulator runs. The packages of libraries `std` and `ieee` are the
IEEE P1076 Working Group sources, built in. VHDL's own simulation cycle
(signal update, events, delta cycles, process resumption in elaboration
order) is generated with the design, so its timing does not depend on the
SystemVerilog scheduler.

`XEZIM_VHDL_DUMP=1` lists the design units and prints the generated
SystemVerilog; `XEZIM_VHDL_DEBUG=1` explains rejected overloads.

## Supported (so far)

- Entities and architectures, processes (with or without sensitivity
  lists, `process (all)`), concurrent signal assignments (simple,
  conditional, selected), concurrent assertions and procedure calls.
- Hierarchy: entity instantiation (`entity work.e(a)`), component
  instantiation, configuration instantiation, with port maps and generic
  maps. A port associated with a signal or a static part of one is that
  signal (no extra delta); `open`, unassociated and literal actuals get a
  signal of their own. Each distinct set of generic values is analysed on
  its own, so subtypes may depend on generics.
- Components bind by default to the entity of the same name, by
  configuration specifications (`for u1 : c use entity ...`, `for others`,
  `for all`, `use open`), or by a configuration declaration, which may also
  be the top: block configurations of architectures, blocks and generate
  iterations (by index, range or alternative label), component
  configurations with nested block configurations.
- Block statements (with generics and ports) and for-, if- and
  case-generate statements, with their own declarations.
- Port maps with conversion functions and type conversions (in and out),
  partial association of formals (`v(3 downto 2) => x`), `inout` and
  `buffer` ports; a non-static actual expression goes through an implicit
  signal, one delta behind, as in GHDL.
- Types: enumeration, integer, real, physical (TIME), arrays (any number
  of dimensions; constrained, unconstrained, and with bounds computed at
  elaboration), records, access types, subtypes with range checks.
- Signals of those types with inertial and transport delays, `reject`,
  multi-element waveforms; `'event`, `'active`, `'last_value`,
  `'last_event`, also of elements. Several processes may drive a signal:
  resolved signals (`std_logic`, user resolution functions, element-wise
  resolution of `std_logic_vector`) and drivers of single elements or
  slices.
- `wait on/until/for`, every sequential statement, labelled `next`/`exit`.
- Functions and procedures: overloading, default and named parameters,
  `out`/`inout`, signal parameters of mode `in` (`rising_edge`),
  recursion, packages with deferred constants, aliases (also with a
  subtype).
- Expressions: all predefined operators (short-circuit `and`/`or`,
  matching `?=` etc.), aggregates, slices, concatenation, type
  conversions, attributes, allocators, `'image` and TO_STRING in GHDL's
  format.
- IEEE `std_logic_1164`, `numeric_std` and the other IEEE packages are
  compiled from their sources.
- TEXTIO output: WRITE of the standard types (justification, field,
  DIGITS, UNIT), SWRITE, JUSTIFY, WRITELINE to OUTPUT.
- `report`/`assert` and run-time checks (range, index, overflow, null
  access) with GHDL's messages and exit status; `std.env.stop`/`finish`.

## Not supported yet

Each is reported at its source location: guarded blocks, conversions and
partial association of `inout` ports, compound configuration
specifications, configuration declarative parts, signal parameters of mode
`out`/`inout`, procedures that wait,
assignments to record elements of a signal, file I/O other than WRITELINE
to OUTPUT and TEXTIO reads, protected types, generic types, subprograms and
packages, external names, force/release, `case?`, and mixing with
SystemVerilog.

## Verification

Every design under `tests/vhdl/designs/` is checked against GHDL 4.1's
output and exit status (`*.golden`, `*.exit`, made by `regen.sh`). The
group is behind a feature, so plain `cargo test` skips it:

    cargo test --features vhdl-tests --test vhdl

Processes resumed in the same simulation cycle run in an order the LRM
leaves to the implementation (§14.7.5.3): xezim uses elaboration order and
GHDL its own, so output written by several processes in one delta may come
in a different order. The test designs print from one process per delta.
