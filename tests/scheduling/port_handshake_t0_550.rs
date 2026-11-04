//! IEEE 1800-2023 §4.4.2 / §9.7.4 / §10.4: a clock-generator ↔ BFM startup
//! handshake across two instances. The BFM's `always @(port iff (=== 0))`
//! must fire on the time-0 X→0 settle of the connected net after every
//! compile-time start has run, and its level-sensitive `wait` and edge
//! control on INPUT ports must see the other instance's drives. The
//! reduced shapes live in `t0_comb_defer_start_batch_550.rs`; this is the
//! issue's own design (#550, reported by Ganesh-TNL).

use xezim::simulate;

const SRC: &str = r#"
`timescale 1ps/1ps
// mwe_port_handshake.sv — a level-sensitive `wait` and an edge control on an
// INPUT port must observe the driver of the connected net, and a time-0
// X->0 settle of that net must kick an `always @(port iff ...)` in the
// receiving instance (IEEE 1800-2017 §10.4, §9.7.4).
//
// This is a clock-generator <-> BFM PLL startup handshake:
//
//   clkgen       : trigger_reset() drives por_l 0 (X->0 at t=0), later 1, then
//                  waits level-sensitively for the BFM's pll_prog_done input
//   pll_prog_bfm : always @(pad_pll_prog_por_l_c iff (=== 0)) kicks
//                  trigger_pll_prog(): wait (pad_pll_prog_por_l_c === 1) — a
//                  level-sensitive wait on an INPUT port; repeat @(posedge
//                  clk_ref) — an edge control on an INPUT port clocked by the
//                  other instance; then pll_prog_done = 1
//
// Expected (LRM): the handshake completes; pll_prog_done is 1 shortly after
//                  por_l is released; the BFM kickoff fired exactly once.
// xezim bug:       the chain never completes — grst_l is never released and
//                  the regression runs on forever with no further output
//                  (the fixed-3-cluster regression run stalled right after
//                  "por_l released now!" for hours at 100% CPU).


// BFM shape: every sense here is on an INPUT port driven by the
// other instance; the output is what the driver waits for.
module pll_prog_bfm_like (
  input  logic clk_ref,
  input  logic pad_pll_prog_por_l_c,
  output logic pll_prog_done
);
  int unsigned kicks = 0;

  // The kickoff: fires on the time-0 X->0 settle of the port's net.
  always @(pad_pll_prog_por_l_c iff (pad_pll_prog_por_l_c === 0)) begin
    kicks = kicks + 1;
    trigger_pll_prog();
  end

  task trigger_pll_prog();
    wait (pad_pll_prog_por_l_c === 1);          // level-sensitive on INPUT port
    repeat (2) @(posedge clk_ref);           // edge control on INPUT port
    #10;
    pll_prog_done = 1;                       // releases the driver's wait
  endtask
endmodule

// Clock-generator shape: drives por_l and the ref clock, then waits for the
// BFM's pll_prog_done (its own INPUT port).
module clkgen_like (
  output logic clk_ref,
  output logic por_l,
  input  logic pll_prog_done
);
  initial begin
    clk_ref = 0;
    forever #500 clk_ref = ~clk_ref;
  end

  initial begin
    trigger_reset();
  end

  task trigger_reset();
    por_l = 0;                               // X->0 at t=0 kicks the BFM
    #1000;
    repeat (2) @(posedge clk_ref);
    #300;
    por_l = 1;                               // release
    wait (pll_prog_done);                    // level-sensitive on INPUT port
    #20;
  endtask
endmodule

module testbench;
  logic clk_ref, por_l, pll_prog_done;
  int unsigned errors = 0;

  clkgen_like clkgen (.clk_ref(clk_ref), .por_l(por_l),
                       .pll_prog_done(pll_prog_done));
  pll_prog_bfm_like pll_prog_bfm (.clk_ref(clk_ref), .pad_pll_prog_por_l_c(por_l),
                                 .pll_prog_done(pll_prog_done));

  // checker: runs once the handshake should be long done
  initial begin
    #5000;
    if (pll_prog_bfm.kicks !== 1) begin
      $display("FAIL: BFM kickoff count = %0d (expected 1)", pll_prog_bfm.kicks);
      errors++;
    end
    if (pll_prog_done !== 1'b1) begin
      $display("FAIL: pll_prog_done = %b (expected 1)", pll_prog_done);
      errors++;
    end
    if (errors == 0) $display("TEST_PASS");
    else             $display("TEST_FAIL : %0d CHECK(S)", errors);
    $finish;
  end

  // watchdog: a stalled handshake must not hang the regression
  initial begin
    #100000;
    $display("FAIL: TIMEOUT — handshake stalled (kicks=%0d por_l=%b pll_prog_done=%b)",
             pll_prog_bfm.kicks, por_l, pll_prog_done);
    $display("TEST_FAIL : timeout");
    $finish;
  end
endmodule


"#;

#[test]
fn port_handshake_completes_at_time_zero_start_batch() {
    let sim = simulate(SRC, 200_000).expect("simulate");
    let msgs: Vec<&str> = sim.output.iter().map(|o| o.message.as_str()).collect();
    assert!(msgs.contains(&"TEST_PASS"), "{msgs:?}");
    assert!(!msgs.iter().any(|m| m.starts_with("FAIL")), "{msgs:?}");
}
