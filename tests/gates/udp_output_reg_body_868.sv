// IEEE 1800-2023 29.3.2: "Sequential UDPs shall contain a reg declaration for the output
// port, either in addition to the output declaration, when the UDP is declared using the
// first form of a UDP header, or as part of the output_declaration."
// A.5.2: udp_output_declaration ::= output port_identifier | output reg port_identifier [ = ... ],
// and a non-ANSI UDP lists these declarations in its body (udp_port_declaration).
//
// Each sequential primitive below is written in three ways with the same table:
//   *_body: non-ANSI header, `output reg q;` in the body            <- the form under test
//   *_sep : non-ANSI header, `output q; reg q;` in the body         (control)
//   *_ansi: ANSI header, `output reg q`                              (control)
// Three kinds of table: level-sensitive (latch), edge-sensitive (dff), and a table that holds
// the state with an initial statement (29.3.3).
//
// Every line should print PASS. With the bug the *_body primitives are treated as
// combinational: their output stays x and the three *_body checks print FAIL.

// ---- level-sensitive ----
primitive latch_body (q, d, en);
  output reg q; input d, en;
  table
    1 1 : ? : 1;
    0 1 : ? : 0;
    ? 0 : ? : -;
  endtable
endprimitive
primitive latch_sep (q, d, en);
  output q; reg q; input d, en;
  table
    1 1 : ? : 1;
    0 1 : ? : 0;
    ? 0 : ? : -;
  endtable
endprimitive
primitive latch_ansi (output reg q, input d, input en);
  table
    1 1 : ? : 1;
    0 1 : ? : 0;
    ? 0 : ? : -;
  endtable
endprimitive

// ---- edge-sensitive ----
primitive dff_body (q, clk, d);
  output reg q; input clk, d;
  table
    (01) 0 : ? : 0;
    (01) 1 : ? : 1;
    (?0) ? : ? : -;
    ?    * : ? : -;
  endtable
endprimitive
primitive dff_sep (q, clk, d);
  output q; reg q; input clk, d;
  table
    (01) 0 : ? : 0;
    (01) 1 : ? : 1;
    (?0) ? : ? : -;
    ?    * : ? : -;
  endtable
endprimitive
primitive dff_ansi (output reg q, input clk, input d);
  table
    (01) 0 : ? : 0;
    (01) 1 : ? : 1;
    (?0) ? : ? : -;
    ?    * : ? : -;
  endtable
endprimitive

// ---- initial statement: the output keeps its initial value whatever the inputs do ----
primitive keep_body (q, a, b);
  output reg q; input a, b;
  initial q = 1'b1;
  table
    ? ? : ? : -;
  endtable
endprimitive
primitive keep_sep (q, a, b);
  output q; reg q; input a, b;
  initial q = 1'b1;
  table
    ? ? : ? : -;
  endtable
endprimitive
primitive keep_ansi (output reg q, input a, input b);
  initial q = 1'b1;
  table
    ? ? : ? : -;
  endtable
endprimitive

module t;
  reg  ld, len, clk, dd, ka, kb;
  wire lat_body, lat_sep, lat_ansi, ff_body, ff_sep, ff_ansi, kp_body, kp_sep, kp_ansi;
  latch_body u_lat_body (lat_body, ld, len);
  latch_sep  u_lat_sep  (lat_sep,  ld, len);
  latch_ansi u_lat_ansi (lat_ansi, ld, len);
  dff_body   u_ff_body  (ff_body,  clk, dd);
  dff_sep    u_ff_sep   (ff_sep,   clk, dd);
  dff_ansi   u_ff_ansi  (ff_ansi,  clk, dd);
  keep_body  u_kp_body  (kp_body,  ka, kb);
  keep_sep   u_kp_sep   (kp_sep,   ka, kb);
  keep_ansi  u_kp_ansi  (kp_ansi,  ka, kb);

  wire [2:0] lat_q = {lat_ansi, lat_sep, lat_body};   // bit 0 = body, 1 = sep, 2 = ansi
  wire [2:0] ff_q  = {ff_ansi,  ff_sep,  ff_body};
  wire [2:0] kp_q  = {kp_ansi,  kp_sep,  kp_body};
  bit  [2:0] lat_ok = 3'b111, ff_ok = 3'b111, kp_ok = 3'b111;

  int fails = 0;
  task automatic check(string label, bit ok);
    if (ok) $display("PASS %s", label);
    else begin $display("FAIL %s", label); fails++; end
  endtask

  // bit i is 1 when output i has the value exp (a 4-state compare)
  function automatic bit [2:0] is_val(input reg [2:0] got, input reg exp);
    bit [2:0] r;
    for (int i = 0; i < 3; i++) r[i] = (got[i] === exp);
    return r;
  endfunction

  initial begin
    // initial statement: the inputs are still x, then driven and changed
    #1 kp_ok &= is_val(kp_q, 1'b1);
    ka = 0; kb = 1; #1 kp_ok &= is_val(kp_q, 1'b1);
    ka = 1;         #1 kp_ok &= is_val(kp_q, 1'b1);

    // level-sensitive: transparent while len is 1, holds while len is 0
    ld = 1; len = 1; #1 lat_ok &= is_val(lat_q, 1'b1);
    len = 0; #1 ld = 0; #1 lat_ok &= is_val(lat_q, 1'b1);
    len = 1;           #1 lat_ok &= is_val(lat_q, 1'b0);

    // edge-sensitive: rising clk loads dd, a change of dd alone does not
    clk = 0; dd = 0; #1;
    clk = 1;         #1 ff_ok &= is_val(ff_q, 1'b0);
    dd = 1;          #1 ff_ok &= is_val(ff_q, 1'b0);
    clk = 0; #1 clk = 1; #1 ff_ok &= is_val(ff_q, 1'b1);
    dd = 0;          #1 ff_ok &= is_val(ff_q, 1'b1);

    check("latch, `output reg q;` in the body of a non-ANSI header",  lat_ok[0]);
    check("latch, `output q; reg q;` (control)",                        lat_ok[1]);
    check("latch, ANSI header `output reg q` (control)",                lat_ok[2]);
    check("dff, `output reg q;` in the body of a non-ANSI header",    ff_ok[0]);
    check("dff, `output q; reg q;` (control)",                          ff_ok[1]);
    check("dff, ANSI header `output reg q` (control)",                  ff_ok[2]);
    check("initial statement, `output reg q;` in the body",           kp_ok[0]);
    check("initial statement, `output q; reg q;` (control)",            kp_ok[1]);
    check("initial statement, ANSI header `output reg q` (control)",    kp_ok[2]);

    if (fails == 0) $display("ALL PASS"); else $display("%0d FAILED", fails);
    $finish;
  end
endmodule
