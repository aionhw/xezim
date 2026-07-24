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
