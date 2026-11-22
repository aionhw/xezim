//! IEEE 1800-2023 §4.4.2.2, §4.5, §10.3: a continuous assignment's update is
//! Active-region work, and so are the processes its change wakes. A process
//! on a clock that an `assign` copies (`assign c2 = clk;`) runs at the edge
//! with its peers on `clk`, before the NBAs they queue there commit.
//!
//! While a level-sensitive `wait` was pending anywhere, the condition-waiter
//! drain settled the copy early in the slot. The edge pass then scanned only
//! the toggled clocks, so `@(posedge c2)` ran in the cascade, after the NBAs,
//! and read their new values. A `wait` on the copy itself rechecked its
//! condition before the drain settled the copy, so it stayed parked until
//! after the NBAs, and the edges of its writes came after them too. With
//! no `wait` pending, a delayed nonblocking assignment due at the edge sends
//! the slot through an edge pass before the NBAs, and the settle there moved
//! the copy behind the clocks-only pass the same way.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 1000).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect()
}

const COPY: &str = r#"
module top;
  logic clk = 0, q = 0, s = 0, d = 0, w = 0, ws = 0;
  logic c2;
  assign c2 = clk;
  always #5 clk = ~clk;
  always @(posedge clk) q <= ~q;
  // Reads q at the edge, before the flop's NBA commits.
  always @(posedge c2) s <= q;
  // A level wait that stays pending.
  initial wait (d === 1'b1);
  // A waiter the copy releases at the first edge, and a write's edge.
  initial begin
    wait (c2 === 1'b1);
    w = 1;
  end
  always @(posedge w) ws <= q;
  always @(negedge clk) $display("T|%0t|q=%b s=%b ws=%b", $time, q, s, ws);
  initial #35 $finish;
endmodule
"#;

#[test]
fn a_copied_clock_runs_before_the_nbas_while_a_wait_is_pending() {
    assert_eq!(
        lines(COPY),
        [
            "T|10|q=1 s=0 ws=0",
            "T|20|q=0 s=1 ws=0",
            "T|30|q=1 s=0 ws=0"
        ]
    );
}

/// A strobe a flop raises for one cycle, counted on the clock a DUT copies
/// to an output, while the testbench waits for that strobe.
const STROBE: &str = r#"
module top;
  logic clk = 0, req = 0, wr = 0;
  logic oclk;
  int n = 0;
  assign oclk = clk;
  always #5 clk = ~clk;
  always @(posedge clk) wr <= req;
  always @(posedge oclk) if (wr) n++;
  initial begin
    #1 req = 1;
    wait (wr === 1'b1);
    #1 req = 0;
    #30 $display("T|count|%0d", n);
    $finish;
  end
endmodule
"#;

#[test]
fn a_strobe_counted_on_a_copied_clock_counts_once() {
    assert_eq!(lines(STROBE), ["T|count|1"]);
}

/// No `wait` is pending, but a delayed nonblocking assignment falls due at
/// the first edge, so the slot runs its edges before the NBAs. The settle
/// there moves the copy, and the edge pass must see it as well as `clk`.
const DUE: &str = r#"
module top;
  logic clk = 0, q = 0, s = 0, x = 0;
  logic c2;
  assign c2 = clk;
  always #5 clk = ~clk;
  always @(posedge clk) q <= ~q;
  always @(posedge c2) s <= q;
  initial begin
    #1 x <= #4 1;
    #5 $display("T|6|q=%b s=%b x=%b", q, s, x);
    $finish;
  end
endmodule
"#;

#[test]
fn a_copied_clock_runs_before_the_nbas_when_a_delayed_nba_is_due() {
    assert_eq!(lines(DUE), ["T|6|q=1 s=0 x=1"]);
}
