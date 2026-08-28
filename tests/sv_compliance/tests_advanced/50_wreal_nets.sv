// SPDX-License-Identifier: MIT
//
// 50_wreal_nets.sv — Verilog-AMS `wreal`: a net whose value is a real, with
// multiple drivers resolved by SUMMING them.
//
// `wreal` is a NET TYPE that names no data type of its own, so a signal
// declared with it has to be given real storage from the net type alone.
// Declared as a plain net it lands as a bit vector and every value is
// rounded on write -- 2.5 reads back 3.0, 1.25 reads back 1.0, and a
// negative value wraps to a huge unsigned integer. Every value below is
// therefore deliberately non-integer.
//
// Multi-driver resolution is the second half. Verilog-AMS leaves it
// tool-defined; summing is the resolution that makes a current-summing
// wrapper mean what it says -- several stages each drive their contribution
// onto a shared node and the node sees the total, which is Kirchhoff's
// current law. Bitwise wire resolution would instead call two drivers a
// conflict and yield x.
//
//   A: a scalar wreal keeps a fractional value            (rounding)
//   B: an undriven wreal reads 0.0
//   C: two drivers on one wreal sum                       (resolution)
//   D: the sum is signed, not an unsigned wrap
//   E: a wreal crossing a module port keeps its fraction
//   F: ONE driver is passed through, not doubled          (fold identity)
//   G: three drivers sum                                  (fold is n-ary)
//
// F is the half of the resolution rule that a two-driver test cannot see: the
// driver chain folds with `+`, so an accumulator seeded wrong gives 2x on a
// single driver while every multi-driver case still looks right.

`include "../common/svtest_defs.svh"

// Drives its input onto a wreal output -- the exact shape a generated
// current-mode wrapper instantiates once per contributing stage.
module idrv (input real i, output wreal o);
  assign o = i;
endmodule

// A wreal that crosses a port boundary in both directions.
module thru (input wreal a, output wreal b);
  assign b = a;
endmodule

module top;
  `SVTEST_INIT

  // ---- A: scalar wreal, fractional ----
  wreal single;
  assign single = 2.5;

  // ---- B/C/D: two drivers onto one node ----
  real a, b;
  wreal node;
  idrv d0 (.i(a), .o(node));
  idrv d1 (.i(b), .o(node));

  // ---- E: across a port ----
  wreal src, dst;
  assign src = 1.25;
  thru t0 (.a(src), .b(dst));

  // ---- F: exactly one driver ----
  real lone;
  wreal solo;
  idrv s0 (.i(lone), .o(solo));

  // ---- G: three drivers ----
  real t1, t2, t3;
  wreal trio;
  idrv e0 (.i(t1), .o(trio));
  idrv e1 (.i(t2), .o(trio));
  idrv e2 (.i(t3), .o(trio));

  initial begin
    a = 0.0;
    b = 0.0;
    lone = 0.0;
    t1 = 0.0;
    t2 = 0.0;
    t3 = 0.0;
    #1;

    `SVTEST_CHECK(single > 2.4999 && single < 2.5001,
                  "A: a scalar wreal must keep 2.5, not round to 3.0")

    `SVTEST_CHECK(node > -0.0001 && node < 0.0001,
                  "B: two drivers of 0.0 sum to 0.0")

    a = 1.5;
    b = 2.25;
    #1;
    `SVTEST_CHECK(node > 3.7499 && node < 3.7501,
                  "C: two drivers on one wreal sum (1.5 + 2.25 = 3.75)")

    a = -0.75;
    b = 2.25;
    #1;
    `SVTEST_CHECK(node > 1.4999 && node < 1.5001,
                  "D: the sum is signed (-0.75 + 2.25 = 1.5)")

    // Guard the specific corruption: an integer slot reads a negative
    // contribution back as a huge unsigned value, not as a small sum.
    `SVTEST_CHECK(node < 1000.0,
                  "D: a negative driver must not wrap to a huge unsigned value")

    `SVTEST_CHECK(dst > 1.2499 && dst < 1.2501,
                  "E: a wreal crossing a module port keeps its fraction")

    // F: a single driver must be passed through unchanged. A fold that
    // seeds its accumulator instead of taking the first driver whole
    // returns 5.0 here while every case above still reads correctly.
    lone = 2.5;
    #1;
    `SVTEST_CHECK(solo > 2.4999 && solo < 2.5001,
                  "F: one driver on a wreal is passed through, not doubled")

    // G: the fold is n-ary, not pairwise-only.
    t1 = 1.5;
    t2 = 2.25;
    t3 = -0.75;
    #1;
    `SVTEST_CHECK(trio > 2.9999 && trio < 3.0001,
                  "G: three drivers sum (1.5 + 2.25 - 0.75 = 3.0)")

    `SVTEST_PASSFAIL
  end
endmodule
