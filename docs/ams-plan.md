# Verilog-AMS support — design and phasing plan

Status: **Stages 1-3 implemented and green.** Branch: `ams-support` (both
`xezim` and `xezim-core`). See [`ams-contributing.md`](ams-contributing.md)
for how to add the next piece.

Reference standard: **Accellera Verilog-AMS LRM 2.4.0** (May 2014). This is a
different document from IEEE 1800; xezim's existing `§x.y` citations refer to
IEEE 1800-2017/2023. To keep them apart, cite Verilog-AMS as **`AMS §x.y`** in
code comments and test doc-comments.

---

## 1. What "Verilog-AMS support" can mean

Verilog-AMS is three fairly separable languages stacked on Verilog. They differ
by orders of magnitude in cost, and only the last one needs a circuit solver.
Naming them explicitly is the point of this document — "AMS support" without a
tier is not a scopable task.

| Tier | Content | Needs a matrix solver? | Rough size |
|---|---|---|---|
| **T1 — Real-number modeling** | `wreal` nets, real ports, driver resolution (`wrealsum`/`wrealavg`/`wrealmin`/`wrealmax`/`wreal4state`), `` `default_discipline `` as a no-op | No — pure event-driven | Small |
| **T2 — Analog behavioral** | `analog` blocks, analog events (`initial_step`, `final_step`, `cross`, `above`, `timer`), `$abstime`, filters on explicit signals (`transition`, `slew`, `ddt`, `idt`), `$bound_step` | No — explicit evaluation on a scheduled analog timepoint | Medium |
| **T3 — Analog kernel** | natures/disciplines, branches, `V(a,b) <+ …` / `I(a,b) <+ …`, KCL/MNA assembly, Newton-Raphson, LTE timestep control, `abstol`/`reltol` convergence, `laplace_*`/`zi_*`, noise | **Yes** — this is a SPICE engine | Large |
| **T4 — Mixed-signal boundary** | discipline resolution, `connectmodule`/`connectrules`, automatic A2D/D2A insertion | Depends on T3 | Medium, T3-gated |

**Recommendation: land T1 first, then T2.** In digital-centric SoC verification
— which is what xezim's existing workloads (UVM, riscv-dv, C910) look like —
the overwhelming majority of "AMS" content in the DUT is real-number modeling,
not a solved analog netlist. T1 is the highest value-per-line by a wide margin,
it reuses machinery xezim already has (see §2), and it is independently
shippable. T3 is a genuine multi-quarter project and should not be started
before T1/T2 are green.

Non-goals for the first pass, to be stated in the README when T1 lands: SPICE
netlist import, `.ac`/`.noise` analyses, and Verilog-A compact device models.

---

## 2. What xezim already has that we can build on

This is the reason T1 is cheap. None of it needs to be invented.

* **Real values through the whole stack.** `Value` carries `is_real` with an
  f64 bit pattern inline (`xezim-core/src/value.rs:384`, `from_f64` at `:703`,
  `to_f64` at `:739`). `Signal` already has an `is_real` flag
  (`xezim-core/src/elaborate.rs:189`).
* **Real waveform dumping.** VCD emits `r<decimal>` records
  (`xezim-core/src/vcd_sink.rs:521`) and XTrace formats reals
  (`:248`). FST needs checking (§6, open item).
* **User-defined nettypes with resolution functions (IEEE §6.6.7).** Landed
  recently, and it is structurally *the same feature* as `wreal` resolution:
  N drivers on one net folded through a user function.
  `elab.nettype_resolvers` (`elaborate.rs:1441`), folded by
  `resolve_user_nettype_drivers` (`elaborate.rs:22758`), with the compiled
  dispatch path in `xezim/src/compiler/bytecode.rs` (commit `2f4b8f7`).
  **`wreal` resolution should be implemented as a set of built-in nettype
  resolvers, not as a parallel mechanism.**
* **Implicit-net coercion to a nettype across a port (§6.6.8).**
  `elaborate.rs:9791` — already written with analog nodes as the motivating
  case.
* **A language-edition switch precedent.** `--sv2017` (`xezim/src/main.rs:1737`)
  shows how a whole-run dialect flag is threaded.
* **A keyword-reservation gate precedent.** `is_sv_only_keyword`
  (`xezim-core/xezim-parser/src/lexer/scanner.rs:564`) already implements
  "these words are only keywords in some dialects" for `` `begin_keywords ``.

---

## 3. Cross-cutting design decisions

### 3.1 AMS keywords must be dialect-gated — do not reserve them globally

Verilog-AMS adds ~60 reserved words, and several are plausible identifiers in
existing SystemVerilog designs (`analog`, `nature`, `discipline`, `branch`,
`flow`, `potential`, `access`, `units`, `ground`, `from`, `exclude`, `timer`,
`above`, `ddt`, `idt`, `transition`, `slew`, `abstol`, `analysis`). Reserving
them unconditionally **will break currently-passing designs and tests**. Note
that `cross` is already an IEEE 1800 keyword (covergroup cross) and must keep
that meaning outside an analog event context.

Proposal:

* New run-level dialect flag `--ams` (mirroring `--sv2017`'s threading), plus
  automatic enablement for files with the `.vams` / `.va` extension.
* The lexer gains `is_ams_only_keyword(s)` next to `is_sv_only_keyword`, and
  the AMS keywords lex as `Identifier` unless AMS mode is on for that file.
* `+libext+` defaults (`xezim/src/main.rs:31`) gain `vams`/`va` **only** when
  `--ams` is passed, so a non-AMS run's library search is unchanged.

**Acceptance gate for the keyword commit:** the full existing suite must be
green with `--ams` *off*, and a targeted subset re-run with `--ams` *on* to
show the gate is real.

### 3.2 Artifact format version

`ElaboratedModule` gains serialized fields in every phase. Per
`xezim-core/AGENTS.md`, bump the last byte of `XEZIM_BYTECODE_MAGIC`
(`xezim-core/src/lib.rs:89`, currently `\x18`) and add a line to the version
ladder above it — **once per phase**, not once per commit.

> Note: `xezim-core/AGENTS.md` currently documents the magic as `\x0c`; the
> code says `\x18`. Correct the guide in the first phase that touches this.

### 3.3 Time model: the analog kernel does not share the digital tick

The digital engine counts integer ticks of `tick_s` (`elaborate.rs:1382`) and
picks the next timepoint as a `min()` over event queue / clock generators /
delayed updates / VPI callbacks (`xezim/src/compiler/simulator.rs:32277-32302`).
Analog time is continuous and its step is chosen by the solver, not by the
design.

The integration point is that `min()`: analog contributes a
`next_analog_time` candidate, and the analog kernel is allowed to *shrink*
(never grow) the accepted step — that is exactly what `$bound_step` and a
detected `cross()` do. Analog timepoints must snap to the tick grid at the
digital boundary; sub-tick analog steps stay internal to the kernel. Choosing
`tick_s` fine enough (fs precision is already supported) is what makes this
tolerable.

**This is the single highest-risk design item in T2/T3** and deserves a
written note in `docs/DEBUGGING.md` once implemented.

### 3.4 Determinism

`xezim-core`'s hard determinism rule applies unchanged: no `std` `HashMap`
iteration where order is observable. Additionally, for T3, floating-point
summation order in matrix assembly is observable in the output waveform — node
ordering must be deterministic, and the solver must not use unordered
iteration to accumulate stamps.

---

## 4. Phase plan

Each phase is independently shippable and ends green on
`cargo test --no-fail-fast` **and** `cargo test --features jit --no-fail-fast`.

### P0 — Preparation (no language change) — **done**

- [x] `ams-support` branches exist in both repos, at parity with `main`.
- [x] Local co-development enabled (`./scripts/use-local-core.sh`), so
      cross-repo edits build without a rev bump on every iteration.
- [x] Green baseline recorded on the branch.
- [x] `ams` test group created (`tests/ams.rs` + `tests/ams/`), following the
      one-binary-per-group convention at the top of `tests/types.rs`. It runs
      as part of plain `cargo test` — CI enumerates no group list.
- [x] `ams_mode.rs` helper: the AMS gate is one process-wide atomic and
      `cargo test` runs a group's cases on several threads, so gate-on and
      gate-off tests serialize on a mutex.
- [ ] Obtain the Accellera Verilog-AMS 2.4.0 LRM PDF and drop it beside the
      existing PDFs in `docs/` (or record the citation policy if it should not
      be committed). **Still open** — see open item 1.

### S1 — real / RNM / UDN foundation — **done**

Audited rather than built: `real` variables, `wire real` nets, reals across
module ports, `real` math/conversion system functions, and §6.6.7 user-defined
nettypes with resolution functions all already worked. The stage's real content
was closing the gaps and pinning the substrate so a later stage cannot regress
it silently.

* Pinned by `tests/ams/rnm_real_foundation.rs` (5 tests).
* **Fixed a pre-existing bug**: FST declared every var
  `FstSignalType::bit_vec` / `FstVarType::Wire`, so a `real` dumped its raw
  IEEE-754 bit pattern and read as an integer in GTKWave — `real r = 1.25`
  decoded as `0x3FF4000000000000`. VCD and XTrace were already correct. Reals
  now declare FST's native real slot and write the f64's 8 raw bytes. The
  repo's own `#[ignore]`d gate test for this (`F3`) is un-ignored and passes.
  Note the encoding is keyed off the DECLARATION, not `Value::is_real`: an
  unassigned `real` reads X, whose `Value` is not flagged real, and an 8-byte
  slot handed a 64-byte bit string panics the writer.
  (`fst_sink.rs`, `simulator.rs` FST header/change paths.)

Not a regression risk for non-AMS designs: no keyword changed, and the FST fix
only alters vars that were already classified `real` for VCD.

### S2 — `wreal` real nets (AMS §3.8) — **done**

Lowered onto the §6.6.7 resolver path rather than a parallel mechanism: a
`wreal` net is tagged with a synthetic nettype (`$wrealsum`, …) whose resolver
is a reserved marker, and `resolve_user_nettype_drivers` expands the marker
into an expression. That reuses the union-find that joins nets across module
ports, so a node driven from several instances is resolved exactly once —
invisible for a sum, wrong for a min/max.

* `wreal`, `wrealsum`, `wrealavg`, `wrealmin`, `wrealmax`, gated by `--ams`.
* Net declarations **and** both ANSI (`output wreal o`) and non-ANSI
  (`output wreal o;`) ports — the port paths needed the implicit `real` data
  type separately, without which the port stayed 1-bit and every real
  truncated to its LSB.
* Built-in resolutions expand to expressions (`a + b + c`, `(a+b+c)/3.0`, a
  fold of `(a<b)?a:b`), so no synthesized SystemVerilog prelude has to exist
  and the existing constant folding and bytecode compilation apply unchanged.
* A plain `wreal` with >1 driver is an error naming the resolved forms.
* Pinned by `tests/ams/wreal_nets.rs` (7 tests, including gate-off).

### S3 — natures and disciplines (AMS §3.4, §3.5) — **done, parse-only**

`nature … endnature` (open attribute set, `access()` accessor, derived
`nature X : Y`) and `discipline … enddiscipline` (`potential`, `flow`,
`domain continuous|discrete`) as top-level `Description` variants.

Attribute names (`units`, `access`, `abstol`, `ddt_nature`, …) are
deliberately **not** reserved — they parse as `<identifier> = <expr>;` inside
the body, so the standard's open set is supported without reserving five more
words. Bodies tolerate unknown items by skipping to the next `;`.

Parse-only by design: no elaboration, no discipline resolution on nets, no
simulation semantics. It exists so the analog stages have the type system to
resolve `access` functions and tolerances against.

* Pinned by `tests/ams/discipline_nature.rs` (6 tests, asserting on the AST —
  "it parsed" would pass just as well if the declaration were skipped and
  discarded, which is the failure this stage rules out).

### P1 — Real-number modeling — superseded

Folded into S1 and S2 above, which is what was actually built. Kept as a
heading only so the stage numbering in older notes still resolves.

### P2 — Analog behavioral (`analog` blocks, no solver)

Parser:
* `ModuleItem` (`src/ast/decl.rs:66`) gains `AnalogConstruct`.
* Dispatch in `parse_module_item` (`src/parse/items.rs:363`) — a
  `TokenKind::KwAnalog` arm alongside the existing `KwProgram` / `KwDefparam`
  arms.
* Analog event expressions in `@(…)`: `initial_step`, `final_step`,
  `cross(expr, dir, tol, etol)`, `above(expr)`, `timer(start, period)`.
* Analog operators as expression forms: `ddt`, `idt`, `transition`, `slew`,
  `absdelay`, `last_crossing`, `limexp`.

Elaboration: an `AnalogBlock` list on `ElaboratedModule` (bump the artifact
version, §3.2). Each block carries its own state — an integrator or a
`transition` filter is stateful across timepoints, unlike an `always` block.

Simulator: a new analog scheduling region driven from the `min()` at
`simulator.rs:32277`; `$abstime`, `$temperature`, `$vt`, `$bound_step`,
`$discontinuity` system functions; per-block filter state.

Restriction to state and enforce: contribution statements (`<+`) are **not**
accepted in P2. An `analog` block in this phase is explicit assignment plus
filters — enough for behavioral PLL / comparator / DAC models, which is the
common case, and it must reject the netlist forms with a clear
"requires the analog kernel (not yet implemented)" diagnostic rather than
mis-simulating them.

Tests: `tests/ams/analog_*.rs` citing `AMS §5`. A `cross()`-driven comparator
and a `transition()`-smoothed DAC are the two shapes worth pinning first.

### P3 — Analog kernel (`V(a,b) <+ …`)

Only start after P1+P2 are green and there is a concrete workload demanding it.
Contents: natures/disciplines/branches in the AST and elaboration; node
discipline resolution; MNA stamp assembly; Newton-Raphson with `abstol`/
`reltol`; trapezoidal + Gear integration with LTE-controlled step; convergence
diagnostics. Determinism constraint from §3.4 binds hardest here.

### P4 — Mixed-signal boundary

`connectmodule` / `connectrules`, automatic A2D/D2A insertion at discipline
mismatches, `` `default_discipline ``. Gated on P3.

---

## 5. Files that will be touched (anchor index)

| Where | Anchor | Phase |
|---|---|---|
| `xezim-parser/src/lexer/token.rs` | `keyword()` `:129` | P1, P2 |
| `xezim-parser/src/lexer/scanner.rs` | `is_sv_only_keyword` `:564` (gate pattern) | P1 |
| `xezim-parser/src/ast/types.rs` | `NetType` `:141` | P1 |
| `xezim-parser/src/ast/decl.rs` | `ModuleItem` `:66`, `NetDeclaration` `:524` | P1, P2 |
| `xezim-parser/src/parse/items.rs` | `parse_module_item` `:363` | P2 |
| `xezim-core/src/elaborate.rs` | `Signal` `:185`, `ElaboratedModule` `:1376`, `nettype_resolvers` `:1441`, `resolve_user_nettype_drivers` `:22758` | P1, P2 |
| `xezim-core/src/lib.rs` | `XEZIM_BYTECODE_MAGIC` `:89` | P1, P2, P3 |
| `xezim/src/main.rs` | `--sv2017` arm `:1737`, lib-ext defaults `:31` | P1 |
| `xezim/src/compiler/simulator.rs` | next-time `min()` `:32277` | P2, P3 |
| `xezim/src/compiler/bytecode.rs` | resolver dispatch (from `2f4b8f7`) | P1, P2 |
| `xezim/tests/ams.rs` + `tests/ams/` | new group | all |
| `xezim/README.md` | Features list `:25`, new `--ams` flag | P1 |

---

## 5a. Verification of S1-S3

| Check | Result |
|---|---|
| `xezim` full suite, release (`cargo test --release --no-fail-fast`) | **2346 passed, 0 failed**, 10 ignored |
| `xezim` full suite, `--features jit` (CI config 2) | **2350 passed, 0 failed**, 10 ignored |
| `ams` group | **25 passed** |
| `xezim-core` unit tests | 48 passed |
| `sv-parser` unit tests | 12 passed |
| `cargo clippy --all-targets` on the new core code | clean |

The FST `F3` gate test (`tests/gates/fst_roundtrip.rs`) is un-ignored and
passes; `F4` (event traced as a level) and `F5` (var type + bit range) remain
`#[ignore]`d and untouched.

**One pre-existing failure, unrelated to AMS**: `sv-parser`'s
`guarded_reserved_macro_fallbacks_are_skipped_in_strict_mode` expects
`` `__LINE__ `` to expand to 7 and gets 11 — a line-accounting bug in the
preprocessor's skipped-`` `ifndef `` handling. Verified to fail on the branch
with all AMS changes stashed, i.e. it is broken on `main` too. Not fixed here.

Two defects found and fixed while building S2, both worth recording because
neither is obvious from the feature description:

* The `wreal` min/max fold was exponential in the driver count before
  balancing (see `wreal_reduction`). Pinned by
  `many_drivers_on_a_minmax_node_stay_tractable`.
* The design-cache `semantic_salt` (`xezim/src/main.rs`) tracked `sv2023` but
  not AMS mode, so a cached `--ams` artifact could be reused by a run without
  the flag and the design would simulate under the wrong dialect. `ams=` is
  now in the salt.

## 6. Open items — decide before writing code

1. **`wreal` resolution spelling — OPEN, and shipped on a judgement call.**
   S2 implements the distinct-net-type-keyword form (`wrealsum x;`), which is
   the spelling in common vendor use. It was **not** verified against the LRM
   text: no copy of Accellera Verilog-AMS 2.4.0 is available in this tree, and
   AMS §3.8 may instead select resolution via a discipline or an attribute on a
   plain `wreal`. Confirm against the standard; if the LRM form differs, add it
   alongside — accepting both is additive and breaks nothing already written.
   The same caveat applies to whether a plain multi-driver `wreal` is an error
   (what S2 does, and the safe choice — it never invents a number) or resolves
   to an unknown real.
2. **Unknown state on a real net.** AMS real nets have an explicit unknown;
   `Value{is_real}` has no X/Z plane for reals. Options: a NaN sentinel, or a
   separate flag on `Signal`. Affects VCD output and every real comparison.
3. ~~**FST real-value support.**~~ **Resolved — it is a real pre-existing
   bug, and P1 must fix it.** VCD (`vcd_sink.rs:521`) and XTrace (`:248`)
   handle reals correctly. FST does **not**: every signal is declared
   `FstSignalType::bit_vec(width)` / `FstVarType::Wire`
   (`xezim/src/compiler/simulator.rs:71932-71935`), so a `real` today dumps its
   raw IEEE-754 bit pattern as an integer and reads as garbage in GTKWave. The
   `fst-writer` 0.3 crate already offers `FstSignalType::real()` and
   `FstVarType::Real`, so the fix is a var-type branch on `Signal::is_real` at
   that call site plus the matching 8-byte value path. This bites `real`
   variables that exist *today*, independently of AMS — worth its own
   regression test in `tests/gates/fst_roundtrip.rs`, which already decodes
   FST dumps.
4. **Dialect flag vs. file extension precedence** when both are present, and
   whether `--ams` should imply anything about the IEEE edition.
5. **Whether the `.vams` LRM PDF may be committed** to `docs/` (licensing).
6. ~~**Scope commitment**~~ **Answered**: S1-S3 shipped, which is T1 plus the
   T3 foundation (natures/disciplines in the AST). The next stage is P2 —
   `analog` blocks without a solver. Whether to go on to the full analog
   kernel (P3) is still an open call and should be driven by a concrete
   workload, not by completeness.
