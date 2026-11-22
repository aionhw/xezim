//! IEEE 1800-2023 §16.14.6: a procedural concurrent assertion's instance
//! starts its attempt at its clock's event in the time step it matures in,
//! if the clock had one, and otherwise at the clock's next event. Whether
//! the clock had its event in that time step does not depend on what woke
//! the procedure: a clock generator's other output, an any-change event
//! control and a write of another process are no event of `clk`, while a
//! procedure an NBA wakes at `clk`'s own edge is at that tick.
//!
//! Registration took the clock's level as an edge unless the running
//! process had resumed from a delay, and the process it asked was whichever
//! ran last: `a` attempted at 9 and `b` at 6, off the clock, and `d` missed
//! the tick at 25.

use xezim::simulate;

const SRC: &str = r#"
module top;
  logic clk = 0, clk2 = 0, go = 0, kick = 0, st = 0;
  always #5 clk = ~clk;
  always #3 clk2 = ~clk2;
  initial begin #6 go = 1; #20 kick = 1; end
  // clk2 rises at 9 while clk is high (it rose at 5): the next posedge is 15.
  always @(posedge clk2) if ($time == 9) a: assert property (@(posedge clk) 1) $display("T|a|%0t", $time);
  // An any-change event control woken at 6.
  always @(go) if (go) b: assert property (@(posedge clk) 1) $display("T|b|%0t", $time);
  // Woken at 26, with clk high since 25: the next negedge is 30.
  always @(posedge kick) c: assert property (@(negedge clk) 1) $display("T|c|%0t", $time);
  // Woken at 25 by an NBA the clock's own edge made: that edge is the tick.
  always @(posedge clk) if ($time == 25) st <= 1;
  always @(posedge st) d: assert property (@(posedge clk) 1) $display("T|d|%0t", $time);
  initial #40 $finish;
endmodule
"#;

#[test]
fn a_procedure_woken_off_its_clock_attempts_at_the_next_tick() {
    let sim = simulate(SRC, 1000).expect("simulate failed");
    let mut got: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect();
    // Same-time reports of different assertions come in no set order.
    got.sort();
    assert_eq!(got, ["T|a|15", "T|b|15", "T|c|30", "T|d|25"]);
}
