//! IEEE 1800-2023 §7.2.2, §7.4.2, §23.3.3.2. The port copies the
//! typed members of corresponding array elements in declared order
//! (§7.6, §23.3.3.5). Sample after each update.

use xezim::simulate;

fn check(source: &str, expected: &[&str]) {
    let sim = simulate(source, 100_000).expect("simulation failed");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|line| line.message.as_str())
        .collect();
    assert_eq!(lines, expected);
}

#[test]
fn input_members_follow_two_connections() {
    check(
        r#"
`timescale 1ns/1ps
package samples;
  typedef struct {real v; shortreal s; int k;} sample_t;
endpackage
module child(input var samples::sample_t a[8:7], output real v, s, output int k);
  assign v = a[8].v;
  assign s = a[7].s;
  assign k = a[7].k;
endmodule
module relay(input var samples::sample_t a[5:6], output real v, s, output int k);
  child c(a, v, s, k);
endmodule
module top;
  samples::sample_t a[4:1];
  real v, s;
  int k;
  relay r(a[3:2], v, s, k);
  initial begin
    #1 a[3].v = 7.25; a[2].s = -1.5; a[2].k = 11;
    #1 $display("v=%0g s=%0g k=%0d leaf=%0g", v, s, k, r.c.a[8].v);
    #1 a[3].v = 8.5; a[2].s = 2.25; a[2].k = 13;
    #1 $display("v=%0g s=%0g k=%0d leaf=%0g", v, s, k, r.c.a[8].v);
    $finish;
  end
endmodule
"#,
        &["v=7.25 s=-1.5 k=11 leaf=7.25", "v=8.5 s=2.25 k=13 leaf=8.5"],
    );
}

#[test]
fn output_member_reaches_the_parent() {
    check(
        r#"
`timescale 1ns/1ps
typedef struct {real v;} S;
module child(output var S a[1], output real q);
  initial #1 a[0].v = 7.25;
  assign q = a[0].v;
endmodule
module top;
  S a[1];
  real q;
  child c(a, q);
  initial begin
    #2;
    $display("child=%0g parent=%0g", q, a[0].v);
    $finish;
  end
endmodule
"#,
        &["child=7.25 parent=7.25"],
    );
}

#[test]
fn scalar_structure_port() {
    check(
        r#"
`timescale 1ns/1ps
typedef struct {real v;} S;
module child(input var S a, output real q);
  assign q = a.v;
endmodule
module top;
  S a;
  real q;
  child c(a, q);
  initial begin
    #1 a.v = 7.25;
    #1;
    $display("q=%0g", q);
    $finish;
  end
endmodule
"#,
        &["q=7.25"],
    );
}

#[test]
fn slice_of_real_array() {
    check(
        r#"
`timescale 1ns/1ps
module child(input var real a[8:7], output real q, r);
  assign q = a[8];
  assign r = a[7];
endmodule
module top;
  real a[1:4];
  real q, r;
  child c(a[2:3], q, r);
  initial begin
    #1 a[2] = 7.25;
    a[3] = 8.5;
    #1 $display("q=%0g r=%0g", q, r);
    $finish;
  end
endmodule
"#,
        &["q=7.25 r=8.5"],
    );
}

#[test]
fn slice_of_rank_two_structure_array() {
    check(
        r#"
`timescale 1ns/1ps
typedef struct {real v; int k;} S;
module child(input var S a[8:7][1], output real q, output int k);
  assign q = a[8][0].v;
  assign k = a[7][0].k;
endmodule
module top;
  S a[1:4][1];
  real q;
  int k;
  child c(a[2:3], q, k);
  initial begin
    #1 a[2][0].v = 7.25;
    a[3][0].k = 11;
    #1 $display("q=%0g k=%0d", q, k);
    #1 a[2][0].v = 8.5;
    a[3][0].k = 13;
    #1 $display("q=%0g k=%0d", q, k);
    $finish;
  end
endmodule
"#,
        &["q=7.25 k=11", "q=8.5 k=13"],
    );
}

/// §6.7.1, §23.2.2.3: the input defaults to a net and the typed output to a variable.
#[test]
fn implicit_input_and_output_carry_four_state_members() {
    check(
        r#"
`timescale 1ns/1ps
typedef struct {logic [3:0] bits;} S;
module child(input S a[2], output S b[2]);
  assign b[0].bits = a[1].bits;
endmodule
module top;
  S a[2], b[2];
  child c(a, b);
  initial begin
    #1 a[1].bits = 4'b10xz;
    #1 $display("input=%b output=%b", c.a[1].bits, b[0].bits);
    #1 a[1].bits = 4'b0101;
    #1 $display("input=%b output=%b", c.a[1].bits, b[0].bits);
    $finish;
  end
endmodule
"#,
        &["input=10xz output=10xz", "input=0101 output=0101"],
    );
}

/// §23.3.3.3: an unconnected input net reads Z.
#[test]
fn open_implicit_input_reads_high_impedance() {
    check(
        r#"
`timescale 1ns/1ps
typedef struct {logic [3:0] bits;} S;
module child(input S a[1], output wire [3:0] q);
  assign q = a[0].bits;
endmodule
module top;
  wire [3:0] q;
  child c(.a(), .q(q));
  initial begin
    #1 $display("open=%b", q);
    $finish;
  end
endmodule
"#,
        &["open=zzzz"],
    );
}

/// §7.10.2.1: the queue contains one element at the sampling time.
#[test]
fn queue_member_after_one_push() {
    check(
        r#"
`timescale 1ns/1ps
typedef struct {int data[]; int queue_values[$]; real v;} S;
module child(input var S a[2], output int size);
  always @* size = a[0].queue_values.size();
endmodule
module top;
  S a[2];
  int size;
  child c(a, size);
  initial begin
    #1 a[0].queue_values.push_back(4);
    #1 $display("size=%0d", size);
    $finish;
  end
endmodule
"#,
        &["size=1"],
    );
}
