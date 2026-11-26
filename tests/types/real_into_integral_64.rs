//! IEEE 1800-2023 §6.12.1, §10.3.2, §10.7, §13.4.1, §13.5, §23.3.3.2: a real stored
//! into a 64-bit integral net or variable by compiled code is rounded (#1319). A
//! real input port actual is an implied continuous assignment, so the port reads
//! the rounded value too (the reference simulator passes the IEEE 754 bits).
#[test]
fn real_into_integral_64() {
    let sim = xezim::simulate(include_str!("real_into_integral_64.sv"), 1000)
        .expect("simulate real_into_integral_64");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|o| o.message.trim())
        .filter(|s| s.starts_with("T|"))
        .collect();
    assert_eq!(
        lines,
        [
            "T|nets 4 9200000000000 4 4 4 4 4 7",
            "T|vars 4 4 4 4 4 4 4 4",
            "T|calls 4 4 4 4",
            "T|nets -3 -3 -3 -3 -3 -3",
            "T|vars -3 -3 -3 -3 3 -3 -3",
            "T|calls -3 -3 -3 -3",
        ]
    );
}
