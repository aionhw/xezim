// IEEE 1800-2023 §27.4: inside each generate-for iteration the genvar is a
// localparam of the integer value of that iteration, negative values
// included. The elaborator writes such a value as a numeric literal; a
// loaded design cache or compiled artifact re-reads the literal text, which
// read a negative value as x.
`timescale 1ns/1ps
module top;
  int arr[-2:1];
  int o[-2:1];
  for (genvar k = -2; k <= 1; k++) begin : gk
    assign o[k] = arr[k] + k;
    initial #(k + 5) $display("T|k=%0d twice=%0d sel=%0d", k, k * 2, arr[k]);
  end
  initial begin
    arr[-2] = 10; arr[-1] = 20; arr[0] = 30; arr[1] = 40;
    #1 $display("T|o %0d %0d %0d %0d", o[-2], o[-1], o[0], o[1]);
  end
endmodule
