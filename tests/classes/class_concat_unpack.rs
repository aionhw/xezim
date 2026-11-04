use xezim::simulate;

#[test]
fn concatenation_assignment_uses_declared_class_property_widths() {
    let sim = simulate(include_str!("../fixtures/class_concat_unpack.sv"), 1000).unwrap();
    let out = sim
        .output
        .iter()
        .map(|o| o.message.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(out.contains("PASS") && !out.contains("Fatal"), "{out}");
}
