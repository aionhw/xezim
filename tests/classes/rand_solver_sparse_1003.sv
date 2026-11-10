`ifndef SVTEST_DEFS_SVH
`define SVTEST_DEFS_SVH

`define SVTEST_INIT \
  int failures = 0;

`define SVTEST_CHECK(expr, msg) \
  if (!(expr)) begin \
    failures++; \
    $display("FAIL: %s", msg); \
  end

`define SVTEST_PASSFAIL \
  if (failures == 0) begin \
    $display("TEST_PASS"); \
  end else begin \
    $display("TEST_FAIL count=%0d", failures); \
    $fatal(1); \
  end

`endif

`timescale 1ns/1ns
`define G0 64'h100000

package pkg0;
  typedef enum logic [1:0] { D0, D1, D2 } e0;
  typedef enum logic [2:0] {
    M0, M1, M2, M3, M4, M5, M6, M7
  } e1;
  typedef struct packed {
    logic f0;
    logic f1;
    logic [7:0]  f2;
    logic [14:0] f3;
    logic [17:0] f4;
    logic f5;
    logic f6;
    logic f7;
    e0   f8;
    e1 f9;
    logic [39:0] f10;
  } s0;
endpackage : pkg0

package pkg1;
  import pkg0::*;

  // Segment record.  The queue `r1` is a plain (non-rand) property that the
  // test fills before randomize(), so these two fields are inputs to the
  // solve and never randomization targets -- and `rand`/`randc` are not
  // allowed inside a packed structure (1800-2023 18.4), which some
  // simulators reject outright.
  typedef struct packed {
    longint unsigned r9;
    longint unsigned a1;
  } s1;

  class c0;
    rand s0 r0;
    s1 r1 [$];
    rand int r2;
    rand int r3;
    rand int r4;
    longint r5;
    rand longint unsigned r6;
    rand longint unsigned r7;
    rand longint unsigned r8;
    rand longint unsigned r9;
    rand longint unsigned r10;
    rand longint unsigned r11;
    rand longint unsigned r12;
    rand longint unsigned r13;
    int r14, r15;

    constraint k0 {
      if(r0.f8 == D0)  r13 == 0;
      if(r0.f8 == D1) r13 == 1;
      if(r0.f8 == D2) r13 == 2;
    }
    constraint k1 {
      r2 >= 0;
      r2 < r1.size();
    }
    constraint k2 {
      r0.f1 == 1;
    }
    constraint k3 {
      if((r1[r2].r9==0) || (r5 != -1) || r15) {
        r6 == r1[r2].r9;
      } else {
        r6 == (r1[r2].r9 + `G0);
      }
      if(r15) {
        r7 == (r1[r2].r9 + r1[r2].a1 - 1);
      } else {
        r7 == ((r1[r2].r9 + r1[r2].a1 - `G0) - 1);
      }
      r8 == r7 - r6;
    }
    constraint k4 {
      solve r9 before r11;
      solve r9 before r10;
      r9 >= 0;
      r9 < 64'h100_0000_0000;
      r10 > 0;
      r10 dist {1:=10, [2:4] :=1, [5:8] :=1, [9:16] :=1, [16:32] :=1, [33:48] :=1, [49:64] :=1, [65:96]:=1
              ,[97:112] :=1, [113:128] :=1, [129:144] :=1, [145:160] :=1, [161:176] :=1, [177:192] :=1
              ,[193:208] :=1, [209:224] :=1, [225:240] :=1, [241:255] :=1, 256 :=10 };
      r11 > 0;
      r11 dist {1 :=10, [2:4] :=1, [5:8] :=1, [9:16] :/10, [16:32] :/10, [33:64] :/10, [65:128] :/10
              , [129:256] :/10, [257:512] :=1, [513:1024] :/10, [1025:2048] :/10, [2049:4096] :/10
              , [4097:8192] :/10, [8193:16384] :/100, [16385:32767] :/1000, 32768 :=10 };
      r12 > 0;
      r12 <= 64'h400000;
      r12[3:0] == 0;
      r0.f10 == r9;
      r0.f3 == r11 - 1;
      r0.f2 == r10 - 1;
      r0.f4 == (r12>>4) - 1;
    }
    constraint k5 {
      if(r0.f9 == M2) {
        if(r0.f5 == 0) {
          r3 == ((r11+15) / 16) * 128;
          r4 == (((r10-1)/8) * 8 * r12);
          r8 >= r3 + r4;
          if(r0.f1 == 1) {
            if(r0.f6 == 0) {
              r9 >= r6;
              r9 + r3 + r4 <= r7;
            } else {
              (r9 - 8'h70) >= r6 + r4;
              (r9 - 8'h70) <= r7 - r3;
            }
          } else {
            if(r0.f6 == 0) {
              r9 >= r6 + 128;
              r9 + r3 + r4 + 8*r12 + 128 <= r7;
            } else {
              r9 >= r6 + r4 + 8*r12 + 128;
              r9 + r3 + 128 <= r7;
            }
          }
        } else {
          r3 == (((r10 << r13) + 15) / 16) * 128;
          r4 == ((((r11 >> r13)-1)/8) * 8 * r12);
          r8 >= r3 + r4;
          if(r0.f7 == 0) {
            r9 >= r6 + r4 + r3 + 8*r12 + 128;
            r9 + r3 + 128 <= r7;
          } else {
            r9 >= r6 + 128;
            r9 + r4 + r3 + 8*r12 + 128 <= r7;
          }
        }
      } else {
        if(r0.f5 == 0) {
          if(r0.f6 == 0) {
            r9 >= r6;
            r9 <= r7 - (r10 * r12);
            r8 >= (r10 * r12);
          }
          if(r0.f6 == 1) {
            r9 >= r6 + (r10 * r12);
            r9 <= r7;
            r8 >= (r10 * r12);
          }
        } else {
          if(r0.f7 == 0) {
            r9 >= r6 + (r11 * r12) + 128;
            r9 + (r11 * r12) + 128 <= r7;
          } else {
            r9 >= r6 + 128;
            r9 + (r11 * r12) + 128 <= r7;
          }
        }
      }
    }
    constraint k6 {
      if(r0.f9 inside { M7, M6, M5, M4 } && r0.f1 == 1) {
        r9[3:0] == 0;
        if(r0.f4 == 31)   {r9[11:9]  == 3'd7;}
        if(r0.f4 == 63)   {r9[12:10] == 3'd7;}
        if(r0.f4 == 127)  {r9[13:11] == 3'd7;}
        if(r0.f4 == 255)  {r9[14:12] == 3'd7;}
        if(r0.f4 == 511)  {r9[15:13] == 3'd7;}
        if(r0.f4 == 1023) {r9[16:14] == 3'd7;}
        if(r0.f4 == 2047) {r9[17:15] == 3'd7;}
        if(r0.f4 == 4095) {r9[18:16] == 3'd7;}
        if(r0.f4 == 8191) {r9[19:17] == 3'd7;}
      } else {
        if(r0.f4 == 31)   {r9[11:9]  == 3'd0;}
        if(r0.f4 == 63)   {r9[12:10] == 3'd0;}
        if(r0.f4 == 127)  {r9[13:11] == 3'd0;}
        if(r0.f4 == 255)  {r9[14:12] == 3'd0;}
        if(r0.f4 == 511)  {r9[15:13] == 3'd0;}
        if(r0.f4 == 1023) {r9[16:14] == 3'd0;}
        if(r0.f4 == 2047) {r9[17:15] == 3'd0;}
        if(r0.f4 == 4095) {r9[18:16] == 3'd0;}
        if(r0.f4 == 8191) {r9[19:17] == 3'd0;}
      }
    }
    constraint k7 {
      if(r0.f9 inside { M2 } && r0.f1 == 1) {
        r9[3:0] == 0;
        if(r0.f6 == 1) {
          r9[6:4] == 3'd7;
        } else {
          r9[6:4] == 3'd0;
        }
      }
    }
    constraint k8 {
      if(r0.f8 == D1) r0.f3[0] == 1;
      if(r0.f8 == D2) r0.f3[1:0] == 3;
    }
    constraint k9 {
      if(r0.f9 inside { M7, M6, M5, M4 }) {
        r0.f4 dist {31,63,127,255,511,1023,2047,4095,8191};
      }
    }
    function new(string name="c0");
    endfunction : new
  endclass : c0
endpackage : pkg1

// ---------------------------------------------------------------------------
// Cousin A: MAX_VARS cliff.  Two classes with *identical* constraint
// sets; only the element count differs.  The joint solver caps at 8192
// variables, so the 8193-element problem is never jointly solved and
// the draw-and-repair fallback cannot repair an array-wide exact sum:
// randomize() returns 0 although the constraint set is satisfiable.
// (Satisfiable: 0 <= 67_000_000 <= 8193 * 16383.)
// ---------------------------------------------------------------------------
package pkg2;
  class c_cap;
    rand int unsigned a[8192];
    constraint cb { foreach (a[i]) a[i] < 16384; a.sum() == 67_000_000; }
  endclass : c_cap
  class c_over;
    rand int unsigned a[8193];
    constraint cb { foreach (a[i]) a[i] < 16384; a.sum() == 67_000_000; }
  endclass : c_over
endpackage : pkg2

// ---------------------------------------------------------------------------
// Cousin B: the main MWE's class with the address r9 widened from
// longint unsigned (64 bits) to bit [64:0] (65 bits).  Everything else,
// including the segment struct s1 (kept at 64 bits) and all constraint
// bodies, is verbatim identical to pkg1.  Variables wider than 64 bits
// are split into <=64-bit segments inside the solver; the split search
// inherits the same give-up, so the failure is not an artifact of
// exactly-64-bit addressing.
// ---------------------------------------------------------------------------
package pkg3;
  import pkg0::*;

  // Same segment record as pkg1::s1; the fields are pre-assigned inputs, and
  // `rand` is not legal inside a packed structure (1800-2023 18.4).
  typedef struct packed {
    longint unsigned r9;
    longint unsigned a1;
  } s1;

  class c_wide;
    rand s0 r0;
    s1 r1 [$];
    rand int r2;
    rand int r3;
    rand int r4;
    longint r5;
    rand longint unsigned r6;
    rand longint unsigned r7;
    rand longint unsigned r8;
    rand bit [64:0] r9;
    rand longint unsigned r10;
    rand longint unsigned r11;
    rand longint unsigned r12;
    rand longint unsigned r13;
    int r14, r15;

    constraint k0 {
      if(r0.f8 == D0)  r13 == 0;
      if(r0.f8 == D1) r13 == 1;
      if(r0.f8 == D2) r13 == 2;
    }
    constraint k1 {
      r2 >= 0;
      r2 < r1.size();
    }
    constraint k2 {
      r0.f1 == 1;
    }
    constraint k3 {
      if((r1[r2].r9==0) || (r5 != -1) || r15) {
        r6 == r1[r2].r9;
      } else {
        r6 == (r1[r2].r9 + `G0);
      }
      if(r15) {
        r7 == (r1[r2].r9 + r1[r2].a1 - 1);
      } else {
        r7 == ((r1[r2].r9 + r1[r2].a1 - `G0) - 1);
      }
      r8 == r7 - r6;
    }
    constraint k4 {
      solve r9 before r11;
      solve r9 before r10;
      r9 >= 0;
      r9 < 64'h100_0000_0000;
      r10 > 0;
      r10 dist {1:=10, [2:4] :=1, [5:8] :=1, [9:16] :=1, [16:32] :=1, [33:48] :=1, [49:64] :=1, [65:96]:=1
              ,[97:112] :=1, [113:128] :=1, [129:144] :=1, [145:160] :=1, [161:176] :=1, [177:192] :=1
              ,[193:208] :=1, [209:224] :=1, [225:240] :=1, [241:255] :=1, 256 :=10 };
      r11 > 0;
      r11 dist {1 :=10, [2:4] :=1, [5:8] :=1, [9:16] :/10, [16:32] :/10, [33:64] :/10, [65:128] :/10
              , [129:256] :/10, [257:512] :=1, [513:1024] :/10, [1025:2048] :/10, [2049:4096] :/10
              , [4097:8192] :/10, [8193:16384] :/100, [16385:32767] :/1000, 32768 :=10 };
      r12 > 0;
      r12 <= 64'h400000;
      r12[3:0] == 0;
      r0.f10 == r9;
      r0.f3 == r11 - 1;
      r0.f2 == r10 - 1;
      r0.f4 == (r12>>4) - 1;
    }
    constraint k5 {
      if(r0.f9 == M2) {
        if(r0.f5 == 0) {
          r3 == ((r11+15) / 16) * 128;
          r4 == (((r10-1)/8) * 8 * r12);
          r8 >= r3 + r4;
          if(r0.f1 == 1) {
            if(r0.f6 == 0) {
              r9 >= r6;
              r9 + r3 + r4 <= r7;
            } else {
              (r9 - 8'h70) >= r6 + r4;
              (r9 - 8'h70) <= r7 - r3;
            }
          } else {
            if(r0.f6 == 0) {
              r9 >= r6 + 128;
              r9 + r3 + r4 + 8*r12 + 128 <= r7;
            } else {
              r9 >= r6 + r4 + 8*r12 + 128;
              r9 + r3 + 128 <= r7;
            }
          }
        } else {
          r3 == (((r10 << r13) + 15) / 16) * 128;
          r4 == ((((r11 >> r13)-1)/8) * 8 * r12);
          r8 >= r3 + r4;
          if(r0.f7 == 0) {
            r9 >= r6 + r4 + r3 + 8*r12 + 128;
            r9 + r3 + 128 <= r7;
          } else {
            r9 >= r6 + 128;
            r9 + r4 + r3 + 8*r12 + 128 <= r7;
          }
        }
      } else {
        if(r0.f5 == 0) {
          if(r0.f6 == 0) {
            r9 >= r6;
            r9 <= r7 - (r10 * r12);
            r8 >= (r10 * r12);
          }
          if(r0.f6 == 1) {
            r9 >= r6 + (r10 * r12);
            r9 <= r7;
            r8 >= (r10 * r12);
          }
        } else {
          if(r0.f7 == 0) {
            r9 >= r6 + (r11 * r12) + 128;
            r9 + (r11 * r12) + 128 <= r7;
          } else {
            r9 >= r6 + 128;
            r9 + (r11 * r12) + 128 <= r7;
          }
        }
      }
    }
    constraint k6 {
      if(r0.f9 inside { M7, M6, M5, M4 } && r0.f1 == 1) {
        r9[3:0] == 0;
        if(r0.f4 == 31)   {r9[11:9]  == 3'd7;}
        if(r0.f4 == 63)   {r9[12:10] == 3'd7;}
        if(r0.f4 == 127)  {r9[13:11] == 3'd7;}
        if(r0.f4 == 255)  {r9[14:12] == 3'd7;}
        if(r0.f4 == 511)  {r9[15:13] == 3'd7;}
        if(r0.f4 == 1023) {r9[16:14] == 3'd7;}
        if(r0.f4 == 2047) {r9[17:15] == 3'd7;}
        if(r0.f4 == 4095) {r9[18:16] == 3'd7;}
        if(r0.f4 == 8191) {r9[19:17] == 3'd7;}
      } else {
        if(r0.f4 == 31)   {r9[11:9]  == 3'd0;}
        if(r0.f4 == 63)   {r9[12:10] == 3'd0;}
        if(r0.f4 == 127)  {r9[13:11] == 3'd0;}
        if(r0.f4 == 255)  {r9[14:12] == 3'd0;}
        if(r0.f4 == 511)  {r9[15:13] == 3'd0;}
        if(r0.f4 == 1023) {r9[16:14] == 3'd0;}
        if(r0.f4 == 2047) {r9[17:15] == 3'd0;}
        if(r0.f4 == 4095) {r9[18:16] == 3'd0;}
        if(r0.f4 == 8191) {r9[19:17] == 3'd0;}
      }
    }
    constraint k7 {
      if(r0.f9 inside { M2 } && r0.f1 == 1) {
        r9[3:0] == 0;
        if(r0.f6 == 1) {
          r9[6:4] == 3'd7;
        } else {
          r9[6:4] == 3'd0;
        }
      }
    }
    constraint k8 {
      if(r0.f8 == D1) r0.f3[0] == 1;
      if(r0.f8 == D2) r0.f3[1:0] == 3;
    }
    constraint k9 {
      if(r0.f9 inside { M7, M6, M5, M4 }) {
        r0.f4 dist {31,63,127,255,511,1023,2047,4095,8191};
      }
    }
    function new(string name="c_wide");
    endfunction : new
  endclass : c_wide
endpackage : pkg3

// ---------------------------------------------------------------------------
// Cousins C and D: rand arrays whose elements are wider than 64 bits.
//  - c_wsum (cousin C): per-element bound + exact array sum.  Linear
//    relations are only propagated while every operand fits its SV
//    width, so the sum over 65-bit elements is never propagated and
//    both the class and the std::randomize entry points give up.
//  - c_wslc (cousin D): per-element slice constraints only.  The class
//    entry solves it (wide elements are split into segments), but
//    std::randomize rejects arrays with >64-bit elements outright, so
//    even this trivial constraint set returns 0 through the scope path.
// ---------------------------------------------------------------------------
package pkg4;
  class c_wsum;
    rand bit [64:0] a[256];
    constraint cw { foreach (a[i]) a[i] <= 65'hff; a.sum() == 65'h8000; }
  endclass : c_wsum
  class c_wslc;
    rand bit [64:0] a[8];
    constraint cs { foreach (a[i]) a[i][7:0] == 8'h42; }
  endclass : c_wslc
endpackage : pkg4

// ---------------------------------------------------------------------------
// Cousins E-H: constraint expressions the joint solver cannot model.  The CSP
// arithmetic translator (`csp_ae_raw`, rand_csp.rs) builds the model out of
// Add/Sub/Mul, constant shifts, part-selects and `sum()` only -- `/`, `%`,
// `**` and a general concatenation have no case there, so they stay opaque
// evaluator-only nodes that bounds propagation cannot use.  The search then
// degenerates into uniform random draws over a sparse solution set: it burns
// the whole WORK_BUDGET (1 000 000 constraint executions) and returns GaveUp,
// and the draw-and-repair fallback does not hit the sparse set either, so
// randomize() returns 0 on a satisfiable problem.  Every cousin has a control
// that states the same relation with a modelled operator, and xezim solves it.
// ---------------------------------------------------------------------------
package pkg5;
  // E: constant division.  chunk / 4096 == 123456 has 4096 solutions in the
  // 2^31 domain (chunk in [505675776 : 505679871]); the control is the same
  // shape written with the modelled `*`.
  class c_div;
    rand bit [30:0] chunk;
    constraint cd { chunk / 4096 == 123456; }
  endclass : c_div
  class c_divctl;
    rand int unsigned beats;
    constraint cd { beats inside {[0:100000]}; beats * 7 == 8638; }
  endclass : c_divctl

  // F: concatenation.  `{f3,f2,f1,f0} == 32'hdead_beef` has exactly one
  // solution out of 2^32 draws, and `{q1,q0} == {q3,q2}` couples four
  // 16-bit fields; the control constrains the same fields one at a time.
  class c_cat;
    rand bit [7:0] f0;
    rand bit [7:0] f1;
    rand bit [7:0] f2;
    rand bit [7:0] f3;
    constraint cc { {f3, f2, f1, f0} == 32'hdead_beef; }
  endclass : c_cat
  class c_catpair;
    rand bit [15:0] q0;
    rand bit [15:0] q1;
    rand bit [15:0] q2;
    rand bit [15:0] q3;
    constraint cp { {q1, q0} == {q3, q2}; q0 inside {[0:16'hffff]}; }
  endclass : c_catpair
  class c_catctl;
    rand bit [7:0] f0;
    rand bit [7:0] f1;
    rand bit [7:0] f2;
    rand bit [7:0] f3;
    constraint cc { f3 == 8'hde; f2 == 8'had; f1 == 8'hbe; f0 == 8'hef; }
  endclass : c_catctl

  // G: power.  x ** 2 == 3481 has the single solution x == 59 in a 100001
  // element domain; the control is the same relation as a product.
  class c_pow;
    rand int unsigned x;
    constraint cp { x inside {[0:100000]}; x ** 2 == 3481; }
  endclass : c_pow
  class c_powctl;
    rand int unsigned x;
    constraint cp { x inside {[0:100000]}; x * 59 == 3481; }
  endclass : c_powctl

  // H: modulo on an array element.  The same modulo on a scalar variable is
  // solved (the repair loop can walk a single variable), but with the modulo
  // on a dynamic-array element the whole solve is left to random draws:
  // 32768 of 2^31 values per element qualify.
  class c_emod;
    rand int unsigned a[];
    constraint cs { a.size() == 4; }
    constraint cm { foreach (a[i]) a[i] inside {[0:2147483647]};
                    a[0] % 65536 == 12345; }
  endclass : c_emod
  class c_emodctl;
    rand int unsigned x;
    constraint cm { x inside {[0:2147483647]}; x % 65536 == 12345; }
  endclass : c_emodctl
endpackage : pkg5

module tb_top;
  import pkg1::*;
  localparam int N0 = 10;
  longint unsigned v0 [16] = '{
    64'hf0defff106, 64'hcae46772af, 64'h85a2b06775, 64'ha0209553bd,
    64'h5cb989f7f4, 64'h4f1047ea18, 64'hc52fa572f9, 64'h19533fbc3,
    64'ha36dbbf981, 64'h655cb24c6b, 64'h45132d3288, 64'h24efa0b08d,
    64'h7a57089f5a, 64'hf44201f4fe, 64'ha1e40bbe3e, 64'h5704b1ba22 };
  longint unsigned v1 [16] = '{
    64'h17fbf160, 64'h154850c0, 64'h1d665ec0, 64'h1f184740,
    64'h154edb60, 64'h164a5320, 64'h156eb300, 64'h1d1f6b00,
    64'h1b57c180, 64'h156b6000, 64'h18da2520, 64'h17dc6780,
    64'h17c69bc0, 64'h1d258120, 64'h1c6fa8c0, 64'h1dc6dc00 };
  // Scope-entry targets for cousins C/D and their one-bit-narrower controls
  bit [64:0] arr65 [256];
  bit [63:0] arr64 [256];
  bit [64:0] slc65 [8];
  bit [63:0] slc64 [8];
  initial begin
    `SVTEST_INIT
    c0 x;
    x = new("c0");
    for (int i = 0; i < 16; i++) begin
      s1 s;
      s.r9 = v0[i];
      s.a1   = v1[i];
      x.r1.push_back(s);
    end
    for (int i = 0; i < N0; i++) begin
      if (!x.randomize()) begin
        failures++;
        $display("FAIL: randomize() failed at iteration %0d", i);
        break;
      end
      $display("iter %0d OK: f9=%0d f5=%0d f6=%0d f7=%0d r11=%0d r10=%0d r12=%0d r9=%0h",
               i, x.r0.f9, x.r0.f5,
               x.r0.f6, x.r0.f7,
               x.r11, x.r10, x.r12, x.r9);
    end

    // ------------------------------------------------------------------
    // Cousin A: MAX_VARS cliff — 8192 vs 8193 rand array elements.
    // ------------------------------------------------------------------
    begin : cousin_a
      pkg2::c_cap  cap;
      pkg2::c_over over;
      int okc;
      cap = new();
      okc = cap.randomize();
      $display("cousin A: 8192-element randomize() ok=%0d sum=%0d", okc, cap.a.sum());
      `SVTEST_CHECK(okc, "cousin A: 8192-element array (at the cap) failed to randomize")
      `SVTEST_CHECK(okc && cap.a.sum() == 67_000_000, "cousin A: 8192-element solution violates the sum")
      over = new();
      okc = over.randomize();
      $display("cousin A: 8193-element randomize() ok=%0d", okc);
      `SVTEST_CHECK(okc, "cousin A: 8193-element array (one past the cap) failed to randomize")
    end

    // ------------------------------------------------------------------
    // Cousin B: the main MWE with a 65-bit address (r9 widened from
    // longint unsigned to bit [64:0]); the wide-split search inherits
    // the same give-up.
    // ------------------------------------------------------------------
    begin : cousin_b
      pkg3::c_wide xw;
      pkg3::s1   sw;
      int okc;
      xw = new("c_wide");
      for (int i = 0; i < 16; i++) begin
        sw.r9 = v0[i];
        sw.a1 = v1[i];
        xw.r1.push_back(sw);
      end
      okc = xw.randomize();
      $display("cousin B: 65-bit r9 randomize() ok=%0d", okc);
      `SVTEST_CHECK(okc, "cousin B: main MWE with 65-bit r9 failed to randomize")
    end

    // ------------------------------------------------------------------
    // Cousin C: >64-bit array elements with an exact-sum coupling —
    // gives up on both entry points; one bit narrower it solves.
    // ------------------------------------------------------------------
    begin : cousin_c
      pkg4::c_wsum cw;
      int okc;
      cw = new();
      okc = cw.randomize();
      $display("cousin C: class 65-bit-element sum randomize() ok=%0d", okc);
      `SVTEST_CHECK(okc, "cousin C: class randomize with 65-bit elements + exact sum failed")
      okc = std::randomize(arr65) with {
        foreach (arr65[i]) arr65[i] <= 65'hff;
        arr65.sum() == 65'h8000;
      };
      $display("cousin C: scope 65-bit-element sum randomize() ok=%0d", okc);
      `SVTEST_CHECK(okc, "cousin C: std::randomize with 65-bit elements + exact sum failed")
      okc = std::randomize(arr64) with {
        foreach (arr64[i]) arr64[i] <= 64'hff;
        arr64.sum() == 64'h8000;
      };
      $display("cousin C: scope 64-bit-element control ok=%0d sum=%0h", okc, arr64.sum());
      `SVTEST_CHECK(okc && arr64.sum() == 64'h8000, "cousin C: 64-bit-element control failed")
    end

    // ------------------------------------------------------------------
    // Cousin D: >64-bit array elements with per-element slice
    // constraints — the class entry solves it, std::randomize returns 0;
    // one bit narrower the scope entry solves it too.
    // ------------------------------------------------------------------
    begin : cousin_d
      pkg4::c_wslc ck;
      int okc;
      int bad;
      ck = new();
      okc = ck.randomize();
      bad = 0;
      for (int i = 0; i < 8; i++)
        if (ck.a[i][7:0] != 8'h42) bad++;
      $display("cousin D: class 65-bit-element slice randomize() ok=%0d bad=%0d", okc, bad);
      `SVTEST_CHECK(okc && bad == 0, "cousin D: class control with 65-bit elements + slice failed")
      okc = std::randomize(slc65) with {
        foreach (slc65[i]) slc65[i][7:0] == 8'h42;
      };
      $display("cousin D: scope 65-bit-element slice randomize() ok=%0d", okc);
      `SVTEST_CHECK(okc, "cousin D: std::randomize with 65-bit elements + slice failed (class entry passes)")
      okc = std::randomize(slc64) with {
        foreach (slc64[i]) slc64[i][7:0] == 8'h42;
      };
      $display("cousin D: scope 64-bit-element control ok=%0d", okc);
      `SVTEST_CHECK(okc, "cousin D: 64-bit-element scope control failed")
    end
    // ------------------------------------------------------------------
    // Cousin E: constant division in a constraint (unmodelled `/`).
    // ------------------------------------------------------------------
    begin : cousin_e
      pkg5::c_div    dv;
      pkg5::c_divctl dc;
      int okc;
      dv = new();
      okc = dv.randomize();
      $display("cousin E: chunk / 4096 == 123456 randomize() ok=%0d chunk=%0d", okc, dv.chunk);
      `SVTEST_CHECK(okc && (dv.chunk / 4096) == 123456, "cousin E: constant division constraint failed to randomize")
      dc = new();
      okc = dc.randomize();
      $display("cousin E: control (beats * 7 == 8638) randomize() ok=%0d beats=%0d", okc, dc.beats);
      `SVTEST_CHECK(okc && (dc.beats * 7) == 8638, "cousin E: multiplication control failed")
    end

    // ------------------------------------------------------------------
    // Cousin F: concatenation in a constraint (unmodelled `{...}`).
    // ------------------------------------------------------------------
    begin : cousin_f
      pkg5::c_cat     ct;
      pkg5::c_catpair cp;
      pkg5::c_catctl  ck;
      int okc;
      ct = new();
      okc = ct.randomize();
      $display("cousin F: {f3,f2,f1,f0} == 32'hdead_beef randomize() ok=%0d", okc);
      `SVTEST_CHECK(okc && {ct.f3, ct.f2, ct.f1, ct.f0} == 32'hdead_beef, "cousin F: concatenation against a constant failed to randomize")
      cp = new();
      okc = cp.randomize();
      $display("cousin F: {q1,q0} == {q3,q2} randomize() ok=%0d", okc);
      `SVTEST_CHECK(okc && {cp.q1, cp.q0} == {cp.q3, cp.q2}, "cousin F: concatenation against a concatenation failed to randomize")
      ck = new();
      okc = ck.randomize();
      $display("cousin F: control (per-field equality) randomize() ok=%0d", okc);
      `SVTEST_CHECK(okc && {ck.f3, ck.f2, ck.f1, ck.f0} == 32'hdead_beef, "cousin F: per-field equality control failed")
    end

    // ------------------------------------------------------------------
    // Cousin G: power operator in a constraint (unmodelled `**`).
    // ------------------------------------------------------------------
    begin : cousin_g
      pkg5::c_pow    pw;
      pkg5::c_powctl pc;
      int okc;
      pw = new();
      okc = pw.randomize();
      $display("cousin G: x ** 2 == 3481 randomize() ok=%0d x=%0d", okc, pw.x);
      `SVTEST_CHECK(okc && (pw.x ** 2) == 3481, "cousin G: power constraint failed to randomize")
      pc = new();
      okc = pc.randomize();
      $display("cousin G: control (x * 59 == 3481) randomize() ok=%0d x=%0d", okc, pc.x);
      `SVTEST_CHECK(okc && (pc.x * 59) == 3481, "cousin G: product control failed")
    end

    // ------------------------------------------------------------------
    // Cousin H: modulo on an array element (unmodelled `%` on an element);
    // the identical modulo on a scalar variable is the control.
    // ------------------------------------------------------------------
    begin : cousin_h
      pkg5::c_emod    em;
      pkg5::c_emodctl ec;
      int okc;
      em = new();
      okc = em.randomize();
      $display("cousin H: a[0] %% 65536 == 12345 over rand array randomize() ok=%0d", okc);
      `SVTEST_CHECK(okc && em.a.size() == 4 && (em.a[0] % 65536) == 12345, "cousin H: modulo on an array element failed to randomize")
      ec = new();
      okc = ec.randomize();
      $display("cousin H: control (same modulo on a scalar) randomize() ok=%0d x=%0d", okc, ec.x);
      `SVTEST_CHECK(okc && (ec.x % 65536) == 12345, "cousin H: scalar modulo control failed")
    end


    `SVTEST_PASSFAIL
  end
endmodule


