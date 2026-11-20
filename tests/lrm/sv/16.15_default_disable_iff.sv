// top: t16_15
module t16_15;
  initial $assertvacuousoff(0);
  bit clk, start, done, rst;
  always #5 clk = ~clk;
  initial begin
    @(negedge clk) start = 1; rst = 1;
    @(negedge clk) start = 0; rst = 0;
    @(negedge clk) done = 1;
    @(negedge clk) done = 0;
    repeat (2) @(negedge clk); $finish;
  end
  default clocking dcb @(posedge clk); endclocking
  default disable iff rst;
  a_dis: assert property (start |-> ##2 done) $display("T|dflt|P t=%0t (should be disabled)", $time); else $display("T|dflt|F t=%0t", $time);
  a_exp: assert property (disable iff (rst) start |-> ##2 done) $display("T|expl|P t=%0t (should be disabled)", $time); else $display("T|expl|F t=%0t", $time);
endmodule
