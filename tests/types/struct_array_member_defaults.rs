//! IEEE 1800-2023 §7.2.2: elements of a module-scope fixed array of unpacked
//! structures start with the members' declared defaults.
#[test]
fn struct_array_member_defaults() {
    let sim = xezim::simulate(include_str!("struct_array_member_defaults.sv"), 1000)
        .expect("simulate struct_array_member_defaults");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|o| o.message.trim())
        .filter(|s| s.starts_with("T|"))
        .collect();
    assert_eq!(
        lines,
        [
            "T|module 1.50 2.50 -3 1.50 2.50 -3",
            "T|two 1.50 -3",
            "T|scalar 1.50 -3",
            "T|local 1.50 2.50",
            "T|written 4.00 1.50",
        ]
    );
}
