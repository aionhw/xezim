`include "../common/svtest_defs.svh"

module test_assertions_basic;
  `SVTEST_INIT

  logic clk;
  logic req;
  logic ack;

  initial clk = 0;
  always #1 clk = ~clk;

  property p_req_ack;
    @(posedge clk) req |=> ack;
  endproperty

  a_req_ack: assert property (p_req_ack)
    else begin
      failures++;
      $display("FAIL: concurrent assertion p_req_ack failed");
    end

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

    assert ((2 + 2) == 4)
      else begin
        failures++;
        $display("FAIL: immediate assertion failed");
      end

    #0;
    `SVTEST_PASSFAIL
  end
endmodule
