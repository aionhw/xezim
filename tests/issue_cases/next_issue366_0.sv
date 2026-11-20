module t;
  initial $assertvacuousoff(0);
  logic clk = 0;
  always #5 clk = ~clk;
  logic [31:0] st = 0;
  int k = 0;
  logic en = 0;
  always_ff @(posedge clk) begin st <= st + 1; k <= k + 1; en <= ~en; end
  // At each tick st and k are sampled as the number of earlier ticks, and en as k[0].
  int h1 = 0, h3 = 0, f1 = 0, f3 = 0;
  a1: assert property (@(posedge clk) k >= 4 |-> $past(st, 1, en) == (st % 2 ? st - 2 : st - 1)) h1++; else f1++;
  a3: assert property (@(posedge clk) k >= 8 |-> $past(st, 3, en) == (st % 2 ? st - 6 : st - 5)) h3++; else f3++;
  initial begin #300; $display("RESULT h1=%0d f1=%0d h3=%0d f3=%0d", h1, f1, h3, f3); $finish; end
endmodule
