// IEEE 1800-2023 §7.2: "the individual data types that make up the structure can be referenced by name"; §§27.3, 23.6 give independent generated declarations and hierarchical access; §10.3.2 copies 5.0.
// The upstream issue testcase, with every LRM value asserted.
`timescale 1ns/1ps
typedef struct { real V; } E;
module top;
  if (1) begin : g
    E a, b;
    assign a = '{1.0};
    assign b = '{2.0};
    initial #1 if (a.V == 1.0 && b.V == 2.0) $display("PASS inside a=%0.1f b=%0.1f", a.V, b.V);
               else $display("FAIL inside a=%0.1f b=%0.1f (expected 1.0, 2.0)", a.V, b.V);
  end
  if (1) begin : h
    E only;
    assign only = '{5.0};
  end
  real copy;
  assign copy = h.only.V;
  initial begin
    #2 if (g.a.V == 1.0 && g.b.V == 2.0 && h.only.V == 5.0)
         $display("PASS outside a=%0.1f b=%0.1f only=%0.1f", g.a.V, g.b.V, h.only.V);
       else $display("FAIL outside a=%0.1f b=%0.1f only=%0.1f (expected 1.0, 2.0, 5.0)", g.a.V, g.b.V, h.only.V);
       if (copy == 5.0) $display("PASS copy=%0.1f", copy);
       else $display("FAIL copy=%0.1f (expected 5.0)", copy);
    $finish;
  end
endmodule
