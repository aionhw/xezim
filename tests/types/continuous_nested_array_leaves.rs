//! Continuous assignments to nested unpacked leaves and hierarchical name-resolution guards.

fn check(source: &str, expected: &[&str]) {
    let result = xezim::simulate(source, 10_000).expect("simulate continuous assignment");
    let actual: Vec<_> = result
        .output
        .iter()
        .map(|entry| entry.message.trim())
        .collect();
    assert_eq!(actual, expected);
}

/// Each pattern item uses the type of its destination (§10.3.2, §10.9.1, §10.9.2).
#[test]
fn typed_nested_assignment_pattern() {
    check(
        r#"
`timescale 1ns/1ps
module top;
  typedef struct { int a[1]; } I;
  typedef struct { I e[1]; } S;
  S s;
  int x;
  assign s = '{e:'{default:I'{a:'{default:x}}}};
  initial begin
    x = 7;
    #1 $display("a=%0d", s.e[0].a[0]);
    $finish;
  end
endmodule
        "#,
        &["a=7"],
    );
}

/// The selected int receives each full-width value (§7.4.2, §10.3.2).
#[test]
fn direct_nested_leaf() {
    check(
        r#"
`timescale 1ns/1ps
module top;
  typedef struct { int a[1]; } I;
  typedef struct { I e[1]; } S;
  S s;
  int x;
  assign s.e[0].a[0] = x;
  initial begin
    x = 7;
    #1 $display("value %0d", s.e[0].a[0]);
    #1 x = 10;
    #1 $display("value %0d", s.e[0].a[0]);
    $finish;
  end
endmodule
        "#,
        &["value 7", "value 10"],
    );
}

/// $clog2(2) selects the second element (§7.4.2, §20.8.1).
#[test]
fn constant_index_on_nested_leaf() {
    check(
        r#"
`timescale 1ns/1ps
module top;
  typedef struct { int a[2]; } I;
  typedef struct { I e[2]; } S;
  S s;
  int x;
  assign s.e[1].a[$clog2(2)] = x;
  initial begin
    x = 7;
    #1 $display("value %0d %0d", s.e[1].a[0], s.e[1].a[1]);
    #1 x = 10;
    #1 $display("value %0d %0d", s.e[1].a[0], s.e[1].a[1]);
    $finish;
  end
endmodule
        "#,
        &["value 0 7", "value 0 10"],
    );
}

/// The assignment drives the selected element in the loop instance (§10.3.2, §27.4).
#[test]
fn element_of_generate_array() {
    check(
        r#"
`timescale 1ns/1ps
module top;
  for (genvar i = 0; i < 1; i++) begin : g
    int mi[2];
  end
  int x;
  assign g[0].mi[1] = x;
  initial begin
    x = 7;
    #1 $display("value %0d %0d", g[0].mi[0], g[0].mi[1]);
    #1 x = 10;
    #1 $display("value %0d %0d", g[0].mi[0], g[0].mi[1]);
    $finish;
  end
endmodule
        "#,
        &["value 0 7", "value 0 10"],
    );
}

/// The selected instance receives full-width drives (§10.3.2, §23.6).
#[test]
fn element_in_instance_array() {
    check(
        r#"
`timescale 1ns/1ps
module bank;
  int mem[4];
endmodule
module top;
  bank u[2]();
  int x;
  assign u[1].mem[3] = x;
  initial begin
    x = 7;
    #1 $display("value %0d %0d", u[0].mem[3], u[1].mem[3]);
    #1 x = 10;
    #1 $display("value %0d %0d", u[0].mem[3], u[1].mem[3]);
    $finish;
  end
endmodule
        "#,
        &["value 0 7", "value 0 10"],
    );
}

/// The selected interface instance receives full-width drives (§10.3.2, §25.3).
#[test]
fn element_in_interface_array() {
    check(
        r#"
`timescale 1ns/1ps
interface bus;
  int mem[4];
endinterface
module top;
  bus b[2]();
  int x;
  assign b[1].mem[3] = x;
  initial begin
    x = 7;
    #1 $display("value %0d %0d", b[0].mem[3], b[1].mem[3]);
    #1 x = 10;
    #1 $display("value %0d %0d", b[0].mem[3], b[1].mem[3]);
    $finish;
  end
endmodule
        "#,
        &["value 0 7", "value 0 10"],
    );
}

/// Disjoint packed selects of an element can have separate drivers (§6.5, §11.5.1).
#[test]
fn packed_selects_of_nested_leaf() {
    check(
        r#"
`timescale 1ns/1ps
module top;
  typedef struct { logic [7:0] a[1]; } I;
  typedef struct { I e[1]; } S;
  S s;
  logic x;
  assign s.e[0].a[0][7:1] = 7'h52;
  assign s.e[0].a[0][0] = x;
  initial begin
    x = 1'b1;
    #1 $display("value %h", s.e[0].a[0]);
    #1 x = 1'b0;
    #1 $display("value %h", s.e[0].a[0]);
    $finish;
  end
endmodule
        "#,
        &["value a5", "value a4"],
    );
}

/// The module qualifier reaches the enclosing module (§10.3.2, §23.8).
#[test]
fn module_qualified_nested_leaf() {
    check(
        r#"
`timescale 1ns/1ps
module top;
  typedef struct { int a[1]; } I;
  typedef struct { I e[1]; } S;
  S s;
  int x;
  assign top.s.e[0].a[0] = x;
  initial begin
    x = 7;
    #1 $display("value %0d", s.e[0].a[0]);
    #1 x = 10;
    #1 $display("value %0d", s.e[0].a[0]);
    $finish;
  end
endmodule
        "#,
        &["value 7", "value 10"],
    );
}

/// The child gets the full-width value and the enclosing element stays zero (§10.3.2, §23.8).
#[test]
fn generated_int_element_in_same_named_child() {
    check(
        r#"
`timescale 1ns/1ps
module child;
  for (genvar i = 0; i < 1; i++) begin : g
    int mi[2];
  end
endmodule
module top;
  for (genvar i = 0; i < 1; i++) begin : g
    int mi[2];
  end
  int x;
  child top();
  assign top.g[0].mi[1] = x;
  initial begin
    x = 7;
    #1 $display("value %0d %0d", g[0].mi[1], top.g[0].mi[1]);
    #1 x = 10;
    #1 $display("value %0d %0d", g[0].mi[1], top.g[0].mi[1]);
    $finish;
  end
endmodule
        "#,
        &["value 0 7", "value 0 10"],
    );
}

/// $root names the top-level instance explicitly (§23.3.1).
#[test]
fn root_target_with_same_named_module_instance() {
    check(
        r#"
`timescale 1ns/1ps
module child;
  typedef struct { int a[1]; } I;
  typedef struct { I e[1]; } S;
  S s;
endmodule
module top;
  typedef struct { int a[1]; } I;
  typedef struct { I e[1]; } S;
  S s;
  int x;
  child top();
  assign $root.top.s.e[0].a[0] = x;
  initial begin
    x = 7;
    #1 $display("value %0d %0d", s.e[0].a[0], top.s.e[0].a[0]);
    #1 x = 10;
    #1 $display("value %0d %0d", s.e[0].a[0], top.s.e[0].a[0]);
    $finish;
  end
endmodule
        "#,
        &["value 7 0", "value 10 0"],
    );
}

/// $root names the top-level instance explicitly (§23.3.1).
#[test]
fn root_target_with_same_named_interface_instance() {
    check(
        r#"
`timescale 1ns/1ps
interface bus;
  typedef struct { int a[1]; } I;
  typedef struct { I e[1]; } S;
  S s;
endinterface
module top;
  typedef struct { int a[1]; } I;
  typedef struct { I e[1]; } S;
  S s;
  int x;
  bus top();
  assign $root.top.s.e[0].a[0] = x;
  initial begin
    x = 7;
    #1 $display("value %0d %0d", s.e[0].a[0], top.s.e[0].a[0]);
    #1 x = 10;
    #1 $display("value %0d %0d", s.e[0].a[0], top.s.e[0].a[0]);
    $finish;
  end
endmodule
        "#,
        &["value 7 0", "value 10 0"],
    );
}

/// $root reaches the top level even when a generate block has the same name (§23.3.1, §27.5).
#[test]
fn root_target_with_same_named_generate_block() {
    check(
        r#"
`timescale 1ns/1ps
module top;
  typedef struct { int a[1]; } I;
  typedef struct { I e[1]; } S;
  S s;
  int x;
  if (1) begin : top
    S s;
  end
  assign $root.top.s.e[0].a[0] = x;
  initial begin
    x = 7;
    #1 $display("value %0d %0d", s.e[0].a[0], top.s.e[0].a[0]);
    #1 x = 10;
    #1 $display("value %0d %0d", s.e[0].a[0], top.s.e[0].a[0]);
    $finish;
  end
endmodule
        "#,
        &["value 7 0", "value 10 0"],
    );
}

/// $root reaches the top level even when a variable has the same name (§7.2, §23.3.1).
#[test]
fn root_target_with_same_named_variable() {
    check(
        r#"
`timescale 1ns/1ps
module top;
  typedef struct { int a[1]; } I;
  typedef struct { I e[1]; } S;
  S top;
  I e[1];
  int x;
  assign $root.top.e[0].a[0] = x;
  initial begin
    x = 7;
    #1 $display("value %0d %0d", e[0].a[0], top.e[0].a[0]);
    #1 x = 10;
    #1 $display("value %0d %0d", e[0].a[0], top.e[0].a[0]);
    $finish;
  end
endmodule
        "#,
        &["value 7 0", "value 10 0"],
    );
}

/// The local instance name resolves to the child (§23.8, §27.4).
#[test]
fn generated_bit_element_in_same_named_child() {
    check(
        r#"
`timescale 1ns/1ps
module child;
  for (genvar i = 0; i < 1; i++) begin : g
    bit mi[2];
  end
endmodule
module top;
  for (genvar i = 0; i < 1; i++) begin : g
    bit mi[2];
  end
  bit x;
  child top();
  assign top.g[0].mi[1] = x;
  initial begin
    x = 1'b1;
    #1 $display("value %0d %0d", g[0].mi[1], top.g[0].mi[1]);
    #1 x = 1'b0;
    #1 $display("value %0d %0d", g[0].mi[1], top.g[0].mi[1]);
    $finish;
  end
endmodule
        "#,
        &["value 0 1", "value 0 0"],
    );
}

/// The local instance name resolves to the child (§7.4.2, §23.8).
#[test]
fn array_element_in_same_named_child() {
    check(
        r#"
`timescale 1ns/1ps
module child;
  int a[2];
endmodule
module top;
  child top();
  int a[2];
  int x;
  assign top.a[1] = x;
  initial begin
    x = 7;
    #1 $display("value %0d %0d", a[1], top.a[1]);
    #1 x = 10;
    #1 $display("value %0d %0d", a[1], top.a[1]);
    $finish;
  end
endmodule
        "#,
        &["value 0 7", "value 0 10"],
    );
}
