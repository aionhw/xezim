`include "../common/svtest_defs.svh"

module test_assignments;
  `SVTEST_INIT

  logic a, b, c;
  logic clk;
  logic q;

  assign c = a & b;

  initial clk = 0;
  always #1 clk = ~clk;

  initial begin
    a = 1'b1;
    b = 1'b1;
    #0;
    `SVTEST_CHECK(c == 1'b1, "continuous assignment failed")

    a = 1'b0;
    b = 1'b1;
    #0;
    `SVTEST_CHECK(c == 1'b0, "continuous reassignment failed")

    q = 1'b0;
    @(posedge clk);
    q <= 1'b1;
    `SVTEST_CHECK(q == 1'b0, "nonblocking should update later in current time step")
    @(negedge clk);
    `SVTEST_CHECK(q == 1'b1, "nonblocking assignment update failed")

    q = 1'b0;
    q = 1'b1;
    `SVTEST_CHECK(q == 1'b1, "blocking assignment failed")

    `SVTEST_PASSFAIL
  end
endmodule
