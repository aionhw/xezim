// SystemVerilog around VHDL around SystemVerilog (mixed_sandwich.vhd).
`timescale 1ns/1ps
module mixed_sandwich;
  logic [7:0] a, b;
  wire [7:0] y;
  wire gt;
  mid u_mid (.a(a), .b(b), .y(y), .gt(gt));
  initial begin
    a = 8'h10; b = 8'h20;
    #1 $display("y=%h gt=%b", y, gt);
    a = 8'hf0;
    #1 $display("y=%h gt=%b", y, gt);
    b = 8'bxxxx0000;
    #1 $display("y=%h gt=%b", y, gt);
    $finish;
  end
endmodule

module leaf (input [7:0] x, input [7:0] z, output [7:0] o);
  assign o = x ^ z;
endmodule
