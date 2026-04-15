`include "../common/svtest_defs.svh"

checker req_ack_checker(input logic clk, input logic req, input logic ack);
  default clocking cb @(posedge clk); endclocking
  a_req_ack: assert property (req |=> ack);
endchecker

module test_checker_blocks;
  `SVTEST_INIT

  logic clk;
  logic req;
  logic ack;

  req_ack_checker c0(.clk(clk), .req(req), .ack(ack));

  initial clk = 0;
  always #1 clk = ~clk;

  initial begin
    req = 0;
    ack = 0;

    @(posedge clk);
    req <= 1;
    ack <= 0;

    @(posedge clk);
    req <= 0;
    ack <= 1;

    @(posedge clk);
    ack <= 0;

    #0;
    `SVTEST_PASSFAIL
  end
endmodule
