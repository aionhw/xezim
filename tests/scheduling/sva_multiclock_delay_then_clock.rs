//! IEEE 1800-2023 §16.13.1, A.2.10: a multiclocked sequence joins singly
//! clocked subsequences with `##1` or `##0`, and `clocking_event
//! sequence_expr` is a sequence_expr, so a cycle delay with no left operand
//! may be followed by a clocking event: `##1 @(posedge clk) c`. The cover
//! below has the shape of one in an open-source I3C target FSM. It was a
//! parse error ("expected RParen, found At") at the `@` after `##1 ##1`.
//!
//! This pins only that the design parses and runs. Clock changes inside a
//! sequence are accepted but not modelled (see the §16.13 known gaps in
//! `tests/lrm/ch16_assertions.rs`), so no cover or assert outcome is checked.

use xezim::simulate;

#[test]
fn delay_then_clocking_event_parses_and_runs() {
    let src = r#"
module t;
  logic clk = 0, clk2 = 0, a = 0, b = 0;
  always #5 clk = ~clk;
  always #7 clk2 = ~clk2;
  property p_mc;
    @(posedge clk)
    ( $rose(a) |=> ##1 b ##1 @(posedge clk2) ##1 ##1 @(posedge clk) ##1 $fell(b) );
  endproperty
  c_mc: cover property (p_mc);
  c_lead: cover property (@(posedge clk) (##1 @(posedge clk2) ##0 b));
  initial begin
    #12 a = 1; b = 1;
    #100 b = 0;
    #100 $display("done");
    $finish;
  end
endmodule
"#;
    let sim = simulate(src, 1000).expect("multiclocked sequence must parse");
    assert!(
        sim.output.iter().any(|o| o.message.trim_end() == "done"),
        "{:?}",
        sim.output
    );
}
