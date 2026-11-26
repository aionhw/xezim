//! Shapes the two-state lowering path (`lower_two_state`) admits, pinned by
//! VALUE so a miscompile is caught rather than just a coverage regression.
//! Each block below is X-free at its inputs, narrow and unsigned, so the
//! eval-site prefilter hands it to the two-state executor; the assertions
//! hold on either path, which is the point.
//!
//!  * §12.5 `case` compiled to a jump table (`CaseJump`) and to a masked
//!    bucket table (`CaseMaskJump`) — both dispatch on an X-free selector,
//!    so the 4-state "no pattern matches / wildcard chain" paths are dead.
//!  * §11.4.9 reduction `|` and `&`.
//!  * §11.5.1 dynamic bit-select TARGET, including an out-of-range index,
//!    which drops the write.
//!  * §5.7.1 `'x` in a `default:` arm: the constant cannot live in the
//!    two-state u64 register file, so it is folded at lowering time and the
//!    STORE writes the x plane.
//!  * A temp register redefined at DIFFERENT widths in different `case`
//!    arms — each use stays inside its own arm, so stream-order width
//!    tracking is exact and the block still lowers.

use xezim::simulate;

fn msgs(src: &str) -> Vec<String> {
    simulate(src, 1000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect()
}

#[test]
fn wide_repetition_preserves_fill_and_general_patterns() {
    let out = msgs(
        r#"
module top;
  logic flag;
  logic [3:0] nibble;
  logic [2:0] triplet;
  logic [31:0] strip;
  logic [63:0] ribbon;
  logic [127:0] spread;
  logic [127:0] tiled;
  logic [95:0] irregular;
  always_comb strip = {32{flag}};
  always_comb ribbon = {16{nibble}};
  always_comb spread = {128{flag}};
  always_comb tiled = {32{nibble}};
  always_comb irregular = {32{triplet}};
  initial begin
    flag = 1'b0; nibble = 4'h3; triplet = 3'b101; #1;
    $display("A_%08h_%016h_%032h_%032h_%024h", strip, ribbon, spread, tiled, irregular);
    flag = 1'b1; nibble = 4'hc; #1;
    $display("B_%08h_%016h_%032h_%032h_%024h", strip, ribbon, spread, tiled, irregular);
  end
endmodule
"#,
    );
    assert!(out.contains(
        &"A_00000000_3333333333333333_00000000000000000000000000000000_33333333333333333333333333333333_b6db6db6db6db6db6db6db6d".to_string()
    ), "{out:?}");
    assert!(out.contains(
        &"B_ffffffff_cccccccccccccccc_ffffffffffffffffffffffffffffffff_cccccccccccccccccccccccccccccccc_b6db6db6db6db6db6db6db6d".to_string()
    ), "{out:?}");
}

#[test]
fn case_jump_table_values() {
    let out = msgs(
        r#"
module top;
  logic [7:0] sel;
  logic [15:0] y;
  always_comb begin
    case (sel)
      8'd0:  y = 16'h1111;
      8'd7:  y = 16'h2222;
      8'd64: y = 16'h3333;
      8'd200:y = 16'h4444;
      8'd255:y = 16'h5555;
      default: y = 16'hDEAD;
    endcase
  end
  initial begin
    sel = 8'd0;   #1 $display("A_%04h", y);
    sel = 8'd7;   #1 $display("B_%04h", y);
    sel = 8'd64;  #1 $display("C_%04h", y);
    sel = 8'd200; #1 $display("D_%04h", y);
    sel = 8'd255; #1 $display("E_%04h", y);
    sel = 8'd123; #1 $display("F_%04h", y);
  end
endmodule
"#,
    );
    assert!(out.contains(&"A_1111".to_string()), "{out:?}");
    assert!(out.contains(&"B_2222".to_string()), "{out:?}");
    assert!(out.contains(&"C_3333".to_string()), "{out:?}");
    assert!(out.contains(&"D_4444".to_string()), "{out:?}");
    assert!(out.contains(&"E_5555".to_string()), "{out:?}");
    assert!(out.contains(&"F_dead".to_string()), "{out:?}");
}

#[test]
fn reduction_or_and() {
    let out = msgs(
        r#"
module top;
  logic [7:0] a;
  logic ro, ra;
  always_comb ro = |a;
  always_comb ra = &a;
  initial begin
    a = 8'h00; #1 $display("Z_%b%b", ro, ra);
    a = 8'h01; #1 $display("O_%b%b", ro, ra);
    a = 8'hFF; #1 $display("F_%b%b", ro, ra);
  end
endmodule
"#,
    );
    assert!(out.contains(&"Z_00".to_string()), "{out:?}");
    assert!(out.contains(&"O_10".to_string()), "{out:?}");
    assert!(out.contains(&"F_11".to_string()), "{out:?}");
}

#[test]
fn dynamic_bit_target_and_out_of_range() {
    // §11.5.1: an out-of-range index on an assignment TARGET drops the
    // write; the rest of the vector is untouched either way.
    let out = msgs(
        r#"
module top;
  logic [7:0] v;
  int idx;
  logic b;
  always_comb begin
    v = 8'h00;
    v[idx] = b;
  end
  initial begin
    idx = 0; b = 1'b1; #1 $display("A_%02h", v);
    idx = 3; b = 1'b1; #1 $display("B_%02h", v);
    idx = 7; b = 1'b1; #1 $display("C_%02h", v);
    idx = 9; b = 1'b1; #1 $display("D_%02h", v);
  end
endmodule
"#,
    );
    assert!(out.contains(&"A_01".to_string()), "{out:?}");
    assert!(out.contains(&"B_08".to_string()), "{out:?}");
    assert!(out.contains(&"C_80".to_string()), "{out:?}");
    assert!(out.contains(&"D_00".to_string()), "{out:?}");
}

#[test]
fn x_constant_default_arm_still_propagates_x() {
    // The folded constant must reach the signal as REAL x, not as 0.
    let out = msgs(
        r#"
module top;
  logic [3:0] s;
  logic [5:0] y;
  logic [7:0] w;
  always_comb begin
    case (s)
      4'd1: y = 6'h15;
      4'd2: y = 6'h2A;
      default: y = 6'bx;
    endcase
  end
  always_comb begin
    w = 8'h00;
    if (s == 4'd3) w[7:4] = 4'bx;
    else           w[7:4] = 4'hC;
  end
  initial begin
    s = 4'd1; #1 $display("A_%0h_%0h", y, w);
    s = 4'd2; #1 $display("B_%0h_%0h", y, w);
    s = 4'd9; #1 $display("C_%0h_%0h", y, w);
    s = 4'd3; #1 $display("D_%0h_%0h", y, w);
  end
endmodule
"#,
    );
    assert!(out.contains(&"A_15_c0".to_string()), "{out:?}");
    assert!(out.contains(&"B_2a_c0".to_string()), "{out:?}");
    // §21.2.1.3: six unknown bits form two x digits. s=4'd3 matches no arm, so
    // `y` takes the default too — the x reaching BOTH a whole-signal store
    // and a [7:4] range store is exactly what the fold has to preserve.
    assert!(out.contains(&"C_xx_c0".to_string()), "{out:?}");
    assert!(out.contains(&"D_xx_x0".to_string()), "{out:?}");
}

#[test]
fn temp_reused_at_different_widths_per_arm() {
    // Each arm defines and consumes its own value; no use is reached by two
    // definitions, so the differing widths are safe to track in stream order.
    let out = msgs(
        r#"
module top;
  logic [2:0] s;
  logic [15:0] y;
  always_comb begin
    case (s)
      3'd0: y = {4{4'h3}};
      3'd1: y = {8{2'b10}};
      3'd2: y = {2{8'hA5}};
      3'd3: y = {16{1'b1}};
      default: y = 16'h0000;
    endcase
  end
  initial begin
    s = 3'd0; #1 $display("A_%04h", y);
    s = 3'd1; #1 $display("B_%04h", y);
    s = 3'd2; #1 $display("C_%04h", y);
    s = 3'd3; #1 $display("D_%04h", y);
    s = 3'd6; #1 $display("E_%04h", y);
  end
endmodule
"#,
    );
    assert!(out.contains(&"A_3333".to_string()), "{out:?}");
    assert!(out.contains(&"B_aaaa".to_string()), "{out:?}");
    assert!(out.contains(&"C_a5a5".to_string()), "{out:?}");
    assert!(out.contains(&"D_ffff".to_string()), "{out:?}");
    assert!(out.contains(&"E_0000".to_string()), "{out:?}");
}

/// A dynamic bit-select write into a >64-bit destination lowers now
/// (`BitStoreDyn` with a wide `w`): the one bit is set in the wide planes and
/// every other bit — X included — survives. Values are checked through the
/// display so the result is the same whichever path evaluates the block.
#[test]
fn dynamic_bit_write_into_wide_destination() {
    let out = msgs(
        r#"
module tb;
  logic [199:0] bus;
  logic [7:0] idx;
  logic v;
  always @* begin
    bus = {200{1'bx}};
    bus[7:0] = 8'h5a;
    bus[idx] = v;
  end
  initial begin
    idx = 8'd3; v = 1'b1; #1;
    $display("A=%b %h", bus[199:196], bus[7:0]);
    idx = 8'd130; v = 1'b1; #1;
    $display("B=%b %h %b", bus[131:129], bus[7:0], bus[199]);
    idx = 8'd130; v = 1'b0; #1;
    $display("C=%b", bus[131:129]);
    idx = 8'd250; v = 1'b1; #1;
    $display("D=%h %b", bus[7:0], bus[131:129]);
    $finish;
  end
endmodule
"#,
    );
    assert!(out.iter().any(|m| m == "A=xxxx 5a"), "{out:?}");
    assert!(out.iter().any(|m| m == "B=x1x 5a x"), "{out:?}");
    assert!(out.iter().any(|m| m == "C=x0x"), "{out:?}");
    assert!(out.iter().any(|m| m == "D=5a xxx"), "{out:?}");
}

/// The lowering-shaped variant: the block does nothing but the dynamic bit
/// write into a 200-bit bus (the C906 decode-bus shape), so it runs on the
/// two-state path when the operands are clean. The rest of the bus is set
/// by another process and must be preserved bit for bit.
#[test]
fn dynamic_bit_write_only_block_on_wide_bus() {
    let out = msgs(
        r#"
module tb;
  logic [199:0] bus2;
  logic [7:0] idx;
  logic v;
  always @* bus2[idx] = v;
  initial begin
    bus2 = {25{8'ha5}};
    idx = 8'd0; v = 1'b0; #1;
    $display("E=%h", bus2[7:0]);
    idx = 8'd199; v = 1'b0; #1;
    $display("F=%h %h", bus2[199:192], bus2[7:0]);
    idx = 8'd64; v = 1'b0; #1;
    $display("G=%h", bus2[71:64]);
    idx = 8'd64; v = 1'b1; #1;
    $display("H=%h %h", bus2[71:64], bus2[199:192]);
    $finish;
  end
endmodule
"#,
    );
    assert!(out.iter().any(|m| m == "E=a4"), "{out:?}");
    assert!(out.iter().any(|m| m == "F=25 a4"), "{out:?}");
    assert!(out.iter().any(|m| m == "G=a4"), "{out:?}");
    assert!(out.iter().any(|m| m == "H=a5 25"), "{out:?}");
}

/// Constant-bound part-select writes into a >64-bit destination lower now
/// (`RangeStoreW` / `RangeStoreXW`): a 64-bit slice copy between two wide
/// buses (the C906 decode-bus shape), a slice above bit 64, and a folded
/// x constant into a wide bus. Untouched bits keep their value and X.
#[test]
fn range_writes_into_wide_destination() {
    let out = msgs(
        r#"
module tb;
  logic [310:0] src;
  logic [179:0] dst;
  logic [199:0] xb;
  always @* dst[63:0] = src[127:64];
  always @* dst[131:100] = src[31:0];
  always @* begin
    xb[7:0] = src[7:0];
    xb[71:64] = 8'bxx01_10xx;
  end
  initial begin
    dst = {180{1'b1}};
    xb = '0;
    src = {311{1'b0}};
    src[127:64] = 64'h0123_4567_89ab_cdef;
    src[31:0] = 32'hfeed_beef;
    #1 $display("A=%h %h %h", dst[63:0], dst[131:100], dst[179:132]);
    $display("B=%b %h", xb[71:64], xb[7:0]);
    src[127:64] = 64'hffff_0000_ffff_0000;
    #1 $display("C=%h %h", dst[63:0], dst[99:64]);
    $finish;
  end
endmodule
"#,
    );
    assert!(
        out.iter()
            .any(|m| m == "A=0123456789abcdef feedbeef ffffffffffff"),
        "{out:?}"
    );
    assert!(out.iter().any(|m| m == "B=xx0110xx ef"), "{out:?}");
    assert!(
        out.iter().any(|m| m == "C=ffff0000ffff0000 fffffffff"),
        "{out:?}"
    );
}
