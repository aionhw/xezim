//! IEEE 1800-2023 §10.9.1: "If more than one type matches the same element,
//! the last value shall be used." §6.12: realtime is treated synonymously with
//! real. §6.22.1: an imported typedef of `int` does not match `integer`.
fn check(src: &str, expected: &[&str]) {
    let sim = xezim::simulate(src, 10_000).expect("legal type-key patterns");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|line| line.message.as_str())
        .filter(|line| line.starts_with("T|"))
        .collect();
    assert_eq!(lines, expected);
}

#[test]
fn real_and_realtime_keys_take_the_last_match() {
    check(
        include_str!("pattern_type_key_last_match_1290_real.sv"),
        &["T|a 9.0 9.0", "T|b 9.0 9.0", "T|d 3.0 9.0"],
    );
}

#[test]
fn later_scoped_typedef_key_takes_the_last_match() {
    check(
        include_str!("pattern_type_key_last_match_1290_scoped.sv"),
        &[
            "T|class 99 99 99",
            "T|generate 99 99 99",
            "T|generate index 99 99 5",
        ],
    );
}

#[test]
fn imported_int_typedef_key_does_not_match_integer() {
    check(
        include_str!("pattern_type_key_last_match_1290_import.sv"),
        &["T|net 99 99", "T|variable 99 99"],
    );
}
