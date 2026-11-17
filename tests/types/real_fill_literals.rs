//! IEEE 1800-2023 §5.7.1, §11.4.4, §11.4.5, §11.4.13, §11.8.2: an unbased
//! unsized literal has one bit when converted to real, shortreal or realtime.
#[test]
fn real_fill_literals() {
    let sim = xezim::simulate(include_str!("real_fill_literals.sv"), 100)
        .expect("simulate real fill literals");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|o| o.message.trim())
        .filter(|s| s.starts_with("T|"))
        .collect();
    assert_eq!(
        lines,
        [
            "T|equal 0 1 0 1",
            "T|unequal 1 0 1 0",
            "T|continuous 0 1 0 1",
            "T|types 1 1 1 1",
            "T|zero 0 1 1 1",
            "T|arithmetic 3.0 2.0 2.0 1.0",
            "T|function 3.0",
            "T|element 2",
            "T|comb 3.0",
            "T|clocked 3.0",
            "T|package 3.0",
            "T|parameter 3.0",
            "T|relations 1 1 1 1",
            "T|range 0",
            "T|integral 1 1 fffffffffffffffe 0000000000000001",
            "T|case 0",
        ]
    );
}
