// Part 6 synthetic: replay iverilog's per-cycle cr_iu_decd input trace from
// the actual E902 hello run, using a clock-driven state machine (no task
// wrapper) so this reproduces the real runtime conditions.
//
// Stimulus captured from /tmp/iv_e902.vvp via $strobe over the full
// hierarchical scope of x_cr_iu_decd. Hello-run cyc 14..30 covers the
// pipeline-startup window where the divergence occurs.
//
// Expected behavior: xezim and iverilog should produce identical
// per-cycle alu_sel/spec/expt_inv/etc. outputs. If they don't, the bug
// is in xezim's evaluation of cr_iu_decd as it sees the same inputs.
//
// Compile:
//   iverilog -g2012 -DSIMULATION=1 -I /tmp/xezim_e902_inc/incdir \
//     -s top -o /tmp/iv_p6 examples/e902_part6_decd_replay.v \
//     /tmp/xezim_e902_inc/cr_iu_decd.v && vvp -N /tmp/iv_p6
//   target/release/xezim --simulate -I /tmp/xezim_e902_inc/incdir \
//     -DSIMULATION=1 -s top examples/e902_part6_decd_replay.v \
//     /tmp/xezim_e902_inc/cr_iu_decd.v
`timescale 1ns/100ps
module top;
  reg clk = 0;
  always #5 clk = ~clk;

  reg [31:0] inst   = 32'h0;
  reg [30:0] cur_pc = 31'h0;
  reg [30:0] add_pc = 31'h0;

  reg [31:0] cyc = 0;
  always @(posedge clk) cyc <= cyc + 1;

  // Captured-from-iverilog trace of x_cr_iu_decd inputs by cyc.
  // hs_split_*, expt_*, prvlg, lsu_wfd, cskyisaee are constant 0
  // through this window; priv stays 11. Only inst / cur_pc / add_pc
  // vary, so just drive those.
  always @(posedge clk) begin
    case (cyc)
      32'd14: begin inst <= 32'h00622023; cur_pc <= 31'h00000012; add_pc <= 31'h00000014; end
      32'd15: begin inst <= 32'h02110191; cur_pc <= 31'h00000014; add_pc <= 31'h00000015; end
      32'd16: begin inst <= 32'h12f10211; cur_pc <= 31'h00000015; add_pc <= 31'h00000016; end
      32'd17: begin inst <= 32'h99e312f1; cur_pc <= 31'h00000016; add_pc <= 31'h00000017; end
      32'd18: begin inst <= 32'hfe0299e3; cur_pc <= 31'h00000017; add_pc <= 31'h00000010; end
      32'd19: begin inst <= 32'hfe021217; cur_pc <= 31'h00000010; add_pc <= 31'h00000012; end
      32'd20: begin inst <= 32'h0001a303; cur_pc <= 31'h00000010; add_pc <= 31'h00000012; end
      32'd21: begin inst <= 32'h0001a303; cur_pc <= 31'h00000010; add_pc <= 31'h00000012; end
      32'd22: begin inst <= 32'h00622023; cur_pc <= 31'h00000012; add_pc <= 31'h00000014; end
      32'd23: begin inst <= 32'h00622023; cur_pc <= 31'h00000012; add_pc <= 31'h00000014; end
      32'd24: begin inst <= 32'h02110191; cur_pc <= 31'h00000014; add_pc <= 31'h00000015; end
      32'd25: begin inst <= 32'h12f10211; cur_pc <= 31'h00000015; add_pc <= 31'h00000016; end
      32'd26: begin inst <= 32'h99e312f1; cur_pc <= 31'h00000016; add_pc <= 31'h00000017; end
      32'd27: begin inst <= 32'hfe0299e3; cur_pc <= 31'h00000017; add_pc <= 31'h00000010; end
      32'd28: begin inst <= 32'hfe021217; cur_pc <= 31'h00000010; add_pc <= 31'h00000012; end
      32'd29: begin cyc <= 30; $finish; end
      default: ;
    endcase
  end

  wire [255:0] sink;
  wire [31:0] alu_imm, branch_imm, lsu_imm, tval;
  wire [11:0] cp0_imm;
  wire [4:0] rd, rs1, rs2, rd2, rs1_2;
  wire [2:0] func3, alu_func;
  wire [3:0] alu_sub;
  wire alu_sel, branch_sel, cp0_sel, lsu_sel, mad_sel, special_sel;
  wire expt_inv, expt_bkpt, expt_ecall, expt_wsc;
  wire alu_dst_vld, alu_rs2_imm_vld, branch_auipc, inst_32bit;

  cr_iu_decd udut(
    .branch_pcgen_add_pc(add_pc), .cp0_iu_cskyisaee(1'b0), .cp0_yy_priv_mode(2'b11),
    .decd_alu_dst_vld(alu_dst_vld), .decd_alu_func(alu_func),
    .decd_alu_rs2_imm_vld(alu_rs2_imm_vld), .decd_alu_sub_func(alu_sub),
    .decd_branch_auipc(branch_auipc),
    .decd_branch_beq(sink[0]), .decd_branch_bge(sink[1]), .decd_branch_bgeu(sink[2]),
    .decd_branch_blt(sink[3]), .decd_branch_bltu(sink[4]), .decd_branch_bne(sink[5]),
    .decd_branch_cbeqz(sink[6]), .decd_branch_cbnez(sink[7]), .decd_branch_cj(sink[8]),
    .decd_branch_cjal(sink[9]), .decd_branch_cjalr(sink[10]), .decd_branch_cjr(sink[11]),
    .decd_branch_jal(sink[12]), .decd_branch_jalr(sink[13]),
    .decd_ctrl_alu_sel(alu_sel), .decd_ctrl_branch_sel(branch_sel),
    .decd_ctrl_cp0_sel(cp0_sel), .decd_ctrl_expt_bkpt(expt_bkpt),
    .decd_ctrl_expt_ecall(expt_ecall), .decd_ctrl_expt_inv(expt_inv),
    .decd_ctrl_expt_wsc(expt_wsc), .decd_ctrl_lsu_sel(lsu_sel),
    .decd_ctrl_mad_sel(mad_sel),
    .decd_mad_inst_div(sink[14]), .decd_mad_inst_divu(sink[15]),
    .decd_mad_inst_mul(sink[16]), .decd_mad_inst_mulh(sink[17]),
    .decd_mad_inst_mulhsu(sink[18]), .decd_mad_inst_mulhu(),
    .decd_mad_inst_rem(), .decd_mad_inst_remu(),
    .decd_oper_alu_imm(alu_imm), .decd_oper_branch_imm(branch_imm),
    .decd_oper_cp0_imm(cp0_imm), .decd_oper_lsu_imm(lsu_imm),
    .decd_retire_cp0_inst(), .decd_retire_inst_mret(),
    .decd_special_fencei(), .decd_special_icall(), .decd_special_icpa(),
    .decd_wb_tval(tval), .decd_xx_inst_32bit(inst_32bit),
    .decd_xx_unit_special_sel(special_sel),
    .hs_split_iu_ctrl_inst_vld(1'b0), .hs_split_iu_dp_inst_op(32'h0),
    .ifu_had_chg_flw_inst(), .ifu_had_match_pc(),
    .ifu_iu_ex_expt_cur(1'b0), .ifu_iu_ex_expt_vld(1'b0),
    .ifu_iu_ex_inst(inst), .ifu_iu_ex_inst_bkpt(1'b0),
    .ifu_iu_ex_prvlg_expt_vld(1'b0),
    .ifu_iu_ex_rd_reg(rd), .ifu_iu_ex_rs1_reg(rs1), .ifu_iu_ex_rs2_reg(rs2),
    .iu_cp0_ex_csrrc(), .iu_cp0_ex_csrrci(), .iu_cp0_ex_csrrs(),
    .iu_cp0_ex_csrrsi(), .iu_cp0_ex_csrrw(), .iu_cp0_ex_csrrwi(),
    .iu_cp0_ex_func3(func3), .iu_cp0_ex_mret(),
    .iu_cp0_ex_rd_reg(rd2), .iu_cp0_ex_rs1_reg(rs1_2), .iu_cp0_ex_wfi(),
    .iu_ifu_lsu_inst(sink[19]), .iu_lsu_ex_byte(sink[20]),
    .iu_lsu_ex_half(sink[21]), .iu_lsu_ex_store(sink[22]),
    .iu_lsu_ex_uns(sink[23]),
    .lsu_iu_wfd(1'b0), .pcgen_xx_cur_pc(cur_pc)
  );

  always @(posedge clk) begin
    if (cyc >= 15 && cyc <= 30) begin
      $strobe("REPLAY cyc=%0d inst=%h alu=%b spec=%b expt_inv=%b mad=%b lsu=%b cp0=%b br=%b 32bit=%b",
        cyc, inst, alu_sel, special_sel, expt_inv, mad_sel, lsu_sel, cp0_sel, branch_sel, inst_32bit);
      $strobe("PROBE cyc=%0d ill16=%b ill32=%b ill=%b dop=%b dfunc3=%b dinst=%h rd16=%b",
        cyc, udut.decd_ill_expt16, udut.decd_ill_expt32, udut.decd_ill_expt,
        udut.decd_op[4:0], udut.decd_func3[2:0], udut.decd_inst, udut.rd_16[4:0]);
    end
  end
endmodule
