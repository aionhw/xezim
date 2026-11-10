// top: cgis
// Each interface instance samples its own covergroup operands: an upward
// hierarchical name (§23.8) and the implicit coverpoints of a cross of
// variables (§19.6). Only d0 and b0 see activity.
module cgis_leaf;
  logic a;
endmodule
interface cgis_if (input logic clk, input logic pa, input logic pb);
  covergroup cg @(posedge clk);
    ha: coverpoint u.a;
    x: cross pa, pb;
  endgroup
  cg c = new;
  initial #10 $display("T|19.6|%m ha=%0.2f x=%0.2f",
                       c.ha.get_inst_coverage(), c.x.get_inst_coverage());
endinterface
module cgis_unit (input logic clk, input logic a, input logic b);
  cgis_leaf u ();
  assign u.a = a;
  cgis_if ci (.clk(clk), .pa(a), .pb(b));
endmodule
module cgis_bound (input logic clk, input logic a, input logic b);
  cgis_leaf u ();
  assign u.a = a;
endmodule
module cgis;
  logic clk = 0, a0 = 0, b0 = 0, a1 = 0, b1 = 0;
  cgis_unit d0 (.clk(clk), .a(a0), .b(b0));
  cgis_unit d1 (.clk(clk), .a(a1), .b(b1));
  cgis_bound b0i (.clk(clk), .a(a0), .b(b0));
  cgis_bound b1i (.clk(clk), .a(a1), .b(b1));
  bind cgis_bound cgis_if ci (.clk(clk), .pa(a), .pb(b));
  initial begin
    #1 a0 = 0; b0 = 0; #1 clk = 1; #1 clk = 0;
    #1 a0 = 1; b0 = 0; #1 clk = 1; #1 clk = 0;
    #1 a0 = 1; b0 = 1; #1 clk = 1; #1 clk = 0;
  end
endmodule
