//! IEEE 1800-2023 §12.5, §11.8.2: a real case expression or item compares
//! numerically, interpreted and compiled.
#[test]
fn real_case_items() {
    let sim = xezim::simulate(include_str!("real_case_items.sv"), 1000)
        .expect("simulate real_case_items");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|o| o.message.trim())
        .filter(|s| s.starts_with("T|"))
        .collect();
    assert_eq!(
        lines,
        [
            "T|interpreted 1 1 1 0 1",
            "T|compiled 0 0 1 1 1",
            "T|compiled 1 1 1 1 0",
        ]
    );
}
