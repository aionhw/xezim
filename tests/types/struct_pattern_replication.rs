//! IEEE 1800-2023 §10.9.2: structure replication binds expressions to members.
use xezim::simulate;

fn check(src: &str, passes: usize) {
    let sim = simulate(src, 20_000_000).expect("legal assignment patterns");
    let lines: Vec<_> = sim.output.iter().map(|o| o.message.as_str()).collect();
    assert!(!lines.iter().any(|s| s.starts_with("FAIL")), "{lines:?}");
    assert_eq!(
        lines.iter().filter(|s| s.starts_with("PASS")).count(),
        passes,
        "{lines:?}"
    );
}

#[test]
fn replicated_struct_members() {
    check(include_str!("struct_pattern_replication.sv"), 30);
}

#[test]
fn array_and_packed_replication_guards() {
    check(include_str!("struct_pattern_replication_guards.sv"), 4);
}

#[test]
fn replicated_member_contexts() {
    check(include_str!("struct_pattern_replication_contexts.sv"), 5);
}

#[test]
fn inactive_generate() {
    check(
        include_str!("struct_pattern_replication_inactive_generate.sv"),
        1,
    );
}

#[test]
fn inactive_generate_parameter() {
    check(
        include_str!("struct_pattern_replication_inactive_generate_parameter.sv"),
        1,
    );
}

/// Illegal sources deliberately check main compatibility, not LRM values.
/// Core refusal of legal, unproven counts is checked in the core suite.
#[test]
fn unproven_and_unknown_counts_keep_fallback() {
    // Main now expands the inherited constant evaluator's count in continuous
    // assignments. The unsized-overflow source would allocate billions of
    // items there; keep it as an external compatibility probe. Core's
    // unknown_oversized_and_comparison_mismatches_are_declined checks refusal.
    for (src, first, constant, continuous) in [
        (
            include_str!("struct_pattern_replication_signed_comparison_mismatch.sv"),
            "7",
            "0700",
            "0,0",
        ),
        (
            include_str!("struct_pattern_replication_case_equal_mismatch.sv"),
            "7",
            "0700",
            "0,0",
        ),
        (
            include_str!("struct_pattern_replication_case_unequal_mismatch.sv"),
            "7",
            "0700",
            "0,0",
        ),
        (
            include_str!("struct_pattern_replication_shift_overflow.sv"),
            "7",
            "0700",
            "7,7",
        ),
        (
            include_str!("struct_pattern_replication_comparison_mismatch.sv"),
            "7",
            "0700",
            "0,0",
        ),
        (
            include_str!("struct_pattern_replication_unknown_x.sv"),
            "7",
            "0000",
            "0,0",
        ),
        (
            include_str!("struct_pattern_replication_unknown_z.sv"),
            "7",
            "0000",
            "0,0",
        ),
        (
            include_str!("struct_pattern_replication_oversized.sv"),
            "7",
            "0700",
            "0,0",
        ),
        (
            include_str!("struct_pattern_replication_cond_negative.sv"),
            "7",
            "0000",
            "0,0",
        ),
        (
            include_str!("struct_pattern_replication_cond_positive.sv"),
            "7",
            "0700",
            "0,0",
        ),
        (
            include_str!("struct_pattern_replication_param_inferred.sv"),
            "7",
            "0700",
            "0,0",
        ),
        (
            include_str!("struct_pattern_replication_localparam_inferred.sv"),
            "7",
            "0700",
            "0,0",
        ),
    ] {
        let sim = simulate(src, 20_000_000).expect("preserve existing acceptance");
        let observed: Vec<_> = sim
            .output
            .iter()
            .map(|o| o.message.as_str())
            .filter(|s| s.starts_with("OBSERVE"))
            .collect();
        assert_eq!(
            observed,
            [format!(
                "OBSERVE procedural={first},2 continuous={continuous} argument={first},x constant={constant} class={first},2"
            )]
        );
    }
}

#[test]
fn literal_counts_in_all_contexts() {
    check(
        include_str!("struct_pattern_replication_literal_counts.sv"),
        6,
    );
}

#[test]
fn nonliteral_array_and_packed_guards() {
    check(
        include_str!("struct_pattern_replication_nonliteral_guards.sv"),
        4,
    );
}

/// §10.9.1/§10.9.2: preserve the array elements already assigned correctly.
#[test]
fn array_member_elements_keep_existing_handling() {
    check(
        include_str!("struct_pattern_replication_array_members.sv"),
        5,
    );
}

/// §7.2.1/§10.9.2: a packed constant is not excluded by an array typedef of
/// the same name in another scope.
#[test]
fn packed_constant_beside_outer_array_typedef() {
    check(
        include_str!("struct_pattern_replication_packed_constant_scope.sv"),
        1,
    );
}
