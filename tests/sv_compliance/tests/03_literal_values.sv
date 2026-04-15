`include "../common/svtest_defs.svh"

module test_literal_values;
  `SVTEST_INIT

  logic [7:0] h;
  logic [7:0] b;
  logic [7:0] o;
  int dec;
  logic [3:0] with_x;

  initial begin
    h = 8'hA5;
    b = 8'b1010_0101;
    o = 8'o245;
    dec = 42;
    with_x = 4'b10x1;

    `SVTEST_CHECK(h == 8'hA5, "hex literal failed")
    `SVTEST_CHECK(b == 8'hA5, "binary literal with underscore failed")
    `SVTEST_CHECK(o == 8'hA5, "octal literal failed")
    `SVTEST_CHECK(dec == 42, "decimal literal failed")
    `SVTEST_CHECK($isunknown(with_x), "x/z state literal failed")

    `SVTEST_PASSFAIL
  end
endmodule
