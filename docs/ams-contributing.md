# Adding a Verilog-AMS feature on `ams-support`

How to take one Verilog-AMS capability from nothing to a pushed, tested
commit — driving the work with **Claude Code** or **OpenAI Codex**, and the
exact sequence that has to pass **before** you commit and push.

Read [`ams-plan.md`](ams-plan.md) first for the staging and the design
decisions. This document is the mechanics.

Quickest loop while working on syntax — no simulator build needed:

```bash
cd xezim-core/xezim-parser
cargo run --bin sv-parse -- --ams --check   design.sv    # errors only
cargo run --bin sv-parse -- --ams --dump-ast design.sv   # the AST you built
# `.vams` / `.va` inputs enable AMS on their own; --ams is for `.sv`
```

Standard: **Accellera Verilog-AMS LRM 2.4.0**. Cite it as `AMS §x.y` in code
comments and test doc-comments — never as a bare `§`, which everywhere else in
this repo means IEEE 1800.

---

## 1. One-time setup

AMS work almost always spans **two repos**: `xezim-core` (lexer, parser, AST,
elaboration) and `xezim` (simulator, CLI, tests). They are separate git repos —
not submodules — and `xezim` normally consumes `xezim-core` as a git dependency
pinned to an exact revision. For AMS work you want the local checkout instead,
or every parser edit needs a push-and-repin round trip.

```bash
# Both repos on the branch.
git -C xezim       switch ams-support
git -C xezim-core  switch ams-support

# Point xezim's build at the local core (writes a git-ignored
# .cargo/config.toml). Do this ONCE; plain `cargo build` then uses it.
cd xezim && ./scripts/use-local-core.sh

# Confirm: a path in parentheses means the local checkout is live.
cargo tree -p xezim-core | head -3
```

Record a green baseline before you change anything, so a later failure is
attributable:

```bash
cargo build --release && cargo test --release --no-fail-fast
```

---

## 2. Where an AMS feature goes

The pipeline is `source → preprocessor → parse/AST → elaborate →
ElaboratedModule → bytecode → execute`. An AMS feature enters at the earliest
stage it needs and stops there. Most of Stage 1–3 never reached the simulator
at all.

| What you are adding | Files |
|---|---|
| A new reserved word | `xezim-core/xezim-parser/src/lexer/token.rs` — `ams_keyword()` |
| A declaration or statement form | `xezim-core/xezim-parser/src/ast/{decl,types,expr}.rs`, then `parse/{mod,items,types,statements}.rs` |
| Typing, width, port binding, driver resolution | `xezim-core/src/elaborate.rs` |
| Runtime behavior (scheduling, analog timepoints) | `xezim/src/compiler/simulator.rs` |
| A CLI flag | `xezim/src/main.rs` (arg match **and** `print_usage`) |
| Tests | `xezim/tests/ams/<name>.rs` + one line in `xezim/tests/ams.rs` |
| CLI flag (parser only) | `xezim-core/xezim-parser/src/main.rs` |
| User-facing docs | `xezim/README.md` "Verilog-AMS" section |

### The two rules that are not negotiable

**1. Every AMS keyword is gated.** AMS reserves words that are legal
SystemVerilog identifiers. A new keyword goes in `ams_keyword()`, never in
`keyword()`, and the scanner consults it only under `sv_parser::is_ams()`.
A feature that makes `analog` or `nature` reserved for every run is a
regression, however well it works. Every keyword-adding change ships with a
`gate_is_off_by_default` test showing the words still lex as identifiers.

**2. Reuse the existing mechanism before building a parallel one.** `wreal`
driver resolution is not a second resolution engine — it lowers onto the
§6.6.7 user-defined-nettype path, which already unions nets joined across
module ports so a node resolves exactly once. (Resolving per port net and
again on the results is invisible for a sum and wrong for a min/max.) Look for
the machinery that already does the hard part.

---

## 3. Driving it with Claude Code or Codex

Both agents work well here **because the repo tells them what to do**:
`xezim-core/AGENTS.md` and `CLAUDE.md` are read automatically at session start,
and Codex reads `AGENTS.md` too. You do not need to re-explain the invariants.
What you do need to supply is the part the agent cannot infer.

**Claude Code**

```bash
cd xezim && claude
```

Use plan mode (per `CLAUDE.md`) for anything touching more than one file — an
AMS feature always does. A good opening prompt names the LRM section, the
stage, and the acceptance test:

> Add AMS §5 `analog` blocks to the ams-support branch, parse-only for now:
> `analog begin … end` as a module item. Gate the keyword behind `is_ams()`.
> Add tests to the `ams` group asserting the AST shape, plus the
> gate-off-by-default case. Do not touch the simulator.

**Codex**

```bash
cd xezim && codex
```

Same prompt shape. Codex does not have this repo's plan mode, so state the file
boundary explicitly ("only `xezim-parser/` and `tests/ams/`") — it is more
likely to widen scope on its own.

**What to give either agent**

* The **LRM section number**. Both will otherwise reconstruct AMS syntax from
  memory, and Verilog-AMS is exactly the kind of standard where that produces
  confident, wrong details (attribute spellings and resolution-function names
  especially). If you are unsure of a spelling yourself, say so and ask for it
  to be flagged in a comment rather than guessed silently.
* The **stage boundary** — "parse-only", "no solver", "do not touch
  `simulator.rs`". AMS features have a natural pull toward the analog kernel;
  without a boundary you get a half-built solver.
* The **acceptance test**, in words. "Two instances driving one `wrealsum`
  node through ports must read 5.0, not 2.5" is worth more than a paragraph of
  design description.

**What to check in the agent's output, every time**

* Did it add the keyword to `ams_keyword()` — not `keyword()`?
* Is there a gate-off test?
* Did it change any serialized type? Then the artifact magic must be bumped
  (§4 below). Agents miss this: nothing fails at build time, and stale `.xez`
  artifacts deserialize as garbage later.
* Are the LRM citations in the comments real, or plausible-looking? Spot-check
  one against the standard.
* Did it delete or `#[ignore]` an existing test to get green? Retiring a test
  requires a named successor in the same change.

---

## 4. Before you commit

Run this in order. Every step, not the ones you think are affected — the lexer
and elaborator are shared by the whole suite, and an AMS change reaches designs
that contain no AMS at all.

```bash
# 1. Core's own unit tests (if you touched xezim-core).
cd xezim-core && cargo test

# 2. The AMS group, focused — fastest signal on your own work.
cd ../xezim && cargo test --release --test ams

# 3. The gate: the FULL suite must be green with AMS off, which is how
#    every non-AMS test runs. This is the step that catches an
#    over-reserved keyword.
cargo test --release --no-fail-fast

# 4. Exactly what CI runs — both configurations.
cargo test --no-fail-fast
cargo test --features jit --no-fail-fast
```

**One known pre-existing failure**, unrelated to AMS: `sv-parser`'s
`guarded_reserved_macro_fallbacks_are_skipped_in_strict_mode` expects
`` `__LINE__ `` to expand to 7 and gets 11 — a line-accounting bug in the
preprocessor's skipped-`ifndef` handling. It fails on `main` too. Confirm any
failure you see is this one (`git stash` your work and re-run) before assuming
you caused it.

Then the checklist:

- [ ] **Regression test added** in `tests/ams/`, with an `AMS §` citation in
      its doc comment, and one `#[path]` line in `tests/ams.rs`.
- [ ] **Gate test present** if the change added a keyword.
- [ ] **Artifact version bumped** if any serialized type changed — a new AST
      node, a new enum variant, a new `ElaboratedModule` field. Bump the last
      byte of `XEZIM_BYTECODE_MAGIC` in `xezim-core/src/lib.rs` and add one
      line to the version-ladder comment above it. Once per change, not once
      per commit.
- [ ] **Determinism preserved** — `crate::hasher::{HashMap, HashSet}`, never
      `std`, anywhere iteration order can reach output. For anything numeric,
      accumulation order is observable in a waveform too.
- [ ] **README updated** if the change added a flag or moved something from
      "does not work yet" to "works today".
- [ ] **`docs/ams-plan.md` updated** — tick the stage, or record what you
      learned that changes the plan.
- [ ] **Minimal diff** — no unrelated refactors, no dead code.

---

## 5. Pushing across the two repos

Order matters. `xezim` pins `xezim-core` by revision hash, so core has to be
pushed before xezim can name it.

```bash
# 1. Push core first.
cd xezim-core
git add -A && git commit -m "Parse AMS analog blocks"
git push origin ams-support

# 2. Take the hash it landed at.
git rev-parse origin/ams-support

# 3. In xezim, point BOTH rev fields in Cargo.toml at that hash.
#    (There are two: xezim-core and sv-parser.)
cd ../xezim
$EDITOR Cargo.toml

# 4. Verify the pin builds WITHOUT your local checkout — this is the
#    bare-clone path CI uses, and the one a mismatched pin breaks.
./scripts/use-local-core.sh --remove
cargo build --release && cargo test --release --test ams
./scripts/use-local-core.sh          # back to local co-development

# 5. Commit the pin bump together with the xezim-side change.
git add -A && git commit -m "Compile AMS analog blocks"
git push origin ams-support
```

Step 4 is the one people skip. The local `[patch]` hides a wrong pin
completely: everything builds and passes for you, and CI fails on a bare clone
with an error that points at the dependency rather than at the omission.

If you pushed core and only then realised xezim needs another core change,
push core again and re-bump — do not leave `xezim` pinned to a revision that
was never tested with it.

---

## 6. Picking up the next piece

`docs/ams-plan.md` §4 has the remaining stages in dependency order, and §6 lists
the open questions that must be answered before the analog kernel can start.
Answering one of those and writing it down is a contribution in itself — the
cost of an unanswered question here is a stage built on a guess.
