`include "../common/svtest_defs.svh"

module test_lexical_identifiers;
  `SVTEST_INIT

  logic simple_id;
  logic _leading_underscore;
  logic CamelCase123;
  logic \escaped-id ;
  string s;

  initial begin
    simple_id = 1'b1;
    _leading_underscore = 1'b0;
    CamelCase123 = 1'b1;
    \escaped-id  = 1'b1;
    s = "systemverilog";

    `SVTEST_CHECK(simple_id == 1'b1, "simple identifier assignment failed")
    `SVTEST_CHECK(_leading_underscore == 1'b0, "underscore identifier failed")
    `SVTEST_CHECK(CamelCase123 == 1'b1, "mixed-case identifier failed")
    `SVTEST_CHECK(\escaped-id  == 1'b1, "escaped identifier failed")
    `SVTEST_CHECK(s.len() == 13, "string literal length failed")

    `SVTEST_PASSFAIL
  end
endmodule
