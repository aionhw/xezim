`timescale 1ns/1ps
module tiny;
  logic clk, rst, din, dout;
  always #5 clk = ~clk;
  always_ff @(posedge clk or posedge rst) begin
    if (rst) dout <= 0;
    else     dout <= din;
  end
  initial begin
    clk = 0; rst = 1; din = 0;
    #10 rst = 0;
    #10 din = 1;
    #20 din = 0;
    #20 $finish;
  end
endmodule

