//! IEEE 1800-2023 §7.4.2 / §10.9.1 / §10.9.2: array-member pattern keys
//! name declared indices, including negative indices in either direction.
//! The same leaf expansion serves structure ports (§23.3.3), generate
//! scopes (§27.5) and user nettypes (§6.6.7).

#[test]
fn negative_array_member_patterns_follow_declared_indices() {
    let sim = xezim::simulate(include_str!("struct_negative_array_pattern.sv"), 100_000)
        .expect("simulate failed");
    let checks: Vec<_> = sim
        .output
        .iter()
        .map(|o| o.message.trim())
        .filter(|l| l.starts_with("PASS ") || l.starts_with("FAIL "))
        .collect();
    assert_eq!(checks.len(), 230, "missing checks: {checks:?}");
    for check in checks {
        assert!(check.starts_with("PASS "), "{check}");
    }
}
