`include "../common/svtest_defs.svh"

module test_covergroups_basic;
  `SVTEST_INIT

  logic clk;
  logic [1:0] a;
  logic b;
  real cov;

  covergroup cg @(posedge clk);
    coverpoint a;
    coverpoint b;
    cross a, b;
  endgroup

  cg cov_i = new();

  initial clk = 0;
  always #1 clk = ~clk;

  initial begin
    a = 0; b = 0;
    repeat (8) begin
      @(posedge clk);
      a <= a + 1;
      b <= ~b;
    end

    #0;
    cov = cov_i.get_inst_coverage();
    `SVTEST_CHECK(cov > 0.0, "covergroup coverage did not accumulate")

    `SVTEST_PASSFAIL
  end
endmodule
