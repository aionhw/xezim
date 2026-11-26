//! IEEE 1800-2023 §6.12: "The realtime declarations shall be treated
//! synonymously with real declarations and can be used interchangeably."
//! §10.9.1 (arrays) and §10.9.2 (structures): the last matching type key wins,
//! before `default:`. `shortreal` stays a different type.
#[test]
fn real_and_realtime_keys_match_as_one_type() {
    let sim = xezim::simulate(
        include_str!("pattern_type_key_real_realtime_1290.sv"),
        10_000,
    )
    .expect("legal real and realtime type keys");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|line| line.message.as_str())
        .filter(|line| line.starts_with("T|"))
        .collect();
    assert_eq!(
        lines,
        [
            "T|array 2.5 2.5",
            "T|realtime array 2.5 2.5",
            "T|one key 1.5 1.5",
            "T|struct 2.5 2.5 3.5 7",
            "T|struct later 2.5 2.5 3.5 7",
            "T|localparam 2.5 2.5",
        ]
    );
}
