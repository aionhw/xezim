// IEEE 1800-2017 14.4: `clocking_skew ::= edge_identifier [ delay_control ]
// | delay_control`, and "output (or inout) signals are driven skew simulation
// time units after the corresponding clock event". With `default output
// negedge`, a drive at the clocking event (u) changes the net on the next
// negedge, and its fan-out (uz) wakes in that negedge's time slot. A
// per-signal `#1` skew replaces the default edge (d).
`timescale 1ns/1ps
module c14e;
  logic clk = 0;
  always #5 clk = ~clk;
  logic u = 0, d = 0;
  wire uy = u;
  logic uz;
  always_comb uz = uy;
  always @(uz) if (uz) $display("T|edge|uz rose t=%0t", $time);
  clocking cb @(posedge clk);
    default output negedge;
    output u;
    output #1 d;
  endclocking
  initial begin
    @(cb); @(cb);                // t=15
    cb.u <= 1'b1;
    cb.d <= 1'b1;
  end
  realtime at[3] = '{14, 18, 21};
  initial begin
    foreach (at[i]) begin
      #(at[i] - $realtime);
      $display("T|edge|t=%0t u=%b d=%b", $time, u, d);
    end
    $finish;
  end
endmodule
