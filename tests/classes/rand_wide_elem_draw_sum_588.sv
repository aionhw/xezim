`timescale 1ns/1ps

// ---------------------------------------------------------------------------
// Wide (>64-bit) rand-element draws and wide sum()/product() targets.
//
// Regression test for the full-width draw / 128-bit sum fixes:
//   C1  draw_elem() built every element with Value::from_u64(v, width),
//       which zero-fills bits above 63, so the high half of a wide
//       element was pinned to 0 in every draw -- both the fresh
//       randomize() baselines and the generate-and-test backstop.
//       Guarded by an unconstrained statistical check (the high halves
//       must come up nonzero) and by a `!= 0` constraint on the high
//       slice (mirrors the comprehensive-grind C1 probe).
//   G2  the same zero-fill pinned the MSB field (`vld`, bit 112) of a
//       113-bit packed struct element, so `beats[i].vld == 1` was
//       structurally unsatisfiable for the backstop and every one of
//       the 256 retries per element was a guaranteed-wasted interpreted
//       check (the grind).  Guarded by a constrained struct array plus
//       an unconstrained one whose vld must come up 1 statistically.
//   C10 the sum() path truncated at 64 bits three times over: the point
//       target was read with to_i64 (a 96-bit target of 2^64 read as
//       0), the element domain was capped at u64::MAX, distributed
//       shares were written with from_u64, and the evaluator
//       accumulated in u64 building the result at min(width, 64).
//       Guarded by a feasible 2^64 target (checked by independent
//       manual accumulation), pinned-element readback through sum()
//       and sum() with (item), a signed 2^63+2^63 sum, an infeasible
//       2^97 target that must prove UNSAT, and evaluator-only
//       sum()/product() checks on a plain fixed array.
//   Family guards (pass on both the old and the fixed solver; they pin
//       the NEIGHBOURING truncation/CSP-gate sites the audit
//       identified, ported from the comprehensive grind MWE):
//       C2/C2d `!=` repair vs wide pins, C6 wide fixed-array foreach
//       bit-select body, C7 foreach relational bound, C8a nested
//       [][2] struct member select, C8b implication contradiction.
//
// NOT carried over from the comprehensive grind MWE (by design): the
// production-stack replicas (type-parameterized bus_xfer, uvm-like
// base chain, the wallfail equation web) and every wall-clock
// budget/plusarg knob -- cargo's simulate() cannot inject plusargs
// and timing asserts are flaky in unit tests.  Those stay guarded by
// the comprehensive MWE in the eval area, run via the CLI binary.
//
// ---------------------------------------------------------------------------

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

// ----- C1: fresh draws of wide dynamic-array / queue elements -----
class wide_free_c;
  rand logic [95:0] wda[];
  rand logic [95:0] wq[$];
  constraint c_sz { wda.size() == 8; wq.size() == 8; }
endclass

// ----- C1: high slice of a wide element constrained nonzero -----
class wide_slice_c;
  rand logic [95:0] wsb[];
  constraint c_sz { wsb.size() == 8; }
  constraint c_hi { foreach (wsb[i]) wsb[i][95:64] != 64'd0; }
endclass

// ----- C2/C2d (family guards): `!=` repair vs wide pinned elements.
//       constraint_domains.rs' != repair compares to_u64 (low word) and
//       re-picks via from_u64; the exact pins must survive it. -----
class neq_fixed_c;
  rand logic [95:0] fa[2];
  constraint pins_c {
    fa[0] == 96'h000A_0000_0000_0000;
    fa[1] == 96'h000B_0000_0000_0000;
  }
  constraint neq_c { fa[0] != fa[1]; }
endclass

class neq_dyn_c;
  rand logic [95:0] da[];
  constraint c_sz { da.size() == 2; }
  constraint pins_c {
    da[0] == 96'h000A_0000_0000_0000;
    da[1] == 96'h000B_0000_0000_0000;
  }
  constraint neq_c { da[0] != da[1]; }
endclass

// ----- C6 (family guard): wide fixed-array foreach, bit-select body.
//       solve_forced()'s generate-and-test backstop is gated
//       elem_w <= 64, so a non-structural body on a >64-bit fixed
//       array element must be solved without it. -----
class bitsel_fixed_c;
  rand logic [95:0] wf[4];
  constraint bits_c { foreach (wf[i]) wf[i][0] == 1'b1; }
endclass

// ----- C7 (family guard): foreach relational bound on a wide element.
//       force_elem_item()'s Lt/Gt arm reads the bound via to_i64. -----
class rel_gt_c;
  rand logic [95:0] wd[];
  constraint c_sz { wd.size() == 4; }
  constraint gt_c { foreach (wd[i]) wd[i] > 96'h1_0000_0000_0000_0000; }
endclass

// ----- C8a/C8b (family guards): the CSP-gate family. Any rand dynamic
//       array disables the pre-trial joint solve (rand_csp models
//       integral scalars and 1-D shapes only), leaving these to the
//       trial machinery alone. -----
typedef struct packed { logic en; logic [34:0] payload; logic stop; } t37_t;

class nested2d_c;
  rand t37_t items[][2];
  constraint c_sz { items.size() == 1; }
endclass

class impl_contra_c;
  rand t37_t items[];
  constraint c_sz { items.size() == 1; }
endclass


// ----- G2: wide packed-struct element with its MSB field constrained -----
typedef struct packed {
  logic        vld;    // bit 112: the MSB
  logic [95:0] wdata;
  logic [15:0] strb;
  logic        last;   // bit 0
} wbeat_t;             // 113 bits

class wide_beats_c;
  rand wbeat_t beats[];
  rand wbeat_t raw[];  // unconstrained: vld must come up 1 statistically
  constraint c_sz  { beats.size() == 8; raw.size() == 32; }
  constraint c_fld { foreach (beats[i]) {
                       beats[i].vld == 1'b1;
                       beats[i].strb inside {[16'h1 : 16'hFFFF]};
                     } }
endclass

// ----- C10: wide sum() point target over free elements -----
class wide_sum_c;
  rand logic [95:0] s[];
  constraint c_sz { s.size() == 4; }
  constraint c_tgt { s.sum() == 96'h1_0000_0000_0000_0000; }  // 2^64
endclass

// ----- C10: pinned wide elements read back through sum() -----
class wide_pin_c;
  rand logic [95:0] pin[];
  constraint c_sz { pin.size() == 2; }
  constraint c_p0 { pin[0] == 96'd3; }
  constraint c_p1 { pin[1] == 96'h1_0000_0000_0000_0000; }    // 2^64
endclass

// ----- C10: signed wide elements -- 2^63 + 2^63 must not wrap at 64 -----
class wide_signed_c;
  rand logic signed [95:0] sp[];
  constraint c_sz { sp.size() == 2; }
  constraint c_s0 { sp[0] == 96'h8000_0000_0000_0000; }       // 2^63
  constraint c_s1 { sp[1] == 96'h8000_0000_0000_0000; }       // 2^63
endclass

// ----- C10: infeasible wide target -- 2^97 > 2*(2^96-1), must be UNSAT -----
class wide_unsat_c;
  rand logic [95:0] uns[];
  constraint c_sz { uns.size() == 2; }
  constraint c_u { uns.sum() == 98'h2_0000_0000_0000_0000_0000_0000; }
endclass

module tb_top;
  `SVTEST_INIT

  // C10: evaluator-only checks on a plain (non-rand) fixed array -- the
  // u64 accumulator read 2^64 as 0, so the sum missed by 2^64 and the
  // product collapsed to 0.
  logic [95:0] fe [0:1];

  initial begin
    // --- C1: unconstrained wide elements get nonzero high halves ---
    begin
      wide_free_c wf;
      int n_da, n_q;
      wf = new();
      n_da = 0;
      n_q = 0;
      `SVTEST_CHECK(wf.randomize() == 1, "C1: randomize(wda/wq) must solve")
      foreach (wf.wda[i]) if (wf.wda[i][95:64] != 64'd0) n_da++;
      foreach (wf.wq[i])   if (wf.wq[i][95:64]   != 64'd0) n_q++;
      // P(all 8 high halves zero) = 2^-256 per array: a miss means the
      // high half is still being zero-filled.
      `SVTEST_CHECK(n_da > 0, "C1: wda high halves all zero (from_u64 zero-fill)")
      `SVTEST_CHECK(n_q > 0, "C1: wq high halves all zero (from_u64 zero-fill)")
    end

    // --- C1: `!= 0` on the high slice is honoured (it was silently
    //     unsatisfiable for the backstop when the MSBs could not move) ---
    begin
      wide_slice_c ws;
      int n_hi;
      ws = new();
      `SVTEST_CHECK(ws.randomize() == 1, "C1: randomize(wsb) must solve")
      n_hi = 0;
      foreach (ws.wsb[i]) if (ws.wsb[i][95:64] != 64'd0) n_hi++;
      `SVTEST_CHECK(n_hi == 8, "C1: wsb[i][95:64] != 0 not honoured")
    end

    // --- C2/C2d (family guards): `!=` repair keeps exact wide pins ---
    begin
      neq_fixed_c of;
      neq_dyn_c od;
      int good_f, good_d;
      good_f = 0;
      for (int k = 0; k < 10; k++) begin
        of = new();
        if (of.randomize() == 1
              && of.fa[0] == 96'h000A_0000_0000_0000
              && of.fa[1] == 96'h000B_0000_0000_0000) good_f++;
      end
      good_d = 0;
      for (int k = 0; k < 10; k++) begin
        od = new();
        if (od.randomize() == 1
              && od.da[0] == 96'h000A_0000_0000_0000
              && od.da[1] == 96'h000B_0000_0000_0000) good_d++;
      end
      `SVTEST_CHECK(good_f == 10, "C2: wide fixed != repair broke a pinned value")
      `SVTEST_CHECK(good_d == 10, "C2d: wide dyn != repair broke a pinned value")
    end

    // --- C6 (family guard): wide fixed-array foreach bit-select body ---
    begin
      bitsel_fixed_c ob;
      int n_b0;
      ob = new();
      `SVTEST_CHECK(ob.randomize() == 1, "C6: randomize(wf) must solve")
      n_b0 = 0;
      foreach (ob.wf[i]) if (ob.wf[i][0] === 1'b1) n_b0++;
      `SVTEST_CHECK(n_b0 == 4, "C6: wf[i][0] == 1 not honoured on all 4")
    end

    // --- C7 (family guard): foreach relational bound on wide elements ---
    begin
      rel_gt_c og;
      int n_gt;
      og = new();
      `SVTEST_CHECK(og.randomize() == 1, "C7: randomize(wd) must solve")
      n_gt = 0;
      foreach (og.wd[i]) if (og.wd[i] > 96'h1_0000_0000_0000_0000) n_gt++;
      `SVTEST_CHECK(n_gt == 4, "C7: wd[i] > 2^64 not honoured on all 4")
    end

    // --- C8a (family guard): nested dyn [][2] struct member select ---
    begin
      nested2d_c o2;
      bit ok2;
      o2 = new();
      ok2 = o2.randomize() with { items[0][0].en == 1; };
      `SVTEST_CHECK(ok2 == 1, "C8a: nested dyn [][2] member select must solve")
    end

    // --- C8b (family guard): contradiction via implication is UNSAT ---
    begin
      impl_contra_c oc;
      bit okc;
      oc = new();
      okc = oc.randomize() with { items[0].en == 1; items[0].stop == 0;
                                  items[0].en == 1 -> items[0].stop == 1; };
      `SVTEST_CHECK(okc == 0,
                    "C8b: implication contradiction is UNSAT; randomize must return 0")
    end

    // --- G2: wide packed-struct elements, MSB field constrained ---
    begin
      wide_beats_c wb;
      int n_vld, n_strb, n_raw_vld;
      wb = new();
      `SVTEST_CHECK(wb.randomize() == 1, "G2: randomize(beats/raw) must solve")
      n_vld = 0;
      n_strb = 0;
      foreach (wb.beats[i]) begin
        if (wb.beats[i].vld == 1'b1) n_vld++;
        if (wb.beats[i].strb != 16'd0) n_strb++;
      end
      `SVTEST_CHECK(n_vld == 8, "G2: beats[i].vld == 1 not honoured (MSB zero-filled)")
      `SVTEST_CHECK(n_strb == 8, "G2: beats[i].strb inside [1:FFFF] not honoured")
      // Unconstrained struct draws: vld is bit 112 and must be able to
      // come up 1 (P(all 32 zero) = 2^-32).
      n_raw_vld = 0;
      foreach (wb.raw[i]) if (wb.raw[i].vld == 1'b1) n_raw_vld++;
      `SVTEST_CHECK(n_raw_vld > 0, "G2: raw[i].vld never 1 (MSB zero-filled)")
    end

    // --- C10: feasible wide point target, verified by independent
    //     manual accumulation (the constraint itself wraps at 96 bits,
    //     so the check accumulates in a 96-bit reg the same way) ---
    begin
      wide_sum_c wsum;
      logic [95:0] acc;
      wsum = new();
      `SVTEST_CHECK(wsum.randomize() == 1, "C10: randomize(s) with sum==2^64 must solve")
      acc = 96'd0;
      foreach (wsum.s[i]) acc = acc + wsum.s[i];
      `SVTEST_CHECK(acc == 96'h1_0000_0000_0000_0000,
                    "C10: manual accumulation of s[] missed the 2^64 target")
      `SVTEST_CHECK(wsum.s.sum() == 96'h1_0000_0000_0000_0000,
                    "C10: s.sum() evaluator missed the 2^64 target")
    end

    // --- C10: pinned wide elements through sum() and sum() with (item) ---
    begin
      wide_pin_c wp;
      wp = new();
      `SVTEST_CHECK(wp.randomize() == 1, "C10: randomize(pin) must solve")
      `SVTEST_CHECK(wp.pin[0] == 96'd3, "C10: pin[0] == 3 not honoured")
      `SVTEST_CHECK(wp.pin[1] == 96'h1_0000_0000_0000_0000,
                    "C10: pin[1] == 2^64 not honoured")
      `SVTEST_CHECK(wp.pin.sum() == 96'h1_0000_0000_0000_0003,
                    "C10: pin.sum() != 2^64 + 3 (u64 accumulator wrapped)")
      `SVTEST_CHECK(wp.pin.sum() with (item) == 96'h1_0000_0000_0000_0003,
                    "C10: pin.sum() with (item) != 2^64 + 3 (i64 accumulator wrapped)")
    end

    // --- C10: signed wide elements -- 2^63 reads as i64 -2^63, so the
    //     old accumulator summed them to 0 instead of 2^64 ---
    begin
      wide_signed_c wsg;
      wsg = new();
      `SVTEST_CHECK(wsg.randomize() == 1, "C10: randomize(sp) must solve")
      `SVTEST_CHECK(wsg.sp.sum() == 96'h1_0000_0000_0000_0000,
                    "C10: signed 2^63 + 2^63 summed to something other than 2^64")
    end

    // --- C10: an infeasible wide target must prove UNSAT (the old
    //     to_i64 read of 2^97 pinned the sum to 0 and ground or
    //     falsely solved) ---
    begin
      wide_unsat_c wu;
      wu = new();
      `SVTEST_CHECK(wu.randomize() == 0,
                    "C10: sum == 2^97 over two 96-bit elems is UNSAT; randomize must return 0")
    end

    // --- C10: evaluator-only sum()/product() on a plain fixed array ---
    begin
      fe[0] = 96'h1_0000_0000_0000_0000;  // 2^64
      fe[1] = 96'd5;
      `SVTEST_CHECK(fe.sum() == 96'h1_0000_0000_0000_0005,
                    "C10: fe.sum() != 2^64 + 5 (u64 accumulator wrapped)")
      `SVTEST_CHECK(fe.sum() with (item) == 96'h1_0000_0000_0000_0005,
                    "C10: fe.sum() with (item) != 2^64 + 5 (i64 accumulator wrapped)")
      `SVTEST_CHECK(fe.product() == 96'h5_0000_0000_0000_0000,
                    "C10: fe.product() != 5 * 2^64 (u64 accumulator wrapped)")
    end

    `SVTEST_PASSFAIL
  end
endmodule