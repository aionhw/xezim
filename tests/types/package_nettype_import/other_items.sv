// IEEE 1800-2023 §26.3 / §6.18 / §6.20 / §13.4: explicit imports
// of typedefs, parameters and functions retain their ordinary meaning.
// §4.7 / §10.3: sample one time unit after changing the continuous driver.
`timescale 1ns/1ps
package p;
  typedef logic [7:0] word_t;
  parameter int BIAS = 9;
  function automatic int twice(input int x); return 2*x; endfunction
endpackage
module top;
  import p::word_t, p::BIAS, p::twice;
  int source;
  wire word_t w;
  assign w = twice(source) + BIAS;
  initial begin
    #1 source = 3;
    #1 $display("ITEMS %0d %0d", w, $bits(w));
    #1 source = 120;
    #1 $display("ITEMS %0d %0d", w, $bits(w));
    $finish;
  end
endmodule
