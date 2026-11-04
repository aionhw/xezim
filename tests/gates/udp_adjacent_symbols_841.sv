// IEEE 1800-2023 A.5.3 (also 1800-2017): level_input_list ::= level_symbol { level_symbol }
// and edge_input_list ::= { level_symbol } edge_indicator { level_symbol }, with
// level_symbol ::= 0 | 1 | x | X | ? | b | B. Each symbol is one character and the
// grammar asks for no white space between them (5.3: white space only separates
// tokens), so a row may be written 01:0; as well as 0 1 : 0;.
//
// Every primitive below exists twice with the same table: "_adj" has the symbols
// of a field next to each other, "_sp" has white space between them. Both must
// behave the same, and as the expected values in the checks say.
//
// Every line should print PASS. With the bug each _adj primitive is rejected with
// "UNSUPPORTED UDP TABLE ... treated as an EMPTY module", its output stays z, and
// the _adj checks print FAIL (the _sp ones still PASS).

// Combinational, digits: and with the known-zero rows spelled out.
primitive and2_adj (o, a, b);
  output o; input a, b;
  table
    00:0;
    01:0;
    10:0;
    11:1;
    x0:0;
    0x:0;
  endtable
endprimitive
primitive and2_sp (o, a, b);
  output o; input a, b;
  table
    0 0 : 0;
    0 1 : 0;
    1 0 : 0;
    1 1 : 1;
    x 0 : 0;
    0 x : 0;
  endtable
endprimitive

// Combinational, b and x of either case next to each other: 1 when both inputs are 0 or 1,
// 0 when exactly one of them is, x when neither is (all-x inputs must give x, 29.3.4).
primitive known2_adj (o, a, b);
  output o; input a, b;
  table
    bB:1;
    xB:0;
    Bx:0;
    XX:x;
  endtable
endprimitive
primitive known2_sp (o, a, b);
  output o; input a, b;
  table
    b B : 1;
    x B : 0;
    B x : 0;
    X X : x;
  endtable
endprimitive

// Combinational, three inputs, with ?: o = s ? b : a.
primitive sel2_adj (o, s, a, b);
  output o; input s, a, b;
  table
    01?:1;
    00?:0;
    1?1:1;
    1?0:0;
  endtable
endprimitive
primitive sel2_sp (o, s, a, b);
  output o; input s, a, b;
  table
    0 1 ? : 1;
    0 0 ? : 0;
    1 ? 1 : 1;
    1 ? 0 : 0;
  endtable
endprimitive

// Level-sensitive sequential: q follows d while en is 1 and holds while en is 0.
primitive latch_adj (output reg q, input d, input en);
  table
    11:?:1;
    01:?:0;
    ?0:?:-;
  endtable
endprimitive
primitive latch_sp (output reg q, input d, input en);
  table
    1 1 : ? : 1;
    0 1 : ? : 0;
    ? 0 : ? : -;
  endtable
endprimitive

// Edge-sensitive sequential: clk rising loads d, unless rst is 1, which clears q.
// The level symbols after the edge indicator (00, 10) are next to each other.
primitive dff_adj (output reg q, input clk, input d, input rst);
  table
    (01)00:?:0;
    (01)10:?:1;
    (01)?1:?:0;
    (1?)??:?:-;
    ?*?:?:-;
    ??*:?:-;
  endtable
endprimitive
primitive dff_sp (output reg q, input clk, input d, input rst);
  table
    (01) 0 0 : ? : 0;
    (01) 1 0 : ? : 1;
    (01) ? 1 : ? : 0;
    (1?) ? ? : ? : -;
    ? * ? : ? : -;
    ? ? * : ? : -;
  endtable
endprimitive

module t;
  reg  s, a, b;
  wire and_a, and_s, kn_a, kn_s, sel_a, sel_s;
  and2_adj   u_and_a (and_a, a, b);
  and2_sp    u_and_s (and_s, a, b);
  known2_adj u_kn_a  (kn_a, a, b);
  known2_sp  u_kn_s  (kn_s, a, b);
  sel2_adj   u_sel_a (sel_a, s, a, b);
  sel2_sp    u_sel_s (sel_s, s, a, b);

  reg  d, en, clk, rst;
  wire lat_a, lat_s, ff_a, ff_s;
  latch_adj u_lat_a (lat_a, d, en);
  latch_sp  u_lat_s (lat_s, d, en);
  dff_adj   u_ff_a  (ff_a, clk, d, rst);
  dff_sp    u_ff_s  (ff_s, clk, d, rst);

  int fails = 0;
  task automatic check(string label, bit ok);
    if (ok) $display("PASS %s", label);
    else begin $display("FAIL %s", label); fails++; end
  endtask

  function automatic reg lvl(int i);
    case (i)
      0: lvl = 1'b0;
      1: lvl = 1'b1;
      default: lvl = 1'bx;
    endcase
  endfunction

  // the two outputs of one pair against the expected value
  bit and_a_ok = 1, and_s_ok = 1, kn_a_ok = 1, kn_s_ok = 1, sel_a_ok = 1, sel_s_ok = 1;
  reg e_and, e_kn, e_sel;

  initial begin
    for (int i = 0; i < 27; i++) begin
      s = lvl(i / 9);  a = lvl((i / 3) % 3);  b = lvl(i % 3);
      #1;
      e_and = (a === 1'b0 || b === 1'b0) ? 1'b0 : (a === 1'b1 && b === 1'b1) ? 1'b1 : 1'bx;
      e_kn  = (a !== 1'bx && b !== 1'bx) ? 1'b1 : (a === 1'bx && b === 1'bx) ? 1'bx : 1'b0;
      e_sel = (s === 1'b0) ? a : (s === 1'b1) ? b : 1'bx;
      if (and_a !== e_and) and_a_ok = 0;
      if (and_s !== e_and) and_s_ok = 0;
      if (kn_a  !== e_kn)  kn_a_ok  = 0;
      if (kn_s  !== e_kn)  kn_s_ok  = 0;
      if (sel_a !== e_sel) sel_a_ok = 0;
      if (sel_s !== e_sel) sel_s_ok = 0;
    end
    check("and2, symbols next to each other: all 27 input combinations",   and_a_ok);
    check("and2, symbols spaced: all 27 input combinations",               and_s_ok);
    check("known2, symbols next to each other (b, B, x, X)",               kn_a_ok);
    check("known2, symbols spaced",                                         kn_s_ok);
    check("sel2, symbols next to each other (with ?)",                     sel_a_ok);
    check("sel2, symbols spaced",                                           sel_s_ok);

    // level-sensitive: transparent while en is 1, holds while en is 0
    en = 1; d = 1; #1;
    check("latch next to each other: transparent, d=1",   lat_a === 1'b1);
    en = 0; #1 d = 0; #1;
    check("latch next to each other: holds 1 while en=0", lat_a === 1'b1);
    en = 1; #1;
    check("latch next to each other: transparent, d=0",   lat_a === 1'b0);
    check("latch spaced: the same sequence ends at 0",    lat_s === 1'b0);

    // edge-sensitive: clear with rst, then load on the rising edge
    clk = 0; rst = 1; d = 1; #1 clk = 1; #1 clk = 0; #1 rst = 0; #1;
    check("dff next to each other: rising edge with rst=1 clears", ff_a === 1'b0);
    clk = 1; #1;
    check("dff next to each other: rising edge loads d=1",         ff_a === 1'b1);
    clk = 0; d = 0; #1;
    check("dff next to each other: d alone does not change q",     ff_a === 1'b1);
    clk = 1; #1;
    check("dff next to each other: rising edge loads d=0",         ff_a === 1'b0);
    check("dff spaced: the same sequence ends at 0",               ff_s === 1'b0);

    if (fails == 0) $display("ALL PASS"); else $display("%0d FAILED", fails);
    $finish;
  end
endmodule
