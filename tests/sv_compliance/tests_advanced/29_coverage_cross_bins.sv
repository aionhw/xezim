`include "../common/svtest_defs.svh"

module test_coverage_cross_bins;
  `SVTEST_INIT

  bit [1:0] a;
  bit [1:0] b;
  real cov;

  covergroup cg;
    option.per_instance = 1;

    cp_a: coverpoint a {
      bins lo = {0, 1};
      bins hi = {2, 3};
    }

    cp_b: coverpoint b {
      bins even = {0, 2};
      bins odd  = {1, 3};
    }

    x_ab: cross cp_a, cp_b;
  endgroup

  cg c = new();

  initial begin
    a = 0; b = 0; c.sample();
    a = 3; b = 1; c.sample();

    cov = c.get_inst_coverage();
    `SVTEST_CHECK(cov > 0.0, "cross coverage collection failed")

    `SVTEST_PASSFAIL
  end
endmodule
