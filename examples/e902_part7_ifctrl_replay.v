// Part 7 synthetic: replay iverilog's per-cycle cr_ifu_ifctrl input trace.
// Captured by $strobe over `tb.x_soc.....x_cr_ifu_top.x_ifctrl` during the
// hello-test pipeline-startup window (cyc 9..30).
//
// cr_ifu_ifctrl produces ifu_iu_ex_inst_vld — the very signal that diverges
// between iverilog and xezim in the full E902 hello run (xezim sees 0 where
// iverilog has 1 at cyc 13). If standalone driving with the same captured
// input trace matches iverilog, the bug is upstream (IBUF/IBUSIF). If it
// diverges, the bug is in xezim's eval of cr_ifu_ifctrl.
//
// Compile:
//   iverilog -g2012 -DSIMULATION=1 -I /tmp/xezim_e902_inc/incdir \
//     -s top -o /tmp/iv_p7 examples/e902_part7_ifctrl_replay.v \
//     /tmp/xezim_e902_inc/cr_ifu_ifctrl.v && vvp -N /tmp/iv_p7
//   xezim --simulate -I /tmp/xezim_e902_inc/incdir -DSIMULATION=1 -s top \
//     examples/e902_part7_ifctrl_replay.v /tmp/xezim_e902_inc/cr_ifu_ifctrl.v
`timescale 1ns/100ps
module top;
  reg cpuclk = 0;
  always #5 cpuclk = ~cpuclk;

  // Reset: drive low for 2 cyc to mimic the e902 boot sequence.
  reg cpurst_b = 1;
  initial begin
    #20 cpurst_b = 1'b0;   // assert reset (negedge at t=20, post posedge cyc1)
    #40 cpurst_b = 1'b1;   // deassert at t=60 (post posedge cyc4)
  end

  reg [31:0] cyc = 0;
  always @(posedge cpuclk or negedge cpurst_b) begin
    if (!cpurst_b) cyc <= 0; else cyc <= cyc + 1;
  end

  // All ifctrl input signals — driven by NBA on posedge cyc match,
  // mirroring iverilog's captured trace cyc-by-cyc.
  reg had_ifu_ir_vld           = 0;
  reg ibuf_ifctrl_inst_vld     = 0;
  reg ibuf_ifctrl_inst32_low   = 0;
  reg ibuf_ifctrl_pop0_mad32_low = 0;
  reg ibuf_ifdp_inst_dbg_disable = 0;
  reg ibuf_xx_empty            = 1;
  reg ibusif_ifctrl_inst_mad32_high = 0;
  reg ibusif_ifctrl_inst_no_bypass  = 0;
  reg ibusif_xx_16bit_inst      = 0;
  reg ibusif_xx_trans_cmplt     = 1;
  reg ibusif_xx_unalign_fetch   = 0;
  reg iu_ifu_ex_stall           = 0;
  reg iu_ifu_inst_fetch         = 0;
  reg iu_ifu_inst_fetch_without_dbg_disable = 0;
  reg iu_ifu_wb_stall           = 0;
  reg iu_yy_xx_dbgon            = 0;
  reg iu_yy_xx_flush            = 0;
  reg split_ifctrl_hs_stall     = 0;
  reg split_ifctrl_hs_stall_part = 0;

  // Stimulus from the iverilog $strobe capture (post-reset, cyc 9..30 of
  // the hello run). Each case sets the new values to be observed at the
  // NEXT posedge clk.
  always @(posedge cpuclk) begin
    case (cyc)
      32'd8: begin // observed at cyc 9
        ibuf_ifctrl_inst_vld<=0; ibuf_ifctrl_inst32_low<=0; ibuf_ifctrl_pop0_mad32_low<=0;
        ibuf_xx_empty<=1; ibusif_xx_16bit_inst<=0; ibusif_xx_trans_cmplt<=1;
        iu_ifu_ex_stall<=0;
      end
      32'd9: begin
        ibuf_ifctrl_pop0_mad32_low<=1;
      end
      32'd10: begin
        ibuf_ifctrl_pop0_mad32_low<=0;
      end
      32'd11: begin
        iu_ifu_ex_stall<=1;
      end
      32'd12: begin // observed at cyc 13
        ibuf_ifctrl_inst_vld<=1; ibuf_xx_empty<=0; ibusif_xx_16bit_inst<=1; iu_ifu_ex_stall<=0;
      end
      32'd13: begin
        ibusif_xx_trans_cmplt<=0; iu_ifu_ex_stall<=1;
      end
      32'd14: begin
        iu_ifu_ex_stall<=0;
      end
      32'd15: begin
        ibusif_xx_trans_cmplt<=1;
      end
      32'd18: begin
        iu_ifu_inst_fetch<=1; iu_ifu_inst_fetch_without_dbg_disable<=1;
      end
      32'd19: begin
        ibuf_ifctrl_inst_vld<=0; ibuf_xx_empty<=1; ibusif_xx_16bit_inst<=0;
        iu_ifu_inst_fetch<=0; iu_ifu_inst_fetch_without_dbg_disable<=0;
      end
      32'd20: begin
        iu_ifu_ex_stall<=1;
      end
      32'd21: begin
        ibuf_ifctrl_inst_vld<=1; ibuf_xx_empty<=0; ibusif_xx_16bit_inst<=1; iu_ifu_ex_stall<=0;
      end
      32'd22: begin
        ibusif_xx_trans_cmplt<=0; iu_ifu_ex_stall<=1;
      end
      32'd23: begin
        iu_ifu_ex_stall<=0;
      end
      32'd24: begin
        ibusif_xx_trans_cmplt<=1;
      end
      32'd27: begin
        iu_ifu_inst_fetch<=1; iu_ifu_inst_fetch_without_dbg_disable<=1;
      end
      32'd28: begin
        ibuf_ifctrl_inst_vld<=0; ibuf_xx_empty<=1; ibusif_xx_16bit_inst<=0;
        iu_ifu_inst_fetch<=0; iu_ifu_inst_fetch_without_dbg_disable<=0;
      end
      32'd29: begin
        iu_ifu_ex_stall<=1;
      end
      32'd30: begin
        $finish;
      end
      default: ;
    endcase
  end

  wire ifctrl_ibuf_bypass_vld;
  wire ifctrl_ibuf_inst_pipe_down;
  wire ifctrl_ibuf_pop_en;
  wire ifctrl_xx_ifcancel;
  wire ifu_iu_ex_inst_vld;
  wire ifu_iu_inst_buf_inst_dbg_disable;
  wire ifu_iu_inst_buf_inst_vld;

  cr_ifu_ifctrl udut(
    .cpuclk                                 (cpuclk),
    .cpurst_b                               (cpurst_b),
    .had_ifu_ir_vld                         (had_ifu_ir_vld),
    .ibuf_ifctrl_inst32_low                 (ibuf_ifctrl_inst32_low),
    .ibuf_ifctrl_inst_vld                   (ibuf_ifctrl_inst_vld),
    .ibuf_ifctrl_pop0_mad32_low             (ibuf_ifctrl_pop0_mad32_low),
    .ibuf_ifdp_inst_dbg_disable             (ibuf_ifdp_inst_dbg_disable),
    .ibuf_xx_empty                          (ibuf_xx_empty),
    .ibusif_ifctrl_inst_mad32_high          (ibusif_ifctrl_inst_mad32_high),
    .ibusif_ifctrl_inst_no_bypass           (ibusif_ifctrl_inst_no_bypass),
    .ibusif_xx_16bit_inst                   (ibusif_xx_16bit_inst),
    .ibusif_xx_trans_cmplt                  (ibusif_xx_trans_cmplt),
    .ibusif_xx_unalign_fetch                (ibusif_xx_unalign_fetch),
    .ifctrl_ibuf_bypass_vld                 (ifctrl_ibuf_bypass_vld),
    .ifctrl_ibuf_inst_pipe_down             (ifctrl_ibuf_inst_pipe_down),
    .ifctrl_ibuf_pop_en                     (ifctrl_ibuf_pop_en),
    .ifctrl_xx_ifcancel                     (ifctrl_xx_ifcancel),
    .ifu_iu_ex_inst_vld                     (ifu_iu_ex_inst_vld),
    .ifu_iu_inst_buf_inst_dbg_disable       (ifu_iu_inst_buf_inst_dbg_disable),
    .ifu_iu_inst_buf_inst_vld               (ifu_iu_inst_buf_inst_vld),
    .iu_ifu_ex_stall                        (iu_ifu_ex_stall),
    .iu_ifu_inst_fetch                      (iu_ifu_inst_fetch),
    .iu_ifu_inst_fetch_without_dbg_disable  (iu_ifu_inst_fetch_without_dbg_disable),
    .iu_ifu_wb_stall                        (iu_ifu_wb_stall),
    .iu_yy_xx_dbgon                         (iu_yy_xx_dbgon),
    .iu_yy_xx_flush                         (iu_yy_xx_flush),
    .split_ifctrl_hs_stall                  (split_ifctrl_hs_stall),
    .split_ifctrl_hs_stall_part             (split_ifctrl_hs_stall_part)
  );

  always @(posedge cpuclk) begin
    if (cyc >= 9 && cyc <= 30) begin
      $strobe("REPLAY cyc=%0d ibuf_vld=%b ex_stall=%b ifetch=%b flush=%b | OUT pipedwn=%b popen=%b ifcancel=%b ex_inst_vld=%b inst_buf_vld=%b",
        cyc, ibuf_ifctrl_inst_vld, iu_ifu_ex_stall, iu_ifu_inst_fetch, iu_yy_xx_flush,
        ifctrl_ibuf_inst_pipe_down, ifctrl_ibuf_pop_en, ifctrl_xx_ifcancel,
        ifu_iu_ex_inst_vld, ifu_iu_inst_buf_inst_vld);
    end
  end
endmodule
