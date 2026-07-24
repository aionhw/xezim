//! §7.4.1 / §6.10 — a NET declared with packed dimensions but no explicit type
//! keyword (`wire [1:0][7:0] w;`) parses as `DataType::Implicit`, not
//! `IntegerVector`. The packed-dimension metadata helpers only matched
//! `IntegerVector`, so for such a net xezim registered neither the per-element
//! width nor the declared dims: `w[i]` degraded to a BIT-select and a
//! continuous assign to `w[i]` could not resolve to a slice and was silently
//! DROPPED — the net stayed undriven (z). The same declaration written
//! `logic [1:0][7:0]` worked, which is what made it look type-specific.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    simulate(src, 1000)
        .expect("sim")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect()
}

#[test]
fn packed_2d_net_element_continuous_assign() {
    // Oracle-verified: wm[0] drives bits [7:0], wm[1] bits [15:8].
    let out = lines(
        r#"
module tb;
  wire [7:0] w0 = 8'h12, w1 = 8'h34;
  wire [1:0][7:0] wm;
  assign wm[0] = w0;
  assign wm[1] = w1;
  initial begin
    #1 $display("R %h %h %h", wm, wm[0], wm[1]);
  end
endmodule
"#,
    );
    assert!(
        out.iter().any(|l| l == "R 3412 12 34"),
        "packed-2D net element assign/read wrong: {:?}",
        out
    );
}

#[test]
fn packed_2d_net_matches_variable_form() {
    // The `logic` (IntegerVector) spelling must agree with the `wire`
    // (Implicit) spelling.
    let out = lines(
        r#"
module tb;
  wire  [7:0] a = 8'hAB, b = 8'hCD;
  wire  [1:0][7:0] wn;
  logic [1:0][7:0] vn;
  assign wn[0] = a; assign wn[1] = b;
  assign vn[0] = a; assign vn[1] = b;
  initial begin
    #1 $display("EQ %h %h", wn, vn);
  end
endmodule
"#,
    );
    assert!(
        out.iter().any(|l| l == "EQ cdab cdab"),
        "net and variable packed-2D forms disagree: {:?}",
        out
    );
}

#[test]
fn packed_3d_net_element_assign() {
    // Three elements + a deeper packed nest, all through the net path.
    let out = lines(
        r#"
module tb;
  wire [2:0][7:0] t;
  assign t[0] = 8'h11;
  assign t[1] = 8'h22;
  assign t[2] = 8'h33;
  initial begin
    #1 $display("T %h %h %h %h", t, t[0], t[1], t[2]);
  end
endmodule
"#,
    );
    assert!(
        out.iter().any(|l| l == "T 332211 11 22 33"),
        "packed-3-element net assign wrong: {:?}",
        out
    );
}

#[test]
fn packed_element_assign_non_normalized_range() {
    // §7.4.1: the element LSB offset is (idx - low_bound) * elem_w. The plain
    // `idx * elem_w` is right only for a normalized [N-1:0] range — for
    // `[2:1]` it wrote element 1 into the HIGH slot and dropped element 2.
    // Oracle-verified.
    let out = lines(
        r#"
module tb;
  wire [2:1][7:0] nn; assign nn[1] = 8'h12; assign nn[2] = 8'h34;
  wire [3:1][7:0] d3; assign d3[1] = 8'h11; assign d3[2] = 8'h22; assign d3[3] = 8'h33;
  initial begin
    #1 $display("N %h %h %h %h", nn, nn[1], nn[2], d3);
  end
endmodule
"#,
    );
    assert!(
        out.iter().any(|l| l == "N 3412 12 34 332211"),
        "non-normalized packed range element assign wrong: {:?}",
        out
    );
}

#[test]
fn packed_element_assign_dynamic_non_normalized_index() {
    // Same normalization, but with a RUNTIME index.
    let out = lines(
        r#"
module tb;
  logic [2:1][7:0] w;
  int i;
  initial begin
    for (i = 1; i <= 2; i++) w[i] = i * 8'h11;
    #1 $display("D %h", w);
  end
endmodule
"#,
    );
    assert!(out.iter().any(|l| l == "D 2211"), "dynamic non-normalized index wrong: {:?}", out);
}

#[test]
fn continuous_assign_to_unpacked_array_of_packed_element() {
    // §7.4: unpacked dims are indexed first, so `v[i][j]` consumes one
    // unpacked index then one packed index. The lvalue width inference did
    // not subtract the unpacked depth, so the RHS was resized to 1 bit and
    // the assign wrote the LSB (0x12 -> 0). Oracle-verified.
    let out = lines(
        r#"
module tb;
  wire  [1:0][7:0] w [0:1];
  logic [1:0][7:0] v [0:1];
  assign w[0][0] = 8'hAB; assign w[0][1] = 8'hCD;
  assign v[0][0] = 8'h12; assign v[0][1] = 8'h34;
  initial begin
    #1 $display("U %h %h", w[0], v[0]);
  end
endmodule
"#,
    );
    assert!(
        out.iter().any(|l| l == "U cdab 3412"),
        "cont-assign to unpacked-array-of-packed element wrong: {:?}",
        out
    );
}
