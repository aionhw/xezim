// top: t16_9g
// §16.9.4: the global clocking sampled-value functions take one expression.
module t16_9g;
  logic clk, a;
  global clocking gck @(posedge clk); endclocking
  initial $display("%0d", $past_gclk(a, 1));
endmodule
