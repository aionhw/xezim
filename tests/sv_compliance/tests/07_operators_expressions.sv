`include "../common/svtest_defs.svh"

module test_operators_expressions;
  `SVTEST_INIT

  logic [7:0] a, b, c;
  logic reduce_and;
  logic [15:0] cat;
  int tern;

  initial begin
    a = 8'h0F;
    b = 8'h03;
    c = (a << 1) + b;
    reduce_and = &8'hFF;
    cat = {a, b};
    tern = (a > b) ? 1 : 0;

    `SVTEST_CHECK((a + b) == 8'h12, "addition failed")
    `SVTEST_CHECK((a - b) == 8'h0C, "subtraction failed")
    `SVTEST_CHECK((a & b) == 8'h03, "bitwise and failed")
    `SVTEST_CHECK((a | b) == 8'h0F, "bitwise or failed")
    `SVTEST_CHECK((a ^ b) == 8'h0C, "bitwise xor failed")
    `SVTEST_CHECK(c == 8'h21, "shift/expression composition failed")
    `SVTEST_CHECK(reduce_and == 1'b1, "reduction operator failed")
    `SVTEST_CHECK(cat == 16'h0F03, "concatenation failed")
    `SVTEST_CHECK(tern == 1, "ternary operator failed")

    `SVTEST_PASSFAIL
  end
endmodule
