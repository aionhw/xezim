// IEEE 1800-2023 §26.3: import exposes the package declaration.
// §6.6.7: real nettypes use one driver or their associated automatic resolver.
// §4.7 / §10.3: stimulus at 1,3; sample settled continuous drives at 2,4.
`timescale 1ns/1ps
package p; nettype real N; endpackage
// §26.4: the header import makes N visible to the input net port.
module reader import p::N; (input N n, output real seen);
  assign seen = n;
endmodule
module top;
  import p::N;
  real a, b, seen;
  N w;
  reader u(.n(w), .seen(seen));
  assign w = a;
  initial begin
    #1 a = 1.5; b = 2.25;
    #1 $display("PORT %.2f %.2f", w, seen);
    #1 a = -3.25; b = 0.5;
    #1 $display("PORT %.2f %.2f", w, seen);
    $finish;
  end
endmodule
