//! Targeted tests for the `{N{en}} & data` replication-AND pattern that
//! appears throughout the c910 dispatch path. The hypothesis (under
//! investigation): xezim's bytecode emits this pattern such that a wide
//! result is computed wrong when `en` is 1, causing the AIQ0 entry
//! WB-bit at allocation to read 0 instead of 1.

use xezim::simulate;
use xezim_core::value::LogicBit;

fn collect_bits(v: &xezim_core::value::Value) -> Vec<LogicBit> {
    (0..v.width as usize).map(|i| v.get_bit(i)).collect()
}

fn assert_eq_pattern(actual: &xezim_core::value::Value, expected: &str, label: &str) {
    let bits = collect_bits(actual);
    let got: String = bits.iter().rev().map(|b| match b {
        LogicBit::Zero => '0', LogicBit::One => '1',
        LogicBit::X => 'x', LogicBit::Z => 'z',
    }).collect();
    assert_eq!(got, expected, "{}: width={}\n  expected: {}\n     got:  {}", label, actual.width, expected, got);
}

const SRC_REPLICATE_227: &str = r#"
module tb;
  reg en = 0;
  reg [226:0] data = 227'h0;
  wire [226:0] out = {227{en}} & data;
  initial begin
    #1;
    en = 1;
    data = 227'h7FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF;
    #1;
    $finish;
  end
endmodule
"#;

fn lookup_one_of(sim: &xezim::compiler::Simulator, names: &[&str]) -> xezim_core::value::Value {
    for n in names {
        if let Some(v) = sim.get_signal(n) {
            return v.clone();
        }
    }
    panic!("none of these signal names found: {:?}", names);
}

#[test]
fn replicate_227_and_wide_data_with_en_high() {
    let sim = simulate(SRC_REPLICATE_227, 100).expect("simulate failed");
    let out_v = lookup_one_of(&sim, &["tb.out", "out", "tb_out", "tb.tb.out"]);
    let out = &out_v;
    assert_eq!(out.width, 227);
    // When en=1, out should equal data — all 227 ones.
    let expected: String = std::iter::repeat('1').take(227).collect();
    assert_eq_pattern(out, &expected, "out should be all-ones when en=1");
}

const SRC_REPLICATE_108: &str = r#"
module tb;
  reg en = 0;
  reg [107:0] data = 108'h0;
  wire [107:0] out = {108{en}} & data;
  initial begin
    #1;
    en = 1;
    data = 108'hF0F0F0F0F0F0F0F0F0F0F0F0F0F;
    #1;
    $finish;
  end
endmodule
"#;

#[test]
fn replicate_108_and_wide_data_with_en_high() {
    let sim = simulate(SRC_REPLICATE_108, 100).expect("simulate failed");
    let out_v = lookup_one_of(&sim, &["tb.out", "out"]);
    let out = &out_v;
    assert_eq!(out.width, 108);
    // out should equal data.
    let expected = "000011110000111100001111000011110000111100001111000011110000111100001111000011110000111100001111000011110000";
    // Reverse via char-count visual: 108 bits of pattern 0xF0F repeated.
    // Easier: verify bit-by-bit
    for i in 0..108 {
        let expected_bit = if (i % 8) < 4 { LogicBit::One } else { LogicBit::Zero };
        let got = out.get_bit(i);
        assert_eq!(got, expected_bit,
            "bit {}: expected {:?}, got {:?} (out width={}, expected pattern {})",
            i, expected_bit, got, out.width, expected);
    }
}

const SRC_REPLICATE_AND_BIT_SELECT: &str = r#"
module tb;
  reg en = 0;
  reg [226:0] data = 227'h0;
  wire [226:0] full = {227{en}} & data;
  wire bit59 = full[59];
  wire [8:0] slice66_58 = full[66:58];
  initial begin
    #1;
    en = 1;
    // bit 59 = 1, bit 60-66 = 0x50 (preg=0x50, wb=1)
    data = 227'h0;
    data[59] = 1'b1;
    data[66:60] = 7'h50;
    #1;
    $finish;
  end
endmodule
"#;

// MULTI-DRIVER-SLICE pattern: c910 ct_idu_is_dp.v has dozens of
// `assign aiq0_create0_data[range] = ...` for non-overlapping slices of a
// single wire, then reads the whole wire downstream. If xezim fails to
// merge slice updates correctly (e.g., last-writer-wins instead of merging),
// the downstream read sees an incomplete bus.
const SRC_MULTI_SLICE: &str = r#"
module tb;
  reg [3:0]  src_a = 4'h5;
  reg [7:0]  src_b = 8'hAA;
  reg [4:0]  src_c = 5'h1F;
  wire [16:0] bus;
  assign bus[3:0]  = src_a;
  assign bus[11:4] = src_b;
  assign bus[16:12] = src_c;
  wire [16:0] gated = {17{1'b1}} & bus;
  initial begin
    #1;
    src_a = 4'h5;
    src_b = 8'hAA;
    src_c = 5'h1F;
    #1;
    $finish;
  end
endmodule
"#;

#[test]
fn multi_driver_slice_then_replicate_and() {
    let sim = simulate(SRC_MULTI_SLICE, 100).expect("simulate failed");
    let bus_v = lookup_one_of(&sim, &["tb.bus", "bus"]);
    let bus = &bus_v;
    assert_eq!(bus.width, 17);

    // Expected: bus = {5'h1F, 8'hAA, 4'h5} = 17'h1F_AA_5
    let expected_u = (0x1Fu64 << 12) | (0xAAu64 << 4) | 0x5u64;
    for i in 0..17 {
        let want = if (expected_u >> i) & 1 == 1 { LogicBit::One } else { LogicBit::Zero };
        let got = bus.get_bit(i);
        assert_eq!(got, want,
            "bus bit {}: expected {:?}, got {:?} (full bus expected 0x{:X}, got bits)",
            i, want, got, expected_u);
    }

    let gated_v = lookup_one_of(&sim, &["tb.gated", "gated"]);
    let gated = &gated_v;
    assert_eq!(gated.width, 17);
    for i in 0..17 {
        let want = if (expected_u >> i) & 1 == 1 { LogicBit::One } else { LogicBit::Zero };
        let got = gated.get_bit(i);
        assert_eq!(got, want, "gated bit {}: expected {:?}, got {:?}", i, want, got);
    }
}

#[test]
fn replicate_then_bit_select_full_chain() {
    // This mirrors the EXACT chain that fails in c910:
    // dp_aiq0_create0_data = {227{en}} & aiq0_create0_data
    // then aiq0_entry2_create_data <= dp_aiq0_create0_data
    // then x_create_data[1] = create_src0_data[1] = x_create_data[59 of aiq0]
    let sim = simulate(SRC_REPLICATE_AND_BIT_SELECT, 100).expect("simulate failed");
    let bit59_v = lookup_one_of(&sim, &["tb.bit59", "bit59"]);
    let bit59 = &bit59_v;
    assert_eq!(bit59.width, 1);
    assert!(matches!(bit59.get_bit(0), LogicBit::One),
        "bit59 should be 1 when en=1 and data[59]=1, got {:?}", bit59.get_bit(0));

    let slice_v = lookup_one_of(&sim, &["tb.slice66_58", "slice66_58"]);
    let slice = &slice_v;
    assert_eq!(slice.width, 9);
    // bit[8:1] = preg=0x50 (= 0b1010000), bit[0] = wb=1
    // so slice[8:0] = {7'h50, 1'b1, 1'b0} = 9'b101_0000_10 = 0xA2
    // Wait — bit 66 maps to slice[8], bit 58 maps to slice[0]:
    //   slice[8:2] = data[66:60] = 7'h50 = 7'b1010000
    //   slice[1]   = data[59] = 1
    //   slice[0]   = data[58] = 0
    // slice = 9'b101_0000_10 = 9'h0A2 = 0xA2
    let expected_bits: [LogicBit; 9] = [
        LogicBit::Zero, // slice[0] = data[58]
        LogicBit::One,  // slice[1] = data[59]
        LogicBit::Zero, // slice[2] = data[60] = bit 0 of 7'h50
        LogicBit::Zero, // slice[3] = data[61] = bit 1 of 7'h50
        LogicBit::Zero, // slice[4] = data[62] = bit 2 of 7'h50
        LogicBit::Zero, // slice[5] = data[63] = bit 3 of 7'h50
        LogicBit::One,  // slice[6] = data[64] = bit 4 of 7'h50
        LogicBit::Zero, // slice[7] = data[65] = bit 5 of 7'h50
        LogicBit::One,  // slice[8] = data[66] = bit 6 of 7'h50
    ];
    for i in 0..9 {
        assert_eq!(slice.get_bit(i), expected_bits[i],
            "slice bit {}: expected {:?}, got {:?}",
            i, expected_bits[i], slice.get_bit(i));
    }
}
