//! §8.25 / §13.5: a formal typed by a class typedef over a class parameter
//! takes each receiver's specialization width, whether the specialization is
//! the default, explicit, or bound through `extends`, with calls on all of
//! them interleaved, and also when the range names a package parameter.

fn t_lines(src: &str) -> Vec<String> {
    let sim = xezim::simulate(src, 100).expect("simulate");
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .filter(|l| l.starts_with("T|"))
        .collect()
}

#[test]
fn class_param_formal_width_per_specialization() {
    let expected = [
        "T|addr|24 32 16 16",
        "T|wide|28 36 20 20",
        "T|pass|0 ffffff ffffffff ffff ffff",
    ];
    assert_eq!(
        t_lines(include_str!("class_param_formal_width_per_spec.sv")),
        expected
    );
}
