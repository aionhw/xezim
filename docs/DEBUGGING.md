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
   If worker creation fails, the CLI retries 1024, 512, 256, 128 and 64 MiB stacks
   below the requested size, then the current thread, and warns with the
   selected stack and a remedy (`XEZIM_STACK_MB`, or `ulimit -s` for the
   current thread).
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

- `XEZIM_OOM_TRACE=memory.jsonl` — stream memory diagnostics to a JSONL file;
  `1` writes to stderr and `0` disables tracing. See
  [Tracing memory growth](#tracing-memory-growth) below.
- `XEZIM_TRACE_TYPE=<substr>[,<substr>...]` — trace every typedef-width
  registration, overwrite, and resolution whose key contains a pattern.
  Answers "who set this type's width, and to what, in which pass?"
- `XEZIM_TRACE_PARAM=<substr>` — same for parameter values (every insert
  site is tagged with its source line).
- `--elab-stats` style output and `[PHASE]` lines show where compile time
  and table sizes go.

## Tracing memory growth

Run the failing command with tracing enabled. Use `--no-cache` to capture
parsing and elaboration even when a cached design exists:

```sh
XEZIM_OOM_TRACE=memory.jsonl xezim --compile --no-cache -f files.f -s top
tail -n 20 memory.jsonl
```

The trace appends JSON records on entry to and exit from source work, at
pipeline checkpoints, and once per second while work is active. Each record
includes `phase`, `source` (file and line when available), `definition`,
`instance`, elapsed time, current and peak RSS in KiB, the RSS change since
the preceding record, and the latest table counts. Linux supplies the RSS
fields; other platforms report `null`. The trace works with `--simulate`
too, including simulator construction and bytecode compilation milestones.
Array-storage records put the array name in `instance` and report its cell
count and element width before allocating storage. Global reservation
records report total cells or packed capacity bytes.

To inspect growth alongside its active source:

```sh
jq -r '[.elapsed_ms,.rss_kib,.delta_rss_kib,.phase,.source,.instance] | @tsv' memory.jsonl
```

For #721, compare compilation-unit injection records (`unit_variables`,
`unit_nets`, `unit_assertions`, `module_items`) with instance preparation
records (`signals`, `parameters`, `prepared_bodies`) and retained contexts
(`type_bindings`, `type_widths`, `type_queries`, `port_bindings`). Repeated
signal growth for small repeated instances points to duplicated storage;
many distinct prepared bodies or large retained maps suggest other paths.
Counts are lengths of containers at the latest checkpoint, not heap bytes.

Every record is written immediately, without retaining a history or copying
the design. A kill may leave a partial final line; preceding complete lines
remain usable. Files append across invocations, and each trace starts with
`event: "start"` and sequence 1. An absent `finish` record alone does not
prove an OOM: early process exits also bypass cleanup. Keep the memory
watchdog enabled and check the exit status and kernel/container evidence.

RSS measures the whole process, including allocator retention, so growth
beside a source location identifies where to investigate rather than proving
that the named object owns those bytes. Tracing adds I/O and sampling costs;
disable it for performance measurements. `XEZIM_OOM_TRACE=1` writes JSONL
to stderr alongside normal diagnostics. An unwritable destination disables
tracing with a diagnostic and preserves the design's result.

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
