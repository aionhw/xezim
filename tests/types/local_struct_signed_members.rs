//! IEEE 1800-2023 §6.11.3, §6.12.1, §7.2: a signed member of a procedural-block
//! unpacked structure reads signed, also after a real is rounded into it.
#[test]
fn local_struct_signed_members() {
    let sim = xezim::simulate(include_str!("local_struct_signed_members.sv"), 1000)
        .expect("simulate local_struct_signed_members");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|o| o.message.trim())
        .filter(|s| s.starts_with("T|"))
        .collect();
    assert_eq!(
        lines,
        [
            "T|real -2 -2 -2 -2",
            "T|int -2 -2",
            "T|arith -5 1",
            "T|function -2",
            "T|named -2",
        ]
    );
}
