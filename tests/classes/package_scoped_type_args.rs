//! §6.22.1 / §8.25: `db#(p::e_t)` and `db#(e_t)` under `import p::*` name
//! one specialization. The package-scoped spelling used to key its own
//! table, so a `uvm_config_db#(pkg::enum_t)::set` missed the get written
//! with the imported name.

fn t_lines(src: &str) -> Vec<String> {
    let sim = xezim::simulate(src, 100).expect("simulate");
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .filter(|l| l.starts_with("T|"))
        .collect()
}

#[test]
fn package_scoped_type_arg_matches_imported_name() {
    let expected = [
        "T|enum|1",
        "T|enum_val|1",
        "T|struct|1",
        "T|vec|1",
        "T|class|1",
        "T|pclass|1",
        "T|pclass_other|0",
        "T|q_w|0",
    ];
    assert_eq!(t_lines(include_str!("package_scoped_type_args.sv")), expected);
}
