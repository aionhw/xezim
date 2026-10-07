//! IEEE 1800-2023 §6.18/§8.23: class aliases preserve member typedefs.

const SRC: &str = include_str!("factory_typedef_chain.sv");

fn check(src: &str) {
    let sim = xezim::simulate(src, 10).expect("factory alias must simulate");
    let out: Vec<_> = sim.output.iter().map(|o| o.message.as_str()).collect();
    assert!(out.contains(&"FACTORY_ALIAS_PASS"), "output: {out:?}");
}

#[test]
fn imported_factory_typedef_chain_executes_registry_body() {
    check(SRC);
}

#[test]
fn qualified_factory_typedef_chain_executes_registry_body() {
    check(&SRC.replace("rm_t::type_id", "env_pkg::rm_t::type_id"));
}

#[test]
fn factory_typedef_chain_inside_class_method() {
    let src = SRC.replace(
        "  rm_t rm;",
        "  class caller;\n    function rm_t make();\n      return rm_t::type_id::create(\"rm\");\n    endfunction\n  endclass\n  caller c;\n  rm_t rm;",
    );
    check(&src.replace(
        "rm = rm_t::type_id::create(\"rm\");",
        "c = new; rm = c.make();",
    ));
}
