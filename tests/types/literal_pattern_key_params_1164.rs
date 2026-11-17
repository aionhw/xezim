//! IEEE 1800-2023 §10.9.1, §11.6.1, §11.8.1 and §6.20.2: an array pattern
//! key is self-determined, and a parameter operand of the key keeps its
//! declared width and sign, in a localparam array, a packed constant, a
//! continuous assignment, a structure member pattern and a packed target.
//! The reference simulator reads the 4-bit signed key `S7 + 4'sd1` as
//! index 8; its value is -8, which this follows.
#[test]
fn parameter_pattern_keys_keep_their_type() {
    let sim = xezim::simulate(include_str!("literal_pattern_key_params_1164.sv"), 100_000)
        .expect("simulate parameter pattern keys");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|o| o.message.trim())
        .filter(|s| !s.is_empty() && !s.starts_with("Simulation"))
        .collect();
    assert_eq!(
        lines,
        [
            "localparam 7 0 7 0 0000000000000001",
            "continuous 7 0 7 0 7 0",
            "package 7 0",
            "signed_cast 7 0",
            "member 7 0",
            "packed 0000000000000001",
        ]
    );
}
