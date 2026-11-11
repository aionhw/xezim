// IEEE 1800-2023 §26.3: import exposes the package declaration.
// §6.6.7: real nettypes use one driver or their associated automatic resolver.
// §4.7 / §10.3: stimulus at 1,3; sample settled continuous drives at 2,4.
`timescale 1ns/1ps
package p;
  function automatic real resolve(input real drivers[]);
    resolve = 0.0;
    foreach (drivers[i]) resolve += drivers[i];
  endfunction
  nettype real N with resolve;
endpackage
module top;
  import p::*;
  // §26.3: the local function shadows a wildcard import candidate.
  function automatic real resolve(input real drivers[]);
    return 99.0;
  endfunction
  real a, b;
  N w;
  assign w = a;
  assign w = b;
  initial begin
    #1 a = 1.5; b = 2.25;
    #1 $display("NET %.2f", w);
    #1 a = -3.25; b = 0.5;
    #1 $display("NET %.2f", w);
    $display("LOCAL %.2f", resolve('{1.0}));
    $finish;
  end
endmodule
