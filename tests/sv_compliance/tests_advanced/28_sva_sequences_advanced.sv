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

  // "req is acknowledged one cycle later, once out of reset." A BARE sequence
  // used directly as a property is a STRONG check that fails on every cycle the
  // sequence does not start (req low) — confirmed against iverilog/a commercial
  // simulator — so the request/ack intent must be an IMPLICATION. The declared
  // sequence still elaborates and is exercised by the cover below.
  property p_req_ack_after_reset;
    @(posedge clk) disable iff (!rst_n) req |=> ack;
  endproperty

  c_req_ack: cover property (@(posedge clk) disable iff (!rst_n) s_req_ack);

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
    $finish;
  end
endmodule
