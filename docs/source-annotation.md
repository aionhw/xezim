# Source-annotated tracing with `--stems`

`--stems <file>` writes an RTLBrowse `.stems` companion file next to a waveform
dump. GTKWave's `rtlbrowse` view then opens the design source and annotates
live signal values onto the lines while the waveform plays. Without the sidecar
GTKWave can only show the waveform; with it, clicking a hierarchy item jumps to
the module's declaration and the trace appears against the source.

The stems file is the same line format the `xml2stems` converter emits for
Verilator flows. xezim writes it directly from its own elaborated module,
so no converter step and no second tool are needed.

```sh
xezim --simulate --fst wave.fst --stems out.stems counter.sv
gtkwave -t out.stems wave.fst
```

## What the file contains

One record per line, two kinds:

```text
++ module <name> file <path> lines <start> - <end>
++ comp <instance> type <module-type> parent <parent-module-name>
```

* `++ module` names every module definition in the design (the top module plus
  each instantiated definition), its source file, and the line its `module`
  keyword starts on.
* `++ comp` turns the instance tree into the hierarchy RTLBrowse renders:
  each instance, the definition it instantiates, and the module that contains
  it. A module no `++ comp ... type` record references is a tree root.

Line numbers come from a header-keyword scan of the preprocessed source text —
the same text elaboration uses — so comments and attributes above a header do
not shift the reported line. Interfaces, programs, and macromodules resolve
the same way as modules; an instance of any of them appears in the tree like
any other module.

## Worked example

`counter.sv`, two modules and one instance:

```systemverilog
module counter (input logic clk, output logic [3:0] count);  // line 1
  always_ff @(posedge clk) count <= count + 1;
endmodule

module top;                                                   // line 5
  logic clk;
  logic [3:0] count;
  counter u_count (.clk(clk), .count(count));

  always #5 clk = ~clk;

  initial begin
    clk = 1'b0;
    #120;
    $finish;
  end
endmodule
```

Run it:

```sh
xezim --simulate --max-time 120 -s top --fst counter.fst --stems counter.stems counter.sv
```

The sidecar written by that run:

```text
++ module top file counter.sv lines 5 - 5
++ module counter file counter.sv lines 1 - 1
++ comp u_count type counter parent top
```

Open the source-annotated view:

```sh
gtkwave -t counter.stems counter.fst
```

RTLBrowse shows `top` with `u_count` (of type `counter`) instantiated under it.
Clicking either module opens `counter.sv` at the real declaration line — 5 for
`top`, 1 for `counter` — while the waveform shows `clk` toggling and `count`
counting up. `-t` must be followed by the stems file **before** the waveform
file.

## Behavior on edge cases

* A compiled-artifact run (`--compile` then `--simulate`) carries no
  preprocessed source text, so declaration lines fall back to 1; file paths
  stay real.
* A source path containing whitespace breaks the stems grammar: the parser
  tokenizes fields with `%s`, so whitespace splits one path into several
  tokens. Paths pass through verbatim, and paths with spaces are best avoided,
  as they are with `xml2stems`.
* An escaped identifier in a `module` header (`module \top ;`) falls back to
  line 1 rather than scanning the whole symbol.
* A module defined inside a file pulled in by `\`include` is reported against
  the including file, at the position the preprocessor spliced it to. The
  records stay structurally valid (hierarchy and parents all resolve), but
  the file/line pair for such a module can point at a blank or spliced line.
  Keeping module definitions out of include files avoids the ambiguity until
  the preprocessor tracks include provenance.

## Viewing in Surfer

Surfer reads the FST or VCD dump directly, so the same `--fst` run opens in
Surfer today with the full signal tree. Surfer has no source window yet and
does not read `.stems`. The stems records carry exactly what such a window
consumes — module to file:line, plus the instance tree — alongside the FST's
scope hierarchy, so the two artifacts that drive `gtkwave -t` are an open,
gap-free contract for a Surfer source-annotation feature, with no dependency
on the RTLBrowse view.

## Verification

17 integration tests cover the feature: hierarchy and flat designs, multi-file
sources, repeated instantiation, deep parent chains, interface instances,
forward references, comments and attributes above a header, and the fallback
behaviors above; a negative case asserts an elaboration failure writes no
sidecar, and a parser test rejects malformed stems output. The tests parse the
generated files back with an oracle that models the RTLBrowse grammar. The
suite runs green under both feature modes (`cargo test` and
`cargo test --features jit`).