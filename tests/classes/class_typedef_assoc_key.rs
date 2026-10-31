//! §8.25 / §6.18: an associative array key typed by a class typedef over a
//! class parameter takes each object's specialization width. It came out
//! one bit wide, so distinct addresses collapsed onto keys 0 and 1.

fn t_lines(src: &str) -> Vec<String> {
    let sim = xezim::simulate(src, 100).expect("simulate");
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .filter(|l| l.starts_with("T|"))
        .collect()
}

#[test]
fn class_typedef_key_takes_the_specialization_width() {
    let expected = [
        "T|num|4 2 4",
        "T|copy|2 3",
        "T|kd_key|80000000",
        "T|kd_key|80000001",
        "T|kd_key|80000002",
        "T|kd_key|80000003",
        "T|k8_key|0",
        "T|k8_key|ff",
        "T|k40_key|8000000000",
        "T|k40_key|8000000001",
        "T|k40_key|8000000002",
        "T|k40_key|8000000003",
    ];
    assert_eq!(t_lines(include_str!("class_typedef_assoc_key.sv")), expected);
}
