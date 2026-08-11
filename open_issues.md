# Open issues

Known gaps, each with the evidence that pins it and what a fix has to do.
Anything listed here was reproduced against the reference simulator; the
repro paths point at scratchpad files from the sessions that found them.

Closed items are not repeated here — see the git log and `debug_notes.md`
(outside the repo) for the investigation trail.

---

## 1. Continuous-assignment propagation order after a procedural blocking write

**What differs.** xezim settles continuous assignments immediately after a
procedural blocking write, so the *writing process* observes the propagated
value on its very next statement:

```systemverilog
logic [7:0] a; wire [7:0] b, c;
assign b = a + 1;  assign c = b + 1;
initial begin
  a = 5;
  s1 = b;   // reference: 1 (stale)   xezim: 6
  s2 = c;   // reference: 2 (stale)   xezim: 7
  #0 s3 = c; // both: 7
end
```

§5.5/§10.3 make the re-evaluation a separate active-region update event: the
writing process does not yield, so its own reads see pre-update net values
until it hits a delay or wait. Eager settling is already suppressed inside
edge blocks, so the divergence is confined to testbench / initial / task
code — exactly where a BFM drives a request and immediately reads a
continuous-assign-derived grant or status wire. Reading a handshake one
statement early forks the entire stimulus trajectory, which is why this
class of bug reproduces only in full designs.

**State.** All 38 eager-settle sites route through
`settle_after_proc_write()`. Eager is the DEFAULT;
`XEZIM_LAZY_PROC_SETTLE=1` opts into the LRM/reference ordering.

**Why the flag is not simply flipped.** Measured, not assumed — the
reference is lazy on one real shape and eager on another, so no single
global setting matches it:

| shape | reference | xezim eager | xezim lazy |
|---|---|---|---|
| drive-then-read a cont-assign wire (`bigaudit/c1b.sv`) | `2 1 2 7` | `2 6 7 7` | `2 1 2 7` ✅ |
| c910 dep_reg entry, TB drives right after `@(posedge clk)` (`bigaudit/dep.sv`) | `wb=1 rdy=1 rfi=1` | ✅ | `wb=0 …` |

Lazy-as-default also regresses two reference-validated t=0 gate/UDP
initialization traces (`udp_primitives::edge_shorthands` q2 at t=0,
`dep_reg_entry_synth::dep_reg_entry_wb_wakes_rdy`) — 1849 pass / 2 fail.

**Rejected approaches** (both measured):
- Keeping t=0 eager — `c1b` runs entirely at t=0, so the exemption defeats
  the fix.
- Settling at the `check_edges` boundary so deferred writes land before the
  next edge — `c1b` stayed correct but `dep.sv` did not recover, proving
  the dep divergence is not about *when* the deferred settle lands.

**What a fix must do.** Per-evaluator scheduling, not one knob: defer
`ContAssign`/`CompiledContAssign` updates past the writing process's own
reads while keeping `Udp`/`UdpBatch`/`FusedGate` evaluation per input
change. `CombItem` already distinguishes these, so a filtered settle is
expressible.

**Coupled sub-issue.** xezim's UDP has no "simultaneous multi-input change"
rule. Under lazy settling two inputs change in one evaluation and the UDP
matches `* 0 : ? : 0` (output 0) where the reference holds x. Eager hid
this by only ever presenting one changed input per evaluation. Must be
fixed alongside, or the UDP trace regresses the moment propagation is
deferred.

---

## 2. LRM audit backlog (pre-existing)

Each entry is a reference-validated finding from the original audit sweeps.


### H3 remainder — `ref` aggregate/caller-local aliasing
CLOSED for plain module-visible variables (true aliasing: mid-call
visibility both ways, no copy-out clobber, double-ref and chained-ref and
same-named formal all reference-validated — `tests/misc/ref_args_alias.rs`).
Remaining on the legacy copy path: actuals that are caller-frame locals,
aggregate elements (`arr[i]`), class properties, strings. Correct at return,
not aliased mid-call. Freezing an element index at call time (§13.5.2) and
routing caller-local storage are the follow-ups.

(G8+G9 formatting closed: explicit `%h`/`%b`/`%o` widths follow the
reference's minimal+zero-pad model — commercial tools disagree here and the
previous pin followed the other tool; unconsumed display args now print
default-width decimal. `tests/strings/format_sibling_fixes.rs`.)

### F6–F10 — CLOSED
Clocking skews were already reference-exact; `##0` now synchronizes to the
default clocking event (waits off-edge, no-op at the edge — the previous
suite pin of "never waits" contradicted the reference and was re-measured);
`process.status().name()` returns the built-in state name. Residual
simplification: a RUNTIME count expression evaluating to 0 (`##(n)` with
n==0) keeps the repeat form and waits a cycle — literal `##0` is correct.
(G6/G10/G11 also closed: trireg charge storage via an implicit weak
self-driver + x initial value; `#1step` and `$bits("")` were already
reference-exact.)

### J4, J6–J12 — hierarchy
Interface ports, exports, port z-padding, nested modules.

### L5/L6/L11/L12 + L3/L4 — symbol tables
Clash-check family, plus hierarchical-name legality.

### L7–L10, L13–L18, L20 — front-end acceptance batch 2

### J2c, J2e — const-eval remnants
Type-parameter `$bits`, and interface-port parameters. (The rest of the
const-eval silent-zero family — dimension-width function calls, package
const-fn parameters, the hoist fixpoint — is fixed.)

---

## 3. Preprocessor acceptance / rejects-valid (K-A1..A10, K-R2)

K-R1 (`` `ifdef ``/`` `else ``/`` `endif `` inside a macro body) is fixed:
a conditional directive's name now ends at the first non-identifier
character, so `` `endif; `` is recognised, split onto its own line, and the
trailing `;` survives instead of being swallowed with the directive.

The remaining K-A acceptance cases and K-R2 have not been swept since the
K-W (wrong-expansion) family was closed.

---

## 3b. Enum-typed associative-array KEYS on class properties

`foreach (cnt[s]) s.name()` resolves against the wrong enum when `cnt` is a
**class property**. The module- and package-scope declaration paths now record
the key's type name (`assoc_key_type_names`) and the `foreach` index variable
is bound with it, so those cases are correct and reference-validated. The class
path (`elaborate.rs`, the `assoc_properties.insert(...)` site) builds
class-local maps and has no access to the module map, so the key type is not
recorded there.

Symptom: with no type binding, `enum_value_name` scans every enum and returns a
member of whichever has the most entries. UVM's report summary prints
`UVM_NORADIX` and `UVM_PHASE_DORMANT` where severities belong, because
`uvm_report_server`'s `severity_count` is a class property keyed by
`uvm_severity`.

Repro: `scratchpad/uvm/enum4.sv` (class property — fails) vs `enum3.sv`
(module scope — fixed). Fix: carry the key type into the class definition
alongside `assoc_properties`, and consult it from the `foreach` binding when
the root name isn't in the module map.

## 4. Customer performance thread

The for-loop bytecode compilation landed (local repro 73.3 s → 6.0 s), but
on the customer's DRAM run `For_init_vardecl` fallback time did not drop
(~763 s, count only −33%). Their hot loop bodies therefore fail
`for_body_is_simple` — most likely member access or calls in the body.
Needs one of their actual loop bodies to extend the prescan.

Remaining fallback targets, ranked from that run's own table: `ident_lookup`
11.2 s, `Expr_Call_impure` 6.3 s, `nba_ident_unresolved` 4.1 s,
`Expr_MemberAccess` 2.8 s. See `perf_improve_notes.md`.

---

## 4b. Interpreted array-element access is name-based (GitHub #86)

Each `mem[i]` read/write in the AST interpreter materialises the element name
(`format!("{}[{}]", name, idx)`) and hashes it, costing ~1.6-2.2 us per access
— `perf` on a memory fill is dominated by `String::clone`,
`join_generic_copy`, `split_flat_path`, `memcmp` and hashing, not value logic.

The O(1) resolver already exists: `get_array_elem_id` / `array_first_id`
computes `first_id + (idx - lo)` with no string work, but it is wired only
into the JIT bridge and one expression path. Routing the interpreted
read/write paths through it is the substantive fix.

NOTE the issue's premise is wrong and the comment on it explains why: the
event loop is NOT O(array size) per iteration. Holding the array fixed and
running 10x longer adds ~0.85 us/iter (the array-free baseline); the cost is a
one-time O(N) fill. Do not go looking in the settle/process loop.

Separately, the interpreter costs ~1.65 us per statement for a plain `for`
loop in an `initial` block, independent of arrays — a larger question.

Done: (1) the indexed-write path no longer deep-clones two expression trees
per store (`e15a0b4`); (2) `mem[i] = v` on a module-scope 1-D fixed unpacked
array now resolves through `array_first_id` id math with no string work,
skipping the class/queue/assoc guard chain — store overhead 2.2 -> 0.86
us/element, the reporter's 256K shape 1124 -> 880 ms. Guards: no live call
frames, no class objects, and not a queue/dynamic/assoc name (those carry a
fake 0..63 backing range in `module.arrays`).

ALSO FOUND while validating (all PRE-EXISTING, verified identical on the
pre-change build; the fast path is bug-for-bug faithful):
- `force` on an array ELEMENT does not stick (a later procedural write
  overrides it; the reference holds the forced value until `release`).
- 2-state element arrays (`bit [7:0] m [0:3]`) accept X instead of
  fitting X/Z -> 0 on element writes.
- Negative-lo arrays (`m[-2:1]`): element write/read at a negative index is
  lost (reads back X; likely unsigned index evaluation upstream of the
  range check).
Repro for all three: scratchpad/i86/d1.sv vs the reference. These are
correctness bugs on BOTH the fast and general paths — fix them together so
the two paths cannot diverge.

The heap-sentinel discovery matters beyond #86: `heap` is constructed as
`vec![None]` (index 0 = null handle), so every `heap.is_empty()` guard in
the simulator was DEAD CODE and class-free designs paid class-resolution
probes on every indexed access. All guards now use `no_class_objects()`
(len <= 1).

## 4c. Module-scope storage crossed with class-method paths (two gaps)

Both found while validating the #109 fix (class-event member wait/trigger);
both reference-validated as divergences and PRE-EXISTING (reproduced on
clean main, `--no-cache`).

**(a) Property writes through a module-scope handle variable are lost.**
`B h; A m;` at MODULE scope; `h = new; h.direct = new; h.direct.tag = 11;
m = h.direct;` → `m.tag` reads x (reference: 11). The same statements with
`h`/`m` declared inside the initial block work. Queue-property writes
through the same module-scope handle (`h.q.push_back(t)`) DO land, so the
break is specific to plain-property (and assoc-element) stores resolving
through a module-scope receiver. Repro: `scratchpad/uvm/rp_min2.sv` (x)
vs `rp_min3.sv` (locals, correct).

**(b) Hierarchical writes to module vars from class methods are dropped.**
`tb.counter++` inside a class task executes without effect or diagnostic
(reference: increments). Repro: `scratchpad/uvm/t109_cli.sv` — `created`
stays 0 while the surrounding protocol works. A silent no-op write is the
worst failure mode; if unsupported it must at least error.

## 5. Diagnosability

- **Settle-cap silence.** `--settle-limit` hits warn once, then the cap is
  applied silently for the rest of the run. A design that hits it repeatedly
  (the customer's does, from t=151,310) gives no further signal.
- **r11 interpreter melt.** A 45-line interpreted always-block costs ~12 s
  per tick in one repro — a long-standing separate bug with two pinned next
  steps in the Round 18 debug notes.
