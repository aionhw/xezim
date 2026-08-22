// SPDX-License-Identifier: MIT
//
// 44_eenet_rnm.sv — §6.6.7 user-defined nettypes used for REAL NUMBER
// MODELLING: the "EEnet" pattern from the DVCon Europe 2019 TI/Cadence paper
// "Enabling Digital Mixed-Signal Verification of Loading Effects in Power
// Regulation using SystemVerilog User-Defined Nettype" (Caicedo / Fritz).
//
// A UDT struct carries voltage / current / series resistance; a UDN declares
// nets of it; a UDR combines every driver at a shared node the way an analog
// solver would. Unlike 38 (scalar resolvers) and 40 (the LRM's `Tsum`), this
// exercises the combination that motivates UDNs in practice:
//
//   - a struct of THREE reals resolved field-by-field,
//   - arithmetic over the driver set that no fold can express (Millman's
//     theorem: V = (sum Vk/Rk + sum Ik) / (sum 1/Rk)),
//   - drivers that differ in NATURE — voltage sources, current sinks, and
//     passive loads sharing one node,
//   - 2, 3 and 4 simultaneous drivers.
//
// Reference values are hand-derived circuit results, so a resolver that is
// silently skipped, folded, or called with a truncated driver set cannot pass.

`include "../common/svtest_defs.svh"

package EE_pkg;
  typedef struct {
    real V;   // Thevenin source voltage
    real I;   // current injected into the node
    real R;   // series resistance (0.0 => ideal current source, no conductance)
  } EEstruct;
endpackage

import EE_pkg::*;

// Millman / Thevenin combination at a shared node.
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

// Power rail: the regulator sets the voltage, loads add current (KCL).
function automatic EEstruct res_rail (input EEstruct driver []);
  real i_sum, v;
  i_sum = 0.0; v = 0.0;
  foreach (driver[k]) begin
    i_sum += driver[k].I;
    if (driver[k].V != 0.0) v = driver[k].V;
  end
  res_rail.V = v;
  res_rail.I = i_sum;
  res_rail.R = 0.0;
endfunction

nettype EEstruct RailNet with res_rail;

module test_44_eenet_rnm;
  `SVTEST_INIT

  // Two Thevenin sources sharing a node: 5V/100R against 0V/100R.
  //   V = (5/100) / (2/100) = 2.5,  R = 1/(2/100) = 50
  EEnet nA;
  assign nA = '{5.0, 0.0, 100.0};
  assign nA = '{0.0, 0.0, 100.0};

  // Voltage source loaded by a current sink — the loading effect EEnet exists
  // to model.  V = (5/10 - 0.1) / (1/10) = 4.0,  R = 10,  I = -0.1
  EEnet nB;
  assign nB = '{5.0,  0.0, 10.0};
  assign nB = '{0.0, -0.1,  0.0};

  // Three-way divider: 12V/100R against two grounded 100R legs.
  //   V = (12/100) / (3/100) = 4.0,  R = 100/3
  EEnet nC;
  assign nC = '{12.0, 0.0, 100.0};
  assign nC = '{ 0.0, 0.0, 100.0};
  assign nC = '{ 0.0, 0.0, 100.0};

  // 1.8V rail with three loads — 4 simultaneous drivers, KCL current sum.
  RailNet rail;
  assign rail = '{1.8,  0.0,   0.0};
  assign rail = '{0.0, -0.010, 0.0};
  assign rail = '{0.0, -0.025, 0.0};
  assign rail = '{0.0, -0.005, 0.0};

  initial begin
    #1;
    `SVTEST_CHECK(nA.V > 2.4999 && nA.V < 2.5001,
                  "A: two 100R Thevenin sources -> node V = 2.5")
    `SVTEST_CHECK(nA.R > 49.999 && nA.R < 50.001,
                  "A: two 100R in parallel -> Thevenin R = 50")
    `SVTEST_CHECK(nB.V > 3.9999 && nB.V < 4.0001,
                  "B: 5V/10R loaded by 100mA sink -> V = 4.0 (loading effect)")
    `SVTEST_CHECK(nB.I > -0.1001 && nB.I < -0.0999,
                  "B: node current = -0.1")
    `SVTEST_CHECK(nC.V > 3.9999 && nC.V < 4.0001,
                  "C: 12V through 3-way 100R divider -> V = 4.0")
    `SVTEST_CHECK(nC.R > 33.3332 && nC.R < 33.3334,
                  "C: three 100R in parallel -> R = 33.3333")
    `SVTEST_CHECK(rail.V > 1.7999 && rail.V < 1.8001,
                  "D: rail voltage passes through 4 drivers -> 1.8")
    `SVTEST_CHECK(rail.I > -0.0401 && rail.I < -0.0399,
                  "D: KCL over 3 loads -> total current = -40mA")

    `SVTEST_PASSFAIL
  end
endmodule
