# Resolution-Tier Test Suite

This directory contains four reference-validated SystemVerilog test files
that target the multi-driver net-resolution model of xezim:

| File | Tier | What it proves |
|------|------|----------------|
| `37_z_skip_resolution.sv`        | Tier 0 | `tri`/`wire` treats `z` as no contribution when at least one active driver is present; conflicting actives → `x` |
| `39_builtin_nettype_resolution.sv` | Tier 1 | Each built-in net type uses its LRM-correct resolution function: AND for `wand`/`triand`, OR for `wor`/`trior`, pull-down/pull-up defaults for `tri0`/`tri1`, ground/power for `supply0`/`supply1` |
| `38_resolver_dispatch.sv`        | Tier 2 (logic) | A `nettype ... with <fn>` declaration actually invokes the named resolver function with the simultaneous-driver queue (single-bit `logic` element type) |
| `40_struct_nettype_resolution.sv` | Tier 2 (struct) | A `nettype` wrapping a user-defined `struct` (with a `real` field and a `bit` field) invokes the resolver, including struct field access (`driver[i].field1`) and `real` arithmetic in the resolver body |

## Implementation status

| Tier | Reference passes | xezim passes | Status |
|------|------------------|--------------|--------|
| 0    | ✓                | ✓            | **Implemented** — multi-driver resolution post-pass at `xezim-core/src/elaborate.rs` synthesizes per-bit OR-fold with z-skip + x-on-conflict for `tri`/`wire`/`tri0`/`tri1`. |
| 1    | ✓                | ✓            | **Implemented** — same post-pass emits per-NetType fold: AND for `wand`/`triand`, OR for `wor`/`trior`, plus `tri0`/`tri1` default pull at declaration time. |
| 2 (logic) | ✓          | ✓            | **Implemented** — the post-pass synthesizes `assign <var> = <resolver>('{d0, d1, ...})` for any user-defined nettype whose `with <fn>` resolver is registered at $unit scope. Falls back to BitOr fold only when no resolver is named. |
| 2 (struct) | ✓        | ✗            | **Resolver dispatch implemented; struct-element storage broken** — `Tsum(...)` IS called with the right driver count, but xezim's `eval_expr` for `'{1.5, 1'b1}`-style struct AssignmentPatterns yields zeroed fields. Pre-existing bug, unrelated to net-resolution. See "Tier 2 (struct + real) status" below for the chain of fixes needed. |

The implementation lives in `xezim-core/src/elaborate.rs` and a small
adjunct in `xezim/src/compiler/simulator.rs`:

- A new field `elab.net_types: HashMap<String, NetType>` records each
  signal's net type at declaration time (top-level NetDeclaration,
  port-attach, and implicit-net creation sites).
- `user_nettypes` is now `HashMap<String, Option<String>>` — nettype name
  → resolver function name.
- `tri0` and `tri1` declarations now initialize their value to `0` / `1`
  (LRM §28.4 default pull), not `x`.
- A post-pass after `create_implicit_nets` groups each signal's continuous
  drivers and synthesizes a single resolution expression per NetType:
  - `tri`/`wire`/`tri0`/`tri1`: per-bit OR-fold with z-skip + x-on-conflict.
  - `wand`/`triand`: per-bit AND-fold with z-skip.
  - `wor`/`trior`: per-bit OR-fold with z-skip.
  - User nettype with a registered resolver: a single `Call` to the
    resolver with a queue literal `'{d0, d1, ...}` as the argument.
  - User nettype without a resolver: fall back to the original BitOr
    fold (backward compatibility).
- Helper functions appended to `elaborate.rs`:
  `synthesize_tri_or_per_bit`, `synthesize_wand_per_bit`,
  `synthesize_wor_per_bit`, `merge_1bit_{tri,and,or}`,
  `build_per_bit_cont_assigns`, `bit_select`, `sized_bit_literal`,
  `sized_int_literal`, `is_default_pulled`.
- `bind_queue_param` in `simulator.rs` now takes the formal parameter's
  `DataType` and (a) uses its full bit-width for the queue's element
  storage (was hardcoded to 32), and (b) copies `packed_struct_fields`
  from the source struct type so `param[i].field` access works for
  queue params of struct element type.

The Tier 0 + Tier 1 + Tier 2 (logic) implementation matches
QuestaSim-64 2021.2 and Icarus 12.0 on every tested case.

### Tier 2 (struct + real) status

**Not implemented.** The Tier 2 resolver dispatch works correctly for the
logic-element case (`count_net_t {logic [3:0] cnt}` with resolver
`function [3:0] my_resolve_count_ones(...)`) but the struct-element case
fails. The chain of pre-existing xezim issues that need to be fixed for
`Tsum(driver[])` with `typedef struct {real f1; bit f2;} T;` to work:

1. **`real` fields in continuous-assign** — the CA path interprets
   non-real struct values stored to real signals as integers rather than
   IEEE 754 bit patterns (`to_f64()` on a non-real value uses
   `to_u64() as f64`, not `f64::from_bits(val_bits)`).
2. **Local struct field member write inside function bodies** —
   `result.field1 += x` reads `result` as a local var, then the
   MemberAccess write to a local struct field isn't routed through the
   packed_struct_fields read/write paths, so the value doesn't get
   updated.
3. **Unpacked-struct whole-value AssignmentPattern** — when the
   elaborated code synthesizes `Tsum('{'{1.0, 1'b0}, '{2.0, 1'b1}})`,
   the xezim parser flattens the inner `{}` brackets inside the queue
   literal, parsing it as a 4-element queue of `1.0, 1'b0, 2.0, 1'b1`
   rather than a 2-element queue of struct values.

These are independent of the multi-driver resolution work and would
require significant xezim infrastructure work to fix.

## Reference simulators

- **QuestaSim-64 2021.2_1** (`vlog`/`vsim`) — primary reference; supports
  user-defined nettypes with resolver functions (§6.6.7 of IEEE 1800-2023).
- **Icarus Verilog 12.0** (`iverilog`/`vvp`) — secondary cross-check for
  built-in net types. **Icarus does not support `nettype` at all**, so Tier 2
  cannot be run there.

Both simulators agree byte-for-byte on every built-in net-type scenario.

## Current xezim behavior (Tier 0–2 not yet implemented)

A diagnostic probe (`probe_xezim_value.sv`, equivalent to
`probe_questa_resolution.sv` run on xezim) shows:

| Scenario | Questa / Icarus | xezim | Bug caught by |
|----------|-----------------|-------|---------------|
| `tri [0,1]` | `x` | `0` (first-driver-wins) | Tier 0 / Tier 1 |
| `tri [0,1,z]` | `x` | `0` (first-driver-wins) | **Tier 0 B3** |
| `tri [1,z]` | `1` | `1` ✓ | — |
| `tri [z,z]` | `z` | `z` ✓ | — |
| `wand [0,1]` | `0` | `0` ✓ (first wins) | — (lucky) |
| `wor [0,1]`  | `1` | `0` (first wins) | **Tier 1** |
| `trior [0,1]` | `1` | `0` (first wins) | **Tier 1** |
| `tri0` (no drivers) | `0` | `x` (uninit) | **Tier 1** |
| `tri1` (no drivers) | `1` | `x` (uninit) | **Tier 1** |
| `tri0 [0,1]` | `x` | `0` (first wins) | **Tier 1** |
| `tri1 [0,1]` | `x` | `0` (first wins) | **Tier 1** |
| `nettype ... with my_fn` resolver (logic element) | function called | hardcoded BitOr fold | **Tier 2** |
| `nettype T ...` with `struct { real f1; bit f2; }` resolver | `Tsum` called, real summed, bit OR'd | first-driver-wins, no resolver call | **Tier 2 (struct)** |

In short, xezim's current implementation:
1. Treats multiple `assign` statements on a net as **first-driver-wins** (no
   resolution at all), at `xezim-core/src/elaborate.rs:2802-2844` which only
   applies the BitOr fold to user-defined nettype nets.
2. Initializes `tri0`/`tri1` to `x` instead of their LRM-mandated default
   pull-down / pull-up.
3. Never invokes the resolver function registered via `nettype ... with <fn>`.

## Running the tests

### Questa

```bash
cd ~/prog/git/xezim/xezim/tests/sv_compliance/tests_advanced
./questa_run_tiers.sh             # all three tiers
./questa_run_tiers.sh 37 39       # specific tiers (Tier 2 = 38)
./questa_run_probe.sh             # print Questa's actual values for every
                                  # built-in net-type scenario
```

### Icarus (Tier 0 + Tier 1 only)

```bash
cd ~/prog/git/xezim/xezim/tests/sv_compliance/tests_advanced
./iverilog_run_tiers.sh           # tiers 37 and 39
./iverilog_run_tiers.sh 37        # individual tier
```

### xezim

```bash
XEZIM=../target/debug/xezim make -C ~/prog/git/xezim/xezim/tests/sv_compliance \
     run TEST=37_z_skip_resolution TEST_DIR=./tests_advanced
# (replace TEST= with 38_resolver_dispatch or 39_builtin_nettype_resolution)
```

Or directly:

```bash
cd ~/prog/git/xezim/xezim/tests/sv_compliance/tests_advanced
~/prog/git/xezim/xezim/target/debug/xezim -I ../common 37_z_skip_resolution.sv
```

The runner scripts print `TEST_PASS` / `TEST_FAIL count=N`; a non-zero `count`
means the test caught a behavior gap with the reference simulator.

## Files in this directory

```
37_z_skip_resolution.sv            # Tier 0 source
38_resolver_dispatch.sv            # Tier 2 source (logic element)
39_builtin_nettype_resolution.sv   # Tier 1 source
40_struct_nettype_resolution.sv    # Tier 2 source (struct + real field)

probe_questa_resolution.sv               # diagnostic for Questa values
probe_struct_nettype.sv                  # diagnostic for Questa struct-net values
run_probe.do                             # Questa GUI driver for resolution probe
questa_run_probe.sh                      # Questa CLI driver for resolution probe
questa_run_tiers.sh                      # Questa CLI driver for all four tiers
iverilog_run_tiers.sh                    # Icarus CLI driver for tiers 0 + 1
resolve_run_all.sh                       # top-level cross-simulator orchestrator
run_tier0.do / run_tier1.do / run_tier2.do   # individual Questa GUI drivers

README_resolution_tiers.md               # this file
```

## LRM clauses exercised

- **IEEE 1800-2023 §6.6.7** — User-defined nettypes with resolver functions.
- **IEEE 1800-2023 §6.7** — Wire and tri nets (resolution semantics).
- **IEEE 1800-2023 §28.4** — `tri`/`tri0`/`tri1` net resolution.
- **IEEE 1800-2023 §28.7** — `supply0`/`supply1` declarative-only nets.
- **IEEE 1800-2023 §28.16** — `wand`/`triand`/`wor`/`trior` net resolution.

## After fixing Tiers 0, 1, 2 in xezim

Once xezim implements proper per-NetType resolution and resolver dispatch:

1. Run the three tier files via the Icarus/Questa runner scripts — confirm
   they still pass (defensive: verifies we didn't regress).
2. Run them via `make run TEST=...` against xezim — confirm they now pass
   (regression: shows the bug fixes landed).
3. Optional: add a fourth tier file `40_tier3_strengths.sv` for the §28.3
   strength lattice (a separate ~2–4 week effort, see elaboration in the
   project notes).