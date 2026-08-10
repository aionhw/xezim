# Performance improvement notes — for-loop bytecode compilation (2026-08-09)

Companion to `perf_improve.md` (the earlier interpreter/JIT/AOT campaign).
This documents the fallback-elimination work driven by a customer DRAM run
that took 921 s in xezim vs 25 s in the reference simulator (~36x).

## The finding

The run's own `[PROF] fallback_reason` table attributed the gap to ONE
construct:

```
For_init_vardecl: count=3,742,640  total=765,827.6ms  avg=204.6µs
```

83% of the entire wall time was edge blocks containing `for (int i = ...)`
loops being executed by the AST interpreter, because the bytecode compiler
bailed on the loop-variable declaration (and, separately, on `i++`-style
steps for signal-backed counters, and on `N'(x)` size-casts). The settle
engine itself (gate/UDP waves) accounted for only ~30–45 s — within ~2x of
the reference for the work it counts. The order-of-magnitude gap was
entirely the interpreter escape hatch.

Local reproduction (16-lane wdata-style loop in an `always_ff`, 2M cycles,
reference startup subtracted, outputs byte-identical):

| variant                     | before | after | reference | ratio before → after |
|-----------------------------|--------|-------|-----------|----------------------|
| `for (int i = ...)` loop    | 73.3 s | 6.0 s | 2.1 s     | 35x → 2.9x           |
| module-level `i` with `i++` | 61.9 s | 6.9 s | 0.5 s     | ~110x → ~10x         |
| identical logic unrolled    | 4.5 s  | 4.5 s | 1.6 s     | 2.8x (control)       |

Projected effect on the customer run: 921 s → roughly 2.5–4 minutes
(remaining: ~28 s of other fallback constructs, ~30–45 s settle, NBA/VCD).

## What was implemented (bytecode.rs)

1. **`ForInit::VarDecl` compiles to a VM register slot.** The loop var has
   no signal; reads resolve through `local_var_regs`, which `compile_expr`
   consults before the signal tables, so a same-named outer signal can
   never capture. Array indexing through the register is safe by
   construction: the register path (`Nba/BlockingAssignArray`) takes a
   RegId, and the SigId-fusion peepholes pattern-match `LoadSignal`,
   which a register-backed index never emits.

2. **StmtFallback is forbidden while a register loop var is live**
   (`reg_var_loop_depth`). The AST interpreter cannot see VM registers, so
   a fallback statement inside such a loop silently reads the loop var as
   X. Any unsupported construct fails the whole loop back to the AST path
   instead (old behavior — correct, just slow).

3. **Signal-backed `i++`/`i--` steps compile** (were `For_step_other`
   bails): load signal, ±1, resize, blocking store.

4. **`$__xz_size_cast` compiles** (was `SystemCall_other`): §6.24.1
   context-width evaluation of the operand, then resize.

5. **Conservative simple-body prescan** gates capabilities 1 and 3. A loop
   body qualifies only if every statement is an assign/if/begin-end/null,
   every lvalue is a plain ident or single index on a plain ident, no
   expression contains member access or calls, and **no NBA target array
   is also read in the body** (`ptr[i] <= ptr[i] + 1` stays on the AST
   path — in an inlined instance its compiled read and write resolved the
   array through different aliases). Bodies failing the prescan keep the
   pre-existing whole-loop AST fallback, so the gate can never regress.

6. **Loop-var signedness**: the register slot is Set/ClearSigned per the
   declared type (`int` is signed even when the init literal isn't), and
   the step constant `1` is emitted SIGNED — an unsigned 1 stripped the
   var's sign on the first `i--` (signed+unsigned → unsigned), turning
   `i > -3` into an unsigned compare that exited after one iteration.
   A signed 1 is universally correct: signed+signed stays signed;
   signed+unsigned still yields unsigned for unsigned loop vars.

## Verification

- Adversarial audit vs the reference simulator (all byte-identical):
  descending loops with signed conditions (`i >= 0`, `i > -3`, `i >= -2`),
  nested register-var loops, break/continue, loop var shadowing a
  same-named module signal, byte-typed vars with `i += 2` strides,
  zero-trip loops, parameter bounds, negative-crossing accumulation.
- Regression tests: `tests/scheduling/compiled_for_loops.rs` (6 tests,
  including the self-reading-counter exclusion and the signed-compare
  case).
- Bring-up caught 7 full-gate failures (nested-index NBA freeze,
  struct-port member sensitivity, instance array aliasing) — all traced
  to shapes now excluded by the prescan; final gate 1830+ passing, 0
  failing. `work_counters` insn ceiling re-baselined 16,200 → 55,000
  (loop work moved from uncounted AST execution into the counted insn
  stream; wall time drops).

## Diagnostic tooling added alongside

- `XEZIM_EDGE_FIRE_TRACE=<substr>` v2: per-block match bitmap computed
  once (v1 string-scanned every fired block per dispatch — a measurable
  share of the customer's traced run), plus `XEZIM_EDGE_FIRE_WATCH=<sigs>`
  appending watched signal values to each fire line.

## Remaining fallback work, ranked by the customer run's table

| reason               | count | total   | avg     | note |
|----------------------|-------|---------|---------|------|
| ident_lookup         | 1.8M  | 11.2s   | 6.2µs   | idents failing compile-time signal resolution (instance scoping) |
| Expr_Call_impure     | 39k   | 6.3s    | 158.8µs | impure function calls in edge blocks |
| nba_ident_unresolved | 5.0M  | 4.1s    | 0.8µs   | cheap each; huge count |
| Expr_MemberAccess    | 177k  | 2.8s    | 15.8µs  | struct member reads |
| nba_index_other      | 2.3M  | 1.8s    | 0.8µs   | |
| ProceduralContinuous | 11.7M | 1.8s    | 0.2µs   | |

After those: UDP dirty-propagation round trip (route comb-UDP output
writes through the settle-local `trigger_deps!` like the fused gate arms,
measured −3.9% when done for gates), then the armed-worklist edge-detect
inversion. All "representation" levers (JIT, AOT, BSP settle) were
previously measured as losses for this workload class — see
`perf_improve.md`.
