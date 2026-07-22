//! IEEE 1800-2017 §11 operator fixes found by differential audit against a
//! reference simulator (byte-identical output confirmed):
//!
//! * §11.4.8 reduction AND — a known 0 bit forces 0 even with X/Z present
//!   (`&4'b1x0z` is 0, not x). The 0-check must precede the X/Z short-circuit.
//! * §11.5.1 out-of-range bit/part-selects read as x, not 0 — for a
//!   constant index, a variable index, and indexed part-selects (`+:`/`-:`),
//!   including a `-:` whose low bound goes negative. Verified in both the
//!   interpreted (initial-block) and compiled (continuous-assign) paths.

use xezim::simulate;

fn out1(src: &str, needle: &str) -> bool {
    simulate(src, 100)
        .expect("sim")
        .output
        .iter()
        .any(|o| o.message == needle)
}

#[test]
fn reduction_and_zero_dominates_xz() {
    let src = r#"
module t;
  logic [3:0] a = 4'b1x0z; // has a 0
  logic [3:0] b = 4'b11x1; // no 0, has x
  logic [3:0] c = 4'b1111;
  logic [71:0] w = {8'b1x0z_1111, 64'hFFFF_FFFF_FFFF_FFFF};
  initial $display("R=%b,%b,%b,%b,%b", &a, &b, &c, ~&a, &w);
endmodule
"#;
    assert!(out1(src, "R=0,x,1,1,0"), "reduction-AND precedence wrong");
}

#[test]
fn out_of_range_selects_read_x_interpreted() {
    let src = r#"
module t;
  logic [7:0] data = 8'hAB;
  int idx = 9;
  logic [3:0] p1, p2;
  initial begin
    p1 = data[6 +: 4]; // bits 9,8 OOR -> xx10
    p2 = data[1 -: 4]; // bits -1,-2 OOR -> 11xx
    $display("S=%b,%b,%b,%b", data[9], data[idx], p1, p2);
  end
endmodule
"#;
    // data[9] const OOR -> x; data[idx=9] var OOR -> x; xx10; 11xx
    assert!(out1(src, "S=x,x,xx10,11xx"), "OOR select (interpreted) wrong");
}

#[test]
fn out_of_range_selects_read_x_compiled() {
    let src = r#"
module t;
  logic [7:0] data = 8'hAB;
  logic [3:0] r_dn, r_up;
  logic bs;
  assign r_dn = data[1 -: 4]; // 11xx
  assign r_up = data[6 +: 4]; // xx10
  assign bs   = data[9];      // x
  initial begin #1; $display("C=%b,%b,%b", r_dn, r_up, bs); end
endmodule
"#;
    assert!(out1(src, "C=11xx,xx10,x"), "OOR select (compiled) wrong");
}

#[test]
fn in_range_selects_unaffected() {
    // Guard: the common in-range path must be byte-for-byte unchanged.
    let src = r#"
module t;
  logic [7:0] data = 8'hAB; // 1010_1011
  int i = 2;
  initial $display("K=%h,%h,%b,%h", data[5:2], data[3 -: 4], data[i], data[i +: 4]);
endmodule
"#;
    // data[5:2]=1010=a; data[3:0]=1011=b; data[2]=0; data[5:2]=a
    assert!(out1(src, "K=a,b,0,a"), "in-range select regressed");
}
