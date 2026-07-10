//! An edge-sensitive `always` block whose only effect is a side effect
//! (`$display`) must re-fire on every edge — not just the first. Such a block
//! compiles to a single `StmtFallback` insn with an empty tracked read-set;
//! the event-driven gateable-skip (on by default) treated "no tracked input
//! changed" as "skippable" and dropped every fire after the first. It is now
//! excluded from gating.

use xezim::simulate;

const SRC: &str = r#"
module tb;
  logic clk = 0;
  int   d = 0;
  always #5 clk = ~clk;
  always @(posedge clk) d <= d + 1;
  // display-only edge block — no signal writes, only a side effect:
  always @(posedge clk) $display("MON t=%0t d=%0d", $time, d);
  initial begin repeat (6) @(posedge clk); $finish; end
endmodule
"#;

#[test]
fn display_only_edge_block_fires_every_edge() {
    let sim = simulate(SRC, 1000).expect("simulate failed");
    let fires = sim
        .output
        .iter()
        .filter(|o| o.message.contains("MON t="))
        .count();
    // 6 posedges → 6 prints. (Before the fix: exactly 1.)
    assert_eq!(
        fires, 6,
        "display-only always block should fire on every posedge, got {}",
        fires
    );
}
