`timescale 1ns/1ps
module tb;
  logic [68:0] a;
  wire [68:0] y;
  wide_logic dut(.a(a), .y(y));
  initial begin
    a = 69'h125431087abcdef023;
    #1;
    if (y !== {a[67:0], a[68]}) $fatal(1, "two-state conversion");
    a[65] = 1'bx;
    a[4] = 1'bz;
    #1;
    if (y !== {a[67:0], a[68]}) $fatal(1, "four-state conversion");
    $display("PASS wide logic");
    $finish;
  end
endmodule
