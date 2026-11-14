//! IEEE 1800-2023 §6.19, §7.4.1, §10.3.3: continuous-assignment
//! delays follow an enum's base shape, including one-bit vectors. Scalar
//! enum controls retain §28.16's minimum delay for transitions to x.

#[test]
fn enum_continuous_assignments_keep_the_base_shape() {
    let sim = xezim::simulate(include_str!("enum_continuous_delay.sv"), 200_000)
        .expect("legal enum continuous assignments");
    let output: Vec<_> = sim.output.iter().map(|o| o.message.trim()).collect();
    assert!(
        !output.iter().any(|line| line.starts_with("FAIL ")),
        "{output:#?}"
    );
    assert_eq!(
        output
            .iter()
            .filter(|line| line.starts_with("PASS "))
            .count(),
        25,
        "{output:#?}"
    );
}
