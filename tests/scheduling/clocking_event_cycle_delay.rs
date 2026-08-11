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
fn cycle_delay_zero_synchronizes_to_the_clocking_event() {
    // §14.11: `##0` SYNCHRONIZES to the clocking event — it waits for the
    // edge when the process is not executing at one, and is a no-op when it
    // is. Reference-measured (this test previously pinned `Z t=0`, i.e.
    // "never waits", without reference validation — the reference waits).
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
    assert!(out.contains("Z t=5"), "##0 off-edge waits for the event (reference: 5):\n{}", out);
    assert!(out.contains("Z3 t=35"), "##3 then waits three posedges (reference: 35):\n{}", out);
}

#[test]
fn cycle_delay_zero_at_the_event_is_a_no_op() {
    // Reference: not_edge=5 (##0 at t=2 waits), at_edge=5 (##0 executed at
    // the event's own slot does not wait again). Standalone `default
    // clocking cb;` designation form (§14.12).
    const SRC: &str = r#"
`timescale 1ns/1ns
module top;
  logic clk = 0;
  int at_edge = -1, not_edge = -1;
  clocking cb @(posedge clk);
  endclocking
  default clocking cb;
  always #5 clk = ~clk;
  initial begin
    #2;
    ##0;
    not_edge = $time;
    ##0;
    at_edge = $time;
    $display("NE=%0d AE=%0d", not_edge, at_edge);
    $finish;
  end
endmodule
"#;
    let out = output_of(&simulate(SRC, 200).expect("sim"));
    assert!(out.contains("NE=5 AE=5"), "##0 waits off-edge, no-ops at the edge:\n{}", out);
}

#[test]
fn cycle_delay_single_undesignated_clocking_is_rejected() {
    // No `default` keyword, even with only one clocking block in scope.
    // xezim used to fall back to that sole block; §14.11 requires the
    // designation and the reference simulator errors with "A default
    // clocking block must be specified to use the ##n timing statement",
    // so the lenience made xezim accept code the reference rejects.
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
    let err = match simulate(SRC, 200) {
        Ok(_) => panic!("##1 without a `default` clocking block must be rejected"),
        Err(e) => e,
    };
    assert!(
        err.contains("default clocking"),
        "diagnostic should name the missing default clocking block, got: {err}"
    );
}
