`include "../common/svtest_defs.svh"

module test_sva_sequences_advanced;
  `SVTEST_INIT

  logic clk;
  logic rst_n;
  logic req;
  logic ack;

  initial clk = 0;
  always #1 clk = ~clk;

  sequence s_req_ack;
    req ##1 ack;
  endsequence

  property p_req_ack_after_reset;
    @(posedge clk) disable iff (!rst_n) s_req_ack;
  endproperty

  a_req_ack_after_reset: assert property (p_req_ack_after_reset)
    else begin
      failures++;
      $display("FAIL: advanced sequence/property failed");
    end

  initial begin
    rst_n = 0;
    req   = 0;
    ack   = 0;

    repeat (2) @(posedge clk);
    rst_n <= 1;

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
