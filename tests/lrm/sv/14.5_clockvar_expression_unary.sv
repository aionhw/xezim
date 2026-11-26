// top: c145u
// §14.5: an input bound to an expression whose root is a unary operator
// (`~op`, `&ra`, `-sg`) samples that expression, like any other.
`timescale 1ns/1ns
module c145u;
  bit clk = 0;
  always #5 clk = ~clk;
  logic [3:0] op = 4'h5, ra = 4'ha;
  logic signed [7:0] sg = -8'sd4;
  clocking cb @(posedge clk);
    input un = ~op;
    input red = &ra;
    input ng = -sg;
  endclocking
  initial begin
    @(cb); #1;
    $display("T|u|un=%h red=%b ng=%0d", cb.un, cb.red, cb.ng);
    $finish;
  end
endmodule
