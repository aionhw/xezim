//! §8.25 / §6.18: method formals and locals typed by a class typedef or a
//! packed range over a class parameter or localparam take the object's
//! specialization width, in interpreted and in compiled methods. They came
//! out one bit wide, so a memory model lost all but one mask bit.

fn t_lines(src: &str) -> Vec<String> {
    let sim = xezim::simulate(src, 100).expect("simulate");
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .filter(|l| l.starts_with("T|"))
        .collect()
}

#[test]
fn class_param_sized_formals_and_locals() {
    let expected = [
        "T|mem|4 12345678",
        "T|d32_formal|4 4 32",
        "T|d32_local|4 4 32 32",
        "T|d32_formal_val|f f",
        "T|d64_formal|8 8 64",
        "T|d64_local|8 8 64 64",
        "T|d64_formal_val|ff ff",
        "T|hot_pass|0 0 0 1",
        "T|hot_write|3004 01 03",
    ];
    assert_eq!(
        t_lines(include_str!("class_param_sized_method_widths.sv")),
        expected
    );
}
