// IEEE 1800-2017 14.4 and 14.16: an edge output skew drives on that edge of
// the clock after the clocking event, for a whole output, a select of one
// (sel), and an output whose net another clockvar also names (q, seen as
// `observed`). A drive with an edge plus delay skew (t) reaches the net at its
// own time, so drives made one clock cycle apart both appear.
// q, sel: driven at the event at 15, change on the negedge at 20. A whole-output
// drive of sel drops the earlier select of its bit 1; the later select of bit 0 stays.
// w: a whole-output drive drops only the earlier drives that land at the same time, so
// the delayed select of bit 0 reaches the net at 25.
// t: driven at 5 and at 15, change at 10 + 15 and 20 + 15.
`timescale 1ns/1ps
module c14es;
  logic clk = 0;
  always #5 clk = ~clk;          // posedge 5, 15, 25, 35; negedge 10, 20, 30, 40
  logic q = 0, t = 0;
  logic [1:0] sel = 0, w = 0;
  clocking cb @(posedge clk);
    input observed = q;
    output negedge q;
    output negedge #15 t;
    output negedge sel;
    output negedge #5 slowbit = w;
    output negedge w;
  endclocking
  initial begin
    @(cb);                       // t=5
    cb.t <= 1'b1;
    @(cb);                       // t=15
    cb.t <= 1'b0;
    cb.q <= 1'b1;
    cb.sel[1] <= 1'b1;           // replaced by the whole-output drive below
    cb.sel <= 2'b00;
    cb.sel[0] <= 1'b1;
    cb.slowbit[0] <= 1'b1;       // five units after the whole-output drive below
    cb.w <= 2'b00;
  end
  always @(q) $display("T|chg|t=%0t q=%b", $time, q);
  always @(sel) $display("T|chg|t=%0t sel=%b", $time, sel);
  always @(w) $display("T|chg|t=%0t w=%b", $time, w);
  always @(t) $display("T|chg|t=%0t t=%b", $time, t);
  initial #60 $finish;
endmodule
