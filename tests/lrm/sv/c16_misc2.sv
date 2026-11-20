// top: c16_misc2
module c16_misc2;
  // The trace records nonvacuous matches and failures.
  initial $assertvacuousoff(0);
  bit clk; bit start, done, rst;
  always #5 clk = ~clk;
  initial begin
    @(negedge clk) start = 1;
    @(negedge clk) start = 0;
    @(negedge clk) done = 1;
    @(negedge clk) done = 0; start = 1;
    @(negedge clk) start = 0;
    @(negedge clk) done = 1;
    @(negedge clk) done = 0;
    @(negedge clk) start = 1; rst = 1;
    @(negedge clk) start = 0; rst = 0;
    @(negedge clk) done = 1;
    @(negedge clk) done = 0;
    repeat (3) @(negedge clk);
    $finish;
  end
  sequence s_arg(sig, int n); sig ##n done; endsequence
  a_arg: assert property (@(posedge clk) start |-> s_arg(1'b1, 2)) $display("T|16.8|arg P t=%0t", $time); else $display("T|16.8|arg F t=%0t", $time);
  property p_rec(sig); @(posedge clk) sig |-> ##1 !sig; endproperty
  a_par: assert property (p_rec(start)) $display("T|16.12|param P t=%0t", $time); else $display("T|16.12|param F t=%0t", $time);
  sequence s_tr; @(posedge clk) start ##1 !start; endsequence
  always @(posedge clk) if (s_tr.triggered) $display("T|16.13.6|triggered t=%0t", $time);
  a_trig: assert property (@(posedge clk) s_tr.triggered |-> ##[1:2] done) $display("T|16.13.6|trig-prop P t=%0t", $time); else $display("T|16.13.6|trig-prop F t=%0t", $time);
  default clocking dcb @(posedge clk); endclocking
  default disable iff rst;
  a_def: assert property (start |=> !start) $display("T|16.15|def P t=%0t", $time); else $display("T|16.15|def F t=%0t", $time);
  a_dis: assert property (start |-> ##2 done) $display("T|16.15|dis P t=%0t", $time); else $display("T|16.15|dis F t=%0t", $time);
  always @(posedge clk) begin
    if (start) a_inf: assert property (##2 done) $display("T|16.14.6|inf P t=%0t", $time); else $display("T|16.14.6|inf F t=%0t", $time);
  end
  property p_named; @(posedge clk) disable iff (1'b0) start |=> !start; endproperty
  c_named: cover property (p_named) $display("T|16.14.3|cover prop t=%0t", $time);
  int ncov;
  cover property (@(posedge clk) done) ncov++;
  final $display("T|16.14.3|ncov=%0d", ncov);
  // $inferred_clock / $inferred_disable in default args
  property p_inf(x, clk_ = $inferred_clock, rs = $inferred_disable); @(clk_) disable iff (rs) x |=> !x; endproperty
  a_infclk: assert property (p_inf(start)) $display("T|16.14.7|infclk P t=%0t", $time); else $display("T|16.14.7|infclk F t=%0t", $time);
  // let
  let both(x, y) = x && y;
  a_let: assert property (@(posedge clk) !both(start, done)) else $display("T|11.12|let F t=%0t", $time);
endmodule
