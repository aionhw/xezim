//! §6.18, §8.25: a class typedef of a class typedef sized by a class
//! parameter (`typedef word_t alias_t;`) names the same type, so formals,
//! locals and returns declared with it take the receiver's specialization
//! width instead of one bit. Expected values are the reference simulator's.

#[test]
fn class_param_typedef_chain_widths() {
    let _ = super::compiled_method_test_env::policies(&[("1", "0"), ("0", "0")]);
    let sim = xezim::simulate(include_str!("class_param_typedef_chain.sv"), 100).expect("simulate");
    let lines: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect();
    assert_eq!(lines, ["T|w12 12 12 12 fff", "T|w20 20 20 20 fffff"]);
}
