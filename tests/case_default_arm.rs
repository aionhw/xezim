//! Reproduces the c910 IFU/IB case-stmt default-arm bug.
//!
//! Pattern from `ct_ifu_ibdp.v:2553`: a one-hot selector with three
//! explicit arms (3'b100, 3'b010, 3'b001) plus default. When the
//! selector goes 3'b001 → 3'b000 (no buffer valid), the default arm
//! should fire and zero the output. xezim's case lowering instead
//! falls through to the last explicit arm's body, copying that arm's
//! source into the output. Concretely: at sim 44505 with sel=000 and
//! lbuf=0 the output is 0 (default would coincidentally produce the
//! same answer); at sim 44515 with sel=000 and lbuf=0x5847D70B the
//! output becomes 0x5847D70B — the lbuf arm's body fired.

use xezim::simulate;

const SRC_CASE_DEFAULT: &str = r#"
module tb;
  reg [2:0] sel;
  reg [31:0] in_a;
  reg [31:0] in_b;
  reg [31:0] in_c;
  reg [31:0] out;

  always @(*) begin
    case (sel)
      3'b100: out = in_a;
      3'b010: out = in_b;
      3'b001: out = in_c;
      default: out = 32'h00000000;
    endcase
  end

  initial begin
    in_a = 32'hAAAAAAAA;
    in_b = 32'hBBBBBBBB;
    in_c = 32'hCCCCCCCC;
    sel  = 3'b001;
    #10;
    sel  = 3'b000;
    #10;
    $finish;
  end
endmodule
"#;

/// Closer match to ct_ifu_ibdp.v:2553: explicit sensitivity list with
/// bit-range references and concatenated case selector. 3-arm one-hot
/// + default. Drives sel=001 then changes a buffer source while sel
/// stays 000 — same as the smoking-gun T=44515 transition.
const SRC_CASE_DEFAULT_C910_SHAPE: &str = r#"
module tb;
  reg sel_a, sel_b, sel_c;
  reg [31:0] in_a;
  reg [31:0] in_b;
  reg [31:0] in_c;
  reg [31:0] out;

  always @( in_a[31:0]
         or in_b[31:0]
         or in_c[31:0]
         or sel_a
         or sel_b
         or sel_c)
  begin
    case ({sel_a, sel_b, sel_c})
      3'b100: out[31:0] = in_a[31:0];
      3'b010: out[31:0] = in_b[31:0];
      3'b001: out[31:0] = in_c[31:0];
      default: out[31:0] = 32'h00000000;
    endcase
  end

  initial begin
    sel_a = 0; sel_b = 0; sel_c = 0;
    in_a = 32'h0;
    in_b = 32'h0;
    in_c = 32'h0;
    #5;
    sel_c = 1;        // sel = 001
    in_c = 32'hCCCCCCCC;
    #5;
    sel_c = 0;        // sel = 000
    in_c = 32'h5847D70B; // change buffer C while sel=000 — c910 pattern
    #5;
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
fn case_default_fires_when_no_explicit_arm_matches() {
    let sim = simulate(SRC_CASE_DEFAULT, 100).expect("simulate failed");
    let out_v = lookup_one_of(&sim, &["tb.out", "out", "tb_out"]);
    let lo = out_v.to_u64().expect("out should be defined") & 0xFFFF_FFFF;
    assert_eq!(
        lo, 0,
        "after sel=001 (out=0xCCCCCCCC) then sel=000 (no match), default arm \
         should fire and set out=0; got 0x{:08X}",
        lo
    );
}

#[test]
fn case_default_with_c910_shape() {
    let sim = simulate(SRC_CASE_DEFAULT_C910_SHAPE, 100).expect("simulate failed");
    let out_v = lookup_one_of(&sim, &["tb.out", "out", "tb_out"]);
    let lo = out_v.to_u64().expect("out should be defined") & 0xFFFF_FFFF;
    assert_eq!(
        lo, 0,
        "c910 shape: explicit sensitivity list with bit-range refs, then sel \
         goes 001→000 while in_c changes. default arm should fire; got 0x{:08X}",
        lo
    );
}
