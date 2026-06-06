//! `bind` directive (IEEE 1800-2023 §23.11) regression — lightweight
//! "attach a monitor module to an untouched DUT" use case.
//!
//! Validates that `bind <target_module> <bind_module> <inst>(<ports>);`
//! at compilation-unit scope causes the bind_module to be elaborated
//! inside every instance of target_module — so initial blocks fire,
//! assertions count, and ports route correctly. The DUT source is
//! syntactically unchanged.

use xezim::simulate;

const SRC: &str = r#"
module cpu(input bit clk, input int pc);
endmodule

module pc_monitor(input bit clk, input int pc);
  initial begin
    @(posedge clk);
    // pc was 10 at the first posedge.
    assert (pc == 10);
    @(posedge clk);
    assert (pc == 20);
    @(posedge clk);
    // intentional fail — proves bind-side asserts actually run.
    assert (pc == 999);
  end
endmodule

bind cpu pc_monitor mon (.clk(clk), .pc(pc));

module tb;
  bit clk = 0;
  int pc = 0;
  cpu u_cpu(.clk(clk), .pc(pc));
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
fn bound_module_elaborates_and_asserts() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    // 3 asserts ran (one per posedge after pc was set), 2 pass + 1 fail.
    assert_eq!(
        sim.assertion_site_count(), 3,
        "expected 3 assertion sites from the bound monitor, got {}",
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
