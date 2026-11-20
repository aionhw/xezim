// top: t16_12
module t16_12;
  initial $assertvacuousoff(0);
  bit clk, a, b, c;
  always #5 clk = ~clk;
  // a@15; b@25,35; c@45 -> until/s_until should pass at 45 (attempt started 15)
  initial begin
    @(negedge clk) a = 1; @(negedge clk) a = 0; b = 1; @(negedge clk); @(negedge clk) b = 0; c = 1; @(negedge clk) c = 0;
    repeat (3) @(negedge clk); $finish;
  end
  p1: assert property (@(posedge clk) a |=> b until c) $display("T|until|P t=%0t", $time); else $display("T|until|F t=%0t", $time);
  p2: assert property (@(posedge clk) a |=> b s_until c) $display("T|s_until|P t=%0t", $time); else $display("T|s_until|F t=%0t", $time);
  p3: assert property (@(posedge clk) a iff b) else $display("T|iff|F t=%0t", $time);
endmodule
