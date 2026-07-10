//! IEEE 1800-2023 §9.4.2.3 event-control `iff` guard.
//!
//! `@(posedge clk iff rst_l === 1'b1)` must block until a posedge of `clk`
//! that occurs while the guard holds — the classic "wait for the first
//! clock after reset deasserts" idiom. xezim used to parse the `iff` but
//! drop it in `event_to_sens`, so the wait resolved on the very first
//! posedge (still in reset), letting the stimulus start too early.

use xezim::simulate;

const SRC: &str = r#"
`timescale 1ns/1ns
module tb;
  reg clk = 0;
  reg rst_l = 0;
  reg [31:0] woke_at = 0;
  reg        did_wake = 0;
  always #5 clk = ~clk;          // posedges at t = 5, 15, 25, 35, ...

  // Deassert reset at t=22, i.e. between the posedges at t=15 and t=25.
  initial begin
    #22 rst_l = 1'b1;
  end

  initial begin
    @(posedge clk iff rst_l === 1'b1);
    woke_at  = $time;
    did_wake = 1'b1;
  end

  initial #80 $finish;
endmodule
"#;

fn lookup(sim: &xezim::compiler::Simulator, name: &str) -> u64 {
    sim.get_signal(name)
        .or_else(|| sim.get_signal(&format!("tb.{}", name)))
        .unwrap_or_else(|| panic!("signal not found: {}", name))
        .to_u64()
        .unwrap_or_else(|| panic!("signal {} not u64-able", name))
}

#[test]
fn iff_guard_blocks_until_condition_holds() {
    let sim = simulate(SRC, 200).expect("simulate failed");
    let did_wake = lookup(&sim, "did_wake") & 0x1;
    let woke_at = lookup(&sim, "woke_at") & 0xFFFFFFFF;
    assert_eq!(did_wake, 1, "process never resumed past the iff-guarded wait");
    // First posedge with rst_l high is t=25. Without the guard the wait
    // would have fired at the first posedge, t=5.
    assert_eq!(
        woke_at, 25,
        "iff-guarded @(posedge clk) should resume at t=25 (first posedge after reset), got {}",
        woke_at
    );
}
