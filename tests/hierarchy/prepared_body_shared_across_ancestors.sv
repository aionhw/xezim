module leaf #(parameter int W = 4) (input logic clk, output logic [W-1:0] q);
  always_ff @(posedge clk) q <= q + 1'b1;
endmodule
module wrap #(parameter int I = 0) (input logic clk);
  logic [3:0] q0, q1, q2, q3;
  leaf u0 (.clk(clk), .q(q0));
  leaf u1 (.clk(clk), .q(q1));
  leaf u2 (.clk(clk), .q(q2));
  leaf u3 (.clk(clk), .q(q3));
  initial if (I == 63) $display("T|wrap I=%0d bits=%0d", I, $bits(q3));
endmodule
module top;
  logic clk = 0;
  for (genvar i = 0; i < 64; i++) begin : g
    wrap #(.I(i)) w (.clk(clk));
  end
  initial begin #1 $display("T|leaf W=%0d", g[5].w.u2.W); $finish; end
endmodule
