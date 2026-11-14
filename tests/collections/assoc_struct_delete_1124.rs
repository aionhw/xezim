//! §7.9.2 / §10.9: deleting a pattern-written struct removes all its cells.
#[test]
fn delete_pattern_written_class_associative_struct_entries() {
    let sim = xezim::simulate(include_str!("assoc_struct_delete_1124.sv"), 100).unwrap();
    assert_eq!(sim.error_count, 0);
    assert!(sim.output.iter().any(|line| line.message == "TEST PASSED"));
    assert!(!sim.output.iter().any(|line| line.message.contains("FAIL")));
}
