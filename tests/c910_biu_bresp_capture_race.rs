//! Cone-of-influence test for the c910 BIU `cur_bresp_buf_bvalid` capture
//! race localized in round 34 of the c910 memcpy investigation. See
//! docs/c910_memcpy_investigation.md.
//!
//! VCD diff vs a reference simulator shows:
//!  - `pad_biu_bvalid` (raw AXI slave bvalid): 4 pulses fire identically
//!    in xezim and a reference simulator through sim 47695.
//!  - `cur_bresp_buf_bvalid` (registered in BIU at ct_biu_write_channel.v:975
//!    on `posedge bcpuclk`): a reference simulator captures all 4; xezim only captures
//!    the first 3. The 4th pad_biu_bvalid at sim 47695 is missed.
//!
//! `bcpuclk` is gated from `coreclk` via a passthrough `gated_clk_cell`
//! (assign clk_out = clk_in). `back_full = back_valid && back_pending`,
//! where both FFs are clocked on `coreclk` and `pad_biu_back_ready=1'b1`
//! is tied high, forcing both to clear every cycle.
//!
//! This synthetic test reproduces the exact structural pattern:
//!  - shared coreclk drives both bcpuclk (via passthrough gate) and the
//!    back_valid/back_pending FFs
//!  - cur_bresp_buf_bvalid FF clocked on bcpuclk
//!  - 8 pad_biu_bvalid pulses fed in
//!  - check that all 8 are captured
//!
//! If xezim treats bcpuclk and coreclk as distinct clock nets, an NBA
//! ordering race between back_valid/back_pending update and back_full
//! read may cause the FF to spuriously see back_full=1 in the cycle
//! that pad_biu_bvalid first arrives, missing the capture.

use xezim::simulate;

const SRC: &str = r#"
`timescale 1ns/100ps

module gated_clk_cell(
  input  clk_in,
  input  global_en,
  input  module_en,
  input  local_en,
  input  external_en,
  input  pad_yy_icg_scan_en,
  output clk_out
);
  assign clk_out = clk_in;
endmodule

module dut(
  input  coreclk,
  input  cpurst_b,
  input  pad_biu_bvalid,
  output reg cur_bresp_buf_bvalid,
  output     biu_lsu_b_vld
);
  wire bcpuclk;
  reg  back_valid;
  reg  back_pending;
  wire back_full;
  wire blast_done;
  // Tie-high like ct_biu_top.v:1188
  wire pad_biu_back_ready = 1'b1;

  // gated_clk_cell passthrough — same shape as c910's BIU
  gated_clk_cell x_b_gate (
    .clk_in            (coreclk),
    .global_en         (1'b1),
    .module_en         (1'b1),
    .local_en          (1'b1),
    .external_en       (1'b0),
    .pad_yy_icg_scan_en(1'b0),
    .clk_out           (bcpuclk)
  );

  assign back_full   = back_valid && back_pending;
  assign blast_done  = cur_bresp_buf_bvalid && !back_full;
  assign biu_lsu_b_vld = cur_bresp_buf_bvalid && !back_full;

  // The bug-line FF (ct_biu_write_channel.v:972-980)
  always @(posedge bcpuclk or negedge cpurst_b) begin
    if(~cpurst_b)
      cur_bresp_buf_bvalid <= 1'b0;
    else if(pad_biu_bvalid && !back_full)
      cur_bresp_buf_bvalid <= 1'b1;
    else if(!back_full)
      cur_bresp_buf_bvalid <= 1'b0;
  end

  // back_valid/back_pending FFs clocked on coreclk (ct_biu_write_channel.v:1009)
  always @(posedge coreclk or negedge cpurst_b) begin
    if(~cpurst_b)              back_valid <= 1'b0;
    else if(blast_done || back_pending) back_valid <= 1'b1;
    else if(pad_biu_back_ready) back_valid <= 1'b0;
  end

  always @(posedge coreclk or negedge cpurst_b) begin
    if(~cpurst_b)              back_pending <= 1'b0;
    else if(pad_biu_back_ready) back_pending <= 1'b0;
    else if(blast_done && back_valid) back_pending <= 1'b1;
  end
endmodule

module tb;
  reg coreclk = 0;
  reg cpurst_b = 0;
  reg pad_biu_bvalid = 0;
  wire cur_bresp_buf_bvalid;
  wire biu_lsu_b_vld;
  integer capture_count;

  dut u_dut (
    .coreclk             (coreclk),
    .cpurst_b            (cpurst_b),
    .pad_biu_bvalid      (pad_biu_bvalid),
    .cur_bresp_buf_bvalid(cur_bresp_buf_bvalid),
    .biu_lsu_b_vld       (biu_lsu_b_vld)
  );

  always #5 coreclk = ~coreclk;

  // Count every time cur_bresp_buf_bvalid rises.
  always @(posedge cur_bresp_buf_bvalid) begin
    capture_count = capture_count + 1;
  end

  initial begin
    capture_count = 0;
    cpurst_b = 0;
    pad_biu_bvalid = 0;
    #20; cpurst_b = 1;
    @(posedge coreclk);

    // 8 consecutive pulses, each at posedge clk for 1 cycle (the c910
    // shape: pad_biu_bvalid pulses high for ~10ns synchronized to
    // coreclk edges).
    repeat (8) begin
      @(posedge coreclk);
      #1; pad_biu_bvalid = 1;
      @(posedge coreclk);
      #1; pad_biu_bvalid = 0;
      @(posedge coreclk);
    end

    #50;
    $finish;
  end
endmodule
"#;

fn lookup(sim: &xezim::compiler::Simulator, name: &str) -> u64 {
    sim.get_signal(name)
        .or_else(|| sim.get_signal(&format!("tb.{}", name)))
        .unwrap_or_else(|| panic!("signal not found: {}", name))
        .to_u64()
        .unwrap_or_else(|| panic!("signal {} not u64-able", name))
}

#[test]
fn all_8_bvalid_pulses_are_captured() {
    let sim = simulate(SRC, 300).expect("simulate failed");
    let count = lookup(&sim, "capture_count") & 0xFFFFFFFF;
    assert_eq!(
        count, 8,
        "Expected 8 cur_bresp_buf_bvalid captures (one per pad_biu_bvalid pulse), got {}",
        count
    );
}

// More c910-realistic shape: nested hierarchy with pad_biu_bvalid driven
// from a slave-side FSM through multiple wire-layers (slave -> interconnect
// -> cpu_top -> biu_top -> write_channel), mirroring how the real bvalid
// propagates from axi_slave128.v to ct_biu_write_channel.v.
const SRC_NESTED: &str = r#"
`timescale 1ns/100ps

module gated_clk_cell(
  input  clk_in,
  input  global_en, module_en, local_en, external_en, pad_yy_icg_scan_en,
  output clk_out
);
  assign clk_out = clk_in;
endmodule

// Slave FSM mirroring axi_slave128.v's WRITE→WRITE_RESP transition
module slave_axi(
  input  coreclk,
  input  cpurst_b,
  input  trigger_write,   // pulse to start a write transaction
  output reg bvalid_pad   // analogous to pad_biu_bvalid
);
  reg [1:0] cur_state;
  reg [1:0] next_state;
  parameter IDLE = 2'b00, WRITE = 2'b01, WRITE_RESP = 2'b10;

  always @(posedge coreclk or negedge cpurst_b) begin
    if(~cpurst_b) cur_state <= IDLE;
    else          cur_state <= next_state;
  end

  always @(*) begin
    case (cur_state)
      IDLE:       next_state = trigger_write ? WRITE : IDLE;
      WRITE:      next_state = WRITE_RESP;
      WRITE_RESP: next_state = IDLE;
      default:    next_state = IDLE;
    endcase
  end

  always @(*) bvalid_pad = (cur_state == WRITE_RESP);
endmodule

// BIU write-channel mirroring the cur_bresp_buf_bvalid capture FF
module biu_write_channel(
  input  coreclk,
  input  cpurst_b,
  input  pad_biu_bvalid,
  output reg cur_bresp_buf_bvalid,
  output     biu_lsu_b_vld
);
  wire bcpuclk;
  reg  back_valid;
  reg  back_pending;
  wire back_full;
  wire blast_done;
  wire pad_biu_back_ready = 1'b1;

  gated_clk_cell x_b_gate (
    .clk_in(coreclk), .global_en(1'b1), .module_en(1'b1),
    .local_en(1'b1), .external_en(1'b0), .pad_yy_icg_scan_en(1'b0),
    .clk_out(bcpuclk)
  );

  assign back_full   = back_valid && back_pending;
  assign blast_done  = cur_bresp_buf_bvalid && !back_full;
  assign biu_lsu_b_vld = cur_bresp_buf_bvalid && !back_full;

  always @(posedge bcpuclk or negedge cpurst_b) begin
    if(~cpurst_b)                          cur_bresp_buf_bvalid <= 1'b0;
    else if(pad_biu_bvalid && !back_full)  cur_bresp_buf_bvalid <= 1'b1;
    else if(!back_full)                    cur_bresp_buf_bvalid <= 1'b0;
  end

  always @(posedge coreclk or negedge cpurst_b) begin
    if(~cpurst_b)                       back_valid <= 1'b0;
    else if(blast_done || back_pending) back_valid <= 1'b1;
    else if(pad_biu_back_ready)         back_valid <= 1'b0;
  end

  always @(posedge coreclk or negedge cpurst_b) begin
    if(~cpurst_b)                          back_pending <= 1'b0;
    else if(pad_biu_back_ready)            back_pending <= 1'b0;
    else if(blast_done && back_valid)      back_pending <= 1'b1;
  end
endmodule

// CPU-top wrapper, mirroring the wire layers slave -> ... -> write_channel
module cpu_top(
  input  coreclk,
  input  cpurst_b,
  input  trigger_write,
  output cur_bresp_buf_bvalid,
  output biu_lsu_b_vld
);
  wire bvalid_pad;  // slave's bvalid output, fed into BIU as pad_biu_bvalid
  slave_axi u_slave (
    .coreclk(coreclk), .cpurst_b(cpurst_b),
    .trigger_write(trigger_write),
    .bvalid_pad(bvalid_pad)
  );
  biu_write_channel u_wc (
    .coreclk(coreclk), .cpurst_b(cpurst_b),
    .pad_biu_bvalid(bvalid_pad),
    .cur_bresp_buf_bvalid(cur_bresp_buf_bvalid),
    .biu_lsu_b_vld(biu_lsu_b_vld)
  );
endmodule

module tb;
  reg coreclk = 0;
  reg cpurst_b = 0;
  reg trigger_write = 0;
  wire cur_bresp_buf_bvalid;
  wire biu_lsu_b_vld;
  integer capture_count;
  integer wb_vld_count;

  cpu_top u_top (
    .coreclk(coreclk), .cpurst_b(cpurst_b),
    .trigger_write(trigger_write),
    .cur_bresp_buf_bvalid(cur_bresp_buf_bvalid),
    .biu_lsu_b_vld(biu_lsu_b_vld)
  );

  always #5 coreclk = ~coreclk;

  always @(posedge cur_bresp_buf_bvalid) capture_count = capture_count + 1;
  always @(posedge biu_lsu_b_vld)        wb_vld_count   = wb_vld_count + 1;

  initial begin
    capture_count = 0;
    wb_vld_count = 0;
    cpurst_b = 0;
    trigger_write = 0;
    #20; cpurst_b = 1;
    @(posedge coreclk);

    // 8 write triggers with 2-cycle gap between each (mimics c910's
    // request cadence). Slave takes 2 cycles to go IDLE→WRITE→WRITE_RESP,
    // so bvalid pulses are spaced 4 cycles apart.
    repeat (8) begin
      @(posedge coreclk); #1; trigger_write = 1;
      @(posedge coreclk); #1; trigger_write = 0;
      // Wait for the bvalid pulse to complete + a couple slack cycles
      repeat (4) @(posedge coreclk);
    end

    #100;
    $finish;
  end
endmodule
"#;

// Many-clock-domain shape: 8 sibling gated_clk_cells off coreclk (mirroring
// the c910 BIU's 8 gated clocks: vict_awcpuclk, st_awcpuclk, bus_arb_w_fifo_clk,
// vict_wcpuclk, st_wcpuclk, round_wcpuclk, bcpuclk, coreclk). Each has a
// dummy FF. The bresp capture FF still depends on back_full computed from
// FFs on the input coreclk. Tests whether having MANY gated-clock domains
// triggers an NBA ordering bug at the bresp FF.
const SRC_MULTI_CLK: &str = r#"
`timescale 1ns/100ps

module gated_clk_cell(
  input  clk_in,
  input  global_en, module_en, local_en, external_en, pad_yy_icg_scan_en,
  output clk_out
);
  assign clk_out = clk_in;
endmodule

module dut(
  input  coreclk,
  input  cpurst_b,
  input  pad_biu_bvalid,
  output reg cur_bresp_buf_bvalid
);
  // Eight gated clocks, all passthroughs from coreclk
  wire vict_awclk, st_awclk, fifo_clk, vict_wclk, st_wclk, round_wclk, bcpuclk;
  gated_clk_cell g1 (coreclk, 1'b1, 1'b1, 1'b1, 1'b0, 1'b0, vict_awclk);
  gated_clk_cell g2 (coreclk, 1'b1, 1'b1, 1'b1, 1'b0, 1'b0, st_awclk);
  gated_clk_cell g3 (coreclk, 1'b1, 1'b1, 1'b1, 1'b0, 1'b0, fifo_clk);
  gated_clk_cell g4 (coreclk, 1'b1, 1'b1, 1'b1, 1'b0, 1'b0, vict_wclk);
  gated_clk_cell g5 (coreclk, 1'b1, 1'b1, 1'b1, 1'b0, 1'b0, st_wclk);
  gated_clk_cell g6 (coreclk, 1'b1, 1'b1, 1'b1, 1'b0, 1'b0, round_wclk);
  gated_clk_cell g7 (coreclk, 1'b1, 1'b1, 1'b1, 1'b0, 1'b0, bcpuclk);

  reg [7:0] dummy_vict_aw, dummy_st_aw, dummy_fifo, dummy_vict_w;
  reg [7:0] dummy_st_w, dummy_round_w;
  reg       back_valid, back_pending;
  wire      back_full = back_valid && back_pending;
  wire      blast_done = cur_bresp_buf_bvalid && !back_full;
  wire      pad_biu_back_ready = 1'b1;

  // Dummy FFs on each gated clock - simulate the 17 always blocks
  always @(posedge vict_awclk or negedge cpurst_b) begin
    if(~cpurst_b) dummy_vict_aw <= 8'b0;
    else          dummy_vict_aw <= dummy_vict_aw + 1;
  end
  always @(posedge st_awclk or negedge cpurst_b) begin
    if(~cpurst_b) dummy_st_aw <= 8'b0;
    else          dummy_st_aw <= dummy_st_aw + 1;
  end
  always @(posedge fifo_clk or negedge cpurst_b) begin
    if(~cpurst_b) dummy_fifo <= 8'b0;
    else          dummy_fifo <= dummy_fifo + 1;
  end
  always @(posedge vict_wclk or negedge cpurst_b) begin
    if(~cpurst_b) dummy_vict_w <= 8'b0;
    else          dummy_vict_w <= dummy_vict_w + 1;
  end
  always @(posedge st_wclk or negedge cpurst_b) begin
    if(~cpurst_b) dummy_st_w <= 8'b0;
    else          dummy_st_w <= dummy_st_w + 1;
  end
  always @(posedge round_wclk or negedge cpurst_b) begin
    if(~cpurst_b) dummy_round_w <= 8'b0;
    else          dummy_round_w <= dummy_round_w + 1;
  end

  // The actual bresp capture FF on bcpuclk
  always @(posedge bcpuclk or negedge cpurst_b) begin
    if(~cpurst_b)                          cur_bresp_buf_bvalid <= 1'b0;
    else if(pad_biu_bvalid && !back_full)  cur_bresp_buf_bvalid <= 1'b1;
    else if(!back_full)                    cur_bresp_buf_bvalid <= 1'b0;
  end

  // back FFs on coreclk
  always @(posedge coreclk or negedge cpurst_b) begin
    if(~cpurst_b)                       back_valid <= 1'b0;
    else if(blast_done || back_pending) back_valid <= 1'b1;
    else if(pad_biu_back_ready)         back_valid <= 1'b0;
  end
  always @(posedge coreclk or negedge cpurst_b) begin
    if(~cpurst_b)                          back_pending <= 1'b0;
    else if(pad_biu_back_ready)            back_pending <= 1'b0;
    else if(blast_done && back_valid)      back_pending <= 1'b1;
  end
endmodule

module tb;
  reg coreclk = 0;
  reg cpurst_b = 0;
  reg pad_biu_bvalid = 0;
  wire cur_bresp_buf_bvalid;
  integer capture_count;

  dut u_dut (
    .coreclk(coreclk), .cpurst_b(cpurst_b),
    .pad_biu_bvalid(pad_biu_bvalid),
    .cur_bresp_buf_bvalid(cur_bresp_buf_bvalid)
  );

  always #5 coreclk = ~coreclk;
  always @(posedge cur_bresp_buf_bvalid) capture_count = capture_count + 1;

  initial begin
    capture_count = 0;
    cpurst_b = 0; pad_biu_bvalid = 0;
    #20; cpurst_b = 1;
    @(posedge coreclk);

    repeat (8) begin
      @(posedge coreclk); #1; pad_biu_bvalid = 1;
      @(posedge coreclk); #1; pad_biu_bvalid = 0;
      @(posedge coreclk);
    end
    #50; $finish;
  end
endmodule
"#;

#[test]
fn multi_clock_domain_all_8_captures() {
    let sim = simulate(SRC_MULTI_CLK, 500).expect("simulate failed");
    let count = lookup(&sim, "capture_count") & 0xFFFFFFFF;
    assert_eq!(
        count, 8,
        "Expected 8 captures with 8 gated-clock-domain siblings, got {}",
        count
    );
}

#[test]
fn nested_hierarchy_all_8_captures() {
    let sim = simulate(SRC_NESTED, 500).expect("simulate failed");
    let count = lookup(&sim, "capture_count") & 0xFFFFFFFF;
    let wb_count = lookup(&sim, "wb_vld_count") & 0xFFFFFFFF;
    assert_eq!(
        count, 8,
        "Expected 8 cur_bresp_buf_bvalid captures in nested module hierarchy, got {}",
        count
    );
    assert_eq!(
        wb_count, 8,
        "Expected 8 biu_lsu_b_vld rising edges (downstream of capture FF), got {}",
        wb_count
    );
}
