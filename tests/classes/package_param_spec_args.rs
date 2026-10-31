//! §8.25 / §25.9: a value parameter written as a constant expression of
//! package parameters (`cfg_pkg::N + cfg_pkg::K`) keys a specialization by
//! its value. Such keys used to keep the expression text, so a
//! `uvm_config_db#(virtual if#(.D(pkg::N + pkg::K)))` set never reached the
//! agent's `db#(virtual if#(H, D))` get.

fn t_lines(src: &str) -> Vec<String> {
    let sim = xezim::simulate(src, 100).expect("simulate");
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .filter(|l| l.starts_with("T|"))
        .collect()
}

#[test]
fn package_param_expression_keys_by_value() {
    let expected = [
        "T|lit|1",
        "T|pos|1",
        "T|scoped|1",
        "T|imported|1",
        "T|inline_lit|1",
        "T|inline_expr|1",
        "T|other|0",
        "T|agent_expr|1",
        "T|agent_scoped|1",
        "T|agent_pos|1",
        "T|agent_lit|1",
        "T|agent_other|0",
        "T|agent_get|1",
        "T|vif_width|192",
    ];
    assert_eq!(t_lines(include_str!("package_param_spec_args.sv")), expected);
}
