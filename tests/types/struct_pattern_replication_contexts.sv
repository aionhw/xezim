// IEEE 1800-2023 §10.9.2 member typing; §6.20 constant structures;
// §8.5 class properties use the same structure assignment rules.
`timescale 1ns/1ps
module top;
  typedef struct { logic [7:0] a; bit [15:0] b; } pair_t;
  typedef struct packed { bit [7:0] a, b, c, d; } four_t;
  localparam four_t P = '{2{8'h12, 8'h34}};
  class holder;
    pair_t p;
    pair_t elements[1];
  endclass
  typedef struct { logic [7:0] a; } single_t;
  single_t single;
  function automatic logic [7:0] take_single(input single_t s);
    return s.a;
  endfunction
  holder h;
  pair_t p;
  initial begin
    h = new;
    #1;
    h.p = '{2{7}};
    h.elements[0] = '{2{9}};
    p = pair_t'{2{11}};
    single = '{1{'1}};
    #1;
    if (single.a === 8'hff && take_single('{1{'1}}) === 8'hff)
      $display("PASS single member context"); else $display("FAIL single member context");
    if (h.p.a === 7 && h.p.b === 7) $display("PASS class property");
    else $display("FAIL class property");
    if (h.elements[0].a === 9 && h.elements[0].b === 9) $display("PASS class element");
    else $display("FAIL class element");
    if (p.a === 11 && p.b === 11) $display("PASS typed pattern");
    else $display("FAIL typed pattern");
    if (P.a === 8'h12 && P.b === 8'h34 && P.c === 8'h12 && P.d === 8'h34)
      $display("PASS constant structure"); else $display("FAIL constant structure");
    $finish;
  end
endmodule
