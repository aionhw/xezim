// IEEE 1800-2023 §23.3.3: matching user nettypes "shall be merged into a single simulated net"; §6.6.7 averages (2+0)/2=1; §6.6.1 conflicts give x; §§23.6, 27.4 permit constant generate paths.
// The upstream issue testcase, with every LRM value asserted.
`timescale 1ns/1ps
package p;
  typedef struct { int V; } E;
  function automatic E avg(input E d[]);
    E z; z.V = 0;
    foreach (d[k]) z.V += d[k].V;
    z.V /= d.size();
    return z;
  endfunction
  nettype E N with avg;
endpackage
import p::*;
module load(inout N n); assign n = '{0}; endmodule
module drv(inout wire [3:0] p); assign p = 4'b1000; endmodule
module top;
  for (genvar k = 0; k < 2; k++) begin : g
    N node; assign node = '{2};
    wire [3:0] w; assign w = 4'b1010;
    if (k == 0) begin : r
      load l(g[1].node);
      drv d(g[1].w);
    end
  end
  initial begin
    #1;
    if (g[1].node.V == 1 && g[0].r.l.n.V == 1)
      $display("PASS nettype %0d,%0d", g[1].node.V, g[0].r.l.n.V);
    else $display("FAIL nettype %0d,%0d (expected 1,1)", g[1].node.V, g[0].r.l.n.V);
    if (g[1].w === 4'b10x0 && g[0].r.d.p === 4'b10x0)
      $display("PASS wire %b,%b", g[1].w, g[0].r.d.p);
    else $display("FAIL wire %b,%b (expected 10x0,10x0)", g[1].w, g[0].r.d.p);
    $finish;
  end
endmodule
