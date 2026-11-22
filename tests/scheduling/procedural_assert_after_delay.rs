//! IEEE 1800-2023 §16.14.6, §16.4.2: a procedural concurrent assertion that
//! a procedure reaches after a delay or an event control queues one instance.
//! Its attempt starts at the clock's tick in that time step, when there was
//! one (after a `#0`), and otherwise at the clock's next tick. A deferred
//! assertion there reports at the end of its time slot.
//!
//! The process flushed its pending instances and reports when it looped
//! back to its event control in the same time slot, which is no flush point
//! (only a resume after a suspension is), so `a1`-`a3` and `d1` never
//! reported. And an instance first queued after a delay started its attempt
//! at once, outside a clock tick (`a4` at 6, then nothing at 15).

use xezim::simulate;

const SRC: &str = r#"
module top;
  logic clk = 0;
  logic [7:0] a = 1, b = 2;
  wire [8:0] s = a + b;
  always #5 clk = ~clk;
  initial begin
    repeat (3) #20 begin a = a + 3; b = b + 5; end
    #12 $finish;
  end
  // A procedural concurrent assertion after a delay: one attempt per
  // execution, at the clock's next tick.
  always @(posedge clk) begin
    #1;
    a1: assert property (@(posedge clk) s == a + b) $display("T|a1|%0t", $time);
  end
  // The delay as the statement's prefix.
  always @(posedge clk) #2 a2: assert property (@(posedge clk) s == a + b) $display("T|a2|%0t", $time);
  // After an event control on the other edge.
  always @(posedge clk) begin
    @(negedge clk);
    a3: assert property (@(posedge clk) s == a + b) $display("T|a3|%0t", $time);
  end
  // In an initial forever loop, after an edge and a delay.
  initial forever begin
    @(posedge clk);
    #1;
    a4: assert property (@(posedge clk) s == a + b) $display("T|a4|%0t", $time);
  end
  // A forever loop whose delay comes first (no event control before it).
  initial forever begin
    #6;
    a5: assert property (@(posedge clk) s == a + b) $display("T|a5|%0t", $time);
    @(negedge clk);
  end
  // A `#0` stays in the time step of the edge: each attempt is at that tick.
  always @(posedge clk) begin
    #0;
    a6: assert property (@(posedge clk) s == a + b) $display("T|a6|%0t", $time);
  end
  // A `#0` after a delay that moved time on keeps the next tick.
  always @(posedge clk) begin
    #1;
    #0;
    a7: assert property (@(posedge clk) s == a + b) $display("T|a7|%0t", $time);
  end
  // An attempt at its time step's tick reads that tick's `$past`: `a`
  // changes at 20, 40 and 60.
  always @(posedge clk) begin
    #0;
    a8: assert property (@(posedge clk) $past(a) != a) $display("T|a8|%0t", $time);
  end
  // A delay one clock period long ends in the time step of a clock edge.
  // The delay's resume and the clock's toggle are both Active events of
  // that step, which run in either order (§4.7). If the edge comes first,
  // the instance matures and attempts at that edge; if the resume comes
  // first, the edge wakes the procedure before the instance matures, and
  // that resume flushes it (§16.4.2). Either way, an attempt starts only at
  // a tick.
  always @(posedge clk) begin
    #10;
    a9: assert property (@(posedge clk) s == a + b) $display("T|a9|%0t", $time);
  end
  // A deferred assertion after a delay reports at the end of its slot.
  always @(posedge clk) begin
    #1;
    d1: assert #0 (s == a + b) $display("T|d1|%0t", $time);
  end
endmodule
"#;

#[test]
fn procedural_assertions_after_a_delay_attempt_at_the_next_tick() {
    let sim = simulate(SRC, 1000).expect("simulate failed");
    let got: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect();
    // Same-time reports of different assertions come in no set order.
    let times = |label: &str| -> Vec<String> {
        got.iter()
            .filter(|l| l.starts_with(&format!("T|{label}|")))
            .cloned()
            .collect()
    };
    let at = |label: &str, ticks: &[&str]| -> Vec<String> {
        ticks.iter().map(|t| format!("T|{label}|{t}")).collect()
    };
    let ticks = ["15", "25", "35", "45", "55", "65"];
    for label in ["a1", "a2", "a3", "a4", "a5", "a7"] {
        assert_eq!(times(label), at(label, &ticks), "{label}");
    }
    let own = ["5", "15", "25", "35", "45", "55", "65"];
    assert_eq!(times("a6"), at("a6", &own), "a6");
    assert_eq!(times("a8"), at("a8", &["25", "45", "65"]), "a8");
    // An ordering race, not a check on how many attempts `a9` makes.
    let off = times("a9")
        .into_iter()
        .filter(|l| !l.ends_with('5'))
        .collect::<Vec<_>>();
    assert!(off.is_empty(), "a9 attempted off a tick: {off:?}");
    let d1 = ["6", "16", "26", "36", "46", "56", "66"];
    assert_eq!(times("d1"), at("d1", &d1), "d1");
}

/// §16.5.1: the attempt a `#0` leaves at the time-0 edge reads the default
/// sampled values (`a` has no initializer, so x), not the values the
/// testbench wrote at time 0. The block runs as a process (its body has a
/// delay), which the default-value scan did not cover, so the attempt read
/// `a == 3` and passed.
const T0: &str = r#"
module top;
  logic clk = 0;
  logic [3:0] a;
  initial a = 3;
  initial begin
    #0 clk = 1;
    forever #5 clk = ~clk;
  end
  initial #25 $finish;
  always @(posedge clk) begin
    #0;
    t0: assert property (@(posedge clk) a == 3) $display("T|t0|%0t", $time);
      else $display("F|t0|%0t", $time);
  end
endmodule
"#;

#[test]
fn a_time0_attempt_reads_the_default_sampled_values() {
    let sim = simulate(T0, 1000).expect("simulate failed");
    let got: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| l.starts_with("T|") || l.starts_with("F|"))
        .collect();
    assert_eq!(got, ["F|t0|0", "T|t0|10", "T|t0|20"]);
}

/// §16.14.6: a procedure that another event wakes (`@(posedge go)`) reaches
/// the assertion while its clock is high, at 6, and after a falling edge, at
/// 26. Neither time is an event of the clock, so each attempt starts at the
/// clock's next event. The clock's level at registration is no edge.
#[test]
fn a_procedure_another_event_wakes_attempts_at_the_next_tick() {
    let src = r#"
module top;
  logic clk = 0, go = 0;
  always #5 clk = ~clk;
  initial begin #6 go = 1; #10 go = 0; #10 go = 1; #10 $finish; end
  always @(posedge go) a: assert property (@(posedge clk) 1) $display("T|a|%0t", $time);
endmodule
"#;
    let sim = simulate(src, 1000).expect("simulate failed");
    let got: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect();
    assert_eq!(got, ["T|a|15", "T|a|35"]);
}
