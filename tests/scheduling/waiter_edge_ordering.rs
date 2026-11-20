//! Commercial-simulator same-edge process ordering (reference-verified):
//!
//! 1. A process PARKED on `@(posedge clk)` resumes BEFORE the same edge's
//!    always blocks execute — so a testbench check right after `@(posedge)`
//!    reads the value from BEFORE an always block's blocking write (the
//!    OpenRAM SRAM model races exactly this way: it x-blasts `dout0` with a
//!    blocking assign at every posedge while the tb checks after the edge).
//! 2. A `$finish` raised by such a continuation still lets the same slot's
//!    always blocks, SVA actions, and monitors run once (a final-cycle
//!    `$display` monitor racing the finish prints).

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn parked_waiter_reads_pre_edge_block_value() {
    // Mirror of the OpenRAM SRAM read path: negedge NBA drives data, the
    // posedge block x-blasts it with a BLOCKING write, and the tb checks
    // right after `@(posedge)`. The check must see the data, not the blast.
    let sim = simulate(
        r#"
module dut (input clk, input en, output reg [7:0] q);
  reg en_r;
  always @(posedge clk) begin
    en_r = en;
    q = 8'bx;
    if (en_r) ;
  end
  always @(negedge clk) begin : R
    if (en_r) q <= 8'h5A;
  end
endmodule
module top;
  logic clk = 0, en;
  logic [7:0] q;
  dut u (.clk(clk), .en(en), .q(q));
  always #5 clk = ~clk;
  initial begin
    en <= 1;
    @(posedge clk); // 5  (samples en)
    @(posedge clk); // 15 (negedge 10 drove q<=5A; posedge 15 x-blasts)
    $display("CHK=%b", q === 8'h5A);
    $finish;
  end
endmodule
"#,
        1000,
    )
    .expect("sim");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "CHK=1"),
        "tb check after @(posedge) must read the pre-blast value; output: {:?}",
        msgs
    );
}

#[test]
fn finish_from_continuation_lets_same_edge_monitors_run() {
    let sim = simulate(
        r#"
module tb;
  logic clk = 0;
  int   d = 0;
  always #5 clk = ~clk;
  always @(posedge clk) d <= d + 1;
  always @(posedge clk) $display("MON t=%0t d=%0d", $time, d);
  initial begin repeat (4) @(posedge clk); $finish; end
endmodule
"#,
        1000,
    )
    .expect("sim");
    let fires = messages(&sim)
        .iter()
        .filter(|m| m.contains("MON t="))
        .count();
    assert_eq!(
        fires, 4,
        "the final-cycle display racing $finish must still print"
    );
}

#[test]
fn event_fired_by_resumed_waiter_reaches_peer_same_tick() {
    // §14.13 regression: a process resumed on a clock edge that fires `-> ev`
    // must wake a peer parked on `@(ev)` in the SAME time step. The
    // waiters-first inline execution otherwise swallowed the follow-up
    // delivery pass, stranding the peer until the next tick (issue #40 TC9).
    let sim = simulate(
        r#"
module tb;
  logic clk = 0;
  event ev;
  time trig_t, samp_t;
  bit ok = 0;
  always #5 clk = ~clk;
  initial begin
    fork
      begin
        @(posedge clk); @(posedge clk);  // 2 cycles
        trig_t = $time;
        -> ev;
      end
      begin
        @(posedge clk);                  // 1 cycle, then park on ev
        @(ev);
        samp_t = $time;
        if (samp_t == trig_t) ok = 1;
      end
    join
    #1 $display("OK=%b t=%0t,%0t", ok, trig_t, samp_t);
    $finish;
  end
endmodule
"#,
        1000,
    )
    .expect("sim");
    let msgs: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert!(
        msgs.iter().any(|m| m == "OK=1 t=15,15"),
        "event from resumed waiter didn't reach peer same-tick; output: {:?}",
        msgs
    );
}
