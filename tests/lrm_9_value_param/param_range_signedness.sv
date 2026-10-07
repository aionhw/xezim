`timescale 1ns/1ps

// ----- self-check macros (tests/classes SVTEST_* style, inlined) -----
`ifndef SVTEST_DEFS_SVH
`define SVTEST_DEFS_SVH

`define SVTEST_INIT \
int failures = 0;

`define SVTEST_CHECK(expr, msg) \
if (!(expr)) begin \
  failures++; \
  $display("FAIL @%0t : %s", $time, msg); \
end

`define SVTEST_PASSFAIL \
if (failures == 0) begin \
  $display("TEST_PASS"); \
end else begin \
  $display("TEST_FAIL count=%0d", failures); \
  $fatal(1); \
end

`endif

// ============================================================================
// mwe_param_range_signedness_svtest.sv — consolidated single-file MWE for the
// localparam/parameter EXPLICIT-RANGE value-semantics bug family (the
// "locparam2sig" original bug + the cousin bugs found hunting around the
// parameter-value path in src/elaborate.rs: eval_param_value / eval_param_init
// / add_params_from_items / resolve_type_width / is_type_signed).
//
// IEEE 1800 §6.20.2 / §10.7: a parameter or localparam declared with an
// explicit type or range is ASSIGNED its initializer (and each override):
// the value is evaluated in the declared width context, then wrapped to that
// width with the DECLARED signedness.  A bare range `[3:0]` is UNSIGNED
// (`-1` -> 15, `256` in `[7:0]` -> 0); `signed [3:0]` wraps (`15` -> -1);
// an untyped parameter keeps the RHS value/sign unchanged.
//
// ORIGINAL BUG (before the fix): `localparam [1:0] C = 1 + P;` with P=1 gives
// C=2, but every use of C sees the 2-bit pattern treated as SIGNED (-2) and
// sign-extended — `assign y = C` drives 126 instead of 2, and the production
// shape (C feeding a child port) drives 126 through the hierarchy.  NOTE:
// upstream xezim-core d0997ba "Use a typed parameter's width as its value
// context" (included: xezim 289ff7bd pins core d7bd60a) fixed only the WIDTH
// context (`localparam int P = A + B` -> 300, not the 8-bit wrap 44); the
// range wrap + signedness semantics below are still wrong.
//
// COUSIN BUGS (all failed on xezim 0.11.0 git 289ff7bd + core d7bd60a, before the fix):
//  P1. localparam bare range `[1:0]` — value wrapped but treated SIGNED:
//      sign-extends through assign (A1), `bit [1:0]` (A3), generate-if
//      condition (A6), ternary condition (A7), arithmetic `C + C` (A8), and
//      width ranges `logic [C-1:0]` -> $bits=4 not 2 (A11).
//  P2. localparam via a typedef'd SIGNED type — sign LOST, the inverse
//      direction: `typedef logic signed [1:0] T` gives +2 not -2 (A4B).
//  P3. parameter bare range — the DEFAULT value is not wrapped/unsignedified:
//      `parameter [3:0] P = -1` keeps -1 (B1), `[7:0] P = 256` keeps 256 (B5).
//  P4. parameter overrides — same for named `.P(-1)` (B2) and positional
//      `#(-1)` (B3) overrides of a `[3:0]` parameter.
//  P5. parameter `signed [3:0]` — value NOT wrapped to the range: default 15
//      stays +15 (B4) and override `.P(15)` stays +15 (B8); expected -1.
//  P6. localparam declared IN the parameter_port_list — `localparam [3:0]
//      LP = -1` keeps -1 (B6).
//  P7. hierarchical read of the mangled localparam `u.C` returns the signed
//      value -2 (C1) — the wrong value is visible cross-module too.
//  P8. the original shape: localparam C feeding a CHILD PORT `.n(C)` drives
//      126 (ORIG).
//
// [control] checks that PASS today and must keep passing after a fix (they
// pin the correct semantics so the fix does not overcorrect): A2 (signed
// [1:0] wraps 2 -> -2 -> 126), A4 (typedef'd unsigned), A5 (int), A9 (shift
// amount), A10 (replication count), A12 (real), B7 (untyped parameter keeps
// -1), B9 / B9TOP (parameter [3:0] P=15 consumed by localparam [7:0] C=P+P).
//
// SVTEST style: only FAILING checks print ("FAIL @<time> : <msg>"); the
// final block prints the TEST_PASS / TEST_FAIL count=N verdict.
//
// Run:   xezim -s top mwe_param_range_signedness_svtest.sv -l mwe_param_range_signedness_svtest.log
//        xrun  -sv -licqueue mwe_param_range_signedness_svtest.sv
// Today: TEST_FAIL count=24 (xezim 289ff7bd + core d7bd60a).  Xcelium
//        24.09.006: TEST_PASS.  After the fix: TEST_PASS on both.
// ============================================================================

// ----- PART 1 modules: localparam with an explicit range/type -----

module mid_a1  #(parameter P = 1) (output [6:0] y);
  localparam [1:0] C = 1 + P;         // 2, wrapped to 2'b10, UNSIGNED
  assign y = C;                       // BROKEN: sign-extends to 126
endmodule

module mid_a2  #(parameter P = 1) (output [6:0] y);
  localparam signed [1:0] C = 1 + P;  // 2 wraps to -2 in 2-bit signed
  assign y = C;                       // [control] 126 is CORRECT here
endmodule

module mid_a3  #(parameter P = 1) (output [6:0] y);
  localparam bit [1:0] C = 1 + P;     // 2-state 2-bit unsigned
  assign y = C;                       // BROKEN: 126
endmodule

module mid_a4  #(parameter P = 1) (output [6:0] y);
  typedef logic [1:0] T;
  localparam T C = 1 + P;             // [control] typedef'd unsigned works
  assign y = C;
endmodule

module mid_a4b #(parameter P = 1) (output [6:0] y);
  typedef logic signed [1:0] T;
  localparam T C = 1 + P;             // 2 wraps to -2 (signed typedef)
  assign y = C;                       // BROKEN: sign lost, gives 2
endmodule

module mid_a5  #(parameter P = 1) (output [6:0] y);
  localparam int C = 1 + P;           // [control] int: 32-bit, no wrap
  assign y = C;
endmodule

module mid_a6  #(parameter P = 1) (output [6:0] y);
  localparam [1:0] C = 1 + P;
  if (C == 2) assign y = 7'd2;        // generate-if condition: BROKEN (C==-2)
  else        assign y = 7'd126;
endmodule

module mid_a7  #(parameter P = 1) (output [6:0] y);
  localparam [1:0] C = 1 + P;
  assign y = (C == 2) ? 7'd2 : 7'd126;  // ternary condition: BROKEN
endmodule

module mid_a8  #(parameter P = 1) (output [6:0] y);
  localparam [1:0] C = 1 + P;
  assign y = C + C;                   // BROKEN: -2 + -2 = -4 -> 124
endmodule

module mid_a9  #(parameter P = 1) (output [6:0] y);
  localparam [1:0] C = 1 + P;
  assign y = 7'd1 << C;               // [control] shift amount works
endmodule

module mid_a10 #(parameter P = 1) (output [6:0] y);
  localparam [1:0] C = 1 + P;
  assign y = {C{1'b1}};               // [control] replication count works
endmodule

module mid_a11 #(parameter P = 1) (output [6:0] y);
  localparam [1:0] C = 1 + P;
  logic [C-1:0] r;                    // [1:0] -> 2 bits
  assign y = $bits(r);                // BROKEN: range sees C=-2 -> 4 bits
endmodule

module mid_a12 #(parameter P = 1) (output [6:0] y);
  localparam real R = 1 + P;          // [control] real localparam
  assign y = (R == 2.0);
endmodule

// ----- PART 2 modules: parameter with an explicit range (default/override) -----

module mid_b1 #(parameter [3:0] P = -1) (output [6:0] y);
  assign y = P;                       // BROKEN: -1 kept, sign-extends to 127
endmodule

module mid_b2 #(parameter [3:0] P = 0) (output [6:0] y);
  assign y = P;                       // BROKEN: .P(-1) override not wrapped
endmodule

module mid_b3 #(parameter [3:0] P = 0) (output [6:0] y);
  assign y = P;                       // BROKEN: positional #(-1) not wrapped
endmodule

module mid_b4 #(parameter signed [3:0] P = 15) (output [6:0] y);
  assign y = P;                       // BROKEN: 15 not wrapped to -1 -> 15
endmodule

module mid_b5 #(parameter [7:0] P = 256) (output [31:0] y);
  assign y = P;                       // BROKEN: 256 not wrapped to 0
endmodule

module mid_b6 #(parameter P = 1, localparam [3:0] LP = -1) (output [6:0] y);
  assign y = LP;                      // BROKEN: localparam in #() list, -1 kept
endmodule

module mid_b7 #(parameter P = -1) (output [6:0] y);
  assign y = P;                       // [control] untyped keeps -1 (127)
endmodule

module mid_b8 #(parameter signed [3:0] P = 0) (output [6:0] y);
  assign y = P;                       // BROKEN: .P(15) stays +15, not -1
endmodule

module mid_b9 #(parameter [3:0] P = 15) (output [6:0] y);
  localparam [7:0] C = P + P;         // [control] 30
  assign y = C;
endmodule

// ----- PART 3 modules: consumption contexts (hier read, child port) -----

module child (input [1:0] n, output [6:0] y);
  assign y = n;
endmodule

module mid_c1 #(parameter P = 1) (output [6:0] y);
  localparam [1:0] C = 1 + P;
  assign y = C;                       // read hierarchically as u_c1.C too
endmodule

module mid_orig #(parameter P = 0) (output [6:0] y);
  localparam [1:0] C = 1 + P;         // the ORIGINAL bug shape
  child u (.n(C), .y(y));             // BROKEN: 126 reaches the child port
endmodule

// ----- top: instantiate everything, check, verdict -----

module top;
  `SVTEST_INIT

  // [control] B9TOP: module-scope parameters (no instance override path)
  parameter [3:0] TP = 15;
  localparam [7:0] TC = TP + TP;

  wire [6:0]  ya1, ya2, ya3, ya4, ya4b, ya5, ya6, ya7, ya8, ya9, ya10, ya11, ya12;
  wire [6:0]  yb1, yb2, yb3, yb4, yb6, yb7, yb8, yb9;
  wire [31:0] yb5;
  wire [6:0]  yc1, yorig;

  mid_a1   u_a1  (.y(ya1));
  mid_a2   u_a2  (.y(ya2));
  mid_a3   u_a3  (.y(ya3));
  mid_a4   u_a4  (.y(ya4));
  mid_a4b  u_a4b (.y(ya4b));
  mid_a5   u_a5  (.y(ya5));
  mid_a6   u_a6  (.y(ya6));
  mid_a7   u_a7  (.y(ya7));
  mid_a8   u_a8  (.y(ya8));
  mid_a9   u_a9  (.y(ya9));
  mid_a10  u_a10 (.y(ya10));
  mid_a11  u_a11 (.y(ya11));
  mid_a12  u_a12 (.y(ya12));

  mid_b1   u_b1  (.y(yb1));
  mid_b2 #(.P(-1)) u_b2 (.y(yb2));
  mid_b3 #(-1)     u_b3 (.y(yb3));
  mid_b4   u_b4  (.y(yb4));
  mid_b5   u_b5  (.y(yb5));
  mid_b6   u_b6  (.y(yb6));
  mid_b7   u_b7  (.y(yb7));
  mid_b8 #(.P(15)) u_b8 (.y(yb8));
  mid_b9   u_b9  (.y(yb9));

  mid_c1   u_c1  (.y(yc1));
  mid_orig #(.P(1)) u_orig (.y(yorig));

  initial begin
    #1;

    // ---- PART 1: localparam explicit range/type value semantics ----
    `SVTEST_CHECK(ya1 === 7'd2,
      $sformatf("A1 localparam [1:0] C=1+P assign y=%0d (expect 2, unsigned range must not sign-extend)", ya1))
    `SVTEST_CHECK(ya2 === 7'd126,
      $sformatf("A2 [control] localparam signed [1:0] y=%0d (expect 126)", ya2))
    `SVTEST_CHECK(ya3 === 7'd2,
      $sformatf("A3 localparam bit [1:0] y=%0d (expect 2)", ya3))
    `SVTEST_CHECK(ya4 === 7'd2,
      $sformatf("A4 [control] typedef logic [1:0] y=%0d (expect 2)", ya4))
    `SVTEST_CHECK(ya4b === 7'd126,
      $sformatf("A4B typedef logic signed [1:0] y=%0d (expect 126, sign lost)", ya4b))
    `SVTEST_CHECK(ya5 === 7'd2,
      $sformatf("A5 [control] localparam int y=%0d (expect 2)", ya5))
    `SVTEST_CHECK(ya6 === 7'd2,
      $sformatf("A6 generate-if cond C==2 y=%0d (expect 2)", ya6))
    `SVTEST_CHECK(ya7 === 7'd2,
      $sformatf("A7 ternary cond C==2 y=%0d (expect 2)", ya7))
    `SVTEST_CHECK(ya8 === 7'd4,
      $sformatf("A8 C+C y=%0d (expect 4, not -4 sign-extended)", ya8))
    `SVTEST_CHECK(ya9 === 7'd4,
      $sformatf("A9 [control] 1<<C y=%0d (expect 4)", ya9))
    `SVTEST_CHECK(ya10 === 7'd3,
      $sformatf("A10 [control] {C{1'b1}} y=%0d (expect 3)", ya10))
    `SVTEST_CHECK(ya11 === 7'd2,
      $sformatf("A11 $bits([C-1:0]) y=%0d (expect 2)", ya11))
    `SVTEST_CHECK(ya12 === 7'd1,
      $sformatf("A12 [control] real R==2.0 y=%0d (expect 1)", ya12))

    // ---- PART 2: parameter explicit range, defaults and overrides ----
    `SVTEST_CHECK(yb1 === 7'd15,
      $sformatf("B1 parameter [3:0] P=-1 y=%0d (expect 15, wrapped unsigned)", yb1))
    `SVTEST_CHECK(u_b1.P === 15,
      $sformatf("B1P hier u_b1.P=%0d (expect 15)", u_b1.P))
    `SVTEST_CHECK(yb2 === 7'd15,
      $sformatf("B2 .P(-1) override y=%0d (expect 15)", yb2))
    `SVTEST_CHECK(u_b2.P === 15,
      $sformatf("B2P hier u_b2.P=%0d (expect 15)", u_b2.P))
    `SVTEST_CHECK(yb3 === 7'd15,
      $sformatf("B3 positional #(-1) y=%0d (expect 15)", yb3))
    `SVTEST_CHECK(u_b3.P === 15,
      $sformatf("B3P hier u_b3.P=%0d (expect 15)", u_b3.P))
    `SVTEST_CHECK(yb4 === 7'd127,
      $sformatf("B4 parameter signed [3:0] P=15 y=%0d (expect 127 via -1)", yb4))
    `SVTEST_CHECK(u_b4.P === -1,
      $sformatf("B4P hier u_b4.P=%0d (expect -1)", u_b4.P))
    `SVTEST_CHECK(yb5 === 32'd0,
      $sformatf("B5 parameter [7:0] P=256 y=%0d (expect 0, wrapped)", yb5))
    `SVTEST_CHECK(u_b5.P === 0,
      $sformatf("B5P hier u_b5.P=%0d (expect 0)", u_b5.P))
    `SVTEST_CHECK(yb6 === 7'd15,
      $sformatf("B6 localparam [3:0] LP=-1 in #() list y=%0d (expect 15)", yb6))
    `SVTEST_CHECK(u_b6.LP === 15,
      $sformatf("B6P hier u_b6.LP=%0d (expect 15)", u_b6.LP))
    `SVTEST_CHECK(u_b7.P === -1,
      $sformatf("B7 [control] untyped P=-1 hier=%0d (expect -1)", u_b7.P))
    `SVTEST_CHECK(yb8 === 7'd127,
      $sformatf("B8 signed [3:0] .P(15) override y=%0d (expect 127 via -1)", yb8))
    `SVTEST_CHECK(u_b8.P === -1,
      $sformatf("B8P hier u_b8.P=%0d (expect -1)", u_b8.P))
    `SVTEST_CHECK(yb9 === 7'd30,
      $sformatf("B9 [control] [3:0] P=15, [7:0] C=P+P y=%0d (expect 30)", yb9))
    `SVTEST_CHECK(TC === 8'd30,
      $sformatf("B9TOP [control] module-scope TC=%0d (expect 30)", TC))

    // ---- PART 3: consumption contexts ----
    `SVTEST_CHECK(u_c1.C === 2,
      $sformatf("C1 hier u_c1.C=%0d (expect 2)", u_c1.C))
    `SVTEST_CHECK(yc1 === 7'd2,
      $sformatf("C1Y localparam to signal y=%0d (expect 2)", yc1))
    `SVTEST_CHECK(yorig === 7'd2,
      $sformatf("ORIG localparam C to child port y=%0d (expect 2)", yorig))
  end

  final begin
    `SVTEST_PASSFAIL
  end
endmodule