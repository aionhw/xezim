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

  initial begin
    cb.req <= 0;
    @(posedge clk);
    cb.req <= 1;
    @(posedge clk);
    @(posedge clk);
    `SVTEST_CHECK(cb.ack == 1'b1, "clocking block sampled input failed")

    cb.req <= 0;
    @(posedge clk);
    @(posedge clk);
    `SVTEST_CHECK(cb.ack == 1'b0, "clocking block driven output failed")

    `SVTEST_PASSFAIL
  end
endmodule
