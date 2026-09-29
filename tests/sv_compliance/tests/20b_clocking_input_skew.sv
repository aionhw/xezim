`include "../common/svtest_defs.svh"

// §14.4 clocking-block input `#1step` skew: `cb.<in>` samples the value from
// the Preponed region (before the sampling edge), so when a producer updates
// the signal via NBA on the same edge, `cb.<in>` reads the PREVIOUS value —
// one clock behind the live signal. Verified against a reference simulator:
// after 4 posedges the live `data` is 4 while the preponed sample is 3.
module test_clocking_input_skew;
  `SVTEST_INIT

  logic clk;
  logic [7:0] data;

  initial clk = 0;
  always #1 clk = ~clk;
  initial data = 0;
  always @(posedge clk) data <= data + 1;   // producer drives via NBA

  clocking cb @(posedge clk);
    input data;
  endclocking

  initial begin
    repeat (4) @(posedge clk);
    #1; // settle into the cycle so the live value is unambiguous
    // Live `data` has this cycle's value (4); the clocking sample is the
    // pre-edge value (3) — NOT the same-edge NBA update.
    `SVTEST_CHECK(data == 8'd4, "live signal value wrong")
    `SVTEST_CHECK(cb.data == 8'd3, "clocking input did not sample preponed (#1step) value")

    `SVTEST_PASSFAIL
    $finish;
  end
endmodule
