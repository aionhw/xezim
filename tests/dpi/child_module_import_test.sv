// GitHub #108: a DPI import declared inside a CHILD module was never
// registered (only the top module's and packages' items were), so calls in
// any instantiated module silently returned 0/null with no diagnostic.
module nchild;
  import "DPI-C" function chandle child_probe_open(input string name);
  chandle h;
  initial begin
    h = child_probe_open("no-param-child");
    $display("nchild: null=%0d", h == null);
  end
endmodule
module ichild #(parameter int W = 1);
  import "DPI-C" function chandle child_probe_open(input string name);
  chandle h;
  initial begin
    h = child_probe_open("int-param-child");
    $display("ichild: null=%0d", h == null);
  end
endmodule
module top;
  nchild cn ();
  ichild ci ();
  initial #10 $finish;
endmodule
