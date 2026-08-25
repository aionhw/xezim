// SPDX-License-Identifier: MIT
//
// 47_nettype_coercion.sv — §6.6.8 / §23.3.3: an implicitly-declared net used as
// a port actual takes its type from the port it connects to. When that port is
// a user-defined nettype, the implicit net is that nettype — not a 1-bit wire.
//
// This is the "nets are automatically coerced" property that makes the
// loading-effect modelling style practical: an analog node can be named at the
// point of connection without a separate declaration.
//
// Current behaviour (xezim main + xezim-core#28): the implicit net is created
// as a 1-bit wire, so a 64-bit nettype port truncates to it and the node
// resolves to 0.0 with only a width warning. Silently wrong, not diagnosed.

`include "../common/svtest_defs.svh"

function automatic real rsum (input real d []);
  rsum = 0.0;
  foreach (d[i]) rsum += d[i];
endfunction

nettype real rnet with rsum;

module src #(parameter real VAL = 1.0) (inout rnet p);
  assign p = VAL;
endmodule

module snk (inout rnet p, output real seen);
  assign seen = p;
endmodule

module test_47_nettype_coercion;
  `SVTEST_INIT

  // `node_a` is never declared — it exists only as a port actual, so §6.10
  // creates it implicitly and §6.6.8/§23.3.3 give it the port's nettype.
  real seen_a;
  src #(.VAL(1.5)) u1 (.p(node_a));
  src #(.VAL(2.5)) u2 (.p(node_a));
  snk              u3 (.p(node_a), .seen(seen_a));

  // Control: the identical topology on an explicitly declared net. This half
  // passes today, so a failure here would mean something other than coercion
  // regressed.
  rnet node_b;
  real seen_b;
  src #(.VAL(1.5)) v1 (.p(node_b));
  src #(.VAL(2.5)) v2 (.p(node_b));
  snk              v3 (.p(node_b), .seen(seen_b));

  initial begin
    #1;
    `SVTEST_CHECK(seen_a > 3.9999 && seen_a < 4.0001,
                  "implicit net coerced to the port's nettype -> 4.0")
    `SVTEST_CHECK(seen_b > 3.9999 && seen_b < 4.0001,
                  "control: explicitly declared nettype net -> 4.0")
    `SVTEST_PASSFAIL
  end
endmodule
