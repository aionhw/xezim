// SPDX-License-Identifier: MIT
//
// 39_builtin_nettype_resolution.sv — Tier-1 regression: the
// per-NetType resolution function (OR for tri/wire/wor/trior, AND for
// wand/triand, with default pull-down for tri0, default pull-up for tri1,
// and ground/power for supply0/supply1).
//
// Reference behavior: a commercial SV simulator AND Icarus 12.0 agree on:
//
//   tri  [0, 1] -> x     (active conflict -> x; wand/wor resolve cleanly, tri does not)
//   tri  [1, 1] -> 1
//   wand [0, 1] -> 0     (AND-fold; never x)
//   triand [0, 1] -> 0   (same fold as wand)
//   wor  [0, 1] -> 1     (OR-fold; never x)
//   trior [0, 1] -> 1    (same fold as wor)
//   tri0  (no drivers) -> 0  (default pull-down)
//   tri0  [0, 1]      -> x  (active conflict; the 0 default does not override the conflict)
//   tri1  (no drivers) -> 1  (default pull-up)
//   tri1  [0, 1]      -> x
//   supply0 (no drivers) -> 0
//   supply1 (no drivers) -> 1
//
// Test goal: prove that xezim's elaborator picks the *right* resolution
// function per NetType. The current implementation hardcodes BitOr for
// everything (xezim-core/src/elaborate.rs:2802-2844), which gives:
//
//   wand  [0, 1] -> 1  WRONG (should be 0; this test catches it)
//   triand [0, 1] -> 1 WRONG (should be 0)
//   wor   [0, 1] -> 1 OK (OR and AND-with-OR-fold happen to agree here)
//   trior [0, 1] -> 1 OK
//   tri0  (no drivers) -> x  WRONG (should be 0; current code yields x)
//   tri1  (no drivers) -> x  WRONG (should be 1)
//   supply0       -> x  WRONG (should be 0)
//   supply1       -> x  WRONG (should be 1)
//
// So this test will fail on xezim at every NetType that is NOT OR-fold and
// will pass only after Tier 1 is implemented.

`include "svtest_defs.svh"

module test_39_tier1_builtin_nettype_resolution;
  `SVTEST_INIT

  // ----- wand / triand: AND fold (the canonical Tier-1 bug-catch) -----
  wand  wa01;  assign wa01  = 1'b0; assign wa01  = 1'b1; // -> 0
  wand  wa11;  assign wa11  = 1'b1; assign wa11  = 1'b1; // -> 1
  triand ta01; assign ta01  = 1'b0; assign ta01  = 1'b1; // -> 0
  triand ta11; assign ta11  = 1'b1; assign ta11  = 1'b1; // -> 1

  // ----- wor / trior: OR fold (already passes on xezim's current code) -----
  wor   wo01;  assign wo01  = 1'b0; assign wo01  = 1'b1; // -> 1
  trior to01;  assign to01  = 1'b0; assign to01  = 1'b1; // -> 1

  // ----- tri0 / tri1: defaults when no active drivers -----
  tri0  tri0_no; // no drivers -> default 0
  tri1  tri1_no; // no drivers -> default 1

  // ----- tri0 / tri1 with active drivers in conflict (still x; defaults
  //       only apply with NO active drivers) -----
  tri0  tri0_01;  assign tri0_01  = 1'b0; assign tri0_01  = 1'b1; // -> x
  tri1  tri1_01;  assign tri1_01  = 1'b0; assign tri1_01  = 1'b1; // -> x

  // ----- supply nets: hard-tied to ground / power -----
  supply0 s0_no; // no drivers -> 0
  supply1 s1_no; // no drivers -> 1

  // ----- tri / wire with single active driver: pass-through (sanity check) -----
  tri  t_solo_0; assign t_solo_0 = 1'b0; // -> 0
  tri  t_solo_1; assign t_solo_1 = 1'b1; // -> 1

  initial begin
    #0;

    // wand / triand: AND fold (catches xezim's hardcoded OR)
    `SVTEST_CHECK(wa01 === 1'b0, "T1: wand  [0, 1] -> 0 (AND fold)")
    `SVTEST_CHECK(wa11 === 1'b1, "T1: wand  [1, 1] -> 1")
    `SVTEST_CHECK(ta01 === 1'b0, "T1: triand [0, 1] -> 0 (AND fold)")
    `SVTEST_CHECK(ta11 === 1'b1, "T1: triand [1, 1] -> 1")

    // wor / trior: OR fold (already passes)
    `SVTEST_CHECK(wo01 === 1'b1, "T1: wor   [0, 1] -> 1 (OR fold)")
    `SVTEST_CHECK(to01 === 1'b1, "T1: trior [0, 1] -> 1 (OR fold)")

    // tri0 / tri1 defaults (Tier-1 catches the current "uninitialized = x" bug)
    `SVTEST_CHECK(tri0_no === 1'b0, "T1: tri0 no drivers -> 0 (default pull-down)")
    `SVTEST_CHECK(tri1_no === 1'b1, "T1: tri1 no drivers -> 1 (default pull-up)")

    // tri0 / tri1 with conflicting actives
    `SVTEST_CHECK(tri0_01 === 1'bx, "T1: tri0 [0, 1] -> x (active conflict)")
    `SVTEST_CHECK(tri1_01 === 1'bx, "T1: tri1 [0, 1] -> x (active conflict)")

    // supply nets
    `SVTEST_CHECK(s0_no === 1'b0, "T1: supply0 no drivers -> 0")
    `SVTEST_CHECK(s1_no === 1'b1, "T1: supply1 no drivers -> 1")

    // tri single-driver pass-through
    `SVTEST_CHECK(t_solo_0 === 1'b0, "T1: tri [0] -> 0")
    `SVTEST_CHECK(t_solo_1 === 1'b1, "T1: tri [1] -> 1")

    `SVTEST_PASSFAIL
  end
endmodule