//! §8.25, §20.6.2: `$bits` of class-parameter-sized types, class typedefs
//! and type-parameter defaults, and packed returns sized by a class
//! parameter, inside methods of several specializations. These widths are
//! read over the design parameter table without copying it on each call
//! (#1276). Expected values are the reference simulator's.

#[test]
fn class_param_width_paths_per_specialization() {
    let _ = super::compiled_method_test_env::policies(&[("1", "0"), ("0", "0")]);
    let sim = xezim::simulate(include_str!("class_param_width_paths.sv"), 100).expect("simulate");
    let lines: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect();
    let round = ["T|d 8 8 8 ff", "T|b16 16 16 16 ffff", "T|b4 4 4 4 f"];
    let expected: Vec<&str> = round.iter().copied().cycle().take(9).collect();
    assert_eq!(lines, expected);
}
