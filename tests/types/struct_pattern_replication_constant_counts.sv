// IEEE 1800-2023 §10.9.2: "Replication operators can be used to set the
// values for the exact number of members", and A.6.7.1 makes the count of
// '{ constant_expression { expression { , expression } } } a constant
// expression: a parameter, a localparam or a constant operation, not only an
// integer literal. Each member expression is evaluated in the context of an
// assignment to its member's type.
`timescale 1ns/1ps
module top;
  parameter int P = 2;
  localparam int L = 1 + 1;
  localparam int H = 1;
  typedef struct { int a; int b; } pair_t;
  typedef struct { logic [7:0] n; logic [15:0] w; } wide_t;
  typedef struct { int a, b, c, d; } four_t;
  localparam pair_t KP = '{P{5}};
  localparam four_t KF = '{L{1, 2}};
  int x;
  pair_t p, l, e, nb, comb, ret;
  wide_t w;
  four_t f;
  class holder;
    pair_t v;
    function void set(int k);
      v = '{P{k}};
    endfunction
  endclass
  holder h;
  function automatic int sum(input pair_t v);
    return v.a + 10 * v.b;
  endfunction
  function automatic pair_t make(int k);
    return '{L{k}};
  endfunction
  always_comb comb = '{P{x}};
  initial begin
    h = new;
    x = 3;
    #1;
    p = '{P{x}};
    l = '{L{x + 1}};
    e = '{(H + 1){x + 2}};
    nb <= '{P{x}};
    w = '{P{-8'sd2}};
    f = '{P{x, x + 1}};
    h.set(x);
    ret = make(x);
    #1;
    $display("T|param %0d %0d localparam %0d %0d expr %0d %0d", p.a, p.b, l.a, l.b, e.a, e.b);
    $display("T|nba %0d %0d comb %0d %0d", nb.a, nb.b, comb.a, comb.b);
    $display("T|context %h %h", w.n, w.w);
    $display("T|four %0d %0d %0d %0d", f.a, f.b, f.c, f.d);
    $display("T|class %0d %0d return %0d %0d", h.v.a, h.v.b, ret.a, ret.b);
    $display("T|argument %0d", sum('{P{x}}));
    $display("T|constant %0d %0d / %0d %0d %0d %0d", KP.a, KP.b, KF.a, KF.b, KF.c, KF.d);
    $finish;
  end
endmodule
