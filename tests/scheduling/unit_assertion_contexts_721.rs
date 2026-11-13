//! §3.12.1 / §16.8 / §16.12: used unit assertions keep dependencies and local shadowing.

#[test]
fn unit_properties_keep_sequences_and_module_shadowing() {
    let sim = xezim::simulate(include_str!("unit_assertion_contexts_721.sv"), 100)
        .expect("simulate unit assertions");
    assert_eq!(
        sim.output
            .iter()
            .filter(|o| o.message == "PASS unit assertions")
            .count(),
        1,
    );
}
