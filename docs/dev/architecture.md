# xezim Architecture

Orientation for a developer (or agent) who has never seen this codebase.
Complements AGENTS.md — this is the deep-dive it points to.

## 1. The two-crate split

`xezim` is a SystemVerilog **bytecode interpreter** written in Rust. It does not
own its front end: parsing, elaboration, and the 4-state value model live in the
sibling `xezim-core` crate at `../xezim-core` (a path dependency, not a
submodule — clone both repos into the same parent). Within `xezim-core`, the
parser is the `sv-parser` subcrate at `../xezim-core/xezim-parser`.

Native ahead-of-time compilation is a **separate** crate (`xezim-b`) and is out
of scope here — this repo simulates elaborated bytecode, it does not compile.

## 2. End-to-end pipeline (source → running simulation)

| Stage | Owner |
|---|---|
| Source text | your `.sv`/`.v` files + `-I`/`-D`/`-f` handling |
| Preprocessor | `xezim-core/xezim-parser/src/preprocessor/` |
| Parse → AST | `xezim-core/xezim-parser/src/parse/`, `ast/` |
| Elaboration (types, widths, parameters, classes, ports) | `xezim-core/src/elaborate.rs` (the largest file in `xezim-core`) |
| Bytecode compile | `src/compiler/bytecode.rs` |
| Event-driven execution | `src/compiler/simulator.rs` (the largest file in this repo) |

## 3. Compile modes and what each phase runs

| Mode | Runs |
|---|---|
| `--preprocess` | Preprocessor only; emits expanded text |
| `--parse` | Lex + parse only |
| `--compile` | Parse + elaborate; reports diagnostics, no simulation |
| `--simulate` (default) | Parse + elaborate + simulate |

## 4. Central types

| Type | What it is | Where |
|---|---|---|
| `Value` / `LogicBit` | 4-state 0/1/X/Z with width + signedness | `xezim-core/src/value.rs` |
| `ElaboratedModule` | The elaborated design (signals, instances, nets) | `xezim-core/src/elaborate.rs` |
| `Insn` | Register-based bytecode instruction (24-byte enum) | `src/compiler/bytecode.rs` |
| `Simulator` | The event-driven VM | `src/compiler/simulator.rs` |

## 5. The scheduling model (Active / NBA / Reactive)

The `simulator.rs` module doc defines three regions:

- **Active** — blocking assigns, continuous assigns, `always_comb`
- **NBA** — non-blocking assign updates
- **Reactive** — edge-triggered `always_ff` / `always_latch` blocks (and
  `@posedge` waits)

This is why a design mixes `=` and `<=` the way it does: blocking work happens
in the Active region, scheduled NBA updates land in the NBA region, and
edge-sensitive processes re-trigger in the Reactive region.

## 6. Subsystem → file map

| Subsystem | File |
|---|---|
| Event-driven VM | `src/compiler/simulator.rs` |
| Bytecode IR + compiler | `src/compiler/bytecode.rs` |
| Direct-threaded dispatch table (POC) | `src/compiler/dispatch.rs` |
| Per-tick bump allocator | `src/compiler/arena.rs` |
| Structure-of-Arrays signal layout | `src/compiler/soa.rs` |
| Optional cranelift JIT (`--features jit`) | `src/compiler/jit.rs` |
| FST waveform sink | `src/compiler/fst_sink.rs` |
| Pre-parse intra-assignment-delay rewrite | `src/intra_delay.rs` |
| Additive negative-case lint | `src/should_fail_lint.rs` |
| Experimental PDES skeleton (not production) | `src/multikernel.rs` |
| VCD waveform sink | `xezim-core/src/vcd_sink.rs` |
| Locked stdout/stderr sink (feeds `-l`) | `xezim-core/src/stdout_sink.rs` |
| SDF annotation | `xezim-core/src/sdf/` |

## 7. Where each major SystemVerilog feature executes

| Symptom / feature | Owner |
|---|---|
| Parse/preprocessor errors, AST shape | `xezim-core/xezim-parser` |
| Type/width/signedness, parameters, classes, port binding, legality | `xezim-core/src/elaborate.rs` |
| Wrong simulated value, race/timing, event ordering, class/UVM runtime, `randomize`, DPI/VPI, dumping | `src/compiler/simulator.rs` |
| Compile-time codegen / VM perf | `bytecode.rs`, `dispatch.rs`, `soa.rs`, `jit.rs` |
| `$display` / formatting output | `xezim-core/src/stdout_sink.rs`, `simulator.rs` |

## 8. The design cache

`--simulate` stores a content-addressed elaborated design and reuses it on
identical runs. The key covers sources, defines, top, and the executable's own
mtime/size. On stderr it prints `[CACHE] hit`, `miss`, `invalid artifact`, or
`stored`. A cache **hit** skips parse + elaboration entirely, so when you are
debugging a behavior change you expect to see immediately, pass `--no-cache`.
Configured in `src/lib.rs` (`DesignCacheConfig`); CLI `--cache-dir` / `--no-cache`.

## 9. Trace it yourself

Run these against any source to watch the pipeline:

```bash
./target/release/xezim --dump-files-list my.sv        # resolved source list after -f expansion
./target/release/xezim --dump-merged-sv out.sv my.sv  # all sources merged into one self-contained .sv
./target/release/xezim --compile my.sv                # stop after elaboration
./target/release/xezim my.sv                          # run (simulate)
```

## Further reading

- `docs/dev/debugging.md` — reproduce-first workflow, diagnostic markers, the `XEZIM_*` knobs
- `docs/dev/testing.md` — test harnesses, how to run and add a test
- `docs/dev/gotchas.md` — the non-obvious rules (determinism, cache, watchdogs, …)
- `../xezim-core/AGENTS.md` — the shared library's development guide
- `README.md` — features, CLI reference, compliance
