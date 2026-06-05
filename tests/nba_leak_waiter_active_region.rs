//! IEEE 1800-2023 §4.4.5 active-region semantics test.
//!
//! Verifies that an `initial begin forever @(posedge clk) … end` waiter
//! sees the same pre-NBA signal state as a sibling `always @(posedge
//! clk)` block. Before the drain_active_processes_at_current_time fix,
//! xezim ran the waiter continuation in the NEXT event_loop iteration
//! after the cascade's apply_nba already committed — leaking NBA
//! updates into the active region. The waiter then read post-NBA cnt
//! values one cycle too fresh.

use xezim::simulate;

const SRC: &str = r#"
`timescale 1ns/100ps
module tb;
  reg clk = 0;
  reg [31:0] cnt = 0;
  reg [31:0] snap_initial = 99;
  reg [31:0] snap_always = 99;
  reg [31:0] cap_initial_at_t15 = 99;
  reg [31:0] cap_always_at_t15 = 99;
  reg [31:0] cap_cnt_at_t15 = 99;
  always #5 clk = ~clk;

  always @(posedge clk) begin
    cnt <= cnt + 1;
    snap_always <= cnt;
  end

  initial begin
    forever begin
      @(posedge clk);
      snap_initial <= cnt;
      if ($time == 15) begin
        cap_initial_at_t15 = snap_initial;
        cap_always_at_t15  = snap_always;
        cap_cnt_at_t15     = cnt;
      end
    end
  end

  initial #65 $finish;
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
fn waiter_sees_pre_nba_state_of_current_cycle() {
    // Enable active-region drain (opt-in per IEEE 1800-2023 §4.4.5).
    // Default off because some testbenches (e.g. c910 tb.v initial
    // blocks with `@(posedge clk)` reads) depend on the legacy
    // schedule-into-event_queue behavior.
    std::env::set_var("XEZIM_ACTIVE_REGION", "1");
    let sim = simulate(SRC, 100).expect("simulate failed");
    // At sim t=15 (the second posedge clk):
    //   - cnt has the post-NBA value from t=5 (cnt=1) — the value
    //     entering the t=15 active region.
    //   - snap_initial / snap_always likewise have the t=5-NBA values
    //     they were assigned then (both 0, since both wrote cnt's
    //     pre-NBA value of 0 at t=5).
    // If the fix is missing, snap_initial reads cnt POST-the t=15 edge_block
    // apply_nba — so snap_initial would be 1 instead of 0.
    let snap_initial = lookup(&sim, "cap_initial_at_t15") & 0xFFFFFFFF;
    let snap_always  = lookup(&sim, "cap_always_at_t15") & 0xFFFFFFFF;
    let cnt          = lookup(&sim, "cap_cnt_at_t15") & 0xFFFFFFFF;
    assert_eq!(cnt, 1, "cnt at t=15 active region should be 1 (post-t=5-NBA), got {}", cnt);
    assert_eq!(snap_always, 0, "snap_always at t=15 should be 0 (from t=5 NBA), got {}", snap_always);
    assert_eq!(snap_initial, 0,
        "snap_initial at t=15 should be 0 (from t=5 NBA, NOT leaked from t=15 edge_block NBA), got {}",
        snap_initial);
}
