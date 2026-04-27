// Part 2 synthetic: IFCTRL ex_inst_vld FF + downstream pipe-down logic.
// Mirrors cr_ifu_ifctrl.v lines 163-211 — the FF that latches
// `if_inst_vld_for_ex_aft_hs` per posedge cpuclk, gated by stall/cancel.
//
// Test discipline (avoids same-timestep races between $display and NBA
// application that previously hid the actual semantics):
//   - All regs initialised to 0 — no X starting state. The FF that
//     drives `ex_inst_vld` is given an explicit `= 1'b0` initialiser so
//     simulators that don't fire on the initial X→0 transition (iverilog)
//     match those that do (xezim per IEEE 1800 §9.4.2).
//   - Reset is driven through an EXPLICIT transition: rst_b starts at 1,
//     drops to 0 to assert reset (guaranteed negedge), holds, then rises.
//   - Stimulus changes happen at fixed `#10` (full-cycle) boundaries
//     so they never collide with a posedge. Each stimulus comment shows
//     the cycle number where the new value is sampled.
//   - Sampling uses `$strobe` (end-of-timestep, after all NBAs settle)
//     instead of `$display` (active region), removing the
//     `$display`-vs-NBA ordering ambiguity.
`timescale 1ns/100ps
module top;
  reg cpuclk = 0;
  reg cpurst_b = 1;                        // start de-asserted (no edge ambiguity)
  always #5 cpuclk = ~cpuclk;

  reg ibuf_ifctrl_inst_vld = 0;
  reg split_ifctrl_hs_stall = 0;
  reg if_cancel_for_pipeline = 0;
  reg iu_ifu_ex_stall = 0;
  reg ibus_bypass_inst_vld = 0;
  reg iu_yy_xx_dbgon = 0;
  reg had_ifu_ir_vld = 0;
  reg iu_ifu_inst_fetch = 0;
  reg iu_yy_xx_flush = 0;

  wire if_cancel = iu_ifu_inst_fetch || iu_yy_xx_flush;
  wire ibuf_inst_vld = ibuf_ifctrl_inst_vld && !split_ifctrl_hs_stall;
  wire inst_vld = ibuf_inst_vld || ibus_bypass_inst_vld
               || iu_yy_xx_dbgon && had_ifu_ir_vld;
  wire if_inst_vld = inst_vld && !if_cancel;
  wire if_inst_stall = 1'b0;
  wire if_inst_vld_for_ex = if_inst_vld && !if_inst_stall;
  wire split_ifctrl_hs_inst_vld = 1'b0;
  wire if_inst_vld_for_ex_aft_hs = if_inst_vld_for_ex || split_ifctrl_hs_inst_vld;

  reg ex_inst_vld = 1'b0;                  // explicit initialiser ⇒ no X seed
  always @(posedge cpuclk or negedge cpurst_b) begin
    if(!cpurst_b)
      ex_inst_vld <= 1'b0;
    else if(if_cancel_for_pipeline)
      ex_inst_vld <= 1'b0;
    else if(!iu_ifu_ex_stall)
      ex_inst_vld <= if_inst_vld_for_ex_aft_hs;
  end

  reg [31:0] tcyc = 0;
  always @(posedge cpuclk) begin
    tcyc <= tcyc + 1;
    // $strobe samples at end-of-timestep, AFTER all NBAs in this slot
    // have applied. Removes the $display-vs-NBA race that produced
    // off-by-one diffs between iverilog and xezim on this design.
    $strobe("CYC %0d rst=%b ibctrl=%b cancel=%b stall=%b vld_for_ex=%b ex_vld=%b",
      tcyc, cpurst_b, ibuf_ifctrl_inst_vld, if_cancel_for_pipeline, iu_ifu_ex_stall,
      if_inst_vld_for_ex_aft_hs, ex_inst_vld);
  end

  initial begin
    // All stimulus changes land at NEGEDGE clk times (t=10, 20, 30 ...)
    // — never at posedge (t=5, 15, 25 ...) — so they don't race with the
    // active posedge in the same time slot. iverilog and xezim then
    // both deterministically observe the new value at the *next* posedge.
    #20;                                   // t=20  negedge cyc 1
    cpurst_b = 1'b0;                       // assert reset; FF latches 0 at cyc 2 posedge
    #20;                                   // t=40
    cpurst_b = 1'b1;                       // release reset; resume at cyc 4 posedge
    #20;                                   // t=60  negedge cyc 5
    ibuf_ifctrl_inst_vld = 1'b1;           // observed at cyc 6
    #20;                                   // t=80  negedge cyc 7
    iu_ifu_ex_stall = 1'b1;                // observed at cyc 8
    #20;                                   // t=100 negedge cyc 9
    iu_ifu_ex_stall = 1'b0;                // observed at cyc 10
    #20;                                   // t=120 negedge cyc 11
    if_cancel_for_pipeline = 1'b1;         // observed at cyc 12
    #20;                                   // t=140 negedge cyc 13
    if_cancel_for_pipeline = 1'b0;         // observed at cyc 14
    #20;                                   // t=160 negedge cyc 15
    ibuf_ifctrl_inst_vld = 1'b0;           // observed at cyc 16
    #20 $finish;
  end
endmodule
