# Developer Assistance Package for xezim — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a developer-assistance package for the `xezim` repo — a `docs/dev/` reference set (architecture, debugging, testing, gotchas) plus a `scripts/dev/` shell toolbox (quick-repro, check-determinism, new-test) — so anyone using Claude Code or OpenAI Codex can understand, develop, and fix the simulator without re-deriving its non-obvious rules.

**Architecture:** Three layers that reference each other. (1) `docs/dev/*.md` — the "understand/fix/develop" reference, written from facts already verified against the repo (the AGENTS.md/CLAUDE.md work this session captured them; every fact below has its source). (2) `scripts/dev/*.sh` — small bash tools that wrap common agent actions; they resolve the repo root relative to themselves so they work from any CWD, and each has a smoke test. (3) `AGENTS.md` — extended so its "Resources" and "Workflows" sections link to the new docs and scripts ("AGENTS stays general, internals stays deep").

**Tech Stack:** bash (matching the existing `scripts/` precedent: `Makefile`, `build_uvm_dpi.sh`, `install_xezim_on_mac.sh`, `run_cv32e40p_uvm.sh`), Markdown, `cargo test` + `target/release/xezim` for verification.

## Global Constraints

- **No changes to `src/`, `xezim-core/`, or any existing test file.** Docs and tooling only. The one exception is `tests/*.rs` group roots, appended to only by `scripts/dev/new-test.sh` when invoked.
- **Every doc fact must be verified** against the repo before being written — no invented flags, paths, or API names. Sources are given per fact below; re-run the commands to confirm.
- Scripts must work from any CWD (resolve repo root from `$(dirname "$0")`), must be `set -uo pipefail` (no `-e` — they report failures), and must pass `bash -n` syntax check.
- Scripts must not modify the repo (except `new-test.sh`, whose whole job is scaffolding a test file + one append to a group root).
- Docs are tool-agnostic: no mention of skills, plugins, or tool-specific plumbing.
- `.claude/` is gitignored — leave it alone. Agent configuration for Claude Code and an MCP server are **out of scope** for this plan (follow-ups).
- Everything is committed to the `xezim` repo (`git config user.name`/`user.email` already set this session). Commit messages use the repo's `prefix: subject` style and end with the `Co-Authored-By: Claude <noreply@anthropic.com>` trailer.

---

### Task 1: `docs/dev/architecture.md` — the "understand" doc

**Files:**
- Create: `docs/dev/architecture.md`
- Test: none (reference doc) — verified via Step 2 commands

**Interfaces:**
- Consumes: nothing.
- Produces: the file `docs/dev/architecture.md` that later tasks link to. Its final section links to `debugging.md`, `testing.md`, `gotchas.md` (created in Tasks 2–4), `docs/dev/README.md` (Task 5), and the top-level `AGENTS.md`.

- [ ] **Step 1: Write `docs/dev/architecture.md`** with this exact outline and content:

```
# xezim Architecture

Orientation for a developer (or agent) who has never seen this codebase.
Complements AGENTS.md — this is the deep-dive it points to.

## 1. The two-crate split
## 2. End-to-end pipeline (source → running simulation)
## 3. Compile modes and what each phase runs
## 4. Central types
## 5. The scheduling model (Active / NBA / Reactive)
## 6. Subsystem → file map
## 7. Where each major SystemVerilog feature executes
## 8. The design cache
## 9. Further reading
```

The doc **must** state these verified facts (source in parens — keep the fact, drop the source from prose):

1. `xezim` is a SystemVerilog **bytecode interpreter**; parsing/elaboration/4-state value model live in the sibling `xezim-core` crate at `../xezim-core` (path dependency, not a submodule) — verified in `Cargo.toml` (`xezim-core = { path = "../xezim-core" }`, `sv-parser = { path = "../xezim-core/xezim-parser" }`). Native AOT compilation is a separate `xezim-b` crate.
2. Pipeline stages and their owners: source → preprocessor (`xezim-core/xezim-parser/src/preprocessor/`) → parser/AST (`xezim-core/xezim-parser/src/parse/`, `ast/`) → elaboration (`xezim-core/src/elaborate.rs`, 19k LOC) → bytecode compile (`src/compiler/bytecode.rs`) → event-driven execution (`src/compiler/simulator.rs`, 85k LOC).
3. CLI modes `--parse` / `--compile` / `--simulate` (default) / `--preprocess` — verified in `src/main.rs` (Mode enum) and the usage text.
4. Central types: `Value`/`LogicBit` — 4-state 0/1/X/Z with width + signedness (`xezim-core/src/value.rs`); `ElaboratedModule` (`xezim-core/src/elaborate.rs`); `Insn` — register-based bytecode, 24-byte instructions (`src/compiler/bytecode.rs`); `Simulator` (`src/compiler/simulator.rs`).
5. Scheduling regions: Active (blocking assigns, continuous assigns, always_comb), NBA (non-blocking updates), Reactive (edge-triggered always_ff/always_latch, @posedge, wait forks) — quoted from the `simulator.rs` module doc (`//!` header).
6. Subsystem→file map (table): simulator VM `src/compiler/simulator.rs`; bytecode `bytecode.rs`; dispatch table `dispatch.rs`; arena `arena.rs`; soa signal layout `soa.rs`; optional cranelift JIT `jit.rs` (`--features jit`); FST sink `fst_sink.rs`; pre-parse intra-assignment-delay rewrite `src/intra_delay.rs`; additive negative-case lint `src/should_fail_lint.rs`; experimental PDES `src/multikernel.rs` (not production); VCD sink `xezim-core/src/vcd_sink.rs`; stdout/log sink `xezim-core/src/stdout_sink.rs`; SDF `xezim-core/src/sdf/`.
7. Feature→owner table (parse error → sv-parser; type/width/parameter/class/port → elaborate.rs; wrong sim value/race/event ordering/class-UVM runtime/randomize/DPI-VPI/dumping → simulator.rs; `$display` formatting → stdout_sink.rs).
8. Design cache: content-addressed, keyed on sources+defines+top+exe mtime/size, prints `[CACHE] hit/miss/stored` on stderr, skips parse+elab on hit; config in `src/lib.rs` (`DesignCacheConfig`), CLI `--cache-dir`/`--no-cache`.
9. A "trace it yourself" section: `--dump-files-list` shows resolved sources; `--dump-merged-sv out.sv` writes all sources merged; `--compile` stops after elaboration; `--simulate` runs.
10. Final section "Further reading" lists `docs/dev/debugging.md`, `docs/dev/testing.md`, `docs/dev/gotchas.md`, `../xezim-core/AGENTS.md`, `README.md`.

- [ ] **Step 2: Verify the doc references nothing that doesn't exist**

```bash
# every path the doc names must exist; every command must run
test -d docs/dev
test -f src/compiler/simulator.rs && test -f src/compiler/bytecode.rs
test -f src/intra_delay.rs && test -f src/should_fail_lint.rs
test -f ../xezim-core/src/elaborate.rs && test -f ../xezim-core/src/value.rs
printf 'module tb; initial $display("x"); endmodule\n' >/tmp/xz_arch_check.sv
./target/release/xezim --dump-files-list /tmp/xz_arch_check.sv >/dev/null 2>&1 && echo "trace commands OK"
rm -f /tmp/xz_arch_check.sv
```
Expected: all `test` lines succeed; "trace commands OK" printed. If `target/release/xezim` is stale, rebuild first (`cargo build --release`).

- [ ] **Step 3: Commit**

```bash
git add docs/dev/architecture.md
git commit -m "docs(dev): architecture deep-dive

Two-crate split, end-to-end pipeline, scheduling model, subsystem and
feature-to-owner maps, design cache.

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2: `docs/dev/debugging.md` — the "fix" doc

**Files:**
- Create: `docs/dev/debugging.md`
- Test: none (reference doc) — verified via Step 2 commands

**Interfaces:**
- Consumes: nothing.
- Produces: `docs/dev/debugging.md`, referenced from `docs/dev/README.md` (Task 5) and `architecture.md` (Task 1).

- [ ] **Step 1: Write `docs/dev/debugging.md`** with this outline:

```
# Debugging xezim

Reproduce-first workflow, diagnostics, and the runtime knobs.

## 1. Reproduce before you edit
## 2. Diagnostic markers (what each stderr/stdout line means)
## 3. Exit codes and signals
## 4. The XEZIM_* runtime knobs
## 5. Isolating the failing phase (parse → elaborate → simulate)
## 6. Hangs and the stall report
## 7. Memory watchdog
## 8. Design-cache interference
## 9. Determinism checks
## 10. Symptom → owner table
## 11. A worked example (walk one real past fix: intra-assignment delay)
```

The doc **must** state these verified facts:

1. Workflow: write the minimal SV repro → run `./target/release/xezim repro.sv` → identify phase → fix in the owner → add regression test (see `testing.md`) → `cargo test --test <group> <name>`.
2. Markers verified in code: `: error:` (diagnostics), `Parse errors` / `Simulation error` (strings the test runners grep for — `tests/misc/*_runner.rs`), `[CACHE] hit|miss|invalid artifact|stored` (`src/lib.rs`), `[PROF]` per-phase timing (`simulator.rs`), `[PHASE]`, `[EVENT-EDGE]` (armed/skip mode), `[PROGRESS]` (when `XEZIM_PROGRESS` set), `[xezim][mem-watchdog]` (kills pid, `src/main.rs:79`).
3. Exit codes: 0 = clean `$finish`; 1 = usage/compile/simulation error; SIGKILL from the memory watchdog (RSS > 3/4 of MemTotal — message points at `XEZIM_NO_MEM_WATCHDOG=1`); SIGUSR1 installs a hang-report handler (`install_hang_report_handler`, `simulator.rs:1318`).
4. XEZIM_* table — each verified in code: `XEZIM_EVENT_EDGE=1` (`simulator.rs:380` area), `XEZIM_INIT_ZERO=1`, `XEZIM_PROGRESS=N`, `XEZIM_STUCK_CLOCK=off|warn|abort` (`simulator.rs:22447`), `XEZIM_NO_MEM_WATCHDOG=1` (`src/main.rs:64`), `XEZIM_NO_PARALLEL=1` / `XEZIM_FORCE_PARALLEL=1` (`simulator.rs:29578`), `XEZIM_NO_CACHE=1`, `XEZIM_CACHE_DIR`, `XEZIM_COMPILE_PHASES=1`.
5. Phase isolation: run the same file under `--parse`, then `--compile`, then `--simulate`; whichever first reports `: error:` owns the bug.
6. Hangs: the terminal stall report names the parked processes and the signal that should have woken them (`simulator.rs:21420`); `XEZIM_STUCK_CLOCK=abort` turns dead-clock churn into a hard failure; bound runs with `--max-time`.
7. Memory watchdog: kill at 3/4 MemTotal is expected, not a crash; disable only for known-huge runs.
8. Cache: a stale `[CACHE] hit` can mask a change — pass `--no-cache` while iterating; a local rebuild invalidates automatically (exe mtime/size is part of the key).
9. Determinism: run the same command twice, `diff` the output — must be identical with the same seed (`+seed=1` default; `+seed=random` opts into entropy). See `scripts/dev/check-determinism.sh`.
10. Symptom→owner table (same as AGENTS.md §Architecture, expanded with one-line how-to for each).
11. Worked example: intra-assignment delay — `lhs = #d rhs` is rewritten to a `$__xz_intra_delay` marker by `src/intra_delay.rs` **before** parsing; a symptom in delays points at that interplay, not the parser.

- [ ] **Step 2: Verify every command the doc tells the reader to run actually works**

```bash
# phase isolation on a trivial design
printf 'module tb; initial $display("x"); endmodule\n' >/tmp/xz_dbg_check.sv
./target/release/xezim --parse /tmp/xz_dbg_check.sv >/dev/null 2>&1 && echo "parse OK"
./target/release/xezim --compile /tmp/xz_dbg_check.sv >/dev/null 2>&1 && echo "compile OK"
./target/release/xezim --simulate /tmp/xz_dbg_check.sv >/dev/null 2>&1 && echo "simulate OK"
# determinism on the doc's own example
a=$(./target/release/xezim /tmp/xz_dbg_check.sv 2>/dev/null); b=$(./target/release/xezim /tmp/xz_dbg_check.sv 2>/dev/null)
[ "$a" = "$b" ] && echo "determinism OK"
rm -f /tmp/xz_dbg_check.sv
```
Expected: all three phase lines and "determinism OK" print.

- [ ] **Step 3: Commit**

```bash
git add docs/dev/debugging.md
git commit -m "docs(dev): debugging handbook

Diagnostic markers, exit codes, XEZIM_* runtime knobs, phase isolation,
hang/stall debugging, memory watchdog, cache and determinism gotchas.

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 3: `docs/dev/testing.md` — the "develop" doc

**Files:**
- Create: `docs/dev/testing.md`
- Test: none (reference doc) — verified via Step 2 command

**Interfaces:**
- Consumes: nothing.
- Produces: `docs/dev/testing.md`, referenced from `docs/dev/README.md` (Task 5) and `architecture.md` (Task 1). Task 8's `new-test.sh` targets the exact conventions this doc defines.

- [ ] **Step 1: Write `docs/dev/testing.md`** with this outline:

```
# Testing xezim

How the suite is organized, how to run it, and how to add a test.

## 1. The two harness styles (in-process groups / subprocess runners)
## 2. Running tests (group, filter, both feature configs)
## 3. Adding an in-process test (manual + scripts/dev/new-test.sh)
## 4. Conventions for a good test
## 5. The subprocess runner suites and their stdout markers
## 6. Common pitfalls
```

The doc **must** state these verified facts:

1. Eight integration group roots in `tests/`: `classes.rs`, `collections.rs`, `gates.rs`, `hierarchy.rs`, `misc.rs`, `scheduling.rs`, `strings.rs`, `types.rs`; each is a flat list of `#[path = "<group>/<name>.rs"] mod <name>;` declarations (no `{}` wrapper). ~1680 `#[test]` fns total (`grep -rc '#\[test\]' tests/`).
2. In-process pattern (verbatim-shaped): `use xezim::simulate;` then `let sim = simulate(SV, max_time).expect("simulate failed");` and `assert_eq!(u(&sim, "sig"), N, "…")` where `u` is a local helper that tries `sim.get_signal(n)` then `sim.get_signal(&format!("tb.{n}"))` — see the real `fn u` in `tests/classes/issue4_coupled_constraints.rs:21` and the `.or_else(|| sim.get_signal(&format!("tb.{}", n)))` pattern in `tests/classes/class_property_param_width.rs:145-146`.
3. Subprocess runners live in `tests/misc/` (`prtest_runner.rs`, `ivtest_*_cluster.rs`, `lrm_audit*`, `issue30`, `issue_cases`, `sv_compliance_runner.rs`, `sv2023_compliance_runner.rs`); they spawn `env!("CARGO_BIN_EXE_xezim")`. Positive tests must print `TEST_PASS` (not `TEST_FAIL`) or `PASSED` (not `FAILED`); no output may contain `Parse errors` or `Simulation error`; negative tests must exit non-zero and print a diagnostic. The `ivtest` CE clusters use `reject(name, src)` / `accept(name, src)` helpers.
4. Run commands: `cargo test`; `cargo test --test classes`; `cargo test --test classes class_property` (name filter); `cargo test --features jit`; CI (`test.yml`) runs `cargo test --no-fail-fast` and `cargo test --features jit --no-fail-fast`.
5. Adding a test manually: drop `tests/<group>/<name>.rs` **and** add `#[path = "<group>/<name>.rs"] mod <name>;` to the group root. The doc must call out: forgetting the mod line compiles silently and the test never runs.
6. Conventions: `//!` doc comment cites the LRM section and the bug/fix (e.g. `tests/classes/class_property_param_width.rs` cites `§8.25 / §6.9.1`); assertion message names the expected value; use `simulate_multi` for multiple sources/includes/defines (see `tests/classes/uvm_config_db_tests.rs`).
7. Common pitfalls: missing mod line; wrong group; signal path (`tb.` prefix); 4-state X where a 2-state value is expected (use `XEZIM_INIT_ZERO=1` only if X-init is the point); default seed — a test that collects random packets must not assert an exact count without fixing `+seed`.

- [ ] **Step 2: Verify the run commands the doc documents**

```bash
cargo test --test classes class_property_param_width 2>&1 | tail -2
```
Expected: `test result: ok. 5 passed; 0 failed; …; 196 filtered out`. (This proves the group + name-filter commands.)

- [ ] **Step 3: Commit**

```bash
git add docs/dev/testing.md
git commit -m "docs(dev): test-authoring guide

Two harness styles, run commands, add-a-test flow, conventions and the
classic missing-mod-line pitfall.

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 4: `docs/dev/gotchas.md` — the invariant reference

**Files:**
- Create: `docs/dev/gotchas.md`
- Test: none — verified via Step 2 greps

**Interfaces:**
- Consumes: nothing.
- Produces: `docs/dev/gotchas.md`, referenced from `docs/dev/README.md` (Task 5).

- [ ] **Step 1: Write `docs/dev/gotchas.md`** with this outline:

```
# xezim Gotchas — the rules that are not obvious

Each rule: what to do, why, and the source line proving it.

## 1. Determinism is a hard requirement
## 2. The deterministic hasher
## 3. Only one global allocator (mimalloc)
## 4. 32 MiB minimum stack
## 5. Design cache
## 6. Memory watchdog
## 7. Dead-clock watchdog
## 8. Intra-assignment-delay pre-parse rewrite
## 9. should_fail lint is additive
## 10. Logging routing (log_println/log_eprintln)
## 11. Per-module timescales
## 12. xezim-core artifact versioning
```

Each entry must carry its verified source (command or file:line):
1. RNG defaults to seed 1 → `+seed=<n>` for another stream, `+seed=random` for entropy (usage text in `src/main.rs`); two identical runs must be byte-identical.
2. `xezim_core::hasher::{HashMap, HashSet}` use fixed ahash seeds (`../xezim-core/src/lib.rs` `DeterministicState`); never use `std::collections::HashMap/HashSet` where iteration order is observable.
3. mimalloc is installed by `xezim-core`'s default feature; exactly one `#[global_allocator]` per binary; opt-out via `default-features = false` (comment in `../xezim-core/Cargo.toml`).
4. `.cargo/config.toml` sets `RUST_MIN_STACK=33554432` for cargo-invoked processes — deep SV recursion overflows the 2 MiB default in debug; never lower.
5. Cache markers `[CACHE] hit|miss`; key includes exe mtime/size; `--no-cache` while iterating (`src/lib.rs`).
6. Watchdog SIGKILLs at RSS > 3/4 MemTotal (`src/main.rs:79`); `XEZIM_NO_MEM_WATCHDOG=1`.
7. `XEZIM_STUCK_CLOCK=off|warn|abort` (`simulator.rs:22447`).
8. Parser discards `lhs = #d rhs`; `src/intra_delay.rs` rewrites the text to a `$__xz_intra_delay` marker before parsing; keep the interplay when touching the parser.
9. `should_fail` is an additive second pass, validated to a 1005-case baseline (`src/should_fail_lint.rs:12`); never changes clean-design behavior.
10. Route output through `log_println`/`log_eprintln` (`../xezim-core/src/lib.rs:2171-2172`) so the `-l` fd-level redirect (which captures DPI/VPI C output) works.
11. Tick = finest declared precision (down to fs); `--max-time` in ns; `$time` reports ticks.
12. `XEZIM_BYTECODE_MAGIC = b"XEZIMBC\x0c"` — the last byte is the serialized-format version; bump it (and the ladder comment above it) when adding a serialized field in `ElaboratedModule` (`../xezim-core/src/lib.rs:73`).

- [ ] **Step 2: Verify each source reference exists**

```bash
grep -q 'RUST_MIN_STACK' .cargo/config.toml && echo "stack OK"
grep -q 'XEZIM_NO_MEM_WATCHDOG' src/main.rs && echo "watchdog OK"
grep -q '1005' src/should_fail_lint.rs && echo "lint OK"
grep -q 'XEZIMBC' ../xezim-core/src/lib.rs && echo "magic OK"
grep -q 'fn log_println' ../xezim-core/src/lib.rs && echo "logging OK"
```
Expected: all five "OK" lines print.

- [ ] **Step 3: Commit**

```bash
git add docs/dev/gotchas.md
git commit -m "docs(dev): gotchas reference

Determinism, hasher, allocator, stack, caches, watchdogs, the
intra-delay rewrite, the additive should_fail lint, logging, timescales,
and artifact versioning — each with its source.

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 5: `docs/dev/README.md` index + wire into AGENTS.md

**Files:**
- Create: `docs/dev/README.md`
- Modify: `AGENTS.md` (Resources section — add `docs/dev/README.md` line; Workflows "Debug a wrong sim result" line — mention `scripts/dev/quick-repro.sh`)

**Interfaces:**
- Consumes: `docs/dev/architecture.md` (Task 1), `debugging.md` (Task 2), `testing.md` (Task 3), `gotchas.md` (Task 4).
- Produces: the index doc that makes the package discoverable, and AGENTS.md links that let agents find it.

- [ ] **Step 1: Write `docs/dev/README.md`** — index with:

```
# Developer Documentation

How an agent or engineer uses these docs. One paragraph: which doc for which
situation (architecture = understand, debugging = fix, testing = develop,
gotchas = before you edit anything). A table:

| Doc | When to read it |
| architecture.md | First time in this repo; before adding a feature |
| debugging.md | A simulation gives a wrong result or hangs |
| testing.md | Before writing or running any test |
| gotchas.md | Before editing src/ or xezim-core |

Final section: how these relate to AGENTS.md (top-level, general) and
../xezim-core/AGENTS.md (the sibling library). Link all four docs by filename.
```

- [ ] **Step 2: Add the two AGENTS.md references** (exact edits):

In the `## Resources` section of `AGENTS.md`, insert as the first bullet:
```markdown
- `docs/dev/README.md` — developer documentation: architecture, debugging, testing, gotchas
```
In the `## Workflows` → "**Debug a wrong sim result**" sentence, after "…`--max-time` to bound a runaway;", insert:
```markdown
 (or use `scripts/dev/quick-repro.sh '…snippet…'` to iterate without writing a repo file)
```

- [ ] **Step 3: Verify links resolve and docs render**

```bash
for f in architecture debugging testing gotchas; do test -f "docs/dev/$f.md" && echo "$f OK"; done
grep -q 'docs/dev/README.md' AGENTS.md && echo "AGENTS link OK"
grep -q 'quick-repro' AGENTS.md && echo "workflow link OK"
```
Expected: four "OK" lines plus the two link checks.

- [ ] **Step 4: Commit**

```bash
git add docs/dev/README.md AGENTS.md
git commit -m "docs(dev): dev-docs index and wire it into AGENTS.md

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 6: `scripts/dev/quick-repro.sh`

**Files:**
- Create: `scripts/dev/quick-repro.sh` (executable)

**Interfaces:**
- Consumes: `target/release/xezim` (must be built).
- Produces: an executable script at `scripts/dev/quick-repro.sh`, referenced by `docs/dev/debugging.md` (Task 2) and `AGENTS.md` (Task 5). Signature: `scripts/dev/quick-repro.sh '<sv-snippet>' [xezim args...]` — stdout/stderr pass through from xezim; exit code = xezim's.

- [ ] **Step 1: Write the script**

```bash
#!/usr/bin/env bash
# Simulate a one-off SystemVerilog snippet without writing a repo file.
#
#   scripts/dev/quick-repro.sh 'module tb; initial $display("hi"); endmodule'
#   scripts/dev/quick-repro.sh 'module tb; ... endmodule' --max-time 50 +seed=2
#
# The snippet is written to a temp .sv file, simulated with the release
# binary, and the temp file is removed on exit. Exit code = xezim's.
set -uo pipefail

if [ "$#" -lt 1 ]; then
  echo "usage: $(basename "$0") '<sv-snippet>' [xezim args...]" >&2
  exit 2
fi
snippet="$1"
shift

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
bin="$repo/target/release/xezim"
if [ ! -x "$bin" ]; then
  echo "release binary missing — run: cargo build --release" >&2
  exit 2
fi

tmp="$(mktemp "${TMPDIR:-/tmp}/xz-repro-XXXXXX.sv")"
trap 'rm -f "$tmp"' EXIT
printf '%s\n' "$snippet" >"$tmp"
exec "$bin" "$tmp" "$@"
```

- [ ] **Step 2: Smoke-test it**

```bash
chmod +x scripts/dev/quick-repro.sh
bash -n scripts/dev/quick-repro.sh && echo "syntax OK"
out=$(scripts/dev/quick-repro.sh 'module tb; initial $display("GREET=%0d", 42); endmodule' 2>/dev/null | grep GREET)
[ "$out" = "GREET=42" ] && echo "happy path OK"
scripts/dev/quick-repro.sh >/dev/null 2>&1; [ "$?" -eq 2 ] && echo "usage error OK"
sleep 0.1   # let the EXIT trap run
test -z "$(ls /tmp/xz-repro-*.sv 2>/dev/null)" && echo "temp cleaned OK"
```
Expected: "syntax OK", "happy path OK", "usage error OK", "temp cleaned OK".

- [ ] **Step 3: Commit**

```bash
git add scripts/dev/quick-repro.sh
git commit -m "tools(dev): quick-repro script to simulate an SV snippet

Writes the snippet to a temp file, simulates with the release binary,
cleans up on exit. Works from any CWD.

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 7: `scripts/dev/check-determinism.sh`

**Files:**
- Create: `scripts/dev/check-determinism.sh` (executable)

**Interfaces:**
- Consumes: `target/release/xezim`.
- Produces: an executable script `scripts/dev/check-determinism.sh`, referenced by `docs/dev/debugging.md`. Signature: `scripts/dev/check-determinism.sh <sv-file> [xezim args...]` — runs the design twice with the same seed and exits 1 with a diff if the two runs differ; prints `DETERMINISM OK: byte-identical` otherwise.

- [ ] **Step 1: Write the script**

```bash
#!/usr/bin/env bash
# Verify a design is deterministic: run it twice with the same seed and fail
# if the outputs differ. Identical inputs must give byte-identical output.
#
#   scripts/dev/check-determinism.sh design.sv
#   scripts/dev/check-determinism.sh design.sv +seed=7 --max-time 200
#
# Pass +seed=<n> (default is seed 1). Use +seed=random only if you want to
# confirm your analysis ignores entropy — two random runs SHOULD differ.
set -uo pipefail

if [ "$#" -lt 1 ] || [ ! -f "$1" ]; then
  echo "usage: $(basename "$0") <design.sv> [xezim args...]" >&2
  exit 2
fi
design="$1"
shift

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
bin="$repo/target/release/xezim"
if [ ! -x "$bin" ]; then
  echo "release binary missing — run: cargo build --release" >&2
  exit 2
fi

r1="$(mktemp)"; r2="$(mktemp)"
trap 'rm -f "$r1" "$r2"' EXIT

"$bin" "$design" "$@" 2>&1 >"$r1"; s1=$?
"$bin" "$design" "$@" 2>&1 >"$r2"; s2=$?

if [ "$s1" -ne "$s2" ]; then
  echo "DETERMINISM FAILURE: exit codes differ ($s1 vs $s2)" >&2
  exit 1
fi
if ! diff -q "$r1" "$r2" >/dev/null; then
  echo "DETERMINISM FAILURE: two identical runs produced different output" >&2
  diff -u "$r1" "$r2" >&2
  exit 1
fi
echo "DETERMINISM OK: byte-identical (exit $s1)"
```

- [ ] **Step 2: Smoke-test it**

```bash
chmod +x scripts/dev/check-determinism.sh
bash -n scripts/dev/check-determinism.sh && echo "syntax OK"
printf 'module tb; initial $display("n=%0d", $urandom()); endmodule\n' >/tmp/xz_det_check.sv
./scripts/dev/check-determinism.sh /tmp/xz_det_check.sv | grep -q 'DETERMINISM OK' && echo "deterministic design OK"
./scripts/dev/check-determinism.sh >/dev/null 2>&1; [ "$?" -eq 2 ] && echo "usage error OK"
rm -f /tmp/xz_det_check.sv
```
Expected: "syntax OK", "deterministic design OK", "usage error OK". (The design uses `$urandom` to prove the check holds even when random values are involved — same seed ⇒ identical stream.)

- [ ] **Step 3: Commit**

```bash
git add scripts/dev/check-determinism.sh
git commit -m "tools(dev): determinism check script

Runs a design twice with the same seed and diffs the output; fails on
any drift. Works from any CWD.

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 8: `scripts/dev/new-test.sh`

**Files:**
- Create: `scripts/dev/new-test.sh` (executable)

**Interfaces:**
- Consumes: `docs/dev/testing.md` conventions (Task 3) — same group names, same `#[path]` + `mod` shape.
- Produces: an executable scaffold script. Signature: `scripts/dev/new-test.sh <group> <name>` — creates `tests/<group>/<name>.rs` from a template and **appends** the `#[path = "<group>/<name>.rs"]` + `mod <name>;` lines to `tests/<group>.rs`. Exits 2 on bad usage, 1 on a group/name that can't be used, 0 on success.

- [ ] **Step 1: Write the script**

```bash
#!/usr/bin/env bash
# Scaffold a new integration test in a group and wire it into the group root.
#
#   scripts/dev/new-test.sh scheduling regress_foo
#   cargo test --test scheduling regress_foo
#
# Creates tests/<group>/<name>.rs and appends the #[path] + mod lines to
# tests/<group>.rs. The classic failure is forgetting the mod line — this
# script makes it impossible.
set -uo pipefail

if [ "$#" -ne 2 ]; then
  echo "usage: $(basename "$0") <group> <snake_case_name>" >&2
  exit 2
fi
group="$1"
name="$2"

case "$group" in
  classes|collections|gates|hierarchy|misc|scheduling|strings|types) ;;
  *) echo "unknown group '$group' (expected one of: classes collections gates hierarchy misc scheduling strings types)" >&2; exit 1 ;;
esac
if ! printf '%s' "$name" | grep -qE '^[a-z][a-z0-9_]*$'; then
  echo "invalid test name '$name' (use snake_case, lowercase start)" >&2
  exit 1
fi

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
root="$repo/tests/$group.rs"
file="$repo/tests/$group/$name.rs"

if [ -e "$file" ]; then
  echo "already exists: $file" >&2
  exit 1
fi

cat >"$file" <<EOF
//! <LRM § + what this guards> — see docs/dev/testing.md for conventions.
use xezim::simulate;

#[test]
fn $name() {
    let sim = simulate(
        r#"
module tb;
  // minimal repro
endmodule
"#,
        100,
    )
    .expect("simulate failed");
    // assert_eq!(u(&sim, "tb.sig"), N, "expected value");
}
EOF

printf '#[path = "%s/%s.rs"]\nmod %s;\n' "$group" "$name" "$name" >>"$root"

echo "created $file"
echo "appended mod line to $root"
echo "run it with: cargo test --test $group $name"
```

- [ ] **Step 2: Smoke-test it**

```bash
chmod +x scripts/dev/new-test.sh
bash -n scripts/dev/new-test.sh && echo "syntax OK"
./scripts/dev/new-test.sh scheduling zz_probe_scaffold
grep -q 'zz_probe_scaffold' tests/scheduling/zz_probe_scaffold.rs && echo "file created OK"
tail -2 tests/scheduling.rs | grep -q 'zz_probe_scaffold' && echo "mod line appended OK"
cargo test --test scheduling zz_probe_scaffold 2>&1 | tail -1 | grep -q 'test result: ok' && echo "test compiles+passes OK"
./scripts/dev/new-test.sh bogus x 2>/dev/null; [ "$?" -eq 1 ] && echo "bad group OK"
./scripts/dev/new-test.sh scheduling NotSnake 2>/dev/null; [ "$?" -eq 1 ] && echo "bad name OK"
```
Expected: "syntax OK", "file created OK", "mod line appended OK", "test compiles+passes OK", "bad group OK", "bad name OK".

- [ ] **Step 3: Remove the smoke-test scaffold, then commit the script only**

The smoke test above proved the script works; the junk `zz_probe_scaffold` test must not land in the repo (a committed test needs a real LRM citation and a real assertion, per `docs/dev/testing.md`).

```bash
git restore tests/scheduling.rs            # discard the appended mod line
rm tests/scheduling/zz_probe_scaffold.rs   # remove the scaffold test file
git status --short                         # only scripts/dev/new-test.sh may remain untracked
git add scripts/dev/new-test.sh
git commit -m "tools(dev): new-test scaffold script

Creates a test file in a group and appends its #[path] mod line, so a
new test can never silently not run.

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 9: Final verification and README pointer

**Files:**
- Modify: `README.md` — add a "Developer documentation" link near the existing docs references (the README already links `docs/uvm-guide.md` and `docs/dpi-guide.md`; add `docs/dev/README.md` beside them).

**Interfaces:**
- Consumes: everything from Tasks 1–8.
- Produces: a verified, committed package.

- [ ] **Step 1: Add the README pointer**

Find the README line that links `docs/uvm-guide.md` and `docs/dpi-guide.md` and add after it:
```markdown
* [`docs/dev/README.md`](docs/dev/README.md) — developer documentation (architecture, debugging, testing, gotchas)
```

- [ ] **Step 2: End-to-end verification**

```bash
bash -n scripts/dev/*.sh && echo "all scripts: syntax OK"
# every doc file exists
for f in README architecture debugging testing gotchas; do test -f "docs/dev/$f.md" && echo "docs/dev/$f.md OK"; done
# every doc references only existing paths (spot-check the four files' code spans and file refs)
grep -l 'docs/dev' AGENTS.md README.md docs/dev/README.md >/dev/null && echo "package is linked"
# scripts are referenced where claimed
grep -q 'check-determinism' docs/dev/debugging.md && grep -q 'new-test' docs/dev/testing.md && echo "scripts referenced in docs"
# full-suite sanity on the touched area
cargo test --test scheduling 2>&1 | tail -2
```
Expected: all scripts pass `bash -n`; all five doc files exist; "package is linked"; "scripts referenced in docs"; `test result: ok` for the scheduling group.

- [ ] **Step 3: Commit**

```bash
git add README.md
git commit -m "docs: link the developer documentation from README

Co-Authored-By: Claude <noreply@anthropic.com>"
```

- [ ] **Step 4: Confirm clean tree**

```bash
git status --short   # only the files listed in these tasks may appear; nothing in src/ or xezim-core
```

---

## Out of scope / follow-ups (separate plans)

- **Agent configuration** (`.codex/config.toml` permissions; Claude Code `.claude/settings.json` — currently gitignored, needs an un-ignore decision). Intentionally deferred so no unverified config schema is committed.
- **MCP server for xezim** — a separate software project (compile/elaborate/simulate snippets, read signals, run tests via MCP tools).
- **xezim-core docs package** — a parallel `docs/dev/` set for `../xezim-core` if wanted.
