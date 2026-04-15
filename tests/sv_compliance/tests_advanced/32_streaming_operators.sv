`include "../common/svtest_defs.svh"

module test_streaming_operators;
  `SVTEST_INIT

  bit [31:0] src;
  bit [31:0] left_stream;
  bit [31:0] right_stream;

  initial begin
    src = 32'h11223344;

    left_stream  = {<<8{src}};
    right_stream = {>>8{src}};

    `SVTEST_CHECK(left_stream  == 32'h44332211, "left streaming operator failed")
    `SVTEST_CHECK(right_stream == 32'h11223344, "right streaming operator failed")

    `SVTEST_PASSFAIL
  end
endmodule
