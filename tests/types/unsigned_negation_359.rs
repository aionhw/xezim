//! Regression coverage for expression, collection, and scheduling semantics.
use xezim::simulate;
fn check(source: &str, expected: &[&str]) {
    let result = simulate(source, 100_000).unwrap();
    let lines: Vec<_> = result
        .output
        .iter()
        .map(|entry| entry.message.as_str())
        .collect();
    assert!(
        !lines.iter().any(|line| line.starts_with("FAIL")),
        "{lines:?}"
    );
    for line in expected {
        assert!(lines.contains(line), "missing {line:?}: {lines:?}");
    }
}

/// IEEE 1800 §11.8.1: issue #359, original reproducer.
#[test]
fn issue359_unsigned_unary_minus() {
    check(
        include_str!("../issue_cases/next_issue359_0.sv"),
        &["PASS variable", "PASS parameter"],
    );
}

#[test]
fn unary_neighbors() {
    let source = include_str!("../issue_cases/next_unary_neighbors.sv");
    check(source, &[]);
    let result = simulate(source, 100_000).unwrap();
    assert_eq!(
        result
            .output
            .iter()
            .filter(|entry| entry.message.starts_with("PASS"))
            .count(),
        6,
        "{:?}",
        result.output
    );
}

/// §11.8.2: both the parameter and the formal wrap at their unsigned width.
#[test]
fn unsigned_negation_real_context() {
    check(
        include_str!("../issue_cases/next_real_unary_context.sv"),
        &["MODULE 1", "FUNCTION 1"],
    );
}
