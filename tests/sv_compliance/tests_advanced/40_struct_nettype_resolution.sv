// SPDX-License-Identifier: MIT
//
// 40_struct_nettype_resolution.sv — Tier-2 extension: a `nettype`
// wrapping a user-defined struct with a `real` field, where the resolver
// function sums the `real` field and ORs the `bit` field across the driver
// queue. This catches elaboration gaps that file 38 misses:
//
//   - typedef struct { ... } T;                    — must be elaborated
//   - nettype T <name> with <fn>;                 — resolver must be invoked
//   - driver[i].field1  in resolver              — struct field access
//   - real arithmetic in a net-resolution path    — float values on a net
//   - struct field-by-field resolver              — return type is a struct,
//                                                    not a primitive scalar
//
// Reference: QuestaSim-64 2021.2_1. (Icarus 12 does not implement `nettype`;
// this test is Questa-only, just like file 38.)
//
// xezim's current behavior: file 38's resolver never gets called (hardcoded
// BitOr fold in xezim-core/src/elaborate.rs:2802-2844). This file exercises
// the same code path but with a *struct* element type, so it additionally
// catches a class of bugs where the elaborator refuses to compile a
// non-primitive element type at all, or where struct field access is not
// resolved through the driver-queue element.

`include "svtest_defs.svh"

// ---------------------------------------------------------------------------
// User-defined struct type with a `real` field and a `bit` field.
// ---------------------------------------------------------------------------
typedef struct {
  real field1;
  bit  field2;
} T;

// ---------------------------------------------------------------------------
// Resolver: sum of `field1` across the driver queue, OR of `field2`.
// Mirrors the LRM §6.6.7 `Tsum` example pattern.
// ---------------------------------------------------------------------------
function automatic T Tsum (input T driver []);
  T result;
  result.field1 = 0.0;
  result.field2 = 1'b0;
  foreach (driver[i]) begin
    result.field1 += driver[i].field1;
    result.field2 |= driver[i].field2;
  end
  return result;
endfunction

nettype T wTsum with Tsum;

// ---------------------------------------------------------------------------
// Top-level test.
// ---------------------------------------------------------------------------
module test_40_tier2_struct_nettype_resolution;
  `SVTEST_INIT

  // ----- single driver (resolver returns the input unchanged) -----
  wTsum a;  assign a  = '{1.5, 1'b1};   // -> (1.5, 1)
  wTsum b;  assign b  = '{2.5, 1'b0};   // -> (2.5, 0)

  // ----- multi-driver, mixed bit fields (resolver sums) -----
  wTsum m01; assign m01 = '{1.0, 1'b0}; assign m01 = '{2.0, 1'b1}; // -> (3.0, 1)
  wTsum m11; assign m11 = '{1.0, 1'b1}; assign m11 = '{2.0, 1'b1}; // -> (3.0, 1)
  wTsum m00; assign m00 = '{1.0, 1'b0}; assign m00 = '{2.0, 1'b0}; // -> (3.0, 0)

  // ----- 3-driver stress: float addition chains and a multi-driver OR -----
  wTsum m3;  assign m3 = '{1.0, 1'b1};
             assign m3 = '{2.0, 1'b0};
             assign m3 = '{3.0, 1'b1};                            // -> (6.0, 1)

  // ----- float edge values: negative, fractional, large -----
  wTsum mn;  assign mn  = '{-1.5, 1'b1};
             assign mn  = '{ 2.25, 1'b1};                         // -> (0.75, 1)

  initial begin
    #0;

    // Single-driver
    `SVTEST_CHECK(a.field1 == 1.5 && a.field2 === 1'b1,
                  "single: [1.5, 1] -> field1=1.5 field2=1")
    `SVTEST_CHECK(b.field1 == 2.5 && b.field2 === 1'b0,
                  "single: [2.5, 0] -> field1=2.5 field2=0")

    // Multi-driver with mixed bit fields
    `SVTEST_CHECK(m01.field1 == 3.0 && m01.field2 === 1'b1,
                  "2-driver: [1.0,0] + [2.0,1] -> field1=3.0 field2=1")
    `SVTEST_CHECK(m11.field1 == 3.0 && m11.field2 === 1'b1,
                  "2-driver: [1.0,1] + [2.0,1] -> field1=3.0 field2=1")
    `SVTEST_CHECK(m00.field1 == 3.0 && m00.field2 === 1'b0,
                  "2-driver: [1.0,0] + [2.0,0] -> field1=3.0 field2=0")

    // 3-driver
    `SVTEST_CHECK(m3.field1 == 6.0 && m3.field2 === 1'b1,
                  "3-driver: [1,1] + [2,0] + [3,1] -> field1=6.0 field2=1")

    // Float edges
    `SVTEST_CHECK(mn.field1 == 0.75 && mn.field2 === 1'b1,
                  "neg+frac: [-1.5,1] + [2.25,1] -> field1=0.75 field2=1")

    `SVTEST_PASSFAIL
  end
endmodule