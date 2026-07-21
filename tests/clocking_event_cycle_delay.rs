//! LRM §14.3 / §14.11: `@(cb)` naming a clocking block must synchronize to the
//! block's clock event, and procedural `##N` must wait N cycles of the default
//! clocking block. Both were no-ops: `@(cb)` built a sensitivity on a
//! nonexistent signal literally called "cb" (returned at t=0), and `##N`
//! didn't parse as a statement at all. Timing verified against a commercial
//! simulator (t=5/15/25 for @(cb) on a #5 half-period clock; t=15/25 for
//! ##2 / ##1).

use xezim::simulate;

fn output_of(sim: &xezim::compiler::Simulator) -> String {
    sim.output
        .iter()
        .map(|o| o.message.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn clocking_block_event_syncs_to_clock() {
    const SRC: &str = r#"
`timescale 1ns/1ns
module top;
  logic clk = 0;
  logic [7:0] data = 0;
  always #5 clk = ~clk;

  clocking cb @(posedge clk);
    input data;
  endclocking

  initial begin
    data = 8'hAA;
    @(cb); $display("T1 t=%0t", $time);
    @(cb); $display("T2 t=%0t", $time);
    @(cb); $display("T3 t=%0t", $time);
    $finish;
  end
endmodule
"#;
    let out = output_of(&simulate(SRC, 200).expect("sim"));
    for want in ["T1 t=5", "T2 t=15", "T3 t=25"] {
        assert!(out.contains(want), "@(cb) must fire on posedge clk, missing `{}`:\n{}", want, out);
    }
}

#[test]
fn cycle_delay_uses_default_clocking() {
    const SRC: &str = r#"
`timescale 1ns/1ns
module top;
  logic clk = 0;
  always #5 clk = ~clk;
  default clocking cb @(posedge clk);
  endclocking
  initial begin
    ##2;
    $display("HH t=%0t", $time);
    ##1 $display("HH2 t=%0t", $time);
    $finish;
  end
endmodule
"#;
    let out = output_of(&simulate(SRC, 200).expect("sim"));
    assert!(out.contains("HH t=15"), "##2 must wait two posedges:\n{}", out);
    assert!(out.contains("HH2 t=25"), "##1 stmt must wait one more posedge:\n{}", out);
}

#[test]
fn cycle_delay_zero_does_not_wait() {
    const SRC: &str = r#"
`timescale 1ns/1ns
module top;
  logic clk = 0;
  always #5 clk = ~clk;
  default clocking cb @(posedge clk); endclocking
  initial begin
    ##0;
    $display("Z t=%0t", $time);
    ##3;
    $display("Z3 t=%0t", $time);
    $finish;
  end
endmodule
"#;
    let out = output_of(&simulate(SRC, 200).expect("sim"));
    assert!(out.contains("Z t=0"), "##0 must not wait:\n{}", out);
    assert!(out.contains("Z3 t=25"), "##3 must wait three posedges:\n{}", out);
}

#[test]
fn cycle_delay_single_undesignated_clocking_fallback() {
    // No `default` keyword, but only one clocking block in scope — the
    // pragmatic fallback uses it (strict LRM would require the designation).
    const SRC: &str = r#"
`timescale 1ns/1ns
module top;
  logic clk = 0;
  always #5 clk = ~clk;
  clocking cb @(posedge clk); endclocking
  initial begin
    ##1;
    $display("F t=%0t", $time);
    $finish;
  end
endmodule
"#;
    let out = output_of(&simulate(SRC, 200).expect("sim"));
    assert!(out.contains("F t=5"), "##1 must use the sole clocking block:\n{}", out);
}
