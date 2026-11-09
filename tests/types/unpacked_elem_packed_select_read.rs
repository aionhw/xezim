//! IEEE 1800-2017 §7.4.1, §7.4.5, §11.5.1: an index past an unpacked array's
//! dimensions selects into the element's packed dimensions. On
//! `logic [1:0][7:0] d [2]`, `d[1][0]` is the low byte of element `d[1]`.
//! Compiled reads (an `always` block, a continuous assignment) took the
//! first indices as packed selects of the array's root, or bit-selected the
//! element, so the byte read `0X`, `00` or one bit. Writes and `initial`
//! reads were right. Expected values are from the reference simulator.

use xezim::simulate;

fn messages(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn assert_line(msgs: &[String], want: &str) {
    assert!(
        msgs.iter().any(|m| m.trim() == want),
        "missing {want:?} in {msgs:#?}"
    );
}

#[test]
fn continuous_and_procedural_reads() {
    let msgs = messages(
        "module t;
  logic clk = 0;
  logic [1:0][39:0] d [2];
  logic [1:0][39:0] a [2][2];
  wire [39:0] cd = d[0][1];
  wire [39:0] ca = a[0][0][1];
  logic [39:0] pd, pa;
  initial begin
    d[0] = {40'hab00000001, 40'h0};
    a[0][0] = {40'hab00000001, 40'h0};
    #1 clk = 1;
    #1 $display(\"cont d=%h a=%h proc d=%h a=%h\", cd, ca, pd, pa);
  end
  always @(posedge clk) begin pd = d[0][1]; pa = a[0][0][1]; end
endmodule",
    );
    assert_line(
        &msgs,
        "cont d=ab00000001 a=ab00000001 proc d=ab00000001 a=ab00000001",
    );
}

#[test]
fn one_two_and_three_unpacked_dimensions() {
    let msgs = messages(
        "module t;
  logic clk = 0;
  logic [1:0][7:0] d [2];
  logic [1:0][7:0] a [2][2];
  logic [1:0][7:0] m [2][2][2];
  logic [7:0] r1, r2, r3, r4;
  initial begin
    d[1] = 16'h1234; a[1][1] = 16'h5678; m[1][1][0] = 16'h9abc;
    #1 clk = 1;
    #1 $display(\"r1=%h r2=%h r3=%h r4=%h\", r1, r2, r3, r4);
  end
  always @(posedge clk) begin
    r1 = d[1][0]; r2 = d[1][1]; r3 = a[1][1][1]; r4 = m[1][1][0][1];
  end
endmodule",
    );
    assert_line(&msgs, "r1=34 r2=12 r3=56 r4=9a");
}

#[test]
fn dynamic_indices() {
    let msgs = messages(
        "module t;
  logic clk = 0;
  logic [1:0][7:0] d [2];
  logic [1:0][7:0] a [2][2];
  logic [7:0] r1, r2;
  int i = 1, j = 1, k = 0;
  initial begin
    d[1] = 16'h1234; a[1][1] = 16'h5678;
    #1 clk = 1;
    #1 $display(\"r1=%h r2=%h\", r1, r2);
  end
  always @(posedge clk) begin r1 = d[i][j]; r2 = a[i][j][k]; end
endmodule",
    );
    assert_line(&msgs, "r1=12 r2=78");
}

#[test]
fn ascending_offset_and_deeper_packed_dimensions() {
    let msgs = messages(
        "module t;
  logic clk = 0;
  logic [0:1][7:0] u [2];
  logic [2:1][7:0] o [1:2];
  logic [1:0][1:0][3:0] n [2];
  logic [7:0] r1, r2, r3; logic [3:0] r4;
  initial begin
    u[1] = 16'hdef0; o[2] = 16'h4321; n[1] = 16'h8765;
    #1 clk = 1;
    #1 $display(\"r1=%h r2=%h r3=%h r4=%h\", r1, r2, r3, r4);
  end
  always @(posedge clk) begin
    r1 = u[1][0]; r2 = o[2][1]; r3 = n[1][1]; r4 = n[1][1][0];
  end
endmodule",
    );
    assert_line(&msgs, "r1=de r2=21 r3=87 r4=7");
}

#[test]
fn part_select_inside_the_sub_element() {
    let msgs = messages(
        "module t;
  logic clk = 0;
  logic [1:0][7:0] d [2];
  logic [3:0] r;
  initial begin
    d[1] = 16'h1234;
    #1 clk = 1;
    #1 $display(\"r=%h\", r);
  end
  always @(posedge clk) r = d[1][1][7:4];
endmodule",
    );
    assert_line(&msgs, "r=1");
}

#[test]
fn read_back_after_a_compiled_write() {
    let msgs = messages(
        "module t;
  logic clk = 0;
  logic [1:0][7:0] d [2];
  logic [1:0][7:0] a [2][2];
  logic [7:0] w1, w2;
  initial begin
    #1 clk = 1;
    #1 $display(\"w1=%h w2=%h\", w1, w2);
  end
  always @(posedge clk) begin
    a[0][1][1] = 8'hee; w1 = a[0][1][1];
    d[0][0] = 8'hcc; w2 = d[0][0];
  end
endmodule",
    );
    assert_line(&msgs, "w1=ee w2=cc");
}

#[test]
fn queue_dynamic_array_and_typedef_elements() {
    let msgs = messages(
        "module t;
  logic clk = 0;
  logic [1:0][7:0] q [$];
  logic [1:0][7:0] y [];
  typedef logic [1:0][7:0] w_t;
  w_t z [2];
  logic [7:0] r1, r2, r3;
  initial begin
    q.push_back(16'h1234); y = new[2]; y[1] = 16'h5678; z[1] = 16'h9abc;
    #1 clk = 1;
    #1 $display(\"r1=%h r2=%h r3=%h\", r1, r2, r3);
  end
  always @(posedge clk) begin r1 = q[0][1]; r2 = y[1][0]; r3 = z[1][1]; end
endmodule",
    );
    assert_line(&msgs, "r1=12 r2=78 r3=9a");
}

/// The packed elements are packed structs: the element type adds no packed
/// dimension, so a select of one struct is the element width over the
/// array's element count, and a further select goes inside the struct.
#[test]
fn packed_struct_elements() {
    let msgs = messages(
        "package q; typedef struct packed { logic [31:0] pc; logic [7:0] m; } ent_t; endpackage
module t;
  typedef struct packed { logic [31:0] pc; logic [7:0] m; } le_t;
  logic clk = 0;
  q::ent_t [1:0] a [2];
  le_t [0:1] u [2];
  le_t [1:0] b [2][2];
  le_t [2:1] o [1:2];
  struct packed { logic [15:0] x; logic [7:0] y; } [1:0] c [2];
  logic [39:0] r1, r2, r3, r4; logic [23:0] r5; logic [3:0] r6; logic r7; logic [7:0] r8; logic [39:0] r9; logic eq;
  int i = 1, j = 0;
  initial begin
    a[0] = {40'hDEAD0000A5, 40'h1122334455};
    u[1] = {40'hABCDEF0123, 40'h4567890ABC};
    b[1][0] = {40'h0102030405, 40'hA0B0C0D0E0};
    c[1] = {24'h123456, 24'h789ABC};
    o[2] = {40'h1111111111, 40'h2222222222};
    #1 clk = 1;
    #1 $display(\"r1=%h r2=%h r3=%h r4=%h r5=%h r6=%h r7=%b r8=%h r9=%h eq=%b\",
                r1, r2, r3, r4, r5, r6, r7, r8, r9, eq);
  end
  always @(posedge clk) begin
    r1 = a[0][1]; r2 = a[j][i - 1]; r3 = u[1][0]; r4 = b[1][0][0];
    r5 = c[i][j]; r6 = a[0][1][7:4]; r7 = a[0][0][0]; r8 = a[0][1].m; r9 = o[2][1];
    eq = a[0][1] === 40'hDEAD0000A5;
  end
endmodule",
    );
    assert_line(
        &msgs,
        "r1=dead0000a5 r2=1122334455 r3=abcdef0123 r4=a0b0c0d0e0 r5=789abc r6=a r7=1 r8=a5 r9=2222222222 eq=1",
    );
}

/// Inline struct types whose packed ranges are recorded: a select of one
/// struct, a row of structs and a struct inside a row, with constant and
/// run-time indices, in a block of its own so it stays compiled.
#[test]
fn struct_elements_with_recorded_ranges() {
    let msgs = messages(
        "module t;
  logic clk = 0;
  struct packed { logic [31:0] pc; logic [7:0] m; } [0:1] x [2];
  struct packed { logic [31:0] pc; logic [7:0] m; } [2:1] z [2];
  struct packed { logic [15:0] p; logic [7:0] q; } [1:0][1:0] y [2];
  logic [39:0] r1, r2, r3; logic [47:0] r4; logic [23:0] r5; logic [3:0] r6; logic eq;
  int i = 1, j = 0;
  initial begin
    x[1] = {40'hABCDEF0123, 40'h4567890ABC};
    z[0] = {40'h1111111111, 40'h2222222222};
    y[1] = {24'h010203, 24'h040506, 24'h0708A9, 24'h0BCDEF};
    #1 clk = 1;
    #1 $display(\"r1=%h r2=%h r3=%h r4=%h r5=%h r6=%h eq=%b\", r1, r2, r3, r4, r5, r6, eq);
  end
  always @(posedge clk) begin
    r1 = x[1][0]; r2 = x[i][i]; r3 = z[0][2]; r4 = y[1][1]; r5 = y[i][j][i];
    r6 = x[1][0][7:4]; eq = z[j][1] === 40'h2222222222;
  end
endmodule",
    );
    assert_line(
        &msgs,
        "r1=abcdef0123 r2=4567890abc r3=1111111111 r4=010203040506 r5=0708a9 r6=2 eq=1",
    );
}

/// Guard: selects that already read right in compiled code.
#[test]
fn selects_that_already_work() {
    let msgs = messages(
        "module t;
  logic clk = 0;
  logic [7:0] v [2];
  logic [1:0][7:0] p;
  logic [1:0][7:0] d [2];
  logic b1, b2; logic [7:0] e1; logic [15:0] e2; logic [3:0] e3;
  int i = 1;
  initial begin
    v[1] = 8'h0a; p = 16'h1234; d[1] = 16'h5678;
    #1 clk = 1;
    #1 $display(\"b1=%b b2=%b e1=%h e2=%h e3=%h\", b1, b2, e1, e2, e3);
  end
  always @(posedge clk) begin
    b1 = v[1][3]; b2 = v[i][1]; e1 = p[1]; e2 = d[i]; e3 = p[0][7:4];
  end
endmodule",
    );
    assert_line(&msgs, "b1=1 b2=1 e1=12 e2=5678 e3=3");
}
