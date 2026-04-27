// Part 2 synthetic: IFCTRL ex_inst_vld FF + downstream pipe-down logic.
// Mirrors cr_ifu_ifctrl.v lines 163-211 — the FF that latches
// `if_inst_vld_for_ex_aft_hs` per posedge cpuclk, gated by stall/cancel.
//
// Test: drive ibctrl_inst_vld + cancel/stall combinations, verify ex_inst_vld
// follows the spec semantics post-reset.
`timescale 1ns/100ps
module top;
  reg cpuclk = 0;
  reg cpurst_b = 0;
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
  wire if_inst_stall = 1'b0; // simplified for synthetic
  wire if_inst_vld_for_ex = if_inst_vld && !if_inst_stall;
  wire split_ifctrl_hs_inst_vld = 1'b0;
  wire if_inst_vld_for_ex_aft_hs = if_inst_vld_for_ex || split_ifctrl_hs_inst_vld;

  reg ex_inst_vld;
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
    $display("CYC %0d rst=%b ibctrl=%b cancel=%b stall=%b vld_for_ex=%b ex_vld=%b",
      tcyc, cpurst_b, ibuf_ifctrl_inst_vld, if_cancel_for_pipeline, iu_ifu_ex_stall,
      if_inst_vld_for_ex_aft_hs, ex_inst_vld);
  end

  initial begin
    cpurst_b = 0;
    #20 cpurst_b = 1;
    #5 ibuf_ifctrl_inst_vld = 1;        // valid instruction available
    #20 iu_ifu_ex_stall = 1;             // pipeline stalls
    #20 iu_ifu_ex_stall = 0;             // resume
    #20 if_cancel_for_pipeline = 1;      // cancel
    #10 if_cancel_for_pipeline = 0;
    #20 ibuf_ifctrl_inst_vld = 0;
    #20 $finish;
  end
endmodule
