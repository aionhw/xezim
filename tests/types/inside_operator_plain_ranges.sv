// IEEE 1800-2023 §11.4.13: both expressions of a colon range are inclusive bounds.
`timescale 1ns/1ps
module top;
  int value, c;
  initial begin
    c = 1;
    value = 8;
    if ((value inside {[1 << 3 : 12]}) !== 1'b1 ||
        (value inside {[c == 1 : 8]}) !== 1'b1 ||
        (value inside {[c ? 1 : 2 : 8]}) !== 1'b1)
      $fatal(1, "inclusive plain ranges");
    value = 7;
    if ((value inside {[1 << 3 : 12]}) !== 1'b0)
      $fatal(1, "below the shift bound");
    c = 0;
    value = 1;
    if ((value inside {[c == 1 : 3 & 7]}) !== 1'b1 ||
        (value inside {[c ? 1 : 2 : 8]}) !== 1'b0)
      $fatal(1, "equality and conditional bounds");
    $display("PASS");
    $finish;
  end
endmodule
