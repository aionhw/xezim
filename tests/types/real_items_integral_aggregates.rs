//! IEEE 1800-2023 §6.12.1, §10.9: real pattern items of aggregates with 64-bit
//! integral elements or members are rounded.
#[test]
fn real_items_integral_aggregates() {
    let sim = xezim::simulate(include_str!("real_items_integral_aggregates.sv"), 1000)
        .expect("simulate real_items_integral_aggregates");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|o| o.message.trim())
        .filter(|s| s.starts_with("T|"))
        .collect();
    assert_eq!(
        lines,
        [
            "T|arr 9200000000000 9200000000000 la 3 -3 lv 3 -3 ia 3 -3",
            "T|s0 9200000000000 3000000000 -3 3",
            "T|s1 9200000000000 3000000000 -3 -3",
            "T|ls 3 -3 -3 2",
            "T|q 9200000000000 2 d 3 -3 aa 3 -3 p2 -3 -5",
            "T|arr2 2 3 -2 -3 def 3 3 loc 3 -3",
        ]
    );
}
