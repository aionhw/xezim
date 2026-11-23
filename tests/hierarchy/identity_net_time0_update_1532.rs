//! IEEE 1800-2023 §10.3, §23.3.3: time-0 updates of collapsed whole-net
//! identity assignments (`wire w = a;` / `assign bv = b;`) and input ports
//! must reach all combinational, explicit-sensitivity, and wide (`>64`-bit)
//! readers, while uninitialized 4-state X and 2-state 0 aliases must not fire.
//! Expected outputs match the reference simulator.

use xezim::simulate;

fn out_lines(src: &str) -> Vec<String> {
    simulate(src, 1000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .collect()
}

#[test]
fn identity_net_time0_comb_and_edge_readers() {
    let src = r#"
module t;
  logic a = 1'b0;
  logic [3:0] v = 4'd5;
  wire w = a;
  wire [3:0] wv;
  assign wv = v + 4'd1;
  always @(w) $display("M1 %0t @(w) body reads w: w=%b", $time, w);
  always @(w) $display("M2 %0t @(w) body reads nothing", $time);
  always @(wv) $display("M3 %0t @(wv) wv=%0d", $time, wv);
  initial #5 $finish;
endmodule
"#;
    let lines = out_lines(src);
    assert!(lines.contains(&"M1 0 @(w) body reads w: w=0".to_string()), "{lines:?}");
    assert!(lines.contains(&"M2 0 @(w) body reads nothing".to_string()), "{lines:?}");
    assert!(lines.contains(&"M3 0 @(wv) wv=6".to_string()), "{lines:?}");
}

#[test]
fn collapsed_identity_net_four_state_two_state_and_hier_cases() {
    let src = r#"
module kid(input wire w);
  always @(w) $display("KID_READ w=%b", w);
  always @(w) $display("KID_NOREAD");
endmodule

module t;
  logic a = 1'b0;
  logic u;
  logic az = 1'bz;
  logic [127:0] big = 128'h1;
  bit b0 = 1'b0;
  bit b1 = 1'b1;

  wire w_a = a;
  wire w_a2 = w_a;
  wire w_u = u;
  wire w_az = az;
  wire [127:0] w_big = big;
  logic lv_a; assign lv_a = a;
  logic lv_u; assign lv_u = u;
  bit bv0; assign bv0 = b0;
  bit bv1; assign bv1 = b1;

  logic y_from_wa;
  always @(a) $display("A_READ a=%b", a);
  always @(*) $display("A_STAR a=%b", a);
  always @(w_a) $display("WA_READ w_a=%b", w_a);
  always @(*) $display("WA_STAR w_a=%b", w_a);
  always @(w_a) y_from_wa = ~w_a;
  always @(y_from_wa) $display("Y_FROM_WA y=%b", y_from_wa);
  always @(w_a2) $display("WA2_READ w_a2=%b", w_a2);
  always @(w_u) $display("WU_READ w_u=%b", w_u);
  always @(w_u) $display("WU_NOREAD");
  always @(posedge w_u) $display("WU_POS");
  always @(negedge w_u) $display("WU_NEG");
  always @(w_az) $display("WAZ_READ w_az=%b", w_az);
  always @(w_az) $display("WAZ_NOREAD");
  always @(w_big) $display("WBIG_READ lsb=%b", w_big[0]);
  always @(w_big) $display("WBIG_NOREAD");
  always @(lv_a) $display("LVA_READ lv_a=%b", lv_a);
  always @(lv_u) $display("LVU_READ lv_u=%b", lv_u);
  always @(bv0) $display("BV0_READ bv0=%b", bv0);
  always @(bv1) $display("BV1_READ bv1=%b", bv1);
  kid k(.w(a));
  always @(k.w) $display("HIER_KW w=%b", k.w);
  initial #5 $finish;
endmodule
"#;
    let mut lines: Vec<String> = out_lines(src)
        .into_iter()
        .filter(|l| {
            !l.starts_with('[') && !l.starts_with('-') && !l.starts_with("Max") && !l.starts_with("Loaded") && !l.starts_with("Design")
        })
        .collect();
    // Filter strictly to the tagged output lines so banner noise cannot interfere.
    lines.retain(|l| {
        [
            "A_READ", "A_STAR", "WA_READ", "WA_STAR", "Y_FROM_WA", "WA2_READ", "WU_", "WAZ_",
            "WBIG_", "LVA_", "LVU_", "BV0_", "BV1_", "KID_", "HIER_KW",
        ]
        .iter()
        .any(|p| l.starts_with(p))
    });
    lines.sort();
    let expected = vec![
        "BV1_READ bv1=1",
        "HIER_KW w=0",
        "KID_NOREAD",
        "KID_READ w=0",
        "LVA_READ lv_a=0",
        "WA2_READ w_a2=0",
        "WAZ_NOREAD",
        "WAZ_READ w_az=z",
        "WA_READ w_a=0",
        "WA_STAR w_a=0",
        "WBIG_NOREAD",
        "WBIG_READ lsb=1",
        "Y_FROM_WA y=1",
    ];
    assert_eq!(lines, expected);
}

#[test]
fn wide_identity_net_cascade_and_sensitivity_alias_mismatch() {
    // Part A: standalone cascade with no other continuous assigns, child 1'bz port,
    // wide uninit suppression, and wide 2-state high bit update.
    let src_a = r#"
module k(input wire w);
  always @(w) $display("TAG|KID_W w=%b", w);
endmodule
module top;
  logic a = 1'b0;
  wire w_a = a;
  logic y;
  always @(w_a) y = ~w_a;
  always @(y) $display("TAG|ONLY_Y y=%b", y);
  logic az = 1'bz;
  k kz(.w(az));
  logic [127:0] ubig;
  wire [127:0] w_ubig = ubig;
  always @(w_ubig) $display("TAG|UBIG");
  bit [127:0] bbig = 128'd1 << 100;
  bit [127:0] w_bbig;
  assign w_bbig = bbig;
  always @(w_bbig) $display("TAG|BBIG hi=%b", w_bbig[100]);
  always @(posedge w_bbig) $display("TAG|BBIG_POS");
  initial #5 $finish;
endmodule
"#;
    let mut tags: Vec<String> = out_lines(src_a)
        .into_iter()
        .filter(|l| l.starts_with("TAG|"))
        .collect();
    tags.sort();
    assert_eq!(
        tags,
        vec![
            "TAG|BBIG hi=1",
            "TAG|KID_W w=z",
            "TAG|ONLY_Y y=1",
        ]
    );

    // Part B: sensitivity/body alias mismatch and non-double-firing of chained CA alias.
    let src_b = r#"
module top;
  logic a = 1'b0;
  wire w = a;
  logic x = 1'b1;
  wire [3:0] w1 = x + 4'd1;
  wire [3:0] w2 = w1;
  logic y1, y2;
  int w2_count = 0;
  always @(a) y1 = w;
  always @(w) y2 = a;
  always @(w2) w2_count = w2_count + 1;
  initial begin
    #1 $display("CHECK|y1=%b y2=%b w2_count=%0d", y1, y2, w2_count);
    $finish;
  end
endmodule
"#;
    let checks: Vec<String> = out_lines(src_b)
        .into_iter()
        .filter(|l| l.starts_with("CHECK|"))
        .collect();
    assert_eq!(checks, vec!["CHECK|y1=x y2=0 w2_count=1"]);
}
