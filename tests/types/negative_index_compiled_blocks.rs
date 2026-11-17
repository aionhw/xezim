//! IEEE 1800-2023 §7.4.2: an unpacked array range may have negative bounds,
//! and §7.4.6: an index outside the range reads the element type's default.
//! Continuous assignments, `always_comb`, `always @*` and `always_ff` blocks
//! run as bytecode, whose element instructions read the index register's
//! bits as an unsigned number: `arr[-1]`, or `arr[k]` with `int k = -1`,
//! missed the element (a read gave x, a write was lost) while procedural
//! code was right. A signed index is now sign-extended before the access.

#[test]
fn negative_indices_in_compiled_blocks() {
    let sim = xezim::simulate(include_str!("negative_index_compiled_blocks.sv"), 1_000)
        .expect("simulate negative array indices");
    let got: Vec<_> = sim
        .output
        .iter()
        .map(|line| line.message.as_str())
        .filter(|line| line.starts_with("T|"))
        .collect();
    assert_eq!(
        got,
        [
            "T|reads 5 605 43 5 33",
            "T|comb 605 5 0",
            "T|writes 8 7 75",
            "T|after 9 607 609 7",
        ]
    );
}
