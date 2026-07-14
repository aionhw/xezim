// SPDX-License-Identifier: MIT
//
// IEEE 1800-2023 Clause 20.8.2 — Real math functions (trig + hyperbolic set)
//
// Per Table 20-4 each function matches the equivalent C <math.h> function.
// This file exercises the trigonometric and hyperbolic members (the ones that
// are currently unimplemented in xezim) plus a few "anchor" functions that are
// already supported, to validate the tolerance framework on a reference sim.
//
//   Missing trig/hyperbolic under test:
//     $sin $cos $tan $asin $acos $atan $atan2 $hypot
//     $sinh $cosh $tanh $asinh $acosh $atanh
//   Anchors (already supported, cross-check the harness):
//     $sqrt $pow $floor $ceil
//
// All comparisons use an absolute tolerance of 1e-9. Prints "TEST_PASS".

`timescale 1ns/1ps
`include "../common/svtest_defs.svh"

module test_real_math_trig;
  `SVTEST_INIT

  localparam real PI    = 3.14159265358979323846;
  localparam real TOL   = 1.0e-9;
  real r;

  // absolute-difference helper
  function automatic bit approx(real a, real b);
    real d;
    d = a - b;
    if (d < 0.0) d = -d;
    return (d <= TOL);
  endfunction

  initial begin
    // ---------- anchors (validate the tolerance harness) ----------
    r = $sqrt(2.0);
    `SVTEST_CHECK(approx(r, 1.4142135623730951), "anchor $sqrt(2)")
    r = $sqrt(16.0);
    `SVTEST_CHECK(approx(r, 4.0),                "anchor $sqrt(16)")
    r = $pow(2.0, 10.0);
    `SVTEST_CHECK(approx(r, 1024.0),             "anchor $pow(2,10)")
    r = $pow(3.0, 3.0);
    `SVTEST_CHECK(approx(r, 27.0),               "anchor $pow(3,3)")
    r = $floor(3.7);
    `SVTEST_CHECK(approx(r, 3.0),                "anchor $floor(3.7)")
    r = $ceil(3.2);
    `SVTEST_CHECK(approx(r, 4.0),                "anchor $ceil(3.2)")

    // ---------- trigonometric ----------
    r = $sin(0.0);
    `SVTEST_CHECK(approx(r, 0.0),                "$sin(0)")
    r = $sin(PI/2.0);
    `SVTEST_CHECK(approx(r, 1.0),                "$sin(pi/2)")
    r = $sin(PI/6.0);
    `SVTEST_CHECK(approx(r, 0.5),                "$sin(pi/6) == 0.5")
    r = $sin(PI);
    `SVTEST_CHECK(approx(r, 0.0),                "$sin(pi) ~= 0")

    r = $cos(0.0);
    `SVTEST_CHECK(approx(r, 1.0),                "$cos(0)")
    r = $cos(PI/3.0);
    `SVTEST_CHECK(approx(r, 0.5),                "$cos(pi/3) == 0.5")
    r = $cos(PI);
    `SVTEST_CHECK(approx(r, -1.0),               "$cos(pi) == -1")

    r = $tan(0.0);
    `SVTEST_CHECK(approx(r, 0.0),                "$tan(0)")
    r = $tan(PI/4.0);
    `SVTEST_CHECK(approx(r, 1.0),                "$tan(pi/4) == 1")

    r = $asin(0.0);
    `SVTEST_CHECK(approx(r, 0.0),                "$asin(0)")
    r = $asin(0.5);
    `SVTEST_CHECK(approx(r, PI/6.0),             "$asin(0.5) == pi/6")
    r = $asin(1.0);
    `SVTEST_CHECK(approx(r, PI/2.0),             "$asin(1) == pi/2")

    r = $acos(0.0);
    `SVTEST_CHECK(approx(r, PI/2.0),             "$acos(0) == pi/2")
    r = $acos(0.5);
    `SVTEST_CHECK(approx(r, PI/3.0),             "$acos(0.5) == pi/3")
    r = $acos(-1.0);
    `SVTEST_CHECK(approx(r, PI),                 "$acos(-1) == pi")

    r = $atan(0.0);
    `SVTEST_CHECK(approx(r, 0.0),                "$atan(0)")
    r = $atan(1.0);
    `SVTEST_CHECK(approx(r, PI/4.0),             "$atan(1) == pi/4")

    // $atan2(y, x): first arg is y, second is x
    r = $atan2(0.0, 1.0);
    `SVTEST_CHECK(approx(r, 0.0),                "$atan2(0,1) == 0")
    r = $atan2(1.0, 0.0);
    `SVTEST_CHECK(approx(r, PI/2.0),             "$atan2(1,0) == pi/2")
    r = $atan2(1.0, 1.0);
    `SVTEST_CHECK(approx(r, PI/4.0),             "$atan2(1,1) == pi/4")
    r = $atan2(-1.0, -1.0);
    `SVTEST_CHECK(approx(r, -3.0*PI/4.0),        "$atan2(-1,-1) == -3pi/4")

    r = $hypot(3.0, 4.0);
    `SVTEST_CHECK(approx(r, 5.0),                "$hypot(3,4) == 5")
    r = $hypot(5.0, 12.0);
    `SVTEST_CHECK(approx(r, 13.0),               "$hypot(5,12) == 13")
    r = $hypot(1.0, 0.0);
    `SVTEST_CHECK(approx(r, 1.0),                "$hypot(1,0) == 1")

    // ---------- hyperbolic ----------
    r = $sinh(0.0);
    `SVTEST_CHECK(approx(r, 0.0),                "$sinh(0)")
    r = $sinh(1.0);
    `SVTEST_CHECK(approx(r, 1.1752011936438014), "$sinh(1)")

    r = $cosh(0.0);
    `SVTEST_CHECK(approx(r, 1.0),                "$cosh(0)")
    r = $cosh(1.0);
    `SVTEST_CHECK(approx(r, 1.5430806348152437), "$cosh(1)")

    r = $tanh(0.0);
    `SVTEST_CHECK(approx(r, 0.0),                "$tanh(0)")
    r = $tanh(1.0);
    `SVTEST_CHECK(approx(r, 0.7615941559557649), "$tanh(1)")

    r = $asinh(0.0);
    `SVTEST_CHECK(approx(r, 0.0),                "$asinh(0)")
    r = $asinh(1.0);
    `SVTEST_CHECK(approx(r, 0.8813735870195430), "$asinh(1)")

    r = $acosh(1.0);
    `SVTEST_CHECK(approx(r, 0.0),                "$acosh(1)")
    r = $acosh(2.0);
    `SVTEST_CHECK(approx(r, 1.3169578969248166), "$acosh(2)")

    r = $atanh(0.0);
    `SVTEST_CHECK(approx(r, 0.0),                "$atanh(0)")
    r = $atanh(0.5);
    `SVTEST_CHECK(approx(r, 0.5493061443340548), "$atanh(0.5)")

    `SVTEST_PASSFAIL
  end
endmodule
