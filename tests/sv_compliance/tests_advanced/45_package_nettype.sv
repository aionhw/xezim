// SPDX-License-Identifier: MIT
//
// 45_package_nettype.sv — §6.6.7 + §26.2: a user-defined nettype declared in a
// PACKAGE. That is the natural place for one: a nettype used on both sides of a
// port connection must be named identically in both modules, so it has to live
// in a shared scope.
//
// Both reference forms are covered:
//   - `import P::*;` then the bare name
//   - the qualified `P::name` with no import (§26.3)
//
// Before package nettypes were registered, neither form classified the net as a
// net at all: a second continuous driver failed elaboration with the generic
// "Variable 'n' has multiple continuous drivers" instead of resolving.

`include "../common/svtest_defs.svh"

package EE;
  typedef struct {
    real v;
    bit  tag;
  } cell_t;

  // Sum the reals, OR the tags — a fold no built-in net resolution expresses.
  function automatic cell_t cell_sum (input cell_t driver []);
    cell_sum.v   = 0.0;
    cell_sum.tag = 1'b0;
    foreach (driver[i]) begin
      cell_sum.v   += driver[i].v;
      cell_sum.tag |= driver[i].tag;
    end
  endfunction

  nettype cell_t cellnet with cell_sum;

  // A scalar real nettype in the same package, to pin that the registry keeps
  // more than one entry per package.
  function automatic real rmax (input real driver []);
    rmax = driver[0];
    foreach (driver[i]) if (driver[i] > rmax) rmax = driver[i];
  endfunction

  nettype real maxnet with rmax;
endpackage

import EE::*;

module test_45_package_nettype;
  `SVTEST_INIT

  // ----- bare name after a wildcard import -----
  cellnet a;
  assign a = '{1.5, 1'b0};
  assign a = '{2.5, 1'b1};

  // ----- qualified reference, no import needed for this one -----
  EE::cellnet b;
  assign b = '{0.25, 1'b0};
  assign b = '{0.75, 1'b0};
  assign b = '{1.00, 1'b0};

  // ----- second nettype from the same package, scalar real -----
  maxnet m;
  assign m = 3.5;
  assign m = 9.25;
  assign m = 1.0;

  initial begin
    #1;
    `SVTEST_CHECK(a.v > 3.9999 && a.v < 4.0001,
                  "imported package nettype: 1.5 + 2.5 = 4.0")
    `SVTEST_CHECK(a.tag === 1'b1,
                  "imported package nettype: tag OR = 1")
    `SVTEST_CHECK(b.v > 1.9999 && b.v < 2.0001,
                  "qualified EE::cellnet over 3 drivers = 2.0")
    `SVTEST_CHECK(b.tag === 1'b0,
                  "qualified EE::cellnet: tag OR = 0")
    `SVTEST_CHECK(m > 9.2499 && m < 9.2501,
                  "second package nettype (scalar real max) = 9.25")

    `SVTEST_PASSFAIL
  end
endmodule
