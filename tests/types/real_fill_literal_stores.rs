//! IEEE 1800-2023 §5.7.1, §6.12.1, §10.7: an unbased unsized literal stored into
//! a real target converts at its one bit.
#[test]
fn real_fill_literal_stores() {
    let sim = xezim::simulate(include_str!("real_fill_literal_stores.sv"), 1000)
        .expect("simulate real_fill_literal_stores");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|o| o.message.trim())
        .filter(|s| s.starts_with("T|"))
        .collect();
    assert_eq!(
        lines,
        [
            "T|alw blk 1 nba 1 dly 1 sblk 1 snba 1 rt 1 arr 1 cast 1",
            "T|ini blk 1 nba 1 dly 1 cont 1",
            "T|store q 1 0 a 1 s 1 0 port 1 0",
        ]
    );
}
