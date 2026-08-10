//! §4.5 region ordering for `#0` continuations — reference-validated.
//! The inactive region (parked `#0` continuations) activates and drains
//! BEFORE the NBA region of the same time slot. A `#0` parked by an
//! edge-waiter-resumed process was previously promoted only after the
//! cascade's apply_nba, so `@(posedge clk); #0 x = r;` read POST-NBA r —
//! the "sampled one cycle early" checker/BFM divergence.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able (x/z?)", n))
}

#[test]
fn zero_delay_after_edge_wait_reads_pre_nba() {
    let src = r#"
`timescale 1ns/1ns
module tb;
  logic clk = 0; always #5 clk = ~clk;
  logic [7:0] r = 0;
  int seen_plain, seen_zero;
  always @(posedge clk) r <= r + 1;
  initial begin
    @(posedge clk); seen_plain = r;    // active region: pre-NBA -> 0
    @(posedge clk); #0 seen_zero = r;  // inactive region: still pre-NBA -> 1
    #1 $finish;
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulate failed");
    assert_eq!(u(&sim, "seen_plain"), 0, "plain @(posedge) resume is pre-NBA");
    assert_eq!(
        u(&sim, "seen_zero"),
        1,
        "#0 continuation runs in the inactive region, before this slot's NBAs"
    );
    assert_eq!(u(&sim, "r"), 2, "the flop itself still updates normally");
}

#[test]
fn chained_zero_delays_stay_before_nba() {
    // Multiple #0 hops in one slot: all drain before the NBA region.
    let src = r#"
`timescale 1ns/1ns
module tb;
  logic clk = 0; always #5 clk = ~clk;
  logic [7:0] r = 0;
  int s1, s2;
  always @(posedge clk) r <= r + 1;
  initial begin
    @(posedge clk); #0 s1 = r; #0 s2 = r;
    #1 $finish;
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulate failed");
    assert_eq!(u(&sim, "s1"), 0, "first #0 hop pre-NBA");
    assert_eq!(u(&sim, "s2"), 0, "second #0 hop still pre-NBA");
}

#[test]
fn event_triggered_from_edge_block_wakes_pre_nba() {
    // `-> ev` inside an always_ff toggles the event DURING block exec; the
    // waiter must resume in the SAME active region (§15.5.1) — before this
    // slot's NBAs — not in the cascade's post-NBA check_edges. Covers both
    // the plain resume and a #0 hop after it.
    let src = r#"
`timescale 1ns/1ns
module tb;
  event ev;
  logic clk = 0; always #5 clk = ~clk;
  logic [7:0] r = 0;
  always @(posedge clk) begin r <= r + 1; -> ev; end
  int s_plain = -1, s_zero = -1;
  initial begin @(ev) s_plain = r; end
  initial begin @(ev) #0 s_zero = r; end
  initial begin #7 $finish; end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulate failed");
    assert_eq!(u(&sim, "s_plain"), 0, "waiter resumes pre-NBA (active region)");
    assert_eq!(u(&sim, "s_zero"), 0, "#0 hop after the waiter is still pre-NBA");
    assert_eq!(u(&sim, "r"), 1, "the flop still updates");
}
