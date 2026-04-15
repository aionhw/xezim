`include "../common/svtest_defs.svh"

module test_data_types;
  `SVTEST_INIT

  bit b;
  logic l;
  byte by;
  shortint si;
  int i;
  longint li;
  integer legacy_i;
  time t;
  realtime rt;

  initial begin
    b = 1'b1;
    l = 1'b0;
    by = -8'sd3;
    si = 16'sd1234;
    i = 32'd56789;
    li = 64'd987654321;
    legacy_i = -22;
    t = 12;
    rt = 3.25;

    `SVTEST_CHECK(b === 1'b1, "bit assignment failed")
    `SVTEST_CHECK(l === 1'b0, "logic assignment failed")
    `SVTEST_CHECK(by == -3, "byte assignment failed")
    `SVTEST_CHECK(si == 1234, "shortint assignment failed")
    `SVTEST_CHECK(i == 56789, "int assignment failed")
    `SVTEST_CHECK(li == 987654321, "longint assignment failed")
    `SVTEST_CHECK(legacy_i == -22, "integer assignment failed")
    `SVTEST_CHECK(t == 12, "time assignment failed")
    `SVTEST_CHECK(rt > 3.0 && rt < 3.5, "realtime assignment failed")

    `SVTEST_PASSFAIL
  end
endmodule
