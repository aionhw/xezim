//! Cone-of-influence test for c910 `ct_ifu_ibuf.v`'s `create_pointer_pre`
//! rotation logic at lines 3195-3219. Round 24+ of the c910 memcpy
//! investigation; see docs/c910_memcpy_investigation.md.
//!
//! The IBUF rotates its 32-bit one-hot `ibuf_create_pointer` left by
//! `ibdp_ibuf_half_vld_num` positions every cycle. The rotation uses
//! parameter-arithmetic slice bounds:
//!   `{ibuf_create_pointer[ENTRY_NUM-N-1:0],
//!     ibuf_create_pointer[ENTRY_NUM-1:ENTRY_NUM-N]}`
//! for N halfwords-pipedowned-this-cycle. ENTRY_NUM=32 is a parameter
//! defined in the same module (line 3190).
//!
//! If xezim's parameter folding in RangeSelect bounds OR its concat
//! width inference is wrong for this shape, the rotated pointer ends up
//! on the wrong entry and writes go to the wrong place — which would
//! produce exactly the kind of "halfword landed in wrong entry, then
//! pop-mux skips it" symptom we see for PC 0x712.
//!
//! Walks through all 9 rotation amounts (1-9) starting from one-hot
//! at bit 0, and verifies the rotated pointer is one-hot at bit N.

use xezim::simulate;

const SRC: &str = r#"
module tb;
  parameter ENTRY_NUM = 32;

  reg [ENTRY_NUM-1:0] ptr;
  reg [3:0] half_vld_num;
  reg [ENTRY_NUM-1:0] ptr_pre;

  always @(*) begin
    case (half_vld_num)
      4'b0001 : ptr_pre = {ptr[ENTRY_NUM-2:0], ptr[ENTRY_NUM-1]};
      4'b0010 : ptr_pre = {ptr[ENTRY_NUM-3:0], ptr[ENTRY_NUM-1:ENTRY_NUM-2]};
      4'b0011 : ptr_pre = {ptr[ENTRY_NUM-4:0], ptr[ENTRY_NUM-1:ENTRY_NUM-3]};
      4'b0100 : ptr_pre = {ptr[ENTRY_NUM-5:0], ptr[ENTRY_NUM-1:ENTRY_NUM-4]};
      4'b0101 : ptr_pre = {ptr[ENTRY_NUM-6:0], ptr[ENTRY_NUM-1:ENTRY_NUM-5]};
      4'b0110 : ptr_pre = {ptr[ENTRY_NUM-7:0], ptr[ENTRY_NUM-1:ENTRY_NUM-6]};
      4'b0111 : ptr_pre = {ptr[ENTRY_NUM-8:0], ptr[ENTRY_NUM-1:ENTRY_NUM-7]};
      4'b1000 : ptr_pre = {ptr[ENTRY_NUM-9:0], ptr[ENTRY_NUM-1:ENTRY_NUM-8]};
      4'b1001 : ptr_pre = {ptr[ENTRY_NUM-10:0], ptr[ENTRY_NUM-1:ENTRY_NUM-9]};
      default : ptr_pre = ptr;
    endcase
  end

  // Walk through rotation amounts; capture ptr_pre at each step.
  // We start from ptr = one-hot at bit 5 (some non-zero position) to
  // distinguish rotation from no-op.
  reg [31:0] cap_r1, cap_r2, cap_r3, cap_r4, cap_r5;
  reg [31:0] cap_r6, cap_r7, cap_r8, cap_r9, cap_rd;

  initial begin
    ptr = 32'h00000020;  // one-hot at bit 5
    half_vld_num = 4'b0001; #1; cap_r1 = ptr_pre;
    half_vld_num = 4'b0010; #1; cap_r2 = ptr_pre;
    half_vld_num = 4'b0011; #1; cap_r3 = ptr_pre;
    half_vld_num = 4'b0100; #1; cap_r4 = ptr_pre;
    half_vld_num = 4'b0101; #1; cap_r5 = ptr_pre;
    half_vld_num = 4'b0110; #1; cap_r6 = ptr_pre;
    half_vld_num = 4'b0111; #1; cap_r7 = ptr_pre;
    half_vld_num = 4'b1000; #1; cap_r8 = ptr_pre;
    half_vld_num = 4'b1001; #1; cap_r9 = ptr_pre;
    half_vld_num = 4'b0000; #1; cap_rd = ptr_pre;  // default: no rotate
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

/// Rotate-left a 32-bit value by `n` positions.
fn rotl(v: u32, n: u32) -> u32 {
    ((v << n) | (v >> (32 - n))) & 0xFFFFFFFF
}

#[test]
fn create_pointer_rotation_correct_for_all_amounts() {
    let sim = simulate(SRC, 200).expect("simulate failed");
    let base = 0x00000020u32; // one-hot at bit 5

    for (n, name) in [(1, "cap_r1"), (2, "cap_r2"), (3, "cap_r3"), (4, "cap_r4"),
                      (5, "cap_r5"), (6, "cap_r6"), (7, "cap_r7"), (8, "cap_r8"),
                      (9, "cap_r9")] {
        let got = lookup(&sim, name) as u32;
        let exp = rotl(base, n);
        assert_eq!(
            got, exp,
            "rotate-by-{}: got 0x{:08x}, expected 0x{:08x}",
            n, got, exp
        );
    }

    // Default: no rotate.
    let got_d = lookup(&sim, "cap_rd") as u32;
    assert_eq!(got_d, base, "default: ptr_pre should equal ptr");
}
