# xezim Gotchas — the rules that are not obvious

Each rule: what to do, why, and the source line proving it.

## 1. Determinism is a hard requirement

The RNG defaults to **seed 1**; `+seed=<n>` selects a different stream and
`+seed=random` opts into entropy (usage text, `src/main.rs`). Two runs with
identical inputs and seed must be **byte-identical**. Never seed from entropy by
default. After any RNG/solver change, re-verify with the same command twice and
`diff` the output.

## 2. The deterministic hasher

Use `xezim_core::hasher::{HashMap, HashSet}` — they hold fixed ahash seeds
(`DeterministicState`, `../xezim-core/src/lib.rs:35`). **Never** use
`std::collections::HashMap/HashSet` where iteration order can affect observable
behavior; unordered traversal would break determinism run-to-run.

## 3. Only one global allocator (mimalloc)

mimalloc is installed by `xezim-core`'s default feature
(`../xezim-core/Cargo.toml:28`). Rust allows exactly one `#[global_allocator]`
per binary — **never add another**. A downstream crate opts out with
`xezim-core = { path = "../xezim-core", default-features = false }`.

## 4. 32 MiB minimum stack

`.cargo/config.toml:8` sets `RUST_MIN_STACK = "33554432"` for cargo-invoked
processes. Deeply recursive SV expressions (concat chains, nested member access,
recursive functions, parameterized class construction) overflow the Rust default
2 MiB stack in debug builds. **Never lower this.**

## 5. Design cache

`[CACHE] hit|miss` on stderr (`src/lib.rs:183,225`); the key includes the
executable's mtime/size, so a local rebuild invalidates cached artifacts
automatically. While iterating on a behavior change, pass `--no-cache` — a
stale `[CACHE] hit` can mask your edit.

## 6. Memory watchdog

`main.rs:79` SIGKILLs the process when RSS exceeds 3/4 of system memory,
printing `[xezim][mem-watchdog] …`. Expected protection, not a crash bug. Only
disable for known-huge runs: `XEZIM_NO_MEM_WATCHDOG=1`.

## 7. Dead-clock watchdog

`XEZIM_STUCK_CLOCK=off|warn|abort` (`simulator.rs:22447`) turns a frozen-clock
churn into a hard failure. Default `warn`; use `abort` for CI.

## 8. Intra-assignment-delay pre-parse rewrite

The parser discards `lhs = #d rhs` (§9.4.5), so `src/intra_delay.rs` rewrites the
source text into a `$__xz_intra_delay(...)` marker **before** parsing. Any
parser/lexer work must preserve this interplay — a delay symptom points at the
rewrite, not the parser.

## 9. should_fail lint is additive

`src/should_fail_lint.rs` is an additive second pass that rejects illegal
constructs in `--compile` mode for sv-tests negative cases. It is validated
against a 1005-case static baseline (`src/should_fail_lint.rs:12`). It must
never change clean-design behavior and must stay conservative — add checks there
only for definite LRM violations.

## 10. Logging routing

Route output through `log_println` / `log_eprintln`
(`../xezim-core/src/lib.rs:2171-2172`) so the `-l` fd-level redirect works —
it captures DPI/VPI C output too. Bare `println!` / `eprintln!` bypasses `--log`.

## 11. Per-module timescales

The simulation tick is the finest declared precision anywhere (down to `fs`);
`--max-time` is in ns (independent of tick); `$time` reports ticks.
`--module-timescale` applies only to modules with no source-level timescale.

## 12. xezim-core artifact versioning

`XEZIM_BYTECODE_MAGIC = b"XEZIMBC\x0c"` — the last byte is the serialized-format
version (`../xezim-core/src/lib.rs:73`). When you add or change a serialized
field in `ElaboratedModule` (or any bincode-serialized type), **bump `\x0c` to
`\x0d`** and add one line to the version-ladder comment above it. Stale
artifacts then fail with a clear "recompile with current xezim" error instead of
deserializing garbage.
