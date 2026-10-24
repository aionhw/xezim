// SystemVerilog testbench driving the SystemC module `counter`.
`timescale 1ns/1ps
module tb;
  bit clk, rst, en;
  bit [3:0] q;
  counter u(.clk(clk), .rst(rst), .en(en), .q(q));
  always #5 clk = ~clk;
  initial begin
    rst = 1;
    @(negedge clk);
    @(negedge clk);
    rst = 0;
    en = 1;
    repeat (18) @(negedge clk);
    en = 0;
    repeat (2) @(negedge clk);
    $display("tb: final q=%0d", q);
    $finish;
  end
endmodule
