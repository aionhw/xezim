//! IEEE 1800-2023 §20.8.1, §11.4.11, §6.20.2, §6.11.2, §6.11.3, §11.4.13,
//! §20.9, §6.12.2, §11.8.2, §13.4.1 and §12.5: constant expressions whose
//! values changed for the worse with the typed constant evaluator (#1143).
//! The expected values are the reference simulator's.
#[test]
fn constant_expression_types() {
    let sim = xezim::simulate(include_str!("constant_expression_types_1143.sv"), 100_000)
        .expect("simulate constant expressions");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|o| o.message.trim())
        .filter(|s| !s.is_empty() && !s.starts_with("Simulation"))
        .collect();
    assert_eq!(
        lines,
        [
            "7 inside_arm_bits 16",
            "4 two_state_generate 0",
            "1 integer_system_result 4",
            "2 known_one_condition 111",
            "3 class_int_localparam -2",
            "6 unsigned_package_variable 7",
            "8 isunknown 0",
            "9 wide_to_real 1.84467e+19",
            "11 real_exponent_arm 2",
            "12 call_width 37 4",
            "14 case_width 1",
        ]
    );
}
