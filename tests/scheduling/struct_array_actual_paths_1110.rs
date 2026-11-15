//! §10.3.2, §13.5.4, §23.6, §23.9 (#1110): a continuous assignment or an
//! `always_comb` that passes a whole unpacked struct array to a function is
//! evaluated again when a member changes, however the actual names the
//! array: bound by name (`f(.a(a))`), through `$root` or the module's own
//! name (`top.a`), or in a generate block of a child module (`h.a`), which
//! names the child's block, not a same-named block of the top module.

fn check(source: &str, expected: &[&str]) {
    let actual: Vec<_> = xezim::simulate(source, 1_000_000)
        .expect("simulate")
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|s| s.starts_with("T|") || s.starts_with("PASS") || s.starts_with("FAIL"))
        .collect();
    assert_eq!(actual, expected);
}

#[test]
fn struct_array_actual_spellings_wake_the_call() {
    check(
        r#"
`timescale 1ns/1ps
typedef struct { int v; } S;
function automatic int f(input S a[2]); return int'($time); endfunction
module child;
  if (1) begin : h S a[2]; end
  int r_block;
  assign r_block = f(h.a);
endmodule
module top;
  S a[2];
  int r_named, r_root, r_top;
  assign r_named = f(.a(a));
  assign r_root = f($root.top.a);
  assign r_top = f(top.a);
  child c();
  if (1) begin : h S a[2]; end
  initial begin
    #1 a[1].v = 7; c.h.a[1].v = 7;
    #1 begin
      if (r_named === 1) $display("PASS named"); else $display("FAIL named r=%0d expected=1", r_named);
      if (r_root === 1) $display("PASS root"); else $display("FAIL root r=%0d expected=1", r_root);
      if (r_top === 1) $display("PASS top"); else $display("FAIL top r=%0d expected=1", r_top);
      if (c.r_block === 1) $display("PASS block"); else $display("FAIL block r=%0d expected=1", c.r_block);
    end
    $finish;
  end
endmodule
"#,
        &["PASS named", "PASS root", "PASS top", "PASS block"],
    );
}

#[test]
fn struct_array_actual_spellings_read_their_own_array() {
    check(
        r#"
`timescale 1ns/1ps
typedef struct { int v; } S;
function automatic int f(input S a[2]); return int'($time); endfunction
function automatic int g(input S a[2]); return a[1].v; endfunction
module child;
  if (1) begin : h S a[2]; end
  int r_block, v_block, c_block;
  assign r_block = f(h.a);
  assign v_block = g(h.a);
  always_comb c_block = g(h.a);
endmodule
module top;
  S a[2];
  int r_named, r_root, r_top, v_named, v_root, v_top, c_named, c_top;
  assign r_named = f(.a(a));
  assign r_root = f($root.top.a);
  assign r_top = f(top.a);
  assign v_named = g(.a(a));
  assign v_root = g($root.top.a);
  assign v_top = g(top.a);
  always_comb c_named = g(.a(a));
  always_comb c_top = g(top.a);
  child c();
  if (1) begin : h S a[2]; end
  initial begin
    #1 a[1].v = 7; c.h.a[1].v = 9; h.a[1].v = 5;
    #1;
    $display("T|r named=%0d root=%0d top=%0d block=%0d", r_named, r_root, r_top, c.r_block);
    $display("T|v named=%0d root=%0d top=%0d block=%0d", v_named, v_root, v_top, c.v_block);
    $display("T|c named=%0d top=%0d block=%0d", c_named, c_top, c.c_block);
    $finish;
  end
endmodule
"#,
        &[
            "T|r named=1 root=1 top=1 block=1",
            "T|v named=7 root=7 top=7 block=9",
            "T|c named=7 top=7 block=9",
        ],
    );
}

#[test]
fn child_generate_block_names_resolve_in_the_child() {
    check(
        r#"
`timescale 1ns/1ps
typedef struct { int v; } S;
function automatic int g(input S a[2]); return a[1].v; endfunction
function automatic int gi(input int a[2]); return a[1]; endfunction
module child;
  if (1) begin : h S a[2]; int x; int ia[2]; end
  int v_block, x_block, i_block, e_block, m_block;
  assign v_block = g(h.a);
  assign x_block = h.x;
  assign i_block = gi(h.ia);
  assign e_block = h.ia[1];
  assign m_block = h.a[1].v;
endmodule
module top;
  child c();
  if (1) begin : h S a[2]; int x; int ia[2]; end
  initial begin
    #1 c.h.a[1].v = 9; h.a[1].v = 5; c.h.x = 9; h.x = 5; c.h.ia[1] = 9; h.ia[1] = 5;
    #1;
    $display("T|v=%0d x=%0d i=%0d e=%0d m=%0d", c.v_block, c.x_block, c.i_block, c.e_block, c.m_block);
    $finish;
  end
endmodule
"#,
        &["T|v=9 x=9 i=9 e=9 m=9"],
    );
    check(
        r#"
module m; int x; endmodule
module child;
  m u();
  if (1) begin : h int x; end
  int y_inst, y_gen;
  assign y_inst = u.x;
  assign y_gen = h.x;
  always_comb begin end
endmodule
module top;
  child c();
  m u();
  if (1) begin : h int x; end
  initial begin
    #1 c.u.x = 9; u.x = 5; c.h.x = 9; h.x = 5;
    #1 $display("T|inst=%0d gen=%0d", c.y_inst, c.y_gen);
  end
endmodule
"#,
        &["T|inst=9 gen=9"],
    );
}
