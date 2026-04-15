`include "../common/svtest_defs.svh"

let in_range(x, lo, hi) = (((x) >= (lo)) && ((x) <= (hi)));

module test_let_construct;
  `SVTEST_INIT

  int v;

  initial begin
    v = 5;

    `SVTEST_CHECK(in_range(v, 4, 6), "let construct positive case failed")
    `SVTEST_CHECK(!in_range(v, 6, 9), "let construct negative case failed")

    `SVTEST_PASSFAIL
  end
endmodule
