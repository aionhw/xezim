//! cls-agg-members-in-struct: the ORIGINAL failure matrix, ported verbatim
//! from the reproducer (59 of its self-checks failed before the fix). The
//! companion `cls_agg_members_in_struct.rs` distills the core shapes into
//! seven readable pins; THIS test is the exhaustive net that must stay
//! green so no resolution arm can silently rot. Its staged checks:
//!
//!   A    local struct variable: element store -> element read          (baseline)
//!   E    plain packed-array class property: element store -> read      (baseline)
//!   B1   element store inside a method (class property struct member)  (PRIMARY)
//!   B3   same with a small [3:0][255:0] member                         (size dep.)
//!   F2   whole-member store, method + module scope                     (store family)
//!   W    wide SCALAR member store vs array member store, same struct   (width vs shape)
//!   K1   whole-struct property store (`obj.prop = struct_val`)         (cousin)
//!   R    direct element READ at module scope after data injection      (cousin, read side)
//!   R2   element read inside a method (`peek`)                         (cousin, read side)
//!   R3   part-select read `prop.mem[1][255:248]`                       (cousin, read side)
//!   K3w  part-select write `prop.mem[1][255:248] = 8'hAB`              (cousin, write side)
//!   B2m  element store at MODULE scope (flattened `obj.prop.mem[i]=v`) (cousin, parse shape)
//!   K4   nonblocking element store from a clocked always block         (cousin, NBA path)
//!   K5   unpacked-struct property with packed-array member             (contrast, expected ok)
//!   K6   element store at struct depth 2 (`prop.inner.mem[i]`)         (cousin, nesting)
//!   K7   unpacked-ARRAY member of an unpacked-struct property: element
//!        r/w (method+module scope), by-value return, whole-property
//!        copy-out, part-select into an element, NBA element store      (boundary family)
//!   K8   dynamic `[]` / queue `[$]` / associative `[int]` / multi-dim
//!        `[2][2]` container members of an unpacked-struct property:
//!        element stores/reads/pushes, plus string member and DIRECT
//!        container class properties                                    (boundary family)
//!   C    full production chain: element stores -> by-value return ->
//!        module-level variable -> indexed reads (+ head fields)        (end-to-end)
//!   D    control: full chain with a small array member
//!
//! The design is deterministic: every value is injected through whole-
//! assignments and member stores before any check reads it back, so only
//! the member-resolution paths decide the outcome.

use xezim::simulate;

#[test]
fn cls_agg_members_matrix_full_matrix() {
    let sim = simulate(include_str!("cls_agg_members_matrix.sv"), 1_000_000)
        .unwrap_or_else(|e| panic!("simulate failed: {}", e));
    let msgs: Vec<&str> = sim.output.iter().map(|o| o.message.trim()).collect();
    let fails: Vec<&str> = msgs
        .iter()
        .copied()
        .filter(|m| m.starts_with("FAIL"))
        .collect();
    assert!(
        fails.is_empty(),
        "cls-agg-members-in-struct regressions (aggregate struct-member accesses):\n{}",
        fails.join("\n")
    );
    assert!(
        msgs.iter().any(|m| *m == "TEST_PASS"),
        "never reached TEST_PASS — simulation died early:\n{}",
        msgs.join("\n")
    );
}
