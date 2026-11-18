// top: c14sw
`timescale 1ns/1ps
module c14sw;
  logic clk = 0;
  always #5 clk = ~clk;
  logic [3:0] a = 0, b = 0, c = 0;
  default clocking cb @(posedge clk);
    default output #2;
    output a, b, c;
  endclocking
  class drv;
    task run();
      cb.c[0] <= 1'b1;
      cb.c <= 4'b1010;
    endtask
  endclass
  initial begin
    drv d = new();
    @(cb);
    cb.a[0] <= 1'b1;
    cb.a <= 4'b1010;
    #1;
    cb.b[0] <= 1'b1;
    cb.b <= 4'b1010;
    @(cb);
    @(cb);
    d.run();
  end
  always @(a) $display("T|a|t=%0t a=%b", $time, a);
  always @(b) $display("T|b|t=%0t b=%b", $time, b);
  always @(c) $display("T|c|t=%0t c=%b", $time, c);
  initial #30 $finish;
endmodule
