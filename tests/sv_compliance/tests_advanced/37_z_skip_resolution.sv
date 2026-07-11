// SPDX-License-Identifier: MIT
//
// 37_z_skip_resolution.sv — Tier-0 regression: the resolution function
// of a `tri`/`wire` net must treat `z` as "no contribution" when there is at
// least one active driver. With multiple *active* drivers, the result is `x`
// (because the LRM requires `x` for conflicting active drivers).
//
// Reference behavior: QuestaSim-64 2021.2_1 AND Icarus 12.0 agree
// byte-for-byte on every case below:
//
//   tri [1, z]              -> 1   (active 1 wins)
//   tri [0, z]              -> 0   (active 0 wins)
//   tri [0, 1, z]           -> x   (active conflict -> x, regardless of z)
//   tri [1, 1]              -> 1   (active agree)
//   tri [0, 0]              -> 0   (active agree)
//   tri [0, 1]              -> x   (no z, active conflict -> x)
//   tri [z, z]              -> z   (no active drivers)
//   wire [1, z]             -> 1   (same resolution as `tri`)
//
// Test goal: prove that xezim's elaborator at xezim-core/src/elaborate.rs
// produces the same values. The current implementation hardcodes a BitOr
// fold, which gives:
//   tri [1, z]              -> 1     ✓ (matches)
//   tri [0, z]              -> 0     ✓ (matches)
//   tri [0, 1, z]           -> x     ✓ (matches by accident — OR of 0|1 is 1, then 1|z is x)
//   tri [z, z]              -> z     ✓ (matches — no OR contribution)
// So Tier 0 is more of a documentation/regression baseline than a bug
// catch; the bug-catching tier is Tier 1 (per-NetType fold).

`include "svtest_defs.svh"

module test_37_tier0_z_skip_resolution;
  `SVTEST_INIT

  // ----- tri-state: single active driver + z (z must be no-contribution) -----
  tri b1; assign b1 = 1'b1; assign b1 = 1'bz; // -> 1
  tri b2; assign b2 = 1'b0; assign b2 = 1'bz; // -> 0
  tri b3; assign b3 = 1'b0; assign b3 = 1'b1; assign b3 = 1'bz; // -> x (active conflict)

  // ----- tri-state: two active drivers, no z -----
  tri b4; assign b4 = 1'b1; assign b4 = 1'b1; // -> 1
  tri b5; assign b5 = 1'b0; assign b5 = 1'b0; // -> 0

  // ----- wire with z driver -----
  wire b6; assign b6 = 1'b1; assign b6 = 1'bz; // -> 1

  // ----- all-z drivers -----
  tri b7; assign b7 = 1'bz; assign b7 = 1'bz; // -> z

  // ----- classic enable-pattern tri-state buffer driving a tri net -----
  // Demonstrates: when the buffer's enable is low (output = z), the other
  // driver on the net determines the resolved value.
  tri bus; assign bus = 1'b0; // pull-down
  tri en_lo, en_hi;
  assign en_lo = 1'b1;
  assign en_hi = 1'b0;
  // bufif1: out=in when ctrl=1, out=z when ctrl=0
  // We hand-roll it to keep the test focused on the resolver, not the gate:
  //   out = ctrl ? in : 1'bz
  logic in_lo, in_hi;
  assign in_lo = 1'b1;
  assign in_hi = 1'b1;
  tri drv_lo; assign drv_lo = en_lo ? in_lo : 1'bz; // -> 1
  tri drv_hi; assign drv_hi = en_hi ? in_hi : 1'bz; // -> z
  assign bus  = drv_lo;
  assign bus  = drv_hi;
  // Resolved: 0 (pull-down), 1 (drv_lo), z (drv_hi) -> ?

  initial begin
    #0;

    // --- B1: tri [1, z] ---
    `SVTEST_CHECK(b1 === 1'b1, "B1: tri [1, z]   -> 1 (active wins over z)")

    // --- B2: tri [0, z] ---
    `SVTEST_CHECK(b2 === 1'b0, "B2: tri [0, z]   -> 0 (active wins over z)")

    // --- B3: tri [0, 1, z] (the original Tier-0 "z no-contribution" claim
    //     — actually LRM-correct behavior is x, because two active drivers
    //     with different values create a conflict regardless of any z's). ---
    `SVTEST_CHECK(b3 === 1'bx, "B3: tri [0, 1, z] -> x (active conflict dominates z)")

    // --- B4 / B5: same-value active drivers ---
    `SVTEST_CHECK(b4 === 1'b1, "B4: tri [1, 1]   -> 1 (agreement)")
    `SVTEST_CHECK(b5 === 1'b0, "B5: tri [0, 0]   -> 0 (agreement)")

    // --- B6: wire with z driver ---
    `SVTEST_CHECK(b6 === 1'b1, "B6: wire [1, z]  -> 1 (same resolution as tri)")

    // --- B7: all-z ---
    `SVTEST_CHECK(b7 === 1'bz, "B7: tri [z, z]   -> z (no active contribution)")

    // --- enable-pattern tri-state: when enable low, buffer contributes z;
    //     the net resolves against the remaining active drivers only. ---
    `SVTEST_CHECK(drv_lo === 1'b1, "EN: drv_lo (enable=1) -> 1")
    `SVTEST_CHECK(drv_hi === 1'bz, "EN: drv_hi (enable=0) -> z")

    `SVTEST_PASSFAIL
  end
endmodule