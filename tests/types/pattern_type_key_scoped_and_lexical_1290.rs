//! IEEE 1800-2023 §10.9.1, §10.9.2, §8.25.1, §26.3:
//! - Lexical binding of bare typedef keys across mid-module `import p::*`,
//!   with struct member names still taking precedence over typedef keys (§10.9.2).
//! - Parameterized class-scope typedef keys: type-parameter dependent, value-
//!   parameter dependent, default specialization, package-qualified, typedef
//!   alias of a specialization, and inherited class typedefs (§8.25.1).
//! - Generate-block typedef keys across nested/sibling generate blocks,
//!   child module generate blocks, and assignments to outer variables.
//!
//! Verified against reference simulators (struct member precedence follows
//! §10.9.2 where member keys take precedence over type keys).

#[test]
fn scoped_class_generate_and_lexical_import_type_keys() {
    let sim = xezim::simulate(
        include_str!("pattern_type_key_scoped_and_lexical_1290.sv"),
        10_000,
    )
    .expect("legal scoped and lexical type-key patterns");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|line| line.message.as_str())
        .filter(|line| line.starts_with("T|"))
        .collect();
    assert_eq!(
        lines,
        &[
            "T|before 7 7",
            "T|sm_before 11 22",
            "T|after 99 99",
            "T|sm_after 11 22",
            "T|tdep 55 55",
            "T|tdef 7 7",
            "T|pkg 66 66",
            "T|alias 77 77",
            "T|sub 88 88",
            "T|vec a 5",
            "T|sib0 7 7",
            "T|sib1 33 33",
            "T|top_gen 44 44",
            "T|child_gen 42 42",
        ]
    );
}
