`timescale 1ns/1ns
module top;
  bit clk = 0;
  logic [1:0] value = 2'b01;
  int result;

  // IEEE 1800-2023 12.7.1 and 13.4.1: return exits the function, including
  // when it occurs inside a descending for loop.
  function automatic int highest_bit(input logic [1:0] x);
    for (int i = 1; i >= 0; i--)
      if (x[i]) return i + 1;
    return 0;
  endfunction

  always #1 clk = ~clk;
  always @(posedge clk) result <= highest_bit(value);

  initial begin
    #2;
    if (result !== 1) $fatal(1, "expected 1, got %0d", result);
    value = 2'b10;
    #2;
    if (result !== 2) $fatal(1, "expected 2, got %0d", result);
    value = 0;
    #2;
    if (result !== 0) $fatal(1, "expected 0, got %0d", result);
    $display("INLINE_RETURN_ROLLBACK_PASS");
    $finish;
  end
endmodule
