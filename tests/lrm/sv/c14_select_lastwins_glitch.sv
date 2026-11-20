// IEEE 1800-2023 14.16.2: "When more than one synchronous drive on the same
// clocking block output (or inout) is scheduled to mature in the same Re-NBA
// region of the same time step, the last value is the only value driven onto
// the output signal."
// Select drives: e[0] is driven 1 and then 0, f[1:0] 11 and then 00, so
// neither output changes and nothing waiting on it wakes; g[2] is driven 1
// twice and changes once. h, a whole drive 1 then 0, is the control.
// (The reference simulator also changes the bits no drive selects, to x; the
// LRM drives only the selected bits.)
`timescale 1ns/1ps
module c14slg;
  logic clk = 0;
  always #5 clk = ~clk;
  logic [3:0] e = 0, f = 0, g = 0;
  bit h = 0;
  default clocking cb @(posedge clk);
    default output #2;
    output e, f, g, h;
  endclocking
  initial begin
    @(cb);
    cb.e[0] <= 1'b1;    cb.e[0] <= 1'b0;
    cb.f[1:0] <= 2'b11; cb.f[1:0] <= 2'b00;
    cb.g[2] <= 1'b1;    cb.g[2] <= 1'b1;
    cb.h <= 1'b1;       cb.h <= 1'b0;
    @(cb); @(cb);
    $display("T|end e=%b f=%b g=%b h=%b", e, f, g, h);
    #1 $finish;
  end
  always @(e) $display("T|e|t=%0t e=%b", $time, e);
  always @(f) $display("T|f|t=%0t f=%b", $time, f);
  always @(g) $display("T|g|t=%0t g=%b", $time, g);
  always @(h) $display("T|h|t=%0t h=%b", $time, h);
endmodule
