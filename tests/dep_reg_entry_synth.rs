//! Synthetic test for c910 `ct_idu_dep_reg_entry` — the per-source
//! dependency tracker whose `x_read_rdy_for_issue` output (bit 9 of
//! x_read_data) feeds the AIQ0 entry-ready logic. The c910 memcpy bug
//! manifests as: after a producer instruction has retired, a new
//! consumer is allocated into AIQ0 entry 2; in a reference simulator the entry's
//! src0_dep_reg signals rdy_for_issue=1 immediately (or one cycle
//! later via wb→wake_up→rdy_update). In xezim it stays 0.
//!
//! This test extracts the SAME logic and exercises it standalone with
//! the exact stimulus pattern (allocate with x_create_wb=1, x_create_rdy=0,
//! then no further input changes — only the wb flop should wake the
//! rdy flop on the next clock).
//!
//! NOTE: we cannot include the full real c910 ct_idu_dep_reg_entry.v
//! because it depends on `gated_clk_cell` (a vendor-specific glue
//! module). We REPLICATE the logic-equivalent always blocks/asserts
//! verbatim from the real file's lines 247-372.

use xezim::simulate;

fn lookup_one_of(sim: &xezim::compiler::Simulator, names: &[&str]) -> xezim_core::value::Value {
    for n in names {
        if let Some(v) = sim.get_signal(n) {
            return v.clone();
        }
    }
    panic!("none of these signal names found: {:?}", names);
}

const DEP_REG_SHAPE: &str = r#"
module tb;
  // ===== clocks / reset =====
  reg clk = 0;
  reg rst_b = 0;
  always #5 clk = ~clk;

  initial begin
    #1 rst_b = 0;
    #20 rst_b = 1;
  end

  // ===== dep_reg_entry-equivalent module body, inlined =====
  // Lines 224-372 of ct_idu_dep_reg_entry.v transcribed (sans gated clock cells).

  // Stimulus inputs:
  reg          x_write_en;
  reg          x_rdy_clr;
  reg  [9:0]   x_create_data;
  reg          alu0_reg_fwd_vld;
  reg          alu1_reg_fwd_vld;
  reg          ctrl_xx_rf_pipe0_preg_lch_vld_dupx;
  reg          ctrl_xx_rf_pipe1_preg_lch_vld_dupx;
  reg  [6:0]   dp_xx_rf_pipe0_dst_preg_dupx;
  reg  [6:0]   dp_xx_rf_pipe1_dst_preg_dupx;
  reg          iu_idu_div_inst_vld;
  reg  [6:0]   iu_idu_div_preg_dupx;
  reg  [6:0]   iu_idu_ex2_pipe0_wb_preg_dupx;
  reg          iu_idu_ex2_pipe0_wb_preg_vld_dupx;
  reg          iu_idu_ex2_pipe1_mult_inst_vld_dupx;
  reg  [6:0]   iu_idu_ex2_pipe1_preg_dupx;
  reg  [6:0]   iu_idu_ex2_pipe1_wb_preg_dupx;
  reg          iu_idu_ex2_pipe1_wb_preg_vld_dupx;
  reg          lsu_idu_ag_pipe3_load_inst_vld;
  reg  [6:0]   lsu_idu_ag_pipe3_preg_dupx;
  reg          lsu_idu_dc_pipe3_load_fwd_inst_vld_dupx;
  reg          lsu_idu_dc_pipe3_load_inst_vld_dupx;
  reg  [6:0]   lsu_idu_dc_pipe3_preg_dupx;
  reg  [6:0]   lsu_idu_wb_pipe3_wb_preg_dupx;
  reg          lsu_idu_wb_pipe3_wb_preg_vld_dupx;
  reg          rtu_idu_flush_fe;
  reg          rtu_idu_flush_is;
  reg          vfpu_idu_ex1_pipe6_mfvr_inst_vld_dupx;
  reg  [6:0]   vfpu_idu_ex1_pipe6_preg_dupx;
  reg          vfpu_idu_ex1_pipe7_mfvr_inst_vld_dupx;
  reg  [6:0]   vfpu_idu_ex1_pipe7_preg_dupx;

  // State:
  reg          lsu_match;
  reg  [6:0]   preg;
  reg          rdy;
  reg          wb;

  // Decoded create-data
  wire         x_create_lsu_match = x_create_data[9];
  wire [6:0]   x_create_preg      = x_create_data[8:2];
  wire         x_create_wb        = x_create_data[1];
  wire         x_create_rdy       = x_create_data[0];

  // data_ready signals (per-pipe wakeup)
  wire alu0_data_ready  = ctrl_xx_rf_pipe0_preg_lch_vld_dupx
                          && (dp_xx_rf_pipe0_dst_preg_dupx == preg);
  wire alu1_data_ready  = ctrl_xx_rf_pipe1_preg_lch_vld_dupx
                          && (dp_xx_rf_pipe1_dst_preg_dupx == preg);
  wire mult_data_ready  = iu_idu_ex2_pipe1_mult_inst_vld_dupx
                          && (iu_idu_ex2_pipe1_preg_dupx == preg);
  wire div_data_ready   = iu_idu_div_inst_vld
                          && (iu_idu_div_preg_dupx == preg);
  wire load_data_ready  = lsu_idu_dc_pipe3_load_inst_vld_dupx
                          && (lsu_idu_dc_pipe3_preg_dupx == preg);
  wire vfpu0_data_ready = vfpu_idu_ex1_pipe6_mfvr_inst_vld_dupx
                          && (vfpu_idu_ex1_pipe6_preg_dupx == preg);
  wire vfpu1_data_ready = vfpu_idu_ex1_pipe7_mfvr_inst_vld_dupx
                          && (vfpu_idu_ex1_pipe7_preg_dupx == preg);

  wire alu0_issue_data_ready = alu0_reg_fwd_vld;
  wire alu1_issue_data_ready = alu1_reg_fwd_vld;
  wire load_issue_data_ready = lsu_idu_dc_pipe3_load_fwd_inst_vld_dupx && lsu_match;

  wire data_ready = alu0_data_ready || alu1_data_ready || mult_data_ready
                     || div_data_ready || load_data_ready
                     || vfpu0_data_ready || vfpu1_data_ready;
  wire wake_up    = wb;
  wire rdy_clear  = x_rdy_clr;
  wire rdy_update = (rdy || data_ready || wake_up) && !rdy_clear;

  wire x_read_rdy           = rdy_update;
  wire x_read_rdy_for_issue = rdy || alu0_issue_data_ready
                                  || alu1_issue_data_ready
                                  || load_issue_data_ready;
  wire x_read_rdy_for_bypass = rdy;

  always @(posedge clk or negedge rst_b)
    if (!rst_b)               rdy <= 1'b1;
    else if (rtu_idu_flush_fe || rtu_idu_flush_is) rdy <= 1'b1;
    else if (x_write_en)      rdy <= x_create_rdy;
    else                      rdy <= rdy_update;

  wire lsu_match_update = lsu_idu_ag_pipe3_load_inst_vld
                          && (lsu_idu_ag_pipe3_preg_dupx == preg);

  always @(posedge clk or negedge rst_b)
    if (!rst_b)               lsu_match <= 1'b0;
    else if (rtu_idu_flush_fe || rtu_idu_flush_is) lsu_match <= 1'b0;
    else if (x_write_en)      lsu_match <= x_create_lsu_match;
    else                      lsu_match <= lsu_match_update;

  wire pipe0_wb = iu_idu_ex2_pipe0_wb_preg_vld_dupx
                   && (iu_idu_ex2_pipe0_wb_preg_dupx == preg);
  wire pipe1_wb = iu_idu_ex2_pipe1_wb_preg_vld_dupx
                   && (iu_idu_ex2_pipe1_wb_preg_dupx == preg);
  wire pipe3_wb = lsu_idu_wb_pipe3_wb_preg_vld_dupx
                   && (lsu_idu_wb_pipe3_wb_preg_dupx == preg);
  wire write_back = wb || pipe0_wb || pipe1_wb || pipe3_wb;
  wire wb_update  = wb || write_back;
  wire x_read_wb  = wb_update;

  always @(posedge clk or negedge rst_b)
    if (!rst_b)               wb <= 1'b1;
    else if (rtu_idu_flush_fe || rtu_idu_flush_is) wb <= 1'b1;
    else if (x_write_en)      wb <= x_create_wb;
    else                      wb <= wb_update;

  always @(posedge clk or negedge rst_b)
    if (!rst_b)               preg <= 7'b0;
    else if (x_write_en)      preg <= x_create_preg;
    else                      preg <= preg;

  // ===== Test pattern =====
  // Scenario: simulate the c910 allocation that should yield
  // rdy_for_issue=1 within 2 cycles.
  //   x_create_data = {lsu_match=0, preg=0x42, wb=1, rdy=0} = 10'b0_0100_0010_10
  initial begin
    x_write_en = 0; x_rdy_clr = 0; x_create_data = 10'b0;
    alu0_reg_fwd_vld = 0; alu1_reg_fwd_vld = 0;
    ctrl_xx_rf_pipe0_preg_lch_vld_dupx = 0; ctrl_xx_rf_pipe1_preg_lch_vld_dupx = 0;
    dp_xx_rf_pipe0_dst_preg_dupx = 0; dp_xx_rf_pipe1_dst_preg_dupx = 0;
    iu_idu_div_inst_vld = 0; iu_idu_div_preg_dupx = 0;
    iu_idu_ex2_pipe0_wb_preg_dupx = 0; iu_idu_ex2_pipe0_wb_preg_vld_dupx = 0;
    iu_idu_ex2_pipe1_mult_inst_vld_dupx = 0;
    iu_idu_ex2_pipe1_preg_dupx = 0;
    iu_idu_ex2_pipe1_wb_preg_dupx = 0; iu_idu_ex2_pipe1_wb_preg_vld_dupx = 0;
    lsu_idu_ag_pipe3_load_inst_vld = 0; lsu_idu_ag_pipe3_preg_dupx = 0;
    lsu_idu_dc_pipe3_load_fwd_inst_vld_dupx = 0;
    lsu_idu_dc_pipe3_load_inst_vld_dupx = 0;
    lsu_idu_dc_pipe3_preg_dupx = 0;
    lsu_idu_wb_pipe3_wb_preg_dupx = 0; lsu_idu_wb_pipe3_wb_preg_vld_dupx = 0;
    rtu_idu_flush_fe = 0; rtu_idu_flush_is = 0;
    vfpu_idu_ex1_pipe6_mfvr_inst_vld_dupx = 0; vfpu_idu_ex1_pipe6_preg_dupx = 0;
    vfpu_idu_ex1_pipe7_mfvr_inst_vld_dupx = 0; vfpu_idu_ex1_pipe7_preg_dupx = 0;

    @(posedge rst_b);
    @(posedge clk);
    // T0: allocate entry with wb=1 rdy=0 preg=0x42
    x_write_en    = 1;
    x_create_data = {1'b0, 7'h42, 1'b1, 1'b0};   // wb=1, rdy=0
    @(posedge clk);
    // T1: deassert write_en
    x_write_en    = 0;
    x_create_data = 10'b0;
    @(posedge clk);
    // T2: rdy should now be 1 (wake_up via wb)
    @(posedge clk);
    $finish;
  end
endmodule
"#;

#[test]
fn dep_reg_entry_wb_wakes_rdy() {
    let sim = simulate(DEP_REG_SHAPE, 1000).expect("simulate failed");
    let rdy = lookup_one_of(&sim, &["tb.rdy", "rdy"]);
    let wb  = lookup_one_of(&sim, &["tb.wb", "wb"]);
    let rdy_for_issue = lookup_one_of(&sim, &["tb.x_read_rdy_for_issue", "x_read_rdy_for_issue"]);

    let rdy_v = rdy.to_u64().expect("rdy") & 1;
    let wb_v  = wb.to_u64().expect("wb") & 1;
    let rdy_for_issue_v = rdy_for_issue.to_u64().expect("rdy_for_issue") & 1;

    assert_eq!(wb_v, 1, "wb flop should hold 1 after allocation with x_create_wb=1");
    assert_eq!(rdy_v, 1, "rdy flop should be 1 after wb→wake_up→rdy_update path");
    assert_eq!(rdy_for_issue_v, 1, "x_read_rdy_for_issue must be 1 (rdy=1 OR forwards)");
}
