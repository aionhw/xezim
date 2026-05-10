//! Miri-targeted test for the c910 settle/NBA path.
//!
//! Run with:  cargo +nightly miri test --test c910_settle_miri
//!
//! Reproduces the shape of c910's PRF: shared register-file array written
//! by multiple always_ff blocks, read combinationally with sensitivity
//! fanout to several downstream consumers. If any unsafe pointer/aliasing
//! bug exists in the settle hot loop or NBA accumulation, miri's
//! Stacked-Borrows checker should flag it during a short simulation.

use xezim::simulate;

const SETTLE_REPRO: &str = r#"
module tb();
  reg clk = 0;
  reg rst_n = 0;
  always #5 clk = ~clk;

  reg [63:0] prf [0:15];

  reg [3:0]  w0_addr;  reg [63:0] w0_data;  reg w0_en;
  reg [3:0]  w1_addr;  reg [63:0] w1_data;  reg w1_en;
  reg [3:0]  w2_addr;  reg [63:0] w2_data;  reg w2_en;
  reg [3:0]  w3_addr;  reg [63:0] w3_data;  reg w3_en;

  reg [3:0]  r0_addr;  wire [63:0] r0_data = prf[r0_addr];
  reg [3:0]  r1_addr;  wire [63:0] r1_data = prf[r1_addr];

  always @(posedge clk) if (rst_n && w0_en) prf[w0_addr] <= w0_data;
  always @(posedge clk) if (rst_n && w1_en) prf[w1_addr] <= w1_data;
  always @(posedge clk) if (rst_n && w2_en) prf[w2_addr] <= w2_data;
  always @(posedge clk) if (rst_n && w3_en) prf[w3_addr] <= w3_data;

  reg  [63:0] acc;
  wire [64:0] sum_wide = {1'b0, acc} + {1'b0, r0_data};
  reg  [64:0] sum_lat;
  always @(posedge clk) sum_lat <= sum_wide;

  integer i;
  reg [7:0] cycles;
  initial begin
    rst_n = 0;
    cycles = 0;
    acc = 64'hDEADBEEF_CAFEBABE;
    for (i = 0; i < 16; i = i + 1) prf[i] = {32'h0, i[31:0]};
    w0_en = 0; w1_en = 0; w2_en = 0; w3_en = 0;
    #20;
    rst_n = 1;
  end

  always @(posedge clk) begin
    if (rst_n) begin
      cycles <= cycles + 1;
      w0_addr <= cycles[3:0];
      w1_addr <= cycles[3:0] ^ 4'h4;
      w2_addr <= cycles[3:0] ^ 4'h8;
      w3_addr <= cycles[3:0] ^ 4'hC;
      w0_data <= {32'h0, {24'h0, cycles}};
      w1_data <= {32'h1, {24'h0, cycles}};
      w2_data <= {32'h2, {24'h0, cycles}};
      w3_data <= 64'hFFFFFFFF_00000000 + {32'h0, {24'h0, cycles}};
      w0_en <= 1; w1_en <= 1; w2_en <= 1; w3_en <= 1;
      r0_addr <= cycles[3:0];
      r1_addr <= cycles[3:0] ^ 4'h7;
      acc <= acc + r0_data + r1_data;
      if (cycles == 8'd24) $finish;
    end
  end
endmodule
"#;

/// Reproduces the c910 IDU AIQ0 entry shape that fails memcpy:
///
/// - Wide create-data bus gated by `{N{en}} & data` replication-AND
/// - Sticky `wb` flop set EITHER at allocation (`x_create_wb`) OR by
///   writeback pulse (sources merge in always_comb feeding the flop)
/// - Multiple bypass-forward sources OR'd into `rdy_for_issue`
/// - Reader of `rdy_for_issue` gates instruction issue, advancing the
///   "retire counter" — a stuck `rdy=0` would mimic the watchdog stall.
const AIQ_DEP_REG_REPRO: &str = r#"
module tb();
  reg clk = 0;
  reg rst_n = 0;
  always #5 clk = ~clk;

  // ===== AIQ0 entry create-data bus, 227 bits like the real design =====
  // Bit layout (subset that matters):
  //   [59] = SRC0_WB    (was the producer already retired?)
  //   [58] = SRC0_RDY
  //   [9:0] passes through to dep_reg as create_src0_data[9:0]
  reg          ctrl_create_en;
  reg  [226:0] aiq0_create0_data;
  // EXACT shape of the failing assign in ct_idu_is_dp.v line 4837:
  wire [226:0] dp_aiq0_create0_data = {227{ctrl_create_en}} & aiq0_create0_data;

  // ===== Per-entry flop (mirrors aiq0_entry2_create_data) =====
  reg  [226:0] aiq0_entry2_create_data;
  reg          aiq0_entry2_vld;
  always @(posedge clk) if (rst_n) begin
    aiq0_entry2_create_data <= dp_aiq0_create0_data;
    if (ctrl_create_en) aiq0_entry2_vld <= 1'b1;
  end

  // ===== src0_dep_reg flop with multiple write paths =====
  // wb flop sources (matches dep_reg_entry x_create_wb / x_wb_update merge):
  //   1. allocation-time bit (x_create_data[1] = aiq0_entry2_create_data[59])
  //   2. writeback pulse (sticky update)
  //
  // The bug class we suspect: when wide-bus replication-AND mis-evaluates,
  // the create-time wb bit reads as 0 even when it should be 1 (producer
  // already retired). Then the entry waits forever for a writeback that
  // already happened.
  reg          src0_wb_flop;
  reg  [6:0]   src0_preg_flop;
  reg          wb_pulse_for_preg;
  reg  [6:0]   wb_preg;

  wire         x_create_wb   = aiq0_entry2_create_data[59];
  wire [6:0]   x_create_preg = aiq0_entry2_create_data[66:60];
  wire         x_wb_update_match = (wb_preg == src0_preg_flop) && wb_pulse_for_preg;

  always @(posedge clk) begin
    if (!rst_n) begin
      src0_wb_flop   <= 1'b0;
      src0_preg_flop <= 7'h0;
    end else if (ctrl_create_en) begin
      // Allocation: take create-time WB bit
      src0_wb_flop   <= x_create_wb;
      src0_preg_flop <= x_create_preg;
    end else if (x_wb_update_match) begin
      // Writeback for this entry's preg: set wb sticky
      src0_wb_flop   <= 1'b1;
    end
  end

  // ===== Bypass-forward chain (dep_reg_entry x_read_rdy_for_issue) =====
  // {N{x}} & y replication-AND pattern, repeated like the real design.
  reg          alu0_fwd_inst;
  reg  [107:0] alu0_fwd_lch_rdy;
  wire [107:0] alu0_fwd_vld = {108{alu0_fwd_inst}} & alu0_fwd_lch_rdy;

  reg          alu1_fwd_inst;
  reg  [107:0] alu1_fwd_lch_rdy;
  wire [107:0] alu1_fwd_vld = {108{alu1_fwd_inst}} & alu1_fwd_lch_rdy;

  // For each entry, src0 forward-ready = OR of three indexed bits
  wire src0_alu0_ready = alu0_fwd_vld[6] | alu0_fwd_vld[7] | alu0_fwd_vld[8];
  wire src0_alu1_ready = alu1_fwd_vld[6] | alu1_fwd_vld[7] | alu1_fwd_vld[8];

  wire src0_rdy_for_issue = src0_wb_flop | src0_alu0_ready | src0_alu1_ready;

  // ===== Issue gating + retire counter =====
  reg [31:0] retire_count;
  always @(posedge clk) if (rst_n) begin
    if (aiq0_entry2_vld && src0_rdy_for_issue) begin
      retire_count <= retire_count + 1;
      aiq0_entry2_vld <= 1'b0;  // entry consumed
    end
  end

  // ===== Driver: stage a sequence that exercises the failing pattern =====
  reg [7:0] cycles;
  initial begin
    rst_n = 0;
    cycles = 0;
    ctrl_create_en = 0;
    aiq0_create0_data = 227'h0;
    alu0_fwd_inst = 0;
    alu0_fwd_lch_rdy = 0;
    alu1_fwd_inst = 0;
    alu1_fwd_lch_rdy = 0;
    wb_pulse_for_preg = 0;
    wb_preg = 0;
    retire_count = 0;
    src0_wb_flop = 0;
    src0_preg_flop = 0;
    aiq0_entry2_vld = 0;
    aiq0_entry2_create_data = 227'h0;
    #20;
    rst_n = 1;
  end

  always @(posedge clk) if (rst_n) begin
    cycles <= cycles + 1;
    case (cycles)
      8'd1: begin
        // Allocation case A: producer already retired → wb bit = 1
        ctrl_create_en   <= 1'b1;
        aiq0_create0_data <= {161'h0, 7'h50, 1'b1, 1'b0, 57'h0};  // bits 67:60=preg=0x50, bit 59=wb=1, bit 58=rdy=0
      end
      8'd2: begin
        ctrl_create_en   <= 1'b0;
        aiq0_create0_data <= 227'h0;
      end
      8'd3: begin
        // Forward fanout test
        alu0_fwd_inst    <= 1'b1;
        alu0_fwd_lch_rdy <= {108{1'b1}};
      end
      8'd4: begin
        alu0_fwd_inst    <= 1'b0;
        alu0_fwd_lch_rdy <= 108'h0;
      end
      8'd5: begin
        // Allocation case B: producer NOT yet retired → wb=0, then writeback
        ctrl_create_en   <= 1'b1;
        aiq0_create0_data <= {161'h0, 7'h21, 1'b0, 1'b0, 57'h0};
      end
      8'd6: begin
        ctrl_create_en   <= 1'b0;
        wb_pulse_for_preg <= 1'b1;
        wb_preg           <= 7'h21;
      end
      8'd7: begin
        wb_pulse_for_preg <= 1'b0;
      end
      8'd24: $finish;
      default: ;
    endcase
  end
endmodule
"#;

#[test]
fn c910_aiq_dep_reg_shape() {
    let sim = simulate(AIQ_DEP_REG_REPRO, 600).expect("simulate failed");
    assert!(sim.time > 0);
}

#[test]
fn c910_prf_settle_shape() {
    // Run a short simulation. Under miri, this fires the settle hot loop,
    // exec_insns, NBA accumulation, and apply_nba multiple times — enough
    // to catch most pointer/aliasing UB in those paths.
    let sim = simulate(SETTLE_REPRO, 500).expect("simulate failed");
    // Sanity: simulator ran past time 0.
    assert!(sim.time > 0);
}

#[test]
fn wide_arith_carry_not_dropped() {
    // Targeted regression: 0xFFFF_FFFF_FFFF_FFFF + 1 at 65-bit width must
    // produce 0x1_0000_0000_0000_0000, not 0. This was the root cause of
    // c906 cmark "Cannot validate operation" before xezim-core 710a793.
    use xezim::compiler::Value;
    let a = Value::from_u64(u64::MAX, 64);
    let b = Value::from_u64(1, 64);
    let mut a65 = a.resize(65);
    let b65 = b.resize(65);
    a65 = a65.add(&b65);
    // The 65-bit sum has bit 64 set (carry-out).
    let bit64 = a65.get_bit(64);
    assert!(matches!(bit64, xezim_core::value::LogicBit::One),
            "65-bit add lost the carry into bit 64");
    // Low 64 bits should be 0.
    let low = a65.to_u64().unwrap_or(u64::MAX);
    assert_eq!(low, 0, "low 64 bits should be 0 after FFFF...+1");
}
