// Intentionally illegal preservation probe, not an LRM value test.
// §10.9.2 requires an exact count; §11.6/§11.8.2 first make both
// operands unsigned, then zero-extend the two-bit literal to 3'b011.
// The relational comparison is false, so the count is 1, not 2.
// Record main's fallback in procedural, continuous, argument, constant and
// class-property contexts. The Rust guard checks compatibility only.
`timescale 1ns/1ps
module top;
  typedef struct { int a, b; } S;
  typedef struct packed { byte a, b; } P;
  S s, c, arg;
  class Holder; S value; endclass
  Holder h;
  function automatic S identity(input S value); return value; endfunction
  localparam P K = '{((+2'sb11 > 3'd5) ? 2 : 1){8'd7}};
  assign c = '{((+2'sb11 > 3'd5) ? 2 : 1){7}};
  assign arg = identity('{((+2'sb11 > 3'd5) ? 2 : 1){7}});
  initial begin
    h = new;
    s = '{1, 2}; h.value = '{1, 2};
    #1 s = '{((+2'sb11 > 3'd5) ? 2 : 1){7}}; h.value = '{((+2'sb11 > 3'd5) ? 2 : 1){7}};
    #1;
    $display("OBSERVE procedural=%0d,%0d continuous=%0d,%0d argument=%0d,%0d constant=%h class=%0d,%0d", s.a, s.b, c.a, c.b, arg.a, arg.b, K, h.value.a, h.value.b);
    $finish;
  end
endmodule
