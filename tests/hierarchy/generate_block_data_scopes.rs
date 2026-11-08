//! §27.5, §27.6: an unlabelled generate block is a scope like a labelled one.
//! Its variables, parameters, nets and instances belong to it, and its
//! implicit name `genblk<n>` reaches them from outside (§23.9). Verilator
//! 5.055 gives the value expected here on every program but the one with a
//! `genblk02` name, which it reports as unsupported.
use xezim::simulate;

/// The `T|` lines, sorted.
fn t_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 1000).unwrap_or_else(|e| panic!("rejected: {e}"));
    let mut lines: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect();
    lines.sort();
    lines
}

#[test]
fn two_unlabelled_blocks_each_own_their_variable() {
    let src = r#"
module top;
  if (1) begin
    int x = 1;
    initial #1 $display("T|a=%0d", x);
  end
  if (1) begin
    int x = 2;
    initial #1 $display("T|b=%0d", x);
  end
  initial #2 $display("T|%0d %0d", genblk1.x, genblk2.x);
endmodule
"#;
    assert_eq!(t_lines(src), ["T|1 2", "T|a=1", "T|b=2"]);
}

#[test]
fn an_unlabelled_blocks_variable_hides_the_modules() {
    let src = r#"
module top;
  int x = 7;
  if (1) begin
    int x = 3;
    initial #1 $display("T|in=%0d", x);
  end
  initial #2 $display("T|out=%0d", x);
endmodule
"#;
    assert_eq!(t_lines(src), ["T|in=3", "T|out=7"]);
}

#[test]
fn a_nested_unlabelled_block_hides_the_enclosing_blocks_variable() {
    let src = r#"
module top;
  if (1) begin : g
    int x = 1;
    if (1) begin
      int x = 2;
      initial #1 $display("T|inner=%0d", x);
    end
    initial #1 $display("T|outer=%0d", x);
  end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|inner=2", "T|outer=1"]);
}

#[test]
fn two_unlabelled_blocks_each_own_their_parameter() {
    let src = r#"
module top;
  if (1) begin
    localparam int K = 1;
    initial #1 $display("T|a=%0d", K);
  end
  if (1) begin
    localparam int K = 2;
    initial #1 $display("T|b=%0d", K);
  end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|a=1", "T|b=2"]);
}

#[test]
fn a_labelled_block_inside_two_unlabelled_ones_is_two_scopes() {
    let src = r#"
module top;
  if (1) begin
    if (1) begin : h int x = 1; initial #1 $display("T|a=%0d", x); end
  end
  if (1) begin
    if (1) begin : h int x = 2; initial #1 $display("T|b=%0d", x); end
  end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|a=1", "T|b=2"]);
}

#[test]
fn two_unlabelled_blocks_in_an_instance_each_own_their_variable() {
    let src = r#"
module m;
  if (1) begin
    int x = 1;
    initial #1 $display("T|a=%0d", x);
  end
  if (1) begin
    int x = 2;
    initial #1 $display("T|b=%0d", x);
  end
endmodule
module top; m u(); endmodule
"#;
    assert_eq!(t_lines(src), ["T|a=1", "T|b=2"]);
}

#[test]
fn an_unlabelled_blocks_variable_in_an_instance_hides_the_modules() {
    let src = r#"
module m;
  int x = 7;
  if (1) begin
    int x = 3;
    initial #1 $display("T|in=%0d", x);
  end
  initial #2 $display("T|out=%0d", x);
endmodule
module top; m u(); endmodule
"#;
    assert_eq!(t_lines(src), ["T|in=3", "T|out=7"]);
}

#[test]
fn an_implicit_port_in_a_labelled_block_reads_the_blocks_net_beside_an_unlabelled_one() {
    let src = r#"
module child(input logic [3:0] d, output logic [3:0] q);
  assign q = d + 1;
endmodule
module m;
  logic [3:0] d = 4'd1;
  if (1) begin
    logic [3:0] q;
    child u(.*);
    initial #1 $display("T|q=%0d", q);
  end
  if (1) begin : g
    logic [3:0] q;
    child u(.d, .q);
    initial #1 $display("T|gq=%0d", q);
  end
endmodule
module top; m i(); endmodule
"#;
    assert_eq!(t_lines(src), ["T|gq=2", "T|q=2"]);
}

#[test]
fn a_wildcard_in_an_unlabelled_block_of_an_instance_reads_the_blocks_net() {
    let src = r#"
module child(input logic [3:0] d, output logic [3:0] q);
  assign q = d + 1;
endmodule
module m;
  logic [3:0] d = 4'd1;
  if (1) begin
    logic [3:0] q;
    child u(.*);
    initial #1 $display("T|q=%0d", q);
  end
endmodule
module top; m i(); endmodule
"#;
    assert_eq!(t_lines(src), ["T|q=2"]);
}

#[test]
fn a_hierarchical_read_through_the_implicit_name_finds_the_blocks_variable() {
    let outside = r#"
module top;
  if (1) begin
    int x = 4;
  end
  initial #1 $display("T|%0d", genblk1.x);
endmodule
"#;
    assert_eq!(t_lines(outside), ["T|4"]);
    let nested = r#"
module top;
  if (1) begin
    int x = 4;
    if (1) begin : h
      initial #1 $display("T|%0d", x);
    end
  end
endmodule
"#;
    assert_eq!(t_lines(nested), ["T|4"]);
}

#[test]
fn two_unlabelled_case_arms_each_own_their_variable() {
    let src = r#"
module top;
  localparam S = 1;
  case (S) 1: begin int x = 1; initial #1 $display("T|a=%0d", x); end default: begin int x = 9; end endcase
  case (S) 1: begin int x = 2; initial #1 $display("T|b=%0d", x); end endcase
  initial #2 $display("T|%0d %0d", genblk1.x, genblk2.x);
endmodule
"#;
    assert_eq!(t_lines(src), ["T|1 2", "T|a=1", "T|b=2"]);
}

#[test]
fn two_unlabelled_blocks_each_own_their_net() {
    let src = r#"
module top;
  logic [3:0] d = 4'd3;
  if (1) begin wire [3:0] w = d + 1; initial #1 $display("T|a=%0d", w); end
  if (1) begin wire [3:0] w; assign w = d + 2; initial #1 $display("T|b=%0d", w); end
  initial #2 $display("T|%0d %0d", genblk1.w, genblk2.w);
endmodule
"#;
    assert_eq!(t_lines(src), ["T|4 5", "T|a=4", "T|b=5"]);
}

#[test]
fn an_implicit_name_that_a_declaration_takes_gets_a_leading_zero() {
    let src = r#"
module top;
  localparam genblk2 = 5;
  if (1) begin int x = 1; end
  if (1) begin int x = 2; end
  initial #1 $display("T|%0d %0d %0d", genblk1.x, genblk02.x, genblk2);
endmodule
"#;
    assert_eq!(t_lines(src), ["T|1 2 5"]);
}
