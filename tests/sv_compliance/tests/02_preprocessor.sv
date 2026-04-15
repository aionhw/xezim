`include "../common/svtest_defs.svh"
`include "../common/include_value.svh"

`define ADD2(a,b) ((a) + (b))
`define SELECT_WIDTH 8
`define FEATURE_ON

module test_preprocessor;
  `SVTEST_INIT

  logic [`SELECT_WIDTH-1:0] x;
  int y;

  initial begin
    x = 8'h12;
    y = `ADD2(10, 7);

`ifdef FEATURE_ON
    `SVTEST_CHECK(y == 17, "macro expansion failed")
`else
    `SVTEST_CHECK(0, "FEATURE_ON should be defined")
`endif

    `SVTEST_CHECK(x == 8'h12, "macro width substitution failed")
    `SVTEST_CHECK(`INCLUDED_CONSTANT == 99, "include file macro failed")

    `SVTEST_PASSFAIL
  end
endmodule
