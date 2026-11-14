//! IEEE 1800-2023 §10.9.1, §5.7.1 and §11.8.2: literal keys retain
//! their width and sign in constant arrays and packed/unpacked patterns.
#[test]
fn literal_pattern_key_sites() {
    let sim = xezim::simulate(include_str!("literal_pattern_key_sites.sv"), 100_000)
        .expect("simulate literal pattern keys");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|o| o.message.trim())
        .filter(|s| !s.is_empty() && !s.starts_with("Simulation"))
        .collect();
    assert_eq!(
        lines,
        [
            "localparam 0 7 0 7",
            "parameter 0 7 0 7",
            "packed_constant 10000 10000",
            "packed_initializer 10000 10000",
            "packed_continuous 10000 10000",
            "packed_procedural 10000 10000",
            "unsigned_index 7 5",
            "width 7 7 7",
            "divide_remainder 7 7",
            "signed_high_bit 7 7",
            "shift_count 0 7 7 0",
        ]
    );
}
