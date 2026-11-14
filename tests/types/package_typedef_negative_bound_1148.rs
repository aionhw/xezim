//! IEEE 1800-2023 §26.3, §6.18, §7.4.2: a negative package constant in a
//! package typedef's range keeps its value when the type is used as
//! `pkg::T`, and a same-named parameter of another top-level module does not
//! capture it.
#[test]
fn package_typedef_negative_bounds() {
    let sim = xezim::simulate(
        include_str!("package_typedef_negative_bound_1148.sv"),
        1_000,
    )
    .expect("simulate package typedef bounds");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|o| o.message.trim())
        .filter(|s| s.starts_with("T|"))
        .collect();
    assert_eq!(
        lines,
        ["T|array -1 3 5", "T|member -1 3 6", "T|typed -2 -1 7"]
    );
}
