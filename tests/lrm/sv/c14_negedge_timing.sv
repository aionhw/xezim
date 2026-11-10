// IEEE 1800-2017 14.16: "the corresponding signal changes value at the
// indicated clocking event as modified by the output skew", and a drive that
// executes between clocking events "shall perform their drive action as if
// they had executed at the time of the next clocking event". Under
// `default output negedge`:
// u: drive at the event at 15, changes on the negedge at 20.
// w: `##1` drive at 15, matures at the event at 25, changes at 30.
// v: drive at 17 (between events), acts at the event at 25, changes at 30.
// q (renamed, `posedge` skew): drive at 17, changes on the posedge at 35.
// b (two-state, `#0` skew): an X drive at 17 stores 0 at 25.
`timescale 1ns/1ps
module c14nt;
  logic clk = 0;
  always #5 clk = ~clk;          // posedge 5, 15, 25, 35; negedge 10, 20, 30, 40
  logic u = 0, v = 0, w = 0, qn = 0;
  bit b = 1;
  clocking cb @(posedge clk);
    default output negedge;
    output u, v, w;
    output posedge q = qn;
    output #0 b;
  endclocking
  initial begin
    @(cb); @(cb);                // t=15
    cb.u <= 1'b1;
    cb.w <= ##1 1'b1;
    #2 cb.v <= 1'b1;             // t=17
    cb.q <= 1'b1;
    cb.b <= 1'bx;
  end
  always @(qn) $display("T|chg|t=%0t qn=%b", $time, qn);
  always @(b) $display("T|chg|t=%0t b=%b", $time, b);
  always @(u) $display("T|chg|t=%0t u=%b", $time, u);
  always @(v) $display("T|chg|t=%0t v=%b", $time, v);
  always @(w) $display("T|chg|t=%0t w=%b", $time, w);
  initial begin #50 $finish; end
endmodule
