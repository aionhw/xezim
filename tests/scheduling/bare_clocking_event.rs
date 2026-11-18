//! §14.3 — the BARE `@cb` form of a clocking-block event.
//!
//! `@(cb)` (parenthesized) parses as a one-term `EventExpr`, whose sensitivity
//! builder resolves a clocking-block name to the block's clock edge. The bare
//! `@cb` parses as a `HierIdentifier` instead, and that arm had no such
//! resolution — it built a sensitivity on a signal literally named `cb`, found
//! no signal id, and fell into the "not a real signal" delta-yield. So `@cb`
//! returned at t=0 without waiting for a clock edge at all.
//!
//! That silently broke every `@cb;` sampling loop: the loop spun through its
//! iterations inside time 0, the clocking block's `#1step` inputs never
//! sampled a new edge, and outputs driven after it landed at the wrong time.
//! The parenthesized spelling in the same testbench worked, which made the
//! failure look like a sampling bug rather than a wait that never waited.
//!
//! Verified byte-identical to a reference simulator.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("top.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able (x/z?)", n))
}

/// Both spellings must advance to the block's clock edge.
#[test]
fn bare_and_parenthesized_clocking_events_both_wait() {
    let src = r#"
`timescale 1ns/1ns
module top;
  logic clk = 0;
  always #5 clk = ~clk;
  clocking cb @(posedge clk);
  endclocking
  int t_bare, t_paren;
  initial begin @cb;   t_bare  = $time; end
  initial begin @(cb); t_paren = $time; end
  initial #30 $finish;
endmodule
"#;
    let sim = simulate(src, 200).expect("simulate failed");
    assert_eq!(u(&sim, "t_bare"), 5, "bare @cb waits for the clocking edge");
    assert_eq!(u(&sim, "t_paren"), 5, "as does @(cb)");
}

/// §14.10, §9.4.2: `@cb` waits for the block's clocking event on the edge
/// the block declares, as `@(cb)` does.
#[test]
fn bare_clocking_event_waits_for_the_declared_edge() {
    let src = r#"
`timescale 1ns/1ns
module top;
  logic clk = 0;
  always #5 clk = ~clk;
  clocking cbn @(negedge clk);
  endclocking
  clocking cbe @(edge clk);
  endclocking
  int t_neg, t_paren, t_rep, t_any;
  initial begin @cbn;   t_neg   = $time; end
  initial begin @(cbn); t_paren = $time; end
  initial begin repeat (2) @cbn; t_rep = $time; end
  initial begin #6 @cbe; t_any = $time; end
  initial #40 $finish;
endmodule
"#;
    let sim = simulate(src, 200).expect("simulate failed");
    assert_eq!(u(&sim, "t_neg"), 10, "@cbn waits for the negedge");
    assert_eq!(u(&sim, "t_paren"), 10, "as does @(cbn)");
    assert_eq!(u(&sim, "t_rep"), 20, "repeat (2) @cbn waits two negedges");
    assert_eq!(u(&sim, "t_any"), 10, "@cbe wakes on either edge");
}

/// The same in an interface task, called directly and through a virtual
/// interface.
#[test]
fn bare_clocking_event_in_an_interface_task_waits_for_the_declared_edge() {
    let src = r#"
`timescale 1ns/1ns
interface clk_if (input logic clk);
  clocking cb @(posedge clk);
  endclocking
  clocking cbn @(negedge clk);
  endclocking
  task automatic wait_n_clks(int n);
    repeat (n) @cbn;
  endtask
endinterface
module top;
  logic clk = 0;
  always #5 clk = ~clk;
  clk_if u_if (.clk(clk));
  virtual clk_if vif;
  int t_task, t_vif;
  initial begin
    vif = u_if;
    u_if.wait_n_clks(2); t_task = $time;
    vif.wait_n_clks(1);  t_vif  = $time;
    $finish;
  end
endmodule
"#;
    let sim = simulate(src, 200).expect("simulate failed");
    assert_eq!(u(&sim, "t_task"), 20, "two negedges from t=0");
    assert_eq!(u(&sim, "t_vif"), 30, "one more negedge through the vif");
}

/// Successive `@cb` steps advance one clock each, rather than all firing at
/// t=0 — the shape a sampling loop actually uses.
#[test]
fn successive_bare_clocking_events_step_the_clock() {
    let src = r#"
`timescale 1ns/1ns
module top;
  logic clk = 0;
  always #5 clk = ~clk;
  clocking cb @(posedge clk);
  endclocking
  int t1, t2, t3;
  initial begin
    @cb; t1 = $time;
    @cb; t2 = $time;
    @cb; t3 = $time;
    $finish;
  end
endmodule
"#;
    let sim = simulate(src, 200).expect("simulate failed");
    assert_eq!(u(&sim, "t1"), 5, "first edge");
    assert_eq!(u(&sim, "t2"), 15, "second");
    assert_eq!(u(&sim, "t3"), 25, "third");
}

/// §14.4: with the wait fixed, the `#1step` input skew samples the value
/// immediately BEFORE the edge, not the one after it.
#[test]
fn bare_clocking_event_samples_inputs_at_the_edge() {
    let src = r#"
`timescale 1ns/1ns
module top;
  logic clk = 0;
  logic [7:0] d = 8'h11;
  always #5 clk = ~clk;
  clocking cb @(posedge clk);
    default input #1step;
    input d;
  endclocking
  int s1, s2;
  initial begin
    #12 d = 8'h22;
    #1  d = 8'h33;   // t=13, just before the t=15 edge
  end
  initial begin
    @cb; s1 = cb.d;
    @cb; s2 = cb.d;
    #10 $finish;
  end
endmodule
"#;
    let sim = simulate(src, 200).expect("simulate failed");
    assert_eq!(u(&sim, "s1"), 0x11, "first edge samples the pre-edge value");
    assert_eq!(
        u(&sim, "s2"),
        0x33,
        "second edge sees the update made before it"
    );
}

/// An ordinary named event called `e` must still behave as an event — the
/// clocking substitution only applies to names that are clocking blocks.
#[test]
fn a_plain_named_event_is_not_treated_as_clocking() {
    let src = r#"
`timescale 1ns/1ns
module top;
  logic clk = 0;
  always #5 clk = ~clk;
  clocking cb @(posedge clk);
  endclocking
  event e;
  int t_ev;
  initial begin @e; t_ev = $time; end
  initial begin #22 -> e; end
  initial #40 $finish;
endmodule
"#;
    let sim = simulate(src, 200).expect("simulate failed");
    assert_eq!(
        u(&sim, "t_ev"),
        22,
        "@e still waits for the event, not a clock edge"
    );
}
