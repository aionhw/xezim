# Debugging xezim runs

Practical levers for diagnosing a misbehaving simulation — no simulator
source required. Roughly ordered from "look at the design" to "look at the
simulator".

## Golden rules

1. **Use the release build.** `target/debug/xezim` is 10–30× slower and its
   unoptimized stack frames are several times larger. xezim runs the whole
   compile+simulate on a worker thread with a 1 GiB stack (virtual, committed
   only as used; `XEZIM_STACK_MB` overrides, `XEZIM_STACK_MB=0` disables the
   worker), so deep UVM elaborations work in either build — but older
   binaries without that guard overflow the default 8 MiB main stack in
   debug builds ("thread 'main' has overflowed its stack").
2. **Minimize first.** Almost every bug in this repo's history fell to a
   <30-line single-file repro. Cut the design in half repeatedly; keep the
   `$display` markers.
3. **Differential-test against a reference simulator.** When semantics are in
   doubt, the reference's output is the ground truth this project tracks —
   not the LRM prose alone and not another simulator's folklore.

## Front-end questions ("did it even parse like I think?")

- `xezim --parse file.sv` — parse only, report errors.
- `xezim --parse --dump-ast file.sv` — print the full AST. The fastest way
  to answer "does `p1::step` parse as a scoped ident or a member access?"
- `xezim --lint ...` / compile-only invocations surface elaboration
  diagnostics without running.

## Elaboration tracing (env vars)

- `XEZIM_TRACE_TYPE=<substr>[,<substr>...]` — trace every typedef-width
  registration, overwrite, and resolution whose key contains a pattern.
  Answers "who set this type's width, and to what, in which pass?"
- `XEZIM_TRACE_PARAM=<substr>` — same for parameter values (every insert
  site is tagged with its source line).
- `--elab-stats` style output and `[PHASE]` lines show where compile time
  and table sizes go.

## Runtime tracing

- `XEZIM_TRACE_ALWAYS=1` (or `=<substr>`) — per-fire trace of always
  blocks (kind, scope, written signals).
- `XEZIM_VALUE_TRACE=<substr>[,<substr>...]` — the "where does the data
  stop flowing" tool: prints every committed change of any signal whose
  hierarchical name contains a pattern, as
  `[value-trace] t=<time> <name> <old> -> <new> (<phase>; <writer at file:line>)`.
  Blocking writes name the writing process; NBA commits happen outside the
  scheduling process and are labeled `nba`. Bit/part-select writes mutate
  in place and print the arrowless `name = value` form. Each pattern's
  match count is announced at startup, so a typo is visible immediately.
  `XEZIM_VALUE_TRACE_LIMIT=N` caps output (default 20000). To follow a
  pipeline, watch each stage register:
  `XEZIM_VALUE_TRACE=wr_data,buf_q,out_q xezim --simulate ... 2>trace.log`.
- `$display` probes remain the highest-signal tool; `%p`, `%h`, `%b` on the
  suspect expression at the suspect time beats a wall of waveforms.
- Waveforms: `--fst out.fst` writes an FST for any viewer; XTrace
  (`--xtrace`) is the native dump. Coverage lands in `xezim_cov.json`.
- `$printtimescale`, `+UVM_CONFIG_DB_TRACE`, `+UVM_OBJECTION_TRACE` (UVM
  builds without `UVM_NO_DPI` — xezim provides the UVM DPI-C helpers
  natively, no shared library needed).

## Activity profiles

`XEZIM_ACTIVITY_PROFILE=activity.json xezim --simulate --no-cache design.sv`
writes diagnostic activity data without requiring `--profile` or timing
instrumentation. `XEZIM_ACTIVITY_STRIDE=N` selects approximately one in N
settle invocations for the correlation sketch (default 64; use 1 for all).
Selection hashes invocation IDs to avoid aliasing periodic clock activity.

The versioned JSON contains fingerprints of the elaborated input, final
ordered entry layout and running executable; runtime options, plusargs and
tick precision; exact evaluation and distinct-invocation counts; and 16-word
MinHash sketches. Separate invocations at the same timestamp have different
IDs; repeated passes within one invocation share an ID. A null sketch means
no sampled observations. Instruction counts are static and byte footprints
are logical signal widths, not measured execution costs or memory traffic.

Collection covers the canonical combinational worklist, disables its gate
lane, and excludes eager clock-tree and direct port-negedge evaluations.
BSP collection is rejected. These profiles therefore describe an
instrumented path; measure collection overhead separately. Fingerprints are
noncryptographic content keys; external firmware, input files and libraries
still require their own frozen receipts. Output is replaced atomically.
Profiles currently supply observations only: they do not change scheduling,
fusion, partitioning or admission, and are not consumed by `XEZIM_COACT_FILE`.

## Scheduler A/B switches

Same-edge process ordering is reference-verified (always blocks first, then
parked waiters in the active region, pre-NBA reads). Two escape hatches
flip back to legacy behavior — a quick bisect for "ordering race vs value
bug":

- `XEZIM_WAITERS_FIRST=1` — resume `@(posedge)` waiters before that edge's
  always blocks.
- `XEZIM_ACTIVE_REGION=0` — resumed waiters run after the NBA commit
  (post-NBA reads) instead of inline in the active region.

If a testbench passes only with one of these set, it has a same-edge race;
the default matches commercial behavior.

## Hangs and runaways

- **SIGUSR1 hang report**: `kill -USR1 <pid>` prints where the simulation
  is stuck (current time, active process origin) without killing the run.
- A memory watchdog aborts runaway allocation; `XEZIM_STUCK_CLOCK=abort`
  turns a dead-clock spin into exit code 3 for CI.
- Infinite loops in user code vs simulator bugs: add a `$display` in the
  suspect loop; if the index visibly wraps (e.g. a 4-bit counter printing
  `-8`), suspect signedness/width handling and minimize.

## DPI / foreign code

- `--dpi-lib <path.so>` loads user C code; `[DPI]`-prefixed stderr lines
  report load failures, unresolved symbols, and unsupported prototypes.
  Unresolved imports return 0 rather than aborting — grep stderr for
  `[DPI]` before trusting a run that "works" without its C library.
- UVM's own DPI imports (command line, regex, `uvm_hdl_*`) are built in;
  a user `.so` defining the same symbols overrides the builtins.

## Caching

- `--no-cache` forces a fresh compile — always use it while bisecting, so
  a stale bytecode artifact can't mask a front-end change.

## Reference-differential recipe

Run `test.sv` (top module `top`) on the reference simulator and save its
transcript as `ref.out`, stripping any prefix the tool adds to each line.
Then:

```sh
xezim --simulate -s top test.sv --no-cache > mine.out
diff ref.out mine.out
```

Keep test output on `T|`-prefixed lines so the diff ignores banners. If the
reference disagrees with your expectation, the expectation is what changes.
