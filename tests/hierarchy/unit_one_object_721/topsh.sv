int g = 5;
function automatic void addg(int by); g += by; endfunction
class K; function int get(); return g; endfunction endclass
module child; K k = new; initial begin #1; $display("T|child g=%0d k=%0d", g, k.get()); addg(4); end
  always @(g) $display("T|child saw %0d", g);
endmodule
module top;
  int g = 77;
  K k = new;
  child c();
  initial begin #2; $display("T|top local=%0d unit=%0d k=%0d", g, $unit::g, k.get()); addg(10); $display("T|top local=%0d unit=%0d", g, $unit::g); #1 $finish; end
endmodule
