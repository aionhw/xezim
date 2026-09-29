`include "../common/svtest_defs.svh"

module test_clocking_blocks;
  `SVTEST_INIT

  logic clk;
  logic req;
  logic ack;

  initial clk = 0;
  always #1 clk = ~clk;
  always @(posedge clk) ack <= req;

  clocking cb @(posedge clk);
    output req;
    input  ack;
  endclocking

  // §14.4: a clocking-block input samples with #1step skew (Preponed region),
  // so `cb.ack` reflects `ack` as of BEFORE the sampling edge — one clock
  // behind the live signal. Combined with the output drive of `req` and the
  // `ack <= req` flop, the req→ack→cb.ack path takes a few cycles to settle.
  // The checks therefore wait enough edges for the value to be stable, so the
  // expected results match a reference simulator (verified: cb.ack==1 then ==0).
  initial begin
    cb.req <= 1;
    repeat (4) @(posedge clk);
    `SVTEST_CHECK(cb.ack == 1'b1, "clocking block sampled input failed")

    cb.req <= 0;
    repeat (4) @(posedge clk);
    `SVTEST_CHECK(cb.ack == 1'b0, "clocking block driven output failed")

    `SVTEST_PASSFAIL
    $finish;
  end
endmodule
