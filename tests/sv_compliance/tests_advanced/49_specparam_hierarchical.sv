// SPDX-License-Identifier: MIT
//
// 49_specparam_hierarchical.sv — §6.20.5 specparam is a module-scoped
// elaboration-time constant, and §23.3.3 makes it reachable by hierarchical
// name exactly like a localparam.
//
// The parser skipped a `specparam` declaration to its `;` and DROPPED it, so
// the name never existed: `u_child.SPEC_DELAY` resolved to nothing and read x
// (0 inside a generate scope), while the localparam declared on the very next
// line of the same module read back correctly. Every value below is
// reference-simulator verified.
//
// Note for anyone re-checking against the reference: its optimizer ELIDES a
// specparam that is only referenced inside a specify block, and the run then
// reports "Unresolved reference to 'SPEC_DELAY'". That is an optimization
// artifact, not the language rule — with accessibility preserved the
// reference resolves all of these and produces the values asserted here.

`timescale 1ns/1ps

`include "../common/svtest_defs.svh"

// §31.2 timing-check terminals must be NETS, so the ports are `wire`.
module sp_child #(
  parameter int CHILD_DELAY = 2
) (
  input  wire clk,
  input  wire d,
  output wire q
);
  specparam SPEC_DELAY = CHILD_DELAY + 1;
  localparam int LOCAL_DELAY = CHILD_DELAY + 1;

  assign q = d;

  // §6.20.6/§6.20.7: the specparam feeds a module path delay and the timing
  // checks, which is the shape that made the reference elide it.
  specify
    (clk => q) = SPEC_DELAY;
    $setup(d, clk, SPEC_DELAY);
    $hold(clk, d, SPEC_DELAY);
    $width(posedge clk, SPEC_DELAY);
  endspecify
endmodule

module sp_intermediate #(parameter int MID_DELAY = 3);
  specparam MID_SPEC = MID_DELAY + 1;
  localparam int MID_LOCAL = MID_DELAY + 1;
  sp_child #(.CHILD_DELAY(MID_DELAY)) u_child();
endmodule

module top;
  `SVTEST_INIT

  sp_child #(.CHILD_DELAY(5)) u_child();
  sp_intermediate #(.MID_DELAY(7)) u_intermediate();

  parameter int BASE_DELAY = 4;
  sp_child #(.CHILD_DELAY(BASE_DELAY + 3)) u_child_expr();   // CHILD_DELAY = 7

  genvar g;
  generate
    for (g = 0; g < 3; g++) begin : gen_child
      sp_child #(.CHILD_DELAY(g + 2)) u_gen_child();
    end
  endgenerate

  initial begin
    // Direct instance: specparam and localparam agree.
    `SVTEST_CHECK(u_child.SPEC_DELAY  == 6, "child.SPEC_DELAY = 6 (5+1)")
    `SVTEST_CHECK(u_child.LOCAL_DELAY == 6, "child.LOCAL_DELAY = 6 (5+1)")

    // A specparam on an intermediate level, not used in any specify block.
    `SVTEST_CHECK(u_intermediate.MID_SPEC  == 8, "intermediate.MID_SPEC = 8 (7+1)")
    `SVTEST_CHECK(u_intermediate.MID_LOCAL == 8, "intermediate.MID_LOCAL = 8 (7+1)")

    // Two levels down.
    `SVTEST_CHECK(u_intermediate.u_child.SPEC_DELAY  == 8, "intermediate.child.SPEC_DELAY = 8")
    `SVTEST_CHECK(u_intermediate.u_child.LOCAL_DELAY == 8, "intermediate.child.LOCAL_DELAY = 8")

    // The parameter is an EXPRESSION, not a literal.
    `SVTEST_CHECK(u_child_expr.SPEC_DELAY  == 8, "child_expr.SPEC_DELAY = 8 (7+1)")
    `SVTEST_CHECK(u_child_expr.LOCAL_DELAY == 8, "child_expr.LOCAL_DELAY = 8 (7+1)")

    // §27 generate scope: each instance gets its own specialization.
    `SVTEST_CHECK(gen_child[0].u_gen_child.SPEC_DELAY == 3, "gen_child[0].SPEC_DELAY = 3 (2+1)")
    `SVTEST_CHECK(gen_child[1].u_gen_child.SPEC_DELAY == 4, "gen_child[1].SPEC_DELAY = 4 (3+1)")
    `SVTEST_CHECK(gen_child[2].u_gen_child.SPEC_DELAY == 5, "gen_child[2].SPEC_DELAY = 5 (4+1)")
    `SVTEST_CHECK(gen_child[0].u_gen_child.LOCAL_DELAY == 3, "gen_child[0].LOCAL_DELAY = 3 (2+1)")
    `SVTEST_CHECK(gen_child[1].u_gen_child.LOCAL_DELAY == 4, "gen_child[1].LOCAL_DELAY = 4 (3+1)")
    `SVTEST_CHECK(gen_child[2].u_gen_child.LOCAL_DELAY == 5, "gen_child[2].LOCAL_DELAY = 5 (4+1)")

    `SVTEST_PASSFAIL
    $finish;
  end
endmodule
