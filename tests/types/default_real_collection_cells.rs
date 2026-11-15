//! IEEE 1800-2023 §6.8 Table 6-7, §7.5.1, §7.8.6, §11.3.1: class property
//! arrays, dynamic arrays sized by `new[]` and nonexistent associative-array
//! entries of real and shortreal elements default to real 0.0, so arithmetic
//! on them is real. They read as an integral 0, as fixed module arrays did
//! before #1153.

#[test]
fn default_real_collection_cells() {
    let sim = xezim::simulate(include_str!("default_real_collection_cells.sv"), 1_000)
        .expect("simulate real collection defaults");
    let got: Vec<_> = sim
        .output
        .iter()
        .map(|line| line.message.as_str())
        .filter(|line| line.starts_with("T|"))
        .collect();
    assert_eq!(
        got,
        [
            "T|class 0.500000 0.500000",
            "T|class_dyn 0.500000 class_assoc 0.500000",
            "T|dyn 0.500000 0.500000 assoc 0.500000",
            "T|resized 0.500000",
        ]
    );
}
