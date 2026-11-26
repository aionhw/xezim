//! IEEE 1800-2023 §7.3: a real member of an unpacked untagged union keeps the
//! real; the union's storage holds its IEEE 754 bits.
#[test]
fn union_real_member() {
    let sim = xezim::simulate(include_str!("union_real_member.sv"), 1000)
        .expect("simulate union_real_member");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|o| o.message.trim())
        .filter(|s| s.starts_with("T|"))
        .collect();
    assert_eq!(
        lines,
        [
            "T|module 1.50 -2.25 c002000000000000",
            "T|alias 2.50",
        ]
    );
}
