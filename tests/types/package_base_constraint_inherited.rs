//! Regression: a class declared inside a MODULE body whose base class lives
//! in an imported PACKAGE (`mypacket extends packet`, where `packet` is a
//! `packet_pkg` class), with a constraint that references an INHERITED
//! property.
//!
//! `collect_class_member_names` walks the `extends` chain via
//! `defs.get(base_name)`, but packages are kept whole in the definition map
//! (`Definition::Package`), NOT flattened into top-level `Definition::Class`
//! entries. So a bare `extends packet` lookup missed `packet` (it lives inside
//! `packet_pkg`), and the inherited property `addr` never joined the constraint
//! validator's allowed set — the module-declared subclass was wrongly rejected
//! at compile time with "Undeclared identifier 'addr' in class constraint",
//! aborting the whole simulation and failing the UVM print() overload
//! UVM test.
use xezim::simulate;

fn output_of(sim: &xezim::compiler::Simulator) -> String {
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn module_class_extends_package_class_constraint_uses_inherited_property() {
    const SRC: &str = r#"
package packet_pkg;
  class packet;
    rand int addr;
    constraint c1 { addr inside { [0:40] }; }
  endclass
endpackage

module top;
  import packet_pkg::*;
  class mypacket extends packet;
    constraint ct10 { addr inside { [5:10] }; }
  endclass
  mypacket p;
  initial begin
    p = new();
    if (p.randomize()) $display("RAND addr=%0d", p.addr);
    else $display("RAND FAIL");
  end
endmodule
"#;
    // The constraint c1 (addr within [0,40]) AND ct10 (addr within [5,10])
    // both apply to the inherited `addr`, which must resolve to the base
    // packet_pkg class's property — not be rejected as undeclared (the
    // pre-fix behavior aborted the whole simulation at compile time).
    // `inside` ranges are used so the test asserts the package-base scoping
    // fix in isolation from the solver's relational-constraint limitations.
    for _ in 0..4 {
        let out = output_of(&simulate(SRC, 100).expect("sim"));
        assert!(
            out.contains("RAND addr="),
            "constraint compile failure (inherited addr rejected):\n{}",
            out
        );
        let v: i64 = out
            .lines()
            .find_map(|l| l.strip_prefix("RAND addr="))
            .and_then(|s| s.trim().parse().ok())
            .expect("parse addr");
        assert!(
            (5..=10).contains(&v),
            "inherited constraint ct10 (5..=10) violated: got {}",
            v
        );
    }
}