//! #928 (reported by Ganesh-TNL): a class property whose type is a packed
//! struct with packed dimensions written BEFORE the declarator
//! (`rand cand_t [0:2] a;`, `typedef cand_t [0:2] carr_t;`, an inline
//! `struct packed {..} [0:2] a`) is a packed array of structs
//! (IEEE 1800-2023 §7.4.1, §6.20.2): `a[i]` is one element, `a[i].f` a
//! field of it, in the evaluator and in constraints (§18.4, §18.5).
//! Resolving the typedef NAME alone lost the dimensions: `a[i]` read 0
//! after a write, `a[i].f` read 0, member writes did not land, and every
//! constraint on `a[i]` / `a[i].f` was dropped or "no solution".
//!
//! Also §18.5.8.2 / §7.12.3: `and()`, `or()`, `xor()` and `product()`
//! targets over elements wider than 64 bits, and `a[0]+a[1] == K` chains
//! with a target above 2^64-1, keep every bit (only `sum()` had the wide
//! path, #588).
//!
//! Expected values from the reference simulator: every check passes; the
//! probes' `T|` lines were verified against it (untouched 4-state elements
//! read x).
use xezim::simulate;

fn passes(src: &str, max_time: u64) -> (u64, Vec<String>) {
    let sim = simulate(src, max_time).expect("simulate failed");
    let failures = sim
        .get_signal("failures")
        .or_else(|| sim.get_signal("tb_top.failures"))
        .unwrap_or_else(|| panic!("signal not found: failures"))
        .to_u64()
        .unwrap_or_else(|| panic!("failures not u64-able"));
    let out: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .collect();
    (failures, out)
}

/// The reporter's consolidated reproducer: sections A (evaluator), B
/// (constraints), C (wide reductions) and D (wide element chains), each
/// with its CTRL shapes.
///
/// Known open (the reference honours it): `foreach (c[i]) c[i].valid == 1'b1`
/// over a packed array of structs — the solver does not model a FIELD of an
/// element named through the loop variable (the whole element, `c[i] == K`,
/// and explicit `c[0].valid == 1'b1` are modelled). That one check is the
/// only FAIL line this test accepts.
#[test]
fn packed_struct_array_property_928() {
    let (failures, out) = passes(include_str!("packed_struct_array_property_928.sv"), 100);
    let fails: Vec<&String> = out.iter().filter(|m| m.starts_with("FAIL")).collect();
    assert_eq!(
        fails,
        ["FAIL @0 : foreach element.member: honoured"],
        "FAIL lines: {:#?}",
        fails
    );
    assert_eq!(failures, 1);
}

/// Neighbouring shapes: a two-level typedef (`carr_t [1:0]`), a struct
/// array as a MEMBER of a struct property, an unpacked array of packed
/// struct arrays, part-selects of an element field, `inside`, `dist`,
/// `foreach`, descending ranges, a 90-bit array, cross-element compares,
/// `$bits`, methods; and more wide reductions (128-bit `and`, three-element
/// `xor`, dynamic `or`, pinned `and`, zero `product`, a queue `xor`, a
/// reversed chain).
#[test]
fn packed_struct_array_property_928_probes() {
    let (failures, out) = passes(
        include_str!("packed_struct_array_property_928_probes.sv"),
        100,
    );
    let t: Vec<&String> = out.iter().filter(|m| m.starts_with("T|")).collect();
    let fails: Vec<&&String> = t.iter().filter(|m| !m.starts_with("T|PASS")).collect();
    assert!(
        fails.len() == 1 && fails[0].as_str() == "T|TEST_PASS",
        "unexpected T| lines: {:#?}",
        fails
    );
    assert_eq!(t.len(), 37, "every probe must report: {:#?}", t);
    assert_eq!(failures, 0);
}
