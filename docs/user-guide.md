# xezim user guide

How to run simulations with xezim and use its main facilities. The full list
of options and environment variables is in the
[command-line reference](command-line.md). Topic guides cover
[UVM](uvm-guide.md), [DPI-C and VPI](dpi-guide.md),
[coverage](coverage-guide.md), [power intent (UPF)](upf-guide.md) and
[debugging](DEBUGGING.md).

* [Running a simulation](#running-a-simulation)
* [Waveforms](#waveforms)
* [Warm design cache](#warm-design-cache)
* [Native compilation](#native-compilation)
* [Reducing a multi-file build](#reducing-a-multi-file-build)
* [Module-timescale extension](#module-timescale-extension)
* [Non-standard extensions](#non-standard-extensions)

## Running a simulation

```bash
xezim <source files> [+plusargs] [options]
```

One command preprocesses, parses, elaborates and simulates; there are no
separate compile or library steps. Top modules are found automatically, or
named with `-s <top>` (repeat it for several roots, e.g.
`-s hdl_top -s hvl_top`). From a source checkout, `cargo run --release --
<files>` builds and runs in one step.

A run prints the design's own output (`$display`, UVM messages, assertion
failures), warnings and errors, and one closing line —
`Simulation finished at time N`, with ` ($finish called)` when the design
finished itself. The version banner, `[PHASE]` timings and engine counters are
behind `--verbose` (or `--profile`). Parse, preprocessor and elaboration errors
name the original file, line and column — the `include`d file a line came
from, and the macro invocation for text a macro produced — and quote the line:

```text
In file included from top.sv:2:
body.svh:4:10: error: expected expression, found Semicolon ';'
    4 |   x = 3 +;
      |          ^
```

Example — run the picorv32 testbench against a gate-level netlist:

```bash
xezim testbench.v synth.v +firmware=firmware/firmware.hex --max-time 50000000
```

The exit status is 1 for parse and elaboration errors (among them a
hierarchical name that does not resolve, §23.8), `$fatal`, a run-time error
that ends the run (a read or call through a null handle in a process, a wait
on a mailbox that was never constructed, a loop or delta cycle that can never
let time advance, an SDF file `$sdf_annotate` cannot read), a
`--dpi-lib` library that cannot be loaded, and an `-s` that names no module
(`--no-strict-top` falls back to finding tops automatically). `$error` exits 0
unless `--error-exit` is given.

## Waveforms

Waveform support is compiled into the model only on request, because it is
not free: an active dump keeps some loops on the slower path and builds a
per-signal trace table. Pass `--wave` to enable `$dumpfile`/`$dumpvars`
(without it, `$dumpvars` warns once and does nothing). Three formats:

* **VCD** — `$dumpfile`/`$dumpvars` (IEEE 1800-2017 §21.7); it shows the same
  in GTKWave as the output of other open-source simulators.
* **FST** — `--fst <file>`, GTKWave's binary format, written on a dedicated
  thread; `--fst-scope <hier>` limits it to a subtree. Fixed-width signals,
  reals, and events are supported. Variable-length string variables are
  omitted with a warning naming them; their values are never truncated.
* **XTrace v1.0** — `--xtrace <file>`, optionally zstd-compressed
  (`.zst`/`.zstd`), with `--xtrace-scope` filtering.

`--fst` and `--xtrace` imply `--wave`. The three formats are cross-checked
against each other by decoding them.

Ctrl-C (SIGINT) or SIGTERM stops the run at the current time and closes the
dumps normally; this includes a long loop inside one time slot. If the run
cannot stop (a DPI call that does not return), the FST dump is closed at the
current time after half a second, and the process ends 5 seconds after the
signal. A second Ctrl-C ends it at once. An FST dump is also written out every
2 seconds (`XEZIM_FST_FLUSH_SECS`), and its first block as soon as the run
starts. Each block reaches the file whole or not at all, so after a `kill -9`,
an out-of-memory kill or a second Ctrl-C, at any moment, the file is readable
and holds everything up to the last block written.

## Warm design cache

Simulation mode stores a content-addressed elaborated design and compiled
combinational worklist after the first run, then reuses both on identical later
runs. A cache hit skips parsing, elaboration, and combinational dependency-index
construction, while simulator state, plusargs, time-zero initialization, and
event scheduling are rebuilt for every invocation. Timing-annotated and UDP
designs conservatively rebuild the worklist. The key covers source and library
contents, defines, include paths, top selection, language/strictness, timescale
and delay settings, and the xezim executable build.

The default directory is `$XEZIM_CACHE_DIR`, then
`$XDG_CACHE_HOME/xezim/designs`, then `$HOME/.cache/xezim/designs`. Use
`--cache-dir` for a workload-local cache or `--no-cache` for a cold run. With
`--verbose`, xezim prints `[CACHE] miss`, `[CACHE] stored`, or `[CACHE] hit` on
stderr.

## Native compilation

Built with `--features jit`, xezim can turn hot bytecode into machine code.

```bash
cargo build --release --features jit

# in-process JIT
XEZIM_JIT=1 ./target/release/xezim <sources> -s <top>

# AOT: generate Rust, build it with rustc, load the result
# (XEZIM_JIT=1 is required — XEZIM_AOT selects the backend, it does not
#  enable native compilation on its own)
XEZIM_JIT=1 XEZIM_AOT=1 ./target/release/xezim <sources> -s <top>

# AOT plus compiled process state machines
XEZIM_JIT=1 XEZIM_AOT=1 XEZIM_PROC_FSM=1 ./target/release/xezim <sources> -s <top>
```

**Whether it pays depends on the design — measure before adopting it.** Same
binary, warm native cache, wall-clock:

| | interpreter | `XEZIM_JIT` | `+AOT` | `+AOT +PROC_FSM` |
|---|---|---|---|---|
| Ibex CoreMark | 50.8s | **39.0s** (−23%) | **38.8s** | 39.1s |
| C906 memcpy ×100 | 49.3s | 55.7s (**+13%**) | 49.5s | 47.8s (−3%) |

The C906 loss is entirely compile time, not slower simulation: JIT takes its
simulation phase from 43.6s to 42.9s but spends 7.0s more compiling, because
the design has 35,267 combinational entries to Ibex's 1,553 and the per-block
cost is amortized ~37× less. Compiling only the hot subset does not rescue it
— the eval distribution is steep enough (15% of entries carry 99.2% of
evaluations) that a threshold looked promising, but JIT is only worth 2.3% of
C906's simulation phase in the first place, and on Ibex the warmup needed to
measure hotness costs more than the compile it saves. Rule of thumb: native
compilation pays on designs with relatively few, very hot blocks.

The AOT backend covers combinational entries, edge-sensitive blocks, and — when
`XEZIM_PROC_FSM=1` is also set — process FSMs. Blocks it cannot lower (values
wider than 64 bits, unsupported opcodes, X/Z-carrying shapes) stay on the
interpreter, so coverage is partial by design; `XEZIM_JIT_VERBOSE=1` prints the
`[AOT] … compiled N/M` summary.

Generating and compiling that Rust is the dominant cost on a first run — minutes
on a large SoC — so the resulting library is cached under `$XEZIM_CACHE_DIR`,
`$XDG_CACHE_HOME/xezim/native`, or `~/.cache/xezim/native`, keyed on the
generated source, `XEZIM_AOT_OPT`, and the xezim build. Repeat runs load the
cached `.so` directly. Set `XEZIM_NO_NATIVE_CACHE=1` to force a rebuild, and
`XEZIM_AOT_OPT=0` to trade steady-state speed for a faster build.

## Reducing a multi-file build

Three flags answer the questions that come up when a large `-f` build does not
behave: *which* files were read, *what* each contributed, and *what does the
code look like after preprocessing*.

```bash
xezim -f build.args --dump-files-list          # the resolved file list, then exit
xezim -f build.args -s testbench --verbose     # each file as it is parsed, and what it defined
xezim --parse -f build.args -s testbench --dump-merged-sv repro.sv
```

`--dump-merged-sv` writes every source into one self-contained `.sv` with
`` `ifdef `` branches resolved, macros expanded and `` `include ``s inlined — a
125-file build becomes a single re-runnable file. Given `-s <top>` it keeps only
the files that top actually needs, which is what makes the result small enough
to hand to someone else.

Two properties are worth knowing before relying on it:

* **The reduction is per file, not per module.** A file defining both a module
  you need and one you do not drags the second one's dependencies in too.
* **The closure is lexical and runs before parsing**, so the dump still works on
  a design that does not elaborate — the case the flag exists for. It is
  conservative in the safe direction: it may keep a file more than strictly
  needed, never one fewer. Files that declare no design unit at all (a
  file-scope `typedef`/function, a top-level `bind`) are always kept, since
  nothing references them by name and dropping them would change behaviour.

Note `--parse` above: the dump is produced before elaboration, so a design whose
elaboration takes minutes still dumps in seconds. Only the step that appends
adopted `-v`/`-y` library files needs `--compile` or `--simulate`.

## Module-timescale extension

`--module-timescale` is an xezim-specific command-line extension. It assigns a
time unit and precision to module *definitions* that have **no explicit
source-level timescale**, without changing the semantics of the source. It is
handy for retrofitting a timescale onto legacy RTL that omits one, or onto a
mix of files where only some carry `` `timescale ``. The extension is specified
in [Xezim_Module_Timescale_LRM_Extension.pdf](Xezim_Module_Timescale_LRM_Extension.pdf)
and [Xezim_Module_Timescale_User_Guide.pdf](Xezim_Module_Timescale_User_Guide.pdf).

```bash
# Every module without an explicit timescale gets 1ns/1ps:
xezim --module-timescale 1ns/1ps design.sv

# Only the listed definitions (comma-separated), 10ns/1ns:
xezim --module-timescale cpu,cache=10ns/1ns design.sv

# Repeatable; the named form wins over the global one:
xezim --module-timescale 1ns/1ps --module-timescale mem_ctrl=1ps/1fs design.sv
```

**A module has an explicit source-level timescale** — which the option never
overrides — when it has a `timeunit`/`timeprecision` declaration, **or** a
`` `timescale `` directive is active where it is declared (`` `resetall ``
clears that). Effective precedence, highest first:

1. module-local `timeunit` / `timeprecision`
2. an active `` `timescale `` directive
3. a named `--module-timescale mods=<unit>/<prec>`
4. a global `--module-timescale <unit>/<prec>`
5. the 1ns / 1ns default

The precision must be equal to or finer than the unit (`1ns/1ps` is legal,
`1ps/1ns` is an error). Two *different* named assignments for the same module
are an error; an unmatched name, or one that lands on a module that already has
an explicit timescale, is a warning (the assignment is ignored). Assignments
apply to a definition, so every instance of it shares the timescale.

Sub-nanosecond precision is honoured — the simulation tick is the finest
precision declared anywhere in the design, down to `fs`. `--max-time` is
independent of that: it is given in nanoseconds and converted to the tick, so
`--max-time 100` stops at 100 ns whether the design runs at `1ns` or `1fs`
precision. What a finer precision does change is the *number of ticks* covered,
and hence the wall-clock cost of reaching the same simulated time. Reported
times (`$time`, the closing `Simulation finished at time …`) are in ticks, so
the same run prints `100` at `1ns/1ns` and `100000` at `1ns/1ps`.

Because the cap is held in whole nanoseconds, a sub-nanosecond `--max-time`
(`--max-time 1ps`) is rejected rather than silently rounded to zero. To stop a
run as early as possible, prefer `--parse` or `--compile`, which never start a
simulation at all.

### Inspecting resolved timescales

`--dump-timescales` prints the resolved timescale of every module *before* the
run — no source `$printtimescale` calls required. It reports each definition's
`` `timescale `` semantics (an explicit/`--module-timescale` value, or the
`1ns/1ns` default when a module has none) and flags the modules that carry no
`` `timescale ``. Combine it with `--module-timescale` to confirm an assignment
landed where you intended.

```bash
$ xezim --dump-timescales design.sv
=== module timescales (3 modules) ===
  cache                        10ns / 1ns
  cpu                          1ns / 1ps
  glue                         1ns / 1ns   (no `timescale — 1ns/1ns default)
======================================
```

A flagged module also emits the `has no timescale directive` warning in a
mixed-timescale design; give it a source `` `timescale `` or a
`--module-timescale` assignment to resolve it. (The default is tool-defined by
IEEE 1800 §3.14.2.2; xezim uses `1ns/1ns` for both delays and `$realtime`, so
an untimed module's `#1` is one nanosecond — declare a timescale explicitly when
you mean something else.)

## Non-standard extensions

These are **not** part of IEEE 1800 — they are de-facto extensions of commercial
simulators, supported for compatibility with existing gate-level and testbench
flows. Portable code should not rely on them.

* **`$deposit(target, value)`** — sets `target` to `value` immediately *without*
  installing a persistent driver: the value holds until the next driver
  transaction overwrites it (on an undriven net it simply sticks). xezim
  matches the commercial semantics — a variable keeps the deposited value, and
  a real driver on a net overrides a deposit on its next update.
* Gate-level-simulation CLI flags — `+nospecify`, `+notimingcheck`,
  `+no_notifier`/`+no_tchk_msg`, `+delay_mode_zero`/`+delay_mode_unit`,
  `+mindelays`/`+typdelays`/`+maxdelays`, and the `-v`/`-y`/`+libext+` library
  flags — mirror the commercial spellings.
