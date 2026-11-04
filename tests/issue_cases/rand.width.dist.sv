`timescale 1ns/1ps

// =============================================================================
// CONSOLIDATED MWE: xezim randomize() WIDTH-OVERFLOW family + dist obedience
//
// One file, SVTEST-macro self-checks, consolidating every MWE drafted in this
// investigation (originals + cousin hunt). Sections:
//
//   A  `==` assigns an RHS WIDER than the rand variable: 40-bit arithmetic
//      RHS stored into a 32-bit int unsigned; infeasible sets "solve".
//      (mwe_eq_width_overflow.sv)                                 [BUG]
//   B  >64-bit rand struct + member-to-member arithmetic: the CSP cannot
//      model struct members and the trial path never repairs member sums,
//      so randomize() exhausts its budget and returns 0.
//      (mwe_wide_struct_member_arith.sv, 3 checks)                [BUG]
//   C  `!=` re-pick uses the OTHER operand's width: a 40-bit value is
//      stored into a 1-bit rand var (cousin of A, different arm).
//      (mwe_neq_width_overflow.sv)                                [BUG]
//   D  `==` stores a plain 32-bit literal into a signed byte without
//      masking: randomize "succeeds" with b = 300 (cousin of A).
//      (mwe_eq_narrow_signed.sv)                                  [BUG]
//   E  multi-randomized dist distribution checks D1-D8 (:= / :/ weights,
//      ranges, signed items, narrow vars, dist+relational, dist+inside,
//      struct-member dist). xezim currently OBEYS these dists; the checks
//      stay as regression guards.      (mwe_dist_distribution.sv) [BOUNDARY]
//   F  width-boundary contrast checks W1-W6 around the relational / affine
//      / inside / dist forcing arms (the clamping paths that are correct).
//      (mwe_width_boundaries.sv)                                  [BOUNDARY]
//
// Expected:
//   xezim 0.11.0 (git 4751de22): TEST_FAIL (A: 2, B: 1, C: 1, D: 2 = 6;
//                                E and F pass)
//
// Run:  xezim mwe_rand_width_dist_all.sv
//       xrun  -sv -64bit mwe_rand_width_dist_all.sv
// =============================================================================

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

module t;

  // ===========================================================================
  // SECTION B types: register block (16-bit mode sub-struct + 8 x 32-bit)
  // ===========================================================================
  typedef logic [3:0] u4_t;
  typedef logic [1:0] u2_t;

  typedef struct packed {
    bit [1:0] INTLV;
    bit [1:0] FETCH;
    bit [1:0] TYPE;
    bit [1:0] GATING;
    bit [3:0] NUM_HOSTS;
    bit [3:0] FIRST_HOST;
    bit [3:0] SPARE_A;
    bit [3:0] SPARE_B;
  } mode_t;

  typedef struct packed {          // 280 bits  -> wider than 64 bits
    mode_t   MODE;
    bit [31:0] REG_A;
    bit [31:0] REG_B;
    bit [31:0] REG_C;
    bit [31:0] REG_D;
    bit [31:0] REG_E;
    bit [31:0] REG_F;
    bit [31:0] REG_G;
    bit [31:0] REG_H;
  } regs_wide_t;

  // ===========================================================================
  // SECTION B classes
  // ===========================================================================
  class BaseCfg;                   // ~ production AbstractClientCtrlRegisters
    rand regs_wide_t regs;
    integer RUNARG_FIRST_HOST = -1;
    integer RUNARG_NUM_HOSTS  = -1;
    integer RUNARG_INTLV      = -1;
    constraint __runarg_register_parsing {
      if (RUNARG_FIRST_HOST != -1) { regs.MODE.FIRST_HOST == RUNARG_FIRST_HOST; }
      if (RUNARG_NUM_HOSTS  != -1) { regs.MODE.NUM_HOSTS  == RUNARG_NUM_HOSTS; }
      if (RUNARG_INTLV      != -1) { regs.MODE.INTLV      == RUNARG_INTLV; }
    }
  endclass

  class ClientCfg extends BaseCfg; // ~ production DramClientCtrlConfig
    rand u4_t nhosts_pre;
    constraint c_num_hosts {
      nhosts_pre dist { 0 :/ 15, 1 :/ 7, 2 :/ 2, 3 :/ 1 };
      regs.MODE.NUM_HOSTS == (4'hf >> nhosts_pre);
    }
    constraint c_mode_cons {
      ({1'b0, regs.MODE.NUM_HOSTS} + {1'b0, regs.MODE.FIRST_HOST}) <= (2 - 1);
      regs.MODE.INTLV == 0;
      ({1'b0, regs.MODE.FETCH} + {1'b0, regs.MODE.TYPE})   <= (2 - 2);
      ({1'b0, regs.MODE.GATING} + {1'b0, regs.MODE.INTLV}) <= (2 - 2);
      ({1'b0, regs.MODE.TYPE} + {1'b0, regs.MODE.GATING})  <= (2 - 2);
      ({1'b0, regs.MODE.SPARE_A} + {1'b0, regs.MODE.SPARE_B}) <= (2 - 2);
    }
  endclass

  class ScalarCfg;
    rand u4_t nhosts_pre;
    rand u4_t NUM_HOSTS;
    rand u4_t FIRST_HOST;
    rand u2_t INTLV;
    rand u2_t FETCH;
    rand u2_t TYPE;
    rand u2_t GATING;
    constraint c_num_hosts {
      nhosts_pre dist { 0 :/ 15, 1 :/ 7, 2 :/ 2, 3 :/ 1 };
      NUM_HOSTS == (4'hf >> nhosts_pre);
    }
    constraint c_mode_cons {
      ({1'b0, NUM_HOSTS} + {1'b0, FIRST_HOST}) <= (2 - 1);
      INTLV == 0;
      ({1'b0, FETCH} + {1'b0, TYPE})   <= (2 - 2);
      ({1'b0, GATING} + {1'b0, INTLV}) <= (2 - 2);
    }
  endclass

  class WideEqCfg;
    rand regs_wide_t regs;
    rand u4_t nhosts_pre;
    constraint c_num_hosts {
      nhosts_pre dist { 0 :/ 15, 1 :/ 7, 2 :/ 2, 3 :/ 1 };
      regs.MODE.NUM_HOSTS == (4'hf >> nhosts_pre);
    }
    constraint c_mode_cons {
      regs.MODE.INTLV == 0;
      regs.MODE.FIRST_HOST == 0;
      regs.MODE.FETCH == 0;
      regs.MODE.TYPE == 0;
      regs.MODE.GATING == 0;
    }
  endclass

  // ===========================================================================
  // SECTION A class: `==` RHS wider than the variable (40-bit arithmetic)
  // ===========================================================================
  class EqOv;
    rand int unsigned tot;   // 32 bits
    rand int unsigned ma;    // 32 bits, max 4294967295
    constraint c_tot { tot == 0; }
    constraint c_ma  { ma == ((40'h800_0000 * tot) - 1); }
  endclass

  // ===========================================================================
  // SECTION C class: `!=` re-pick at the OTHER operand's width.
  // The >64-bit sibling forces the trial path (csp_vars: NotApplicable).
  // ===========================================================================
  class NeqW;
    rand bit b0;               // 1-bit victim
    rand logic [127:0] wide;
    constraint c_neq { b0 != 40'd1; }
  endclass

  // ===========================================================================
  // SECTION D class: `==` stores a 32-bit literal into a signed byte.
  // ===========================================================================
  class EqN;
    rand byte b;               // signed 8-bit victim
    rand logic [127:0] wide;
    constraint c_eq { b == 32'd300; }
  endclass

  // ===========================================================================
  // SECTION E classes: dist distribution probes (N randomizes each)
  // ===========================================================================
  class DistW;  // E1 plain := weights
    rand int unsigned v;
    constraint c { v dist { 0 := 1, 1 := 2, 2 := 3, 3 := 4 }; }
  endclass

  class DistR;  // E2 :/ range split
    rand int unsigned v;
    constraint c { v dist { 1 := 1, [2:3] :/ 1 }; }
  endclass

  class DistE;  // E3 := over a range
    rand int unsigned v;
    constraint c { v dist { 1 := 1, [2:3] := 1 }; }
  endclass

  class DistS;  // E4 signed dist items
    rand int s;
    constraint c { s dist { -1 := 1, 1 := 1 }; }
  endclass

  class DistN;  // E5 narrow 4-bit var, range item
    rand bit [3:0] x;
    constraint c { x dist { [0:15] := 1 }; }
  endclass

  class DistC;  // E6 dist + hard relational conditioning
    rand int unsigned v;
    constraint c { v dist { 5 := 1, 6 := 3 }; v >= 6; }
  endclass

  class DistI;  // E7 dist + inside
    rand int unsigned v;
    constraint c { v inside { [0:9] }; v dist { 0 := 1, [1:9] :/ 1 }; }
  endclass

  class DistM;  // E8 dist on a member of a >64-bit rand struct
    typedef struct packed {
      bit [3:0]    f;
      bit [127:0]  pad;   // makes the struct >64 bits -> trial path
    } st_t;
    rand st_t s;
    constraint c { s.f dist { 0 := 1, 15 := 1 }; }
  endclass

  // ===========================================================================
  // SECTION F classes: width-boundary contrasts (all carry the >64-bit
  // sibling so the trial-path forcing arms run)
  // ===========================================================================
  class W1C;
    rand bit [3:0]    x;
    rand logic [127:0] wide;
    constraint c { x > 32'hFFFFFFF0; }        // infeasible for a 4-bit var
  endclass

  class W2C;
    rand byte          sb;
    rand logic [127:0] wide;
    constraint c { sb inside { [-128:-120], [120:127] }; }
  endclass

  class W3C;
    rand byte          sb;
    rand logic [127:0] wide;
    constraint c { sb + 5 == 32'd300; }        // 295 not representable in byte
  endclass

  class W4C;
    rand byte          sb;
    rand logic [127:0] wide;
    constraint c { sb + 5 == 32'd100; }        // sb = 95
  endclass

  class W5C;
    rand bit [3:0]    x;
    rand logic [127:0] wide;
    constraint c { x inside { [0:100] }; }     // every 4-bit value qualifies
  endclass

  class W6C;
    rand bit [3:0]    x;
    rand logic [127:0] wide;
    constraint c { x dist { 20 := 1, 5 := 1 }; }  // 20 unreachable in 4 bits
  endclass


  `SVTEST_INIT

  localparam int N = 10000;   // randomize count for the section E dist checks

  initial begin
    // =========================================================================
    // SECTION A: `==` assigns an RHS wider than the rand variable
    //   LRM 11.6.1: ma is zero-extended to 40 bits; with tot == 0 the RHS is
    //   2^40-1, unreachable for a 32-bit variable -> randomize must FAIL.
    //   xezim: randomize()=1 with ma = 2^40-1 stored unmasked.
    // =========================================================================
    begin
      EqOv c = new();
      int r;
      r = c.randomize();
      $display("A: randomize()=%0d ma=%0d (exp 0; int unsigned max 4294967295)", r, c.ma);
      `SVTEST_CHECK(r == 0, "A: randomize() must fail: no 32-bit value equals 2^40-1 under sec. 11.6.1 width semantics")
      if (r != 0)
        `SVTEST_CHECK(c.ma <= 4294967295, "A: int unsigned ma holds a 40-bit value (`==` RHS stored without masking to the property width)")
    end

    // =========================================================================
    // SECTION B: >64-bit rand struct + member-to-member arithmetic
    // =========================================================================
    // ---- B1 (the bug): wide rand struct + dist + shift equality + member-sums
    begin
      ClientCfg c = new();
      int r;
      r = c.randomize();
      $display("B1: wide struct + member-sums      randomize()=%0d", r);
      `SVTEST_CHECK(r != 0, "B1: randomize() failed (budget exhausted) for >64-bit rand struct with member-to-member arithmetic")
      if (r != 0) begin
        `SVTEST_CHECK(c.nhosts_pre == 4'd3, "B1: nhosts_pre should be 3")
        `SVTEST_CHECK(c.regs.MODE.NUM_HOSTS == 4'd1, "B1: NUM_HOSTS should be 1")
        `SVTEST_CHECK(c.regs.MODE.FIRST_HOST == 4'd0, "B1: FIRST_HOST should be 0")
      end
    end

    // ---- B2 (contrast): the identical constraint set on plain rand scalars
    begin
      ScalarCfg c = new();
      int r;
      r = c.randomize();
      $display("B2: scalars (CSP path)             randomize()=%0d", r);
      `SVTEST_CHECK(r != 0, "B2: randomize() failed even though the identical constraint set on rand scalars is solved by the CSP path")
      if (r != 0) begin
        `SVTEST_CHECK(c.nhosts_pre == 4'd3, "B2: nhosts_pre should be 3")
        `SVTEST_CHECK(c.NUM_HOSTS == 4'd1, "B2: NUM_HOSTS should be 1")
        `SVTEST_CHECK(c.FIRST_HOST == 4'd0, "B2: FIRST_HOST should be 0")
      end
    end

    // ---- B3 (contrast): wide struct, repairable member equalities
    begin
      WideEqCfg c = new();
      int r;
      r = c.randomize();
      $display("B3: wide struct + equalities      randomize()=%0d", r);
      `SVTEST_CHECK(r != 0, "B3: randomize() failed even though every item is a repairable member equality")
      if (r != 0) begin
        `SVTEST_CHECK(c.regs.MODE.NUM_HOSTS == (4'hf >> c.nhosts_pre), "B3: NUM_HOSTS == 4'hf >> nhosts_pre violated")
        `SVTEST_CHECK(c.regs.MODE.INTLV == 2'd0, "B3: INTLV should be 0")
      end
    end

    // =========================================================================
    // SECTION C: `!=` re-pick uses the OTHER operand's width
    //   LRM 11.6.1: b0 is zero-extended to 40 bits, so b0 != 40'd1 forces
    //   b0 = 0; a 1-bit variable can never hold anything but 0 or 1.
    //   xezim: when the uniform draw collides (b0 == 1), the re-pick runs at
    //   the literal's 40-bit width and is stored unmasked (~50% of draws).
    // =========================================================================
    begin
      NeqW c = new();
      int out_of_range = 0;
      int failed_rand = 0;
      for (int i = 0; i < 200; i++) begin
        int r;
        r = c.randomize();
        if (r == 0) failed_rand++;
        if (c.b0 > 1'b1) out_of_range++;
      end
      $display("C: iterations=200 randomize_failures=%0d b0_out_of_range=%0d",
               failed_rand, out_of_range);
      `SVTEST_CHECK(failed_rand == 0, "C: randomize() must succeed (b0 != 40'd1 is satisfiable with b0 = 0)")
      `SVTEST_CHECK(out_of_range == 0, "C: 1-bit rand var b0 drew values wider than 1 bit (`!=` re-pick uses the other operand's 40-bit width)")
    end

    // =========================================================================
    // SECTION D: `==` stores a plain 32-bit literal into a signed byte
    //   LRM 11.6.1: b is sign-extended to 32 bits; 300 is outside the
    //   reachable patterns -> randomize must FAIL.
    //   xezim: randomize()=1 with b = 300 stored unmasked.
    // =========================================================================
    begin
      EqN c = new();
      int r;
      r = c.randomize();
      $display("D: randomize()=%0d b=%0d", r, c.b);
      `SVTEST_CHECK(r == 0, "D: randomize() must fail: 32'd300 is not representable in a signed byte (range -128..127)")
      if (r != 0)
        `SVTEST_CHECK(c.b >= -128 && c.b <= 127, "D: byte rand var b holds an out-of-range value (`==` RHS stored without masking to 8 bits)")
    end

    // =========================================================================
    // SECTION E: multi-randomized dist distribution checks (regression guards)
    //   E1 := weights 10/20/30/40%, E2 :/ split 50/25/25%, E3 := range 1/3
    //   each, E4 signed 50/50, E5 narrow var uniform, E6 dist+relational
    //   conditioning, E7 dist+inside, E8 struct-member dist. Frequencies are
    //   compared in per-mille with generous tolerances (>= 12 sigma).
    // =========================================================================
    // ---- E1: plain := weights -> 100/200/300/400 per-mille
    begin
      DistW c = new();
      int cnt[4];
      for (int i = 0; i < N; i++) begin
        int r;
        r = c.randomize();
        if (r != 0 && c.v < 4) cnt[c.v]++;
      end
      $display("E1 pm: %0d %0d %0d %0d (exp 100 200 300 400)",
               1000*cnt[0]/N, 1000*cnt[1]/N, 1000*cnt[2]/N, 1000*cnt[3]/N);
      for (int k = 0; k < 4; k++) begin
        int exp_pm;
        exp_pm = 100*(k+1);
        if (1000*cnt[k]/N < exp_pm-50 || 1000*cnt[k]/N > exp_pm+50)
          `SVTEST_CHECK(0, $sformatf("E1: v=%0d observed %0d permille, expected %0d +-50 (:= weights not obeyed)", k, 1000*cnt[k]/N, exp_pm))
      end
    end

    // ---- E2: :/ range split -> 500/250/250 per-mille
    begin
      DistR c = new();
      int cnt[4];
      for (int i = 0; i < N; i++) begin
        int r;
        r = c.randomize();
        if (r != 0 && c.v >= 1 && c.v <= 3) cnt[c.v]++;
      end
      $display("E2 pm: %0d %0d %0d (exp 500 250 250)",
               1000*cnt[1]/N, 1000*cnt[2]/N, 1000*cnt[3]/N);
      `SVTEST_CHECK(1000*cnt[1]/N >= 450 && 1000*cnt[1]/N <= 550, "E2: value 1 should get half the weight (1 := 1 vs [2:3] :/ 1)")
      `SVTEST_CHECK(1000*cnt[2]/N >= 200 && 1000*cnt[2]/N <= 300, "E2: value 2 should get a quarter of the weight (:/ splits the range weight)")
      `SVTEST_CHECK(1000*cnt[3]/N >= 200 && 1000*cnt[3]/N <= 300, "E2: value 3 should get a quarter of the weight (:/ splits the range weight)")
    end

    // ---- E3: := over a range -> 1/3 each
    begin
      DistE c = new();
      int cnt[4];
      for (int i = 0; i < N; i++) begin
        int r;
        r = c.randomize();
        if (r != 0 && c.v >= 1 && c.v <= 3) cnt[c.v]++;
      end
      $display("E3 pm: %0d %0d %0d (exp 333 333 333)",
               1000*cnt[1]/N, 1000*cnt[2]/N, 1000*cnt[3]/N);
      for (int k = 1; k <= 3; k++)
        if (1000*cnt[k]/N < 283 || 1000*cnt[k]/N > 383)
          `SVTEST_CHECK(0, $sformatf("E3: v=%0d observed %0d permille, expected 333 +-50 (:= gives EACH range value the weight)", k, 1000*cnt[k]/N))
    end

    // ---- E4: signed dist items -> 500/500 per-mille
    begin
      DistS c = new();
      int neg = 0, pos = 0;
      for (int i = 0; i < N; i++) begin
        int r;
        r = c.randomize();
        if (r != 0) begin
          if (c.s == -1) neg++;
          if (c.s == 1) pos++;
        end
      end
      $display("E4 pm: %0d %0d (exp 500 500)", 1000*neg/N, 1000*pos/N);
      `SVTEST_CHECK(1000*neg/N >= 450 && 1000*neg/N <= 550, "E4: signed dist item -1 should get half the weight")
      `SVTEST_CHECK(1000*pos/N >= 450 && 1000*pos/N <= 550, "E4: signed dist item +1 should get half the weight")
    end

    // ---- E5: narrow 4-bit var, range dist -> uniform 0..15
    begin
      DistN c = new();
      int cnt[16];
      for (int i = 0; i < N; i++) begin
        int r;
        r = c.randomize();
        if (r != 0) cnt[c.x]++;
      end
      $display("E5 pm: %0d %0d %0d %0d ... (exp 62 each)",
               1000*cnt[0]/N, 1000*cnt[1]/N, 1000*cnt[14]/N, 1000*cnt[15]/N);
      for (int k = 0; k < 16; k++)
        if (1000*cnt[k]/N < 33 || 1000*cnt[k]/N > 92)
          `SVTEST_CHECK(0, $sformatf("E5: x=%0d observed %0d permille, expected 62 +-30 (uniform over [0:15] in a 4-bit var)", k, 1000*cnt[k]/N))
    end

    // ---- E6: dist + hard relational -> only feasible value 6
    begin
      DistC c = new();
      int ok = 0, other = 0, rf = 0;
      for (int i = 0; i < N; i++) begin
        int r;
        r = c.randomize();
        if (r == 0) rf++;
        else if (c.v == 6) ok++;
        else other++;
      end
      $display("E6: rand_fail=%0d v==6: %0d other: %0d (exp 0/%0d/0)", rf, ok, other, N);
      `SVTEST_CHECK(rf == 0, "E6: randomize() must succeed (v=6 satisfies both the dist support and v >= 6)")
      `SVTEST_CHECK(other == 0, "E6: v took values outside the dist support {5,6} (relational repair must not leave the dist support)")
    end

    // ---- E7: dist + inside -> 0 gets 50%, 1..9 share 50%
    begin
      DistI c = new();
      int cnt[10];
      for (int i = 0; i < N; i++) begin
        int r;
        r = c.randomize();
        if (r != 0 && c.v <= 9) cnt[c.v]++;
      end
      $display("E7 pm: %0d %0d %0d ... (exp 500 55 55 ...)",
               1000*cnt[0]/N, 1000*cnt[1]/N, 1000*cnt[9]/N);
      `SVTEST_CHECK(1000*cnt[0]/N >= 450 && 1000*cnt[0]/N <= 550, "E7: value 0 should get half the weight (0 := 1 vs [1:9] :/ 1)")
      for (int k = 1; k <= 9; k++)
        if (1000*cnt[k]/N < 25 || 1000*cnt[k]/N > 86)
          `SVTEST_CHECK(0, $sformatf("E7: v=%0d observed %0d permille, expected 55 +-30 (:/ splits the range weight over 9 values)", k, 1000*cnt[k]/N))
    end

    // ---- E8: dist on a member of a >64-bit rand struct -> 500/500
    begin
      DistM c = new();
      int z = 0, f = 0, rf = 0;
      for (int i = 0; i < N; i++) begin
        int r;
        r = c.randomize();
        if (r == 0) rf++;
        else if (c.s.f == 0) z++;
        else if (c.s.f == 15) f++;
      end
      $display("E8: rand_fail=%0d f==0: %0d f==15: %0d (exp 0/5000/5000)", rf, z, f);
      `SVTEST_CHECK(rf == 0, "E8: randomize() must succeed for a dist on a struct member")
      `SVTEST_CHECK(1000*z/N >= 450 && 1000*z/N <= 550, "E8: member s.f==0 should get half the weight")
      `SVTEST_CHECK(1000*f/N >= 450 && 1000*f/N <= 550, "E8: member s.f==15 should get half the weight")
    end

    // =========================================================================
    // SECTION F: width-boundary contrast checks (expected PASS on both; any
    // FAIL is a new cousin of the A/C/D width-overflow family)
    // =========================================================================
    // ---- F1: relational with an unreachable wide bound
    begin
      W1C c = new();
      int r;
      r = c.randomize();
      $display("F1: randomize()=%0d x=%0d (exp 0)", r, c.x);
      `SVTEST_CHECK(r == 0, "F1: randomize() must fail (no 4-bit value is > 32'hFFFFFFF0)")
      if (r != 0)
        `SVTEST_CHECK(c.x <= 4'd15, "F1: 4-bit var x drew an out-of-range value")
    end

    // ---- F2: inside with signed negative ranges
    begin
      W2C c = new();
      int r;
      r = c.randomize();
      $display("F2: randomize()=%0d sb=%0d", r, c.sb);
      `SVTEST_CHECK(r != 0, "F2: randomize() must succeed (signed ranges are feasible)")
      if (r != 0)
        `SVTEST_CHECK((c.sb >= -128 && c.sb <= -120) || (c.sb >= 120 && c.sb <= 127),
                      "F2: byte sb outside the inside ranges")
    end

    // ---- F3: affine == with an unrepresentable solution
    begin
      W3C c = new();
      int r;
      r = c.randomize();
      $display("F3: randomize()=%0d sb=%0d (exp 0)", r, c.sb);
      `SVTEST_CHECK(r == 0, "F3: randomize() must fail (sb + 5 == 300 needs sb = 295, not a byte)")
      if (r != 0)
        `SVTEST_CHECK(c.sb >= -128 && c.sb <= 127, "F3: byte sb drew an out-of-range value")
    end

    // ---- F4: affine == with a representable solution
    begin
      W4C c = new();
      int r;
      r = c.randomize();
      $display("F4: randomize()=%0d sb=%0d (exp 95)", r, c.sb);
      `SVTEST_CHECK(r != 0, "F4: randomize() must succeed (sb = 95)")
      if (r != 0)
        `SVTEST_CHECK(c.sb == 95, "F4: sb should be exactly 95")
    end

    // ---- F5: inside whose range exceeds the variable width
    begin
      W5C c = new();
      int r;
      r = c.randomize();
      $display("F5: randomize()=%0d x=%0d", r, c.x);
      `SVTEST_CHECK(r != 0, "F5: randomize() must succeed (all 4-bit values are inside [0:100])")
      if (r != 0)
        `SVTEST_CHECK(c.x <= 4'd15, "F5: 4-bit var x drew an out-of-range value")
    end

    // ---- F6: dist item outside the variable's range
    begin
      W6C c = new();
      int ok5 = 0, other = 0, rf = 0;
      for (int i = 0; i < 1000; i++) begin
        int r;
        r = c.randomize();
        if (r == 0) rf++;
        else if (c.x == 5) ok5++;
        else other++;
      end
      $display("F6: rand_fail=%0d x==5: %0d other: %0d (exp 0/1000/0)", rf, ok5, other);
      `SVTEST_CHECK(rf == 0, "F6: randomize() must succeed (x = 5 satisfies the dist)")
      `SVTEST_CHECK(other == 0, "F6: x took a value other than 5 (dist item 20 is unreachable in 4 bits and must not leak a truncated weight)")
    end

    `SVTEST_PASSFAIL
    $finish;
  end




endmodule
