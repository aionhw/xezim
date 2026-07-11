// SPDX-License-Identifier: MIT
//
// 38_resolver_dispatch.sv — Tier-2 regression: the named resolver
// function registered in `nettype ... with <fn>` actually gets called by
// the simulator with the simultaneous-driver queue.
//
// Reference behavior: QuestaSim-64 2021.2_1 (ModelSim/Questa is one of the
// few open commercial simulators that implements §6.6.7 user-defined
// resolvers; Icarus 12 does NOT support `nettype` at all, so this file
// cannot be cross-checked against Icarus and is Questa-only).
//
// Test goal: prove that xezim's elaborator actually invokes the resolver
// function instead of falling back to a hardcoded OR fold (which is its
// current behavior at xezim-core/src/elaborate.rs:2802-2844).
//
// The canonical-resolver tests would catch a "stuck-OR" bug because the
// AND and XOR resolvers produce different values. The custom-resolver tests
// (`my_resolve_count_ones`) catch anything that returns a folded value
// rather than a real function-call result, because `cnt_mix` returns 3
// which is not reachable by any static fold of [1,0,1,1,0].

`include "svtest_defs.svh"

// ----------------------------------------------------------------------------
// Resolver functions. Each must be `automatic`, take a single unpacked-queue
// argument whose element type matches the nettype element type, and return
// a value compatible with the nettype element type.
// ----------------------------------------------------------------------------

function automatic logic my_resolve_or(logic drivers []);
  automatic logic acc;
  int i;
  begin
    acc = 1'b0;
    foreach (drivers[i]) acc |= drivers[i];
    return acc;
  end
endfunction

function automatic logic my_resolve_and(logic drivers []);
  automatic logic acc;
  int i;
  begin
    acc = 1'b1;
    foreach (drivers[i]) acc &= drivers[i];
    return acc;
  end
endfunction

function automatic logic my_resolve_xor(logic drivers []);
  automatic logic acc;
  int i;
  begin
    acc = 1'b0;
    foreach (drivers[i]) acc ^= drivers[i];
    return acc;
  end
endfunction

// A custom resolver that returns the *count* of non-zero drivers. This
// forces the test value to be a number that no static fold of [1,0,1,1,0]
// can produce (4-bit, max value 5), so the only way the test passes is
// if the simulator actually invokes this function.
function automatic logic [3:0] my_resolve_count_ones(logic [3:0] drivers []);
  automatic logic [3:0] cnt;
  int i;
  begin
    cnt = 4'h0;
    foreach (drivers[i]) if (drivers[i] !== 4'h0) cnt++;
    return cnt;
  end
endfunction

nettype logic          or_net_t  with my_resolve_or;
nettype logic          and_net_t with my_resolve_and;
nettype logic          xor_net_t with my_resolve_xor;
nettype logic [3:0]    count_net_t with my_resolve_count_ones;

// ----------------------------------------------------------------------------
// Single top-level test.
// ----------------------------------------------------------------------------

module test_38_tier2_resolver_dispatch;
  `SVTEST_INIT

  // ----- canonical resolvers (single-bit) -----
  or_net_t  cor1;  assign cor1  = 1'b0; assign cor1  = 1'b1;             // OR:  [0,1] -> 1
  or_net_t  cor2;  assign cor2  = 1'b1; assign cor2  = 1'b1;             // OR:  [1,1] -> 1
  or_net_t  cor3;  assign cor3  = 1'b0; assign cor3  = 1'b0;             // OR:  [0,0] -> 0
  and_net_t cand1; assign cand1 = 1'b0; assign cand1 = 1'b1;             // AND: [0,1] -> 0
  and_net_t cand2; assign cand2 = 1'b1; assign cand2 = 1'b1;             // AND: [1,1] -> 1
  and_net_t cand3; assign cand3 = 1'b1; assign cand3 = 1'b0; assign cand3 = 1'b1; // AND: [1,0,1] -> 0
  xor_net_t cxor1; assign cxor1 = 1'b0; assign cxor1 = 1'b1;             // XOR: [0,1] -> 1
  xor_net_t cxor2; assign cxor2 = 1'b1; assign cxor2 = 1'b1;             // XOR: [1,1] -> 0
  xor_net_t cxor3; assign cxor3 = 1'b1; assign cxor3 = 1'b1; assign cxor3 = 1'b1; // XOR: [1,1,1] -> 1

  // ----- custom resolver (4-bit, counts non-zero drivers) -----
  count_net_t cnt0;    assign cnt0    = 4'h0; // -> 0
  count_net_t cnt1;    assign cnt1    = 4'h1; // -> 1
  count_net_t cnt2;    assign cnt2    = 4'h1; assign cnt2    = 4'h1; // -> 2
  count_net_t cnt3;    assign cnt3    = 4'h1; assign cnt3    = 4'h1; assign cnt3    = 4'h1; // -> 3
  count_net_t cnt4;    assign cnt4    = 4'h1; assign cnt4    = 4'h1; assign cnt4    = 4'h1; assign cnt4    = 4'h1; // -> 4
  count_net_t cnt_mix; assign cnt_mix = 4'h1; assign cnt_mix = 4'h0; assign cnt_mix = 4'h1;
  assign cnt_mix = 4'h1; assign cnt_mix = 4'h0; // -> 3

  // ----- dynamic drivers, resolved per event tick -----
  or_net_t dync;
  logic d_a, d_b;
  assign dync = d_a;
  assign dync = d_b;

  initial begin
    #0;

    // Canonical resolvers
    `SVTEST_CHECK(cor1  === 1'b1,        "OR:  [0,1] -> 1")
    `SVTEST_CHECK(cor2  === 1'b1,        "OR:  [1,1] -> 1")
    `SVTEST_CHECK(cor3  === 1'b0,        "OR:  [0,0] -> 0")
    `SVTEST_CHECK(cand1 === 1'b0,        "AND: [0,1] -> 0")
    `SVTEST_CHECK(cand2 === 1'b1,        "AND: [1,1] -> 1")
    `SVTEST_CHECK(cand3 === 1'b0,        "AND: [1,0,1] -> 0")
    `SVTEST_CHECK(cxor1 === 1'b1,        "XOR: [0,1] -> 1")
    `SVTEST_CHECK(cxor2 === 1'b0,        "XOR: [1,1] -> 0 (parity)")
    `SVTEST_CHECK(cxor3 === 1'b1,        "XOR: [1,1,1] -> 1 (parity)")

    // Custom resolver (function-call probe — values are not reachable by
    // any static OR/AND/XOR fold of the driver queue)
    `SVTEST_CHECK(cnt0    === 4'h0,      "CUSTOM: [0]                  -> 0")
    `SVTEST_CHECK(cnt1    === 4'h1,      "CUSTOM: [1]                  -> 1")
    `SVTEST_CHECK(cnt2    === 4'h2,      "CUSTOM: [1,1]                -> 2")
    `SVTEST_CHECK(cnt3    === 4'h3,      "CUSTOM: [1,1,1]              -> 3")
    `SVTEST_CHECK(cnt4    === 4'h4,      "CUSTOM: [1,1,1,1]            -> 4")
    `SVTEST_CHECK(cnt_mix === 4'h3,      "CUSTOM: [1,0,1,1,0]          -> 3")

    // Dynamic — verifies the resolver runs on event-tick boundary, not
    // just at time 0.
    d_a = 1'b0; d_b = 1'b0;
    #1;
    `SVTEST_CHECK(dync === 1'b0,         "DYN t=1: [0,0] -> 0")

    d_a = 1'b1;
    #1;
    `SVTEST_CHECK(dync === 1'b1,         "DYN t=2: [1,0] -> 1")

    d_b = 1'b1;
    #1;
    `SVTEST_CHECK(dync === 1'b1,         "DYN t=3: [1,1] -> 1")

    d_a = 1'b0;
    #1;
    `SVTEST_CHECK(dync === 1'b1,         "DYN t=4: [0,1] -> 1")

    d_a = 1'b0; d_b = 1'b0;
    #1;
    `SVTEST_CHECK(dync === 1'b0,         "DYN t=5: [0,0] -> 0")

    `SVTEST_PASSFAIL
  end
endmodule