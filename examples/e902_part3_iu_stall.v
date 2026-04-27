// Part 3 synthetic: IU control + retire path leading to iu_pad_inst_retire.
// Mirrors cr_iu_ctrl.v + cr_iu_retire.v + cr_iu_rbus.v patterns. The
// residual E902 stall: iu_ifu_ex_stall stays 1 in xezim past cyc 24 while
// iverilog has it 0, freezing the pipeline.
//
// Test discipline (same as part 2):
//   - All regs initialised explicitly (no X starting state at t=0).
//   - Reset is exercised through an explicit transition.
//   - Stimulus changes on full-cycle (`#10`) boundaries so they don't
//     collide with the active posedge.
//   - Sampling via `$strobe` so NBA application order can't perturb
//     the trace ordering.
`timescale 1ns/100ps
module top;
  reg clk = 0;
  reg cpurst_b = 1;
  always #5 clk = ~clk;

  reg ifu_iu_ex_inst_vld = 0;
  reg decd_xx_unit_special_sel = 1;
  reg ifu_iu_ex_rand_vld = 0;
  reg [3:0] alu_busy = 0;
  reg [3:0] mad_busy = 0;
  reg [3:0] lsu_busy = 0;
  reg special_stall_in = 0;

  wire hs_split_iu_ctrl_inst_vld = 1'b0;
  wire ifu_iu_ex_hs_split_inst_vld = ifu_iu_ex_inst_vld || hs_split_iu_ctrl_inst_vld;
  wire ctrl_internal_stall_raw = |alu_busy || |mad_busy || |lsu_busy;
  wire ctrl_internal_stall = ifu_iu_ex_hs_split_inst_vld && ctrl_internal_stall_raw;
  wire ctrl_ex_inst_vld = ifu_iu_ex_hs_split_inst_vld && !ifu_iu_ex_rand_vld
                       && !ctrl_internal_stall;
  wire ctrl_special_ex_sel = ctrl_ex_inst_vld && decd_xx_unit_special_sel;
  wire special_rbus_req = ctrl_special_ex_sel && !special_stall_in;
  wire alu_rbus_req = 1'b0, mad_rbus_req = 1'b0, lsu_iu_req = 1'b0;
  wire cp0_iu_req = 1'b0, branch_rbus_req = 1'b0, bctm_rbus_req = 1'b0, prgsign_rbus_req = 1'b0;
  wire rbus_cmplt = alu_rbus_req || mad_rbus_req || lsu_iu_req || special_rbus_req
                 || cp0_iu_req || branch_rbus_req || bctm_rbus_req || prgsign_rbus_req;
  wire iu_yy_xx_retire = rbus_cmplt;
  wire retire_split_inst_with_dbg_ack = 1'b0;
  wire iu_pad_inst_retire = iu_yy_xx_retire && !retire_split_inst_with_dbg_ack;
  wire iu_ifu_ex_stall = ctrl_internal_stall;

  reg [31:0] tcyc = 0;
  reg [31:0] retire_count = 0;
  always @(posedge clk) begin
    tcyc <= tcyc + 1;
    if (iu_pad_inst_retire) retire_count <= retire_count + 1;
    $strobe("CYC %0d ifu_vld=%b stall=%b internal_stall=%b retire=%b retire_total=%0d",
      tcyc, ifu_iu_ex_inst_vld, iu_ifu_ex_stall, ctrl_internal_stall,
      iu_pad_inst_retire, retire_count);
  end

  initial begin
    // All stimulus changes happen at NEGEDGE clk times (t=10, 20, 30 ...)
    // so they don't race with the active posedge in the same slot.
    #20;                                   // t=20  negedge cyc 1
    cpurst_b = 1'b0;                       // observed at cyc 2 posedge
    #20;                                   // t=40
    cpurst_b = 1'b1;                       // observed at cyc 4
    #20;                                   // t=60  negedge cyc 5
    ifu_iu_ex_inst_vld = 1'b1;             // observed at cyc 6
    #30;                                   // t=90  negedge cyc 8
    alu_busy = 4'b1000;                    // observed at cyc 9 (stall)
    #20;                                   // t=110 negedge cyc 10
    alu_busy = 0;                          // observed at cyc 11
    #20;                                   // t=130 negedge cyc 12
    mad_busy = 4'b0001;                    // observed at cyc 13
    #20;                                   // t=150 negedge cyc 14
    mad_busy = 0;                          // observed at cyc 15
    #20;                                   // t=170 negedge cyc 16
    ifu_iu_ex_inst_vld = 1'b0;             // observed at cyc 17
    #20 $finish;
  end
endmodule
