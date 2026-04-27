// Part 4 synthetic: drives the REAL cr_iu_decd (cr_iu_decd.v from the
// E902 source tree) directly to isolate whether xezim's decoder
// evaluation diverges from iverilog's on a known instruction word.
//
// Background: at cyc 16 of the full E902 hello run, xezim's IFU feeds
// the decoder `ifu_iu_ex_inst = 0x20000217` (a valid AUIPC). xezim's
// cr_iu_decd then asserts decd_xx_unit_special_sel=1 and
// decd_ctrl_expt_inv=1 — flagging a perfectly valid AUIPC as illegal.
// If iverilog's cr_iu_decd produces decd_ctrl_alu_sel=1 / expt_inv=0
// for the same input, the bug is in xezim's eval of cr_iu_decd.
//
// To run this test, compile cr_iu_decd.v + cpu_cfig.h alongside
// this file:
//   iverilog -g2012 -I /tmp/xezim_e902_inc/incdir \
//            -DSIMULATION=1 -s top -o /tmp/iv_p4 \
//            examples/e902_part4_decd_probe.v \
//            /tmp/xezim_e902_inc/cr_iu_decd.v \
//            /tmp/xezim_e902_inc/cpu_cfig.h    && vvp -N /tmp/iv_p4
//   xezim --simulate -I /tmp/xezim_e902_inc/incdir \
//         -DSIMULATION=1 -s top \
//         examples/e902_part4_decd_probe.v \
//         /tmp/xezim_e902_inc/cr_iu_decd.v \
//         /tmp/xezim_e902_inc/cpu_cfig.h
`timescale 1ns/100ps
module top;
  reg [31:0] inst = 32'h00000013;          // boot value: NOP
  reg [30:0] cur_pc = 31'h0;
  reg [30:0] branch_add_pc = 31'h0;
  reg [31:0] hs_split_inst_op = 32'h0;
  reg        hs_split_inst_vld = 1'b0;
  reg        expt_cur = 1'b0;
  reg        expt_vld = 1'b0;
  reg        inst_bkpt = 1'b0;
  reg        prvlg_expt_vld = 1'b0;
  reg        lsu_wfd = 1'b0;
  reg        cskyisaee = 1'b0;
  reg [1:0]  priv_mode = 2'b11;            // M-mode

  wire alu_sel, branch_sel, cp0_sel, lsu_sel, mad_sel, special_sel;
  wire expt_inv, expt_bkpt, expt_ecall, expt_wsc;
  wire alu_dst_vld, alu_rs2_imm_vld;
  wire [2:0] alu_func;
  wire [3:0] alu_sub_func;
  wire branch_auipc;
  wire [31:0] alu_imm, branch_imm, cp0_imm, lsu_imm;
  wire [31:0] tval;
  wire inst_32bit;
  // ... lots of unused outputs; tie via wires.
  wire [31:0] dummy32a, dummy32b, dummy32c, dummy32d;
  wire [4:0] dummy_rd, dummy_rs1, dummy_rs2, dummy_rd2, dummy_rs1_2;
  wire [2:0] dummy_func3;
  wire dummy_a, dummy_b, dummy_c, dummy_d, dummy_e, dummy_f, dummy_g, dummy_h, dummy_i, dummy_j, dummy_k, dummy_l, dummy_m, dummy_n, dummy_o, dummy_p, dummy_q, dummy_r, dummy_s;
  wire dummy_lsu_inst, dummy_lsu_byte, dummy_lsu_half, dummy_lsu_store, dummy_lsu_uns;

  cr_iu_decd udut(
    .branch_pcgen_add_pc      (branch_add_pc),
    .cp0_iu_cskyisaee         (cskyisaee),
    .cp0_yy_priv_mode         (priv_mode),
    .decd_alu_dst_vld         (alu_dst_vld),
    .decd_alu_func            (alu_func),
    .decd_alu_rs2_imm_vld     (alu_rs2_imm_vld),
    .decd_alu_sub_func        (alu_sub_func),
    .decd_branch_auipc        (branch_auipc),
    .decd_branch_beq          (dummy_a),
    .decd_branch_bge          (dummy_b),
    .decd_branch_bgeu         (dummy_c),
    .decd_branch_blt          (dummy_d),
    .decd_branch_bltu         (dummy_e),
    .decd_branch_bne          (dummy_f),
    .decd_branch_cbeqz        (dummy_g),
    .decd_branch_cbnez        (dummy_h),
    .decd_branch_cj           (dummy_i),
    .decd_branch_cjal         (dummy_j),
    .decd_branch_cjalr        (dummy_k),
    .decd_branch_cjr          (dummy_l),
    .decd_branch_jal          (dummy_m),
    .decd_branch_jalr         (dummy_n),
    .decd_ctrl_alu_sel        (alu_sel),
    .decd_ctrl_branch_sel     (branch_sel),
    .decd_ctrl_cp0_sel        (cp0_sel),
    .decd_ctrl_expt_bkpt      (expt_bkpt),
    .decd_ctrl_expt_ecall     (expt_ecall),
    .decd_ctrl_expt_inv       (expt_inv),
    .decd_ctrl_expt_wsc       (expt_wsc),
    .decd_ctrl_lsu_sel        (lsu_sel),
    .decd_ctrl_mad_sel        (mad_sel),
    .decd_mad_inst_div        (dummy_o),
    .decd_mad_inst_divu       (dummy_p),
    .decd_mad_inst_mul        (dummy_q),
    .decd_mad_inst_mulh       (dummy_r),
    .decd_mad_inst_mulhsu     (dummy_s),
    .decd_mad_inst_mulhu      (),
    .decd_mad_inst_rem        (),
    .decd_mad_inst_remu       (),
    .decd_oper_alu_imm        (alu_imm),
    .decd_oper_branch_imm     (branch_imm),
    .decd_oper_cp0_imm        (cp0_imm),
    .decd_oper_lsu_imm        (lsu_imm),
    .decd_retire_cp0_inst     (),
    .decd_retire_inst_mret    (),
    .decd_special_fencei      (),
    .decd_special_icall       (),
    .decd_special_icpa        (),
    .decd_wb_tval             (tval),
    .decd_xx_inst_32bit       (inst_32bit),
    .decd_xx_unit_special_sel (special_sel),
    .hs_split_iu_ctrl_inst_vld(hs_split_inst_vld),
    .hs_split_iu_dp_inst_op   (hs_split_inst_op),
    .ifu_had_chg_flw_inst     (),
    .ifu_had_match_pc         (),
    .ifu_iu_ex_expt_cur       (expt_cur),
    .ifu_iu_ex_expt_vld       (expt_vld),
    .ifu_iu_ex_inst           (inst),
    .ifu_iu_ex_inst_bkpt      (inst_bkpt),
    .ifu_iu_ex_prvlg_expt_vld (prvlg_expt_vld),
    .ifu_iu_ex_rd_reg         (dummy_rd),
    .ifu_iu_ex_rs1_reg        (dummy_rs1),
    .ifu_iu_ex_rs2_reg        (dummy_rs2),
    .iu_cp0_ex_csrrc          (),
    .iu_cp0_ex_csrrci         (),
    .iu_cp0_ex_csrrs          (),
    .iu_cp0_ex_csrrsi         (),
    .iu_cp0_ex_csrrw          (),
    .iu_cp0_ex_csrrwi         (),
    .iu_cp0_ex_func3          (dummy_func3),
    .iu_cp0_ex_mret           (),
    .iu_cp0_ex_rd_reg         (dummy_rd2),
    .iu_cp0_ex_rs1_reg        (dummy_rs1_2),
    .iu_cp0_ex_wfi            (),
    .iu_ifu_lsu_inst          (dummy_lsu_inst),
    .iu_lsu_ex_byte           (dummy_lsu_byte),
    .iu_lsu_ex_half           (dummy_lsu_half),
    .iu_lsu_ex_store          (dummy_lsu_store),
    .iu_lsu_ex_uns            (dummy_lsu_uns),
    .lsu_iu_wfd               (lsu_wfd),
    .pcgen_xx_cur_pc          (cur_pc)
  );

  task probe(input [31:0] insn, input [255:0] label);
    begin
      inst = insn;
      #1;
      $display("PROBE %0s inst=%h alu=%b mad=%b lsu=%b cp0=%b br=%b spec=%b expt_inv=%b expt_bkpt=%b expt_ecall=%b expt_wsc=%b alu_func=%b alu_sub_func=%b alu_imm=%h",
               label, insn, alu_sel, mad_sel, lsu_sel, cp0_sel, branch_sel, special_sel,
               expt_inv, expt_bkpt, expt_ecall, expt_wsc, alu_func, alu_sub_func, alu_imm);
    end
  endtask

  initial begin
    #1;
    probe(32'h20000217, "AUIPC_x4_0x20000");        // the xezim cyc-16 input
    probe(32'h02110191, "iCOMPILE_cyc16_word");        // the iverilog cyc-16 input
    probe(32'h00000013, "ADDI_x0_x0_0_NOP");        // canonical NOP
    probe(32'h00000033, "ADD_x0_x0_x0");            // R-type add
    probe(32'h0001a303, "LW_x6_0_x3");              // iCOMPILE cyc-21 word
    probe(32'h00622023, "SW_x6_0_x4");              // iCOMPILE cyc-23 word
    probe(32'hfe0299e3, "BNE_neg20");               // iCOMPILE cyc-19 word (branch)
    probe(32'h12f10211, "iCOMPILE_cyc17_word");
    probe(32'h99e312f1, "iCOMPILE_cyc18_word");
    probe(32'hfe021217, "iCOMPILE_cyc20_word");
    $finish;
  end
endmodule
