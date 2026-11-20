// top: t16_10
module t16_10;
  initial $assertvacuousoff(0);
  bit clk, start, done; int din, dout;
  always #5 clk = ~clk;
  initial begin
    @(negedge clk) start = 1; din = 5; @(negedge clk) start = 0;
    @(negedge clk) done = 1; dout = 6; @(negedge clk) done = 0;
    @(negedge clk) start = 1; din = 9; @(negedge clk) start = 0;
    @(negedge clk) done = 1; dout = 3; @(negedge clk) done = 0;
    repeat (3) @(negedge clk); $finish;
  end
  property p_lv; int x; @(posedge clk) (start, x = din) |-> ##[1:3] (done && dout == x + 1); endproperty
  a_lv: assert property (p_lv) $display("T|lv|P t=%0t", $time); else $display("T|lv|F t=%0t", $time);
endmodule
