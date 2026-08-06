# Debugging xezim

Reproduce-first workflow, diagnostics, and the runtime knobs.

## 1. Reproduce before you edit

1. Write the **minimal** SV repro: `./target/release/xezim repro.sv` (use
   `--no-cache` — see §8).
2. Identify which phase fails (parse / elaborate / simulate, §5).
3. Fix the file that owns the symptom (§10).
4. Add a regression test (see `docs/dev/testing.md`) and run it:
   `cargo test --test <group> <name>`.

If the minimal repro behaves, the bug is elsewhere in the full design — bisect by
trimming the source or use `--dump-merged-sv` to see what the tool actually sees.

## 2. Diagnostic markers (what each stderr/stdout line means)

| Marker | Meaning |
|---|---|
| `: error:` | A user-facing diagnostic (compile/elaboration/simulation error) |
| `Parse errors` / `Simulation error` | Failure strings the subprocess test runners grep for |
| `[CACHE] hit` / `miss` / `invalid artifact` / `stored` | Design-cache outcome (`src/lib.rs`) — see §8 |
| `[PHASE] compilation / simulation / total` | Per-phase timing summary |
| `[PROF]` | Per-subsystem timing breakdown of the simulation loop |
| `[EVENT-EDGE]` | Event-driven edge dispatch: armed/skip-mode measurements |
| `[PROGRESS]` | Periodic progress line, only when `XEZIM_PROGRESS` is set |
| `[xezim][mem-watchdog]` | RSS exceeded 3/4 of system memory; process killed — see §7 |
| `[xezim][hang-report]` | On-demand hang report naming parked waiters — see §6 |

## 3. Exit codes and signals

| Code / signal | Meaning |
|---|---|
| `0` | Clean run (reached `$finish`) |
| `1` | Usage, compile, or simulation error |
| `SIGKILL` | Memory watchdog fired (RSS > 3/4 MemTotal) — not a crash bug |
| `SIGUSR1` | `kill -USR1 <pid>` on a live run prints the hang report (`install_hang_report_handler`, `simulator.rs:1318`) |

## 4. The XEZIM_* runtime knobs

| Knob | Effect |
|---|---|
| `XEZIM_EVENT_EDGE=1` | Skip gateable flop fires when inputs didn't change (perf) |
| `XEZIM_INIT_ZERO=1` | Coerce X initialization to 0 (also `XEZIM_INIT_ZERO_PATHS`) |
| `XEZIM_PROGRESS=N` | Emit `[PROGRESS]` every N seconds |
| `XEZIM_STUCK_CLOCK=off\|warn\|abort` | Dead-clock watchdog: `warn` default, `abort` for CI |
| `XEZIM_NO_MEM_WATCHDOG=1` | Disable the memory watchdog |
| `XEZIM_NO_PARALLEL=1` / `XEZIM_FORCE_PARALLEL=1` | Force the edge path sequential / parallel |
| `XEZIM_NO_CACHE=1` / `XEZIM_CACHE_DIR=<dir>` | Disable / relocate the design cache |
| `XEZIM_COMPILE_PHASES=1` | Trace compile-phase timing |

## 5. Isolating the failing phase (parse → elaborate → simulate)

Run the same file under each mode; the first one that reports `: error:` owns
the bug:

```bash
./target/release/xezim --parse repro.sv
./target/release/xezim --compile repro.sv
./target/release/xezim --simulate repro.sv
```

A parse error points at `xezim-core/xezim-parser`; an elaboration error at
`xezim-core/src/elaborate.rs`; a wrong simulated value at
`src/compiler/simulator.rs`.

## 6. Hangs and the stall report

A hung run has two diagnostics:

- **On demand**: `kill -USR1 <pid>` prints a `[xezim][hang-report]` line naming
  the parked event/condition waiters and the sim time they parked at
  (`simulator.rs:21013`).
- **Dead-clock churn**: `XEZIM_STUCK_CLOCK=abort` turns a frozen clock into a
  hard failure instead of infinite churn.

Bound any suspicious run with `--max-time <N>` so it cannot run away.

## 7. Memory watchdog

`main.rs` spawns a thread that **SIGKILLs the process** when RSS exceeds 3/4 of
system memory, printing `[xezim][mem-watchdog] … Set XEZIM_NO_MEM_WATCHDOG=1 to
disable.` This is expected protection against OOM on huge designs, not a crash
bug — only disable it for known-huge (e.g. C910-scale) runs.

## 8. Design-cache interference

A stale `[CACHE] hit` can mask a change — the cache key covers sources, defines,
top, and the executable's mtime/size. Pass `--no-cache` whenever you are
debugging a behavior change you expect to see immediately; a local rebuild
invalidates cached artifacts automatically.

## 9. Determinism checks

Identical inputs must produce byte-identical output with the same seed
(`+seed=<n>`, default `1`; `+seed=random` opts into entropy). Verify by running
the same command twice and diffing:

```bash
./scripts/dev/check-determinism.sh design.sv +seed=1
```

See `scripts/dev/check-determinism.sh` and `docs/dev/gotchas.md` §1–2.

## 10. Symptom → owner table

| Symptom | Fix here |
|---|---|
| Parse/preprocessor errors, AST shape | `xezim-core/xezim-parser` |
| Type/width/signedness, parameters, classes, port binding | `xezim-core/src/elaborate.rs` |
| Wrong simulated value, race/timing, event ordering | `src/compiler/simulator.rs` |
| Class/UVM runtime, `randomize`, DPI/VPI, dumping | `src/compiler/simulator.rs` |
| `$display` / formatting output | `xezim-core/src/stdout_sink.rs`, `simulator.rs` |
| Compile-time codegen / VM perf | `bytecode.rs`, `dispatch.rs`, `soa.rs`, `jit.rs` |

## 11. A worked example (intra-assignment delay)

Consider a symptom in `lhs = #d rhs` (§9.4.5). The parser discards
intra-assignment timing, so `src/intra_delay.rs` rewrites the source text into a
`$__xz_intra_delay(...)` marker **before** parsing. A delay bug therefore points
at that rewrite/execution interplay, not at the parser — if you "fix" the parser
you will fight the rewrite. This is the kind of non-obvious ownership the
gotchas doc catalogs (`docs/dev/gotchas.md` §8).
