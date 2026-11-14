// Intentionally illegal preservation probe, not an LRM value test.
// §10.9.2 requires an exact count.
// §5.7.1 requires enough bits for an unsized integer. Narrowing the
// literal cannot prove a matching multiplier.
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
  localparam P K = '{4294967298{8'd7}};
  assign c = '{4294967298{7}};
  assign arg = identity('{4294967298{7}});
  initial begin
    h = new;
    s = '{1, 2}; h.value = '{1, 2};
    #1 s = '{4294967298{7}}; h.value = '{4294967298{7}};
    #1;
    $display("OBSERVE procedural=%0d,%0d continuous=%0d,%0d argument=%0d,%0d constant=%h class=%0d,%0d", s.a, s.b, c.a, c.b, arg.a, arg.b, K, h.value.a, h.value.b);
    $finish;
  end
endmodule
