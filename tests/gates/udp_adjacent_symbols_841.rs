//! A.5.3, §29.3.4 (#841): each symbol of a UDP table entry is one
//! character, so a row may be written `01:0;` as well as `0 1 : 0;`. The
//! lexer joins adjacent symbols into one token (`01`, `bB`, `x0`), and a
//! table with such a row was rejected: the primitive became an empty module
//! and its output stayed z. The fixture has every primitive twice, with and
//! without white space between the symbols, and checks both.

use xezim::simulate;

#[test]
fn adjacent_udp_table_symbols_are_separate_symbols() {
    let src = include_str!("udp_adjacent_symbols_841.sv");
    let sim = simulate(src, 10_000).expect("simulate failed");
    let lines: Vec<&str> = sim.output.iter().map(|o| o.message.trim()).collect();
    assert!(!lines.iter().any(|l| l.starts_with("FAIL")), "{lines:#?}");
    assert_eq!(lines.last().copied(), Some("ALL PASS"), "{lines:#?}");
}
