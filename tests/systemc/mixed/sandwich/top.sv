// SystemVerilog top: feeds samples to the SystemC module `mac`, which
// accumulates through the SystemVerilog leaf `addsat` (leaf.sv).
`timescale 1ns/1ps
module top;
  bit clk;
  bit clr;
  int x;
  int acc;
  always #5 clk = ~clk;
  mac m(.clk(clk), .clr(clr), .x(x), .acc(acc));
  initial begin
    int samples[8] = '{3, 100, 2000, 30000, 500000, 2147000000, -7, 1};
    clr = 1;
    @(negedge clk);
    clr = 0;
    foreach (samples[i]) begin
      x = samples[i];
      @(negedge clk);
    end
    $display("top: acc=%0d", acc);
    $finish;
  end
endmodule
