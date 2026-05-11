//! Regression test for the c910 memcpy root cause identified
//! after the determinism fix (commits f5e5270/8716b5e).
//!
//! `wid_for_axi4.v:76`:
//! ```verilog
//! assign create_en = biu_pad_awvalid && pad_biu_awready;
//! ```
//!
//! In xezim's c910 run with XEZIM_INIT_ZERO=1, this cont-assign
//! produces X output even when both inputs are valid binary values.
//! Per Verilog: `0 && X = 0` and `1 && 1 = 1`, never X.
//!
//! The xezim symptom: at sim 58305+, biu_pad_awvalid=0, pad_biu_awready=1,
//! yet create_en=X. The comb_entry for create_en evidently doesn't
//! fire after the time-0 settle, leaving create_en at its initial X.
//!
//! This test exercises an isolated version of the pattern:
//!  - a simple submodule with `assign create_en = a && b;`
//!  - inputs driven from above
//!  - verify create_en correctly reflects a && b after inputs become known
//!
//! If this test fails (create_en stuck at X when a/b transition from
//! their initial values), the bug is reproduced. If it passes, the
//! bug requires additional context not captured by this isolated case.

use xezim::simulate;

const SRC: &str = r#"
`timescale 1ns/100ps

module wid_for_axi4_min(
  input  biu_pad_awvalid,
  input  pad_biu_awready,
  output reg [4:0] wid_fifo_create,
  output wire create_en
);
  // The bug-line — direct port-connected cont-assign
  assign create_en = biu_pad_awvalid && pad_biu_awready;

  // FF that increments on create_en, like the c910 RTL
  reg clk = 0;
  always #5 clk = ~clk;
  reg cpurst_b = 0;
  initial begin
    #10 cpurst_b = 1;
  end

  always @(posedge clk or negedge cpurst_b) begin
    if (!cpurst_b) wid_fifo_create <= 5'b0;
    else if (create_en) wid_fifo_create <= wid_fifo_create + 1;
  end
endmodule

module tb;
  reg biu_pad_awvalid = 0;
  reg pad_biu_awready = 0;
  wire create_en;
  wire [4:0] wid_fifo_create;
  reg [31:0] captured_create;
  reg create_en_was_x_after_inputs_known = 0;

  wid_for_axi4_min u_w (
    .biu_pad_awvalid(biu_pad_awvalid),
    .pad_biu_awready(pad_biu_awready),
    .wid_fifo_create(wid_fifo_create),
    .create_en(create_en)
  );

  initial begin
    captured_create = 0;
    // Wait for reset
    #20;
    // Drive inputs through known values
    biu_pad_awvalid = 0; pad_biu_awready = 0;
    #20;
    biu_pad_awvalid = 0; pad_biu_awready = 1;
    #20;
    // If create_en is still X here, the bug is reproduced
    if (create_en !== 1'b0) create_en_was_x_after_inputs_known = 1;
    // Now drive the AND result to 1
    biu_pad_awvalid = 1;
    @(posedge u_w.clk);
    @(posedge u_w.clk);
    captured_create = wid_fifo_create;
    #5;
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
fn cont_assign_and_propagates_after_inputs_known() {
    let sim = simulate(SRC, 200).expect("simulate failed");
    let was_x = lookup(&sim, "create_en_was_x_after_inputs_known") & 1;
    assert_eq!(
        was_x, 0,
        "create_en should be 0 when biu_pad_awvalid=0 and pad_biu_awready=1, \
         but the comb_entry didn't re-evaluate after inputs became known. \
         This reproduces the c910 memcpy root cause."
    );

    let captured = lookup(&sim, "captured_create") & 0x1F;
    assert!(
        captured > 0,
        "Expected wid_fifo_create > 0 after biu_pad_awvalid=1, got {}",
        captured
    );
}
