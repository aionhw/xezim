// SPDX-License-Identifier: MIT
//
// 48_nettype_inlined_real.sv — §6.6.7 a user-defined nettype net keeps its
// REAL-ness when the module declaring it is instantiated rather than top-level.
//
// Inlining a child module's scalar declaration into its parent recomputed the
// signal's is_real from the RAW declared type. A net declared with a nettype
// carries a type NAME that only resolves to `real` through the nettype
// registry, so the inlined net was created as an integer slot and every
// resolution result was rounded on write. The identical declaration at TOP
// level took a resolving path and stayed real.
//
// The rounding is what makes this pernicious: it is invisible whenever the
// resolved value happens to be a whole number. A net resolving to 3.0 or 15.0
// reads back correctly while the same net resolving to 0.001 reads 0.0. Every
// value below is therefore deliberately non-integer, and case D pins the
// rounding directly by checking a value that would round to a DIFFERENT
// integer rather than to zero.
//
//   A: fractional sum on a node inside an instantiated module   (read 0.0)
//   B: the same node at top level                               (control)
//   C: RNM-scale currents through two levels of hierarchy       (read 0.0)
//   D: 3.7 must survive intact, not land on a whole number      (rounding)

`include "../common/svtest_defs.svh"

function automatic real isum (input real d []);
  isum = 0.0;
  foreach (d[i]) isum += d[i];
endfunction

nettype real inet with isum;

// A current-mode source: contributes a fractional current to a shared node.
module isrc #(parameter real K = 1.0) (input real v, output real o);
  assign o = K * v;
endmodule

// Declares the nettype node ITSELF, then is instantiated -- the case that broke.
module leaf (input real v, output real seen);
  inet node;
  isrc #(.K(0.0011)) s0 (.v(v), .o(node));
  isrc #(.K(0.0024)) s1 (.v(v), .o(node));
  assign seen = node;
endmodule

// Two levels: the node lives one module deeper than the instantiation site.
module mid (input real v, output real seen);
  leaf l0 (.v(v), .seen(seen));
endmodule

// Single driver, value chosen to round to a DIFFERENT integer if truncated.
module round_probe (input real v, output real seen);
  inet node;
  isrc #(.K(3.7)) s0 (.v(v), .o(node));
  assign seen = node;
endmodule

module top;
  `SVTEST_INIT

  real drive;

  // ---- A: fractional sum inside an instantiated module ----
  //   0.0011 + 0.0024 = 0.0035 at v = 1.0
  real seen_a;
  leaf a0 (.v(drive), .seen(seen_a));

  // ---- B: the same resolution at TOP level (control: always worked) ----
  inet node_b;
  isrc #(.K(0.0011)) b0 (.v(drive), .o(node_b));
  isrc #(.K(0.0024)) b1 (.v(drive), .o(node_b));

  // ---- C: RNM-scale currents, node two levels down ----
  real seen_c;
  mid c0 (.v(drive), .seen(seen_c));

  // ---- D: 3.7 must survive as 3.7, not round to 4.0 ----
  real seen_d;
  round_probe d0 (.v(drive), .seen(seen_d));

  initial begin
    drive = 1.0;
    #1;

    `SVTEST_CHECK(seen_a > 0.00349 && seen_a < 0.00351,
                  "A: fractional sum on a node inside an instance -> 0.0035")

    `SVTEST_CHECK(node_b > 0.00349 && node_b < 0.00351,
                  "B: same resolution at top level -> 0.0035")

    `SVTEST_CHECK(seen_a > node_b - 0.000001 && seen_a < node_b + 0.000001,
                  "A/B: instantiated and top-level nodes must agree")

    `SVTEST_CHECK(seen_c > 0.00349 && seen_c < 0.00351,
                  "C: node two levels down keeps RNM-scale current -> 0.0035")

    `SVTEST_CHECK(seen_d > 3.6999 && seen_d < 3.7001,
                  "D: 3.7 must not be rounded to an integer")

    // Guard the specific corruption, not just the tolerance band: an integer
    // slot reads a sub-1.0 resolution back as exactly 0.0.
    `SVTEST_CHECK(seen_a != 0.0,
                  "A: node must not collapse to an integer 0")

    `SVTEST_PASSFAIL
  end
endmodule
