// IEEE 1800-2023 §11.4.13, §12.5.4.
`timescale 1ns/1ps
module top;
  logic [7:0] key;
  int result;
  initial begin
    key = 8;
    case (key) inside
      [0:(key inside {[2 +/- 1]})]: result = 0;
      [1 ? 8 : 9 : 12], [20:21]: result = 1;
      default: result = 2;
    endcase
    if (result != 1) $fatal(1, "plain range with a conditional bound");
    key = 36;
    if ((key inside {[10 +/- 2], [40 +%- 10], [20:21]}) !== 1'b1)
      $fatal(1, "inclusive integral relative tolerance");
    key = 35;
    if ((key inside {[10 +/- 2], [40 +%- 10], [20:21]}) !== 1'b0)
      $fatal(1, "outside integral tolerances and plain range");
    $display("PASS");
    $finish;
  end
endmodule
