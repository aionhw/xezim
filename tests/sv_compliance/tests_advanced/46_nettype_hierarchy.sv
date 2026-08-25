// SPDX-License-Identifier: MIT
//
// 46_nettype_hierarchy.sv — §6.6.7 user-defined nettype nets driven from
// several module INSTANCES through ports. This is the topology user-defined
// nettypes exist for (DVCon Europe 2019, TI/Cadence, "Enabling Digital
// Mixed-Signal Verification of Loading Effects in Power Regulation"): a shared
// analog node with drivers and loads contributed by different blocks.
//
// Tests 38/40/44/45 all resolve within ONE module. The failure mode this file
// pins is different in kind: a nettype net crossing a port boundary is
// elaborated as a separate signal per port, joined by identity continuous
// assigns. Resolving each of those separately runs the resolution function once
// per port net and then AGAIN over the results.
//
// That compounding is INVISIBLE for an associative resolver — a plain sum gives
// the right answer either way, which is exactly why it can go unnoticed — so
// case C below deliberately uses Millman's theorem, where resolving twice does
// not equal resolving once. It collapsed to 0.0 before nettype resolution
// became a whole-design pass.
//
//   A: three instances on one scalar node        (sum, would pass either way)
//   B: one parent driver + one instance driver   (mixed sources on a node)
//   C: struct-valued node, NESTED hierarchy, real parameters through #()
//      overrides, resolved by Millman's theorem  (catches double-resolution)

`include "../common/svtest_defs.svh"

// ---------------------------------------------------------------------------
// Scalar real nettype, summed.
// ---------------------------------------------------------------------------
function automatic real rsum (input real d []);
  rsum = 0.0;
  foreach (d[i]) rsum += d[i];
endfunction

nettype real rnet with rsum;

module rsrc #(parameter real VAL = 1.0) (inout rnet p);
  assign p = VAL;
endmodule

// ---------------------------------------------------------------------------
// Electrically-equivalent struct nettype: voltage / current / series R.
// Resolution is Millman's theorem, which is NOT associative over partial
// results — resolving per port net and again at the node gives a wrong answer.
//   V = (sum Vk/Rk + sum Ik) / (sum 1/Rk),   R = 1 / (sum 1/Rk)
// R == 0.0 marks an ideal current source: contributes I, no conductance.
// ---------------------------------------------------------------------------
typedef struct {
  real V;
  real I;
  real R;
} EEstruct;

function automatic EEstruct res_EE (input EEstruct driver []);
  real g_sum, i_sum, vg_sum;
  g_sum = 0.0; i_sum = 0.0; vg_sum = 0.0;
  foreach (driver[k]) begin
    i_sum += driver[k].I;
    if (driver[k].R != 0.0) begin
      g_sum  += 1.0 / driver[k].R;
      vg_sum += driver[k].V / driver[k].R;
    end
  end
  if (g_sum == 0.0) begin
    res_EE.V = 0.0;
    res_EE.R = 0.0;
  end else begin
    res_EE.V = (vg_sum + i_sum) / g_sum;
    res_EE.R = 1.0 / g_sum;
  end
  res_EE.I = i_sum;
endfunction

nettype EEstruct EEnet with res_EE;

// Current source IA in parallel with RA.
module isrc #(parameter real IA = 0.0, parameter real RA = 1.0) (inout EEnet p);
  assign p = '{0.0, IA,  0.0};
  assign p = '{0.0, 0.0, RA };
endmodule

// Thevenin source VB behind RB.
module vsrc #(parameter real VB = 0.0, parameter real RB = 1.0) (inout EEnet p);
  assign p = '{VB, 0.0, RB};
endmodule

// Passive load to ground.
module eload #(parameter real RL = 1.0) (inout EEnet p);
  assign p = '{0.0, 0.0, RL};
endmodule

// One level deeper, so the node is shared across TWO levels of hierarchy.
module dut (inout EEnet node);
  isrc #(.IA(0.010), .RA(1000.0)) u1 (.p(node));
  vsrc #(.VB(5.0),   .RB(100.0))  u2 (.p(node));
endmodule

// ---------------------------------------------------------------------------
module test_46_nettype_hierarchy;
  `SVTEST_INIT

  // ---- A: three instances driving one scalar node -> 1 + 2 + 4 = 7 ----
  rnet node1;
  rsrc #(.VAL(1.0)) a1 (.p(node1));
  rsrc #(.VAL(2.0)) a2 (.p(node1));
  rsrc #(.VAL(4.0)) a3 (.p(node1));

  // ---- B: a driver in the parent plus one from an instance -> 8 + 16 = 24 ----
  rnet node2;
  assign node2 = 8.0;
  rsrc #(.VAL(16.0)) b1 (.p(node2));

  // ---- C: struct node shared by a nested dut and a parent-level load ----
  //   G  = 1/1000 + 1/100 + 1/500 = 0.013
  //   Vg = 5/100 = 0.05,  I = 0.010
  //   V  = (0.05 + 0.010) / 0.013 = 4.615384615...
  //   R  = 1 / 0.013           = 76.923076923...
  EEnet node3;
  dut   d0 (.node(node3));
  eload #(.RL(500.0)) c1 (.p(node3));

  initial begin
    #1;

    `SVTEST_CHECK(node1 > 6.9999 && node1 < 7.0001,
                  "A: three instances on one scalar node -> 7.0")

    `SVTEST_CHECK(node2 > 23.9999 && node2 < 24.0001,
                  "B: parent driver + instance driver -> 24.0")

    `SVTEST_CHECK(node3.V > 4.615384 && node3.V < 4.615386,
                  "C: Millman over nested hierarchy -> V = 4.615385")
    `SVTEST_CHECK(node3.R > 76.923076 && node3.R < 76.923078,
                  "C: Thevenin R of three parallel legs -> 76.923077")
    `SVTEST_CHECK(node3.I > 0.0099999 && node3.I < 0.0100001,
                  "C: current-source contribution survives the port hops -> 0.01")

    `SVTEST_PASSFAIL
  end
endmodule
