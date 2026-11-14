//! IEEE 1800-2023 §6.8 Table 6-7, §7.4.2/§7.4.4 and §11.3.1:
//! default real/shortreal array cells must carry real 0.0 into arithmetic.

#[test]
fn default_real_array_cells() {
    let sim = xezim::simulate(include_str!("default_real_array_cells.sv"), 10_000)
        .expect("simulate real array defaults");
    let got: Vec<_> = sim
        .output
        .iter()
        .map(|line| line.message.as_str())
        .filter(|line| line.starts_with("T|"))
        .collect();
    assert_eq!(
        got,
        [
            "T|one 0.500000 0.500000 0.500000 0.500000",
            "T|two 0.500000 0.500000",
            "T|three 0.500000 0.500000",
            "T|child 0.500000 0.500000 0.500000",
            "T|storage 0.500000 0.500000 0.500000",
            "T|arithmetic 1.500000 1.500000 -1.500000 0.250000",
            "T|explicit 0.250000 0.125000 0.250000 0.500000",
            "T|scalar 0.500000 0.500000",
            "T|integral xxxxxxxx 00000000 xxxxxxxx 0",
            "T|stores 0.250000 0.250000 0.500000",
            "T|compound 0.500000",
        ]
    );
}
