// IEEE 1800-2023 §6.7.1, §10.3.2-3, §28.5, §28.16.
// The early x values are xezim's uncached behavior, not an LRM requirement.
// Static inputs (§6.8) and samples away from updates avoid §4.7 races.
`timescale 1ns/1ps
module top;
  logic a = 1'b1;
  logic [3:0] bits_in = 4'b1010;
  wire w;
  tri0 low_pull;
  tri1 high_pull;
  wire [3:0] vector_out;
  wire gate_out;
  assign #2 w = a;
  assign #2 low_pull = a;
  assign #2 high_pull = a;
  assign #2 vector_out = bits_in;
  buf #2 g(gate_out, a);
  initial begin
    #1 $display("T|t1 wire=%b tri0=%b tri1=%b vector=%b gate=%b",
                w, low_pull, high_pull, vector_out, gate_out);
    #2 $display("T|t3 wire=%b tri0=%b tri1=%b vector=%b gate=%b",
                w, low_pull, high_pull, vector_out, gate_out);
    $finish;
  end
endmodule
