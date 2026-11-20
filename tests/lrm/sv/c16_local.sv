// top: c16_local
module c16_local;
  // The trace records nonvacuous matches and failures.
  initial $assertvacuousoff(0);
  bit clk; bit start, done; int din, dout; bit rst;
  always #5 clk = ~clk;
  initial begin
    @(negedge clk) start = 1; din = 5;
    @(negedge clk) start = 0; din = 0;
    @(negedge clk) done = 1; dout = 6;
    @(negedge clk) done = 0; start = 1; din = 9;
    @(negedge clk) start = 0;
    @(negedge clk) done = 1; dout = 11;
    @(negedge clk) done = 0;
    @(negedge clk) start = 1; din = 1; rst = 1;
    @(negedge clk) start = 0; rst = 0;
    @(negedge clk) done = 1; dout = 0;
    @(negedge clk) done = 0;
    repeat (3) @(negedge clk);
    $finish;
  end
  property p_lv;
    int x;
    @(posedge clk) disable iff (rst) (start, x = din) |-> ##[1:3] (done && dout == x + 1);
  endproperty
  a_lv: assert property (p_lv) $display("T|16.10|P t=%0t", $time); else $display("T|16.10|F t=%0t", $time);
  sequence s_cnt(int lim);
    int k;
    (start, k = 0) ##1 (1, k++)[*1:4] ##0 (k == lim);
  endsequence
  c_cnt: cover property (@(posedge clk) s_cnt(2)) $display("T|16.10|cnt cover t=%0t", $time);
  sequence s_arg(sig, int n); sig ##n done; endsequence
  a_arg: assert property (@(posedge clk) start |-> s_arg(1'b1, 2)) $display("T|16.8|arg P t=%0t", $time); else $display("T|16.8|arg F t=%0t", $time);
  property p_rec(sig); @(posedge clk) sig |-> ##1 !sig; endproperty
  a_par: assert property (p_rec(start)) else $display("T|16.12|param F t=%0t", $time);
  // .triggered / .matched
  sequence s_tr; @(posedge clk) start ##1 !start; endsequence
  always @(posedge clk) if (s_tr.triggered) $display("T|16.13.6|triggered t=%0t", $time);
  // default clocking & default disable
  default clocking dcb @(posedge clk); endclocking
  default disable iff rst;
  a_def: assert property (start |=> !start) else $display("T|16.15|def F t=%0t", $time);
  // assertion in always with inferred clock
  always @(posedge clk) begin
    if (start) a_inf: assert property (##1 !start) $display("T|16.14.6|inf P t=%0t", $time); else $display("T|16.14.6|inf F t=%0t", $time);
  end
endmodule
