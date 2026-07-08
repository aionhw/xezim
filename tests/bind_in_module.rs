//! `bind` directive written as a *module item* (IEEE 1800-2023 §23.11),
//! rather than at compilation-unit scope. Previously xezim parsed an
//! in-module `bind` and silently discarded it (→ `ModuleItem::Null`), so the
//! bound module was never elaborated. It now attaches to every instance of
//! the target module, exactly like a top-level bind.

use xezim::simulate;

const SRC: &str = r#"
module cpu(input bit clk, input int pc);
endmodule

module pc_monitor(input bit clk, input int pc);
  initial begin
    @(posedge clk);
    assert (pc == 10);
    @(posedge clk);
    assert (pc == 20);
    @(posedge clk);
    assert (pc == 999);   // intentional fail — proves the bound assert runs
  end
endmodule

module tb;
  bit clk = 0;
  int pc = 0;
  cpu u_cpu(.clk(clk), .pc(pc));
  // bind INSIDE the module body (not at $unit scope):
  bind cpu pc_monitor mon (.clk(clk), .pc(pc));
  always #5 clk = ~clk;
  initial begin
    pc = 10; #10;
    pc = 20; #10;
    pc = 30; #10;
    $finish;
  end
endmodule
"#;

#[test]
fn in_module_bind_elaborates_and_asserts() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(
        sim.assertion_site_count(), 3,
        "expected 3 assertion sites from the in-module bound monitor, got {}",
        sim.assertion_site_count()
    );
    assert_eq!(
        sim.assertion_pass_total(), 2,
        "expected 2 passes (pc==10, pc==20), got {}",
        sim.assertion_pass_total()
    );
    assert_eq!(
        sim.assertion_fail_total(), 1,
        "expected 1 fail (pc==999), got {}",
        sim.assertion_fail_total()
    );
}
