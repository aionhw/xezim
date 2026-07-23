//! LRM §16.5.1 / §16.9 — a clocked concurrent assertion must evaluate its
//! predicate against values sampled in the PREPONED region (the start of the
//! clock-tick time slot, BEFORE the active/NBA updates of that slot). A signal
//! written via NBA on the SAME edge the assertion is clocked on must therefore
//! be seen with its OLD value, not the just-committed one.
//!
//! Before this fix xezim evaluated the predicate against post-NBA (live)
//! values, so the assertion was effectively one clock cycle early. The
//! expectations below are locked to a commercial simulator: exactly one
//! failure, at time 5, for `a |-> b` when `b <= a` fires on the same posedge.

use xezim::simulate;

fn out(src: &str) -> String {
    let sim = simulate(src, 1000).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

/// `b` is NBA-assigned `a` on the posedge. The property `a |-> b` samples the
/// PREPONED `b` (still 0 on the first edge) and so must FAIL exactly once, at
/// time 5, then pass on every later edge (preponed b == 1 thereafter).
#[test]
fn preponed_nba_same_edge_fails_once() {
    let o = out(r#"
module top;
  logic clk = 0;
  logic a = 1'b1;
  logic b = 1'b0;
  int   fails = 0;
  always #5 clk = ~clk;
  always @(posedge clk) b <= a;   // NBA on the same edge the assertion fires
  ap: assert property (@(posedge clk) a |-> b)
      else begin
        fails = fails + 1;
        $display("[ASSERT-FAIL] time=%0t", $time);
      end
  initial begin #52; $display("[RESULT] total_fails=%0d", fails); $finish; end
endmodule
"#);
    // Exactly one failure, and it happens at the first posedge (time 5).
    assert!(
        o.contains("[RESULT] total_fails=1"),
        "preponed sampling must fail exactly once (was 0 with post-NBA sampling); got: {}",
        o
    );
    assert!(
        o.contains("[ASSERT-FAIL] time=5"),
        "the single failure must fire at time 5 (the first posedge); got: {}",
        o
    );
}

/// Control: when `b` is a plain register that already holds 1 before the edge
/// (no same-edge write), `a |-> b` never fails — confirms the preponed override
/// is a no-op for signals that do not change on the clock edge.
#[test]
fn preponed_stable_signal_never_fails() {
    let o = out(r#"
module top;
  logic clk = 0;
  logic a = 1'b1;
  logic b = 1'b1;
  int   fails = 0;
  always #5 clk = ~clk;
  ap: assert property (@(posedge clk) a |-> b) else fails = fails + 1;
  initial begin #52; $display("[RESULT] total_fails=%0d", fails); $finish; end
endmodule
"#);
    assert!(
        o.contains("[RESULT] total_fails=0"),
        "a stable true signal must never fail the implication; got: {}",
        o
    );
}
