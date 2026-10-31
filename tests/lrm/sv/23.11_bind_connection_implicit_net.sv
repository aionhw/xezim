// top: bcin
// A bind's port actuals name objects in the bind target (§23.11). A name the
// target does not declare is an implicit net there (§6.10), whatever the
// scope holding the bind declares.
interface bcin_edges (input logic clk);
  int n = 0;
  always @(posedge clk) n++;
  initial #30 $display("T|23.11|%m edges=%0d", n);
endinterface
module bcin_bare();
endmodule
module bcin_own();
  logic clk = 0;
  initial #5 clk = 1;
endmodule
module bcin_far();
endmodule
module bcin_binder;
  logic clk = 0;
  initial repeat (6) #2 clk = ~clk;
  bind bcin_far bcin_edges mon (.clk(clk));
endmodule
module bcin;
  logic clk = 0;
  bcin_bare bare();
  bcin_own own();
  bcin_far far();
  bcin_binder binder();
  bind bcin_bare bcin_edges mon (.clk(clk));
  bind bcin_own bcin_edges mon (.clk(clk));
  initial repeat (6) #2 clk = ~clk;
endmodule
