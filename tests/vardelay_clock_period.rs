//! A clock generator whose delay reads a RUNTIME variable
//! (`always #(half) clk = ~clk` with `half` reprogrammed mid-run — the PLL
//! refclk/vco reconfiguration pattern) must re-evaluate the delay on every
//! toggle. xezim was baking the initial delay value into a fixed-period
//! `ClockGen`, so changing `half` had no effect and the clock kept the old
//! period forever (a PLL that never changes frequency). Fixed by bailing to
//! the dynamic `FastDelayAlways` path when the delay expression reads a signal.

use xezim::simulate;

fn line(src: &str) -> Vec<String> {
    simulate(src, 1_000_000)
        .expect("sim")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect()
}

#[test]
fn always_clock_picks_up_runtime_period_change() {
    // half=10 for 100ns (10 edges), then half=2.5 for 100ns (~40 more edges).
    let src = r#"
module tb;
  reg clk = 0;
  real half = 10;
  integer edges = 0;
  always #(half) clk = ~clk;
  always @(clk) edges = edges + 1;
  initial begin
    #100;
    half = 2.5;                 // speed up 4x (PLL reconfig)
    #100;
    $display("edges=%0d", edges);
  end
endmodule
"#;
    let out = line(src);
    // Before the fix this was 20 (stuck at the old 10ns period). With the
    // period change honored it must be well above 20 (~54).
    let e: i64 = out
        .iter()
        .find_map(|m| m.strip_prefix("edges=").and_then(|s| s.parse().ok()))
        .expect("edges= line");
    assert!(
        e > 30,
        "runtime period change ignored (clock stayed at old period): edges={e}, expected >30"
    );
}

#[test]
fn behavioral_pll_reconfigures_frequency() {
    // A PLL whose VCO half-period depends on runtime inputs; multiplier bumped
    // mid-run must raise the VCO frequency.
    let src = r#"
module pll(output reg vco, input real refperiod, input [3:0] mult);
  real vp;
  initial vco = 0;
  always begin
    vp = refperiod/(2.0*mult);
    #(vp) vco = ~vco;
  end
endmodule
module tb;
  real rp = 100.0; reg [3:0] m = 2; wire vco; integer ev = 0;
  pll u(vco, rp, m);
  always @(vco) ev = ev + 1;
  integer slow;
  initial begin
    #1000; slow = ev;
    m = 8;                      // 4x faster VCO
    #1000;
    $display("SLOW=%0d FAST=%0d", slow, ev - slow);
  end
endmodule
"#;
    let out = line(src);
    let pair = out
        .iter()
        .find(|m| m.starts_with("SLOW="))
        .expect("SLOW= line");
    let nums: Vec<i64> = pair
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .map(|s| s.parse().unwrap())
        .collect();
    assert!(
        nums.len() == 2 && nums[1] > nums[0] * 2,
        "PLL did not speed up after mult change: {pair}"
    );
}

#[test]
fn constant_period_clock_still_correct() {
    // Regression guard: a constant (parameter) delay must keep working (fast
    // ClockGen path). period=10 => toggle every 5ns => ~20 edges in 100ns.
    let src = r#"
module tb;
  parameter integer P = 10;
  reg clk = 0; integer edges = 0;
  always #(P/2) clk = ~clk;
  always @(clk) edges = edges + 1;
  initial #100 $display("edges=%0d", edges);
endmodule
"#;
    let out = line(src);
    let e: i64 = out
        .iter()
        .find_map(|m| m.strip_prefix("edges=").and_then(|s| s.parse().ok()))
        .expect("edges= line");
    assert!((18..=22).contains(&e), "constant clock rate wrong: edges={e}");
}
