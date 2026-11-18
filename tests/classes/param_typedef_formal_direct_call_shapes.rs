//! §8.25, §13.5.1: a compiled method that calls another compiled method
//! directly binds each argument at the formal's declared width in the
//! receiver's specialization, as the interpreter does (#1271): a class
//! typedef sized by a class parameter, a typedef of that typedef, a signed
//! one, a packed formal sized by the parameter, the one-statement setter,
//! and explicit and inherited specializations. Expected values are the
//! reference simulator's.

#[test]
fn param_typedef_formal_direct_call_shapes() {
    let _ = super::compiled_method_test_env::policies(&[("1", "0"), ("1", "1000"), ("0", "0")]);
    let sim = xezim::simulate(
        include_str!("param_typedef_formal_direct_call_shapes.sv"),
        100,
    )
    .expect("simulate");
    let lines: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect();
    assert_eq!(lines, ["T|bad='{0, 0, 0, 0, 0, 0, 0, 0, 0}"]);
}
