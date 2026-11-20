//! IEEE 1800-2023 §7.2, §7.4, §10.9.1, §6.18: whole-array writes to array members
//! of procedural-block unpacked structures, and members typed by array typedefs.
#[test]
fn local_struct_member_arrays() {
    let sim = xezim::simulate(include_str!("local_struct_member_arrays.sv"), 1000)
        .expect("simulate local_struct_member_arrays");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|o| o.message.trim())
        .filter(|s| s.starts_with("T|"))
        .collect();
    assert_eq!(
        lines,
        [
            "T|elem 0.50 0.06",
            "T|dflt 2.50 2.50",
            "T|int 3 3",
            "T|ordered 4 5",
            "T|func 5.00",
            "T|atd 1.50",
            "T|atd2 0.25 0.75",
            "T|mod 1.50 2.50",
        ]
    );
}
