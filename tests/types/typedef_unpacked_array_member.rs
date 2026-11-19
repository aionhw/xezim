//! IEEE 1800-2023 §6.18, §7.2, §7.4.2 and §7.4.5: a struct member
//! declared with an unpacked array typedef retains the typedef's indices and
//! element type. Exercise function returns (§13.4.1), nettype resolution
//! (§6.6.7), assignments (§10.3.2 and §10.4.1), and patterns (§10.9.1
//! and §10.9.2).

use xezim::simulate;

fn assert_output(src: &str, expected: &[&str]) {
    let sim = simulate(src, 10_000).expect("simulate failed");
    let output: Vec<_> = sim
        .output
        .iter()
        .map(|line| line.message.as_str())
        .collect();
    assert_eq!(output, expected);
}

fn direct_real_member(src: &str) -> String {
    src.replace(
        "  typedef real A[1];\n  typedef struct { A a; } S;",
        "  typedef struct { real a[1]; } S;",
    )
}

const FUNCTION: &str = r#"
module top;
  typedef real A[1];
  typedef struct { A a; } S;

  function automatic S f();
    f.a[0] = 7.0;
    $display("inside=%g", f.a[0]);
  endfunction

  S s;
  initial begin
    s = f();
    $display("after=%g", s.a[0]);
    $finish;
  end
endmodule
"#;

const NETTYPE: &str = r#"
`timescale 1ns/1ps
module top;
  typedef real A[1];
  typedef struct { A a; } S;

  function automatic S resolve(input S drivers[]);
    resolve.a[0] = 0.0;
    foreach (drivers[i])
      resolve.a[0] += drivers[i].a[0];
  endfunction

  nettype S N with resolve;
  N n;
  S s;
  assign n = s;

  initial begin
    s.a[0] = 7.0;
    #1;
    $display("a=%g", n.a[0]);
    $finish;
  end
endmodule
"#;

const PACKED_FUNCTION: &str = r#"
module top;
  typedef bit [1:0][3:0] A;
  typedef struct { A a; } S;

  function automatic S f();
    f.a[0] = 4'd7;
    f.a[1] = 4'd9;
    $display("inside=%0d %0d", f.a[0], f.a[1]);
  endfunction

  S s;
  initial begin
    s = f();
    $display("after=%0d %0d", s.a[0], s.a[1]);
    $finish;
  end
endmodule
"#;

const STRUCT_ARRAY_FUNCTION: &str = r#"
module top;
  typedef struct { real v; } I;
  typedef I IA[2];
  typedef struct { IA a; } S;

  function automatic S f();
    f.a[0].v = 7.0;
    f.a[1].v = 2.5;
    $display("inside=%g %g", f.a[0].v, f.a[1].v);
  endfunction

  S s;
  initial begin
    s = f();
    $display("after=%g %g", s.a[0].v, s.a[1].v);
    $finish;
  end
endmodule
"#;

const ASSIGNMENT: &str = r#"
`timescale 1ns/1ps
module top;
  typedef real A[1];
  typedef struct { A a; } S;
  S src, proc_copy, cont_elem, cont_copy;
  real value;

  assign cont_elem.a[0] = value;
  assign cont_copy = src;

  initial begin
    src.a[0] = 7.0;
    proc_copy = src;
    value = 2.5;
    #1;
    $display("procedural=%g proc_copy=%g continuous=%g cont_copy=%g",
             src.a[0], proc_copy.a[0], cont_elem.a[0], cont_copy.a[0]);
    $finish;
  end
endmodule
"#;

const FUNCTION_PATTERN: &str = r#"
module top;
  typedef real A[1];
  typedef struct { A a; } S;

  function automatic S f();
    f = '{a: '{7.0}};
    $display("inside=%g", f.a[0]);
  endfunction

  S s;
  initial begin
    s = f();
    $display("after=%g", s.a[0]);
    $finish;
  end
endmodule
"#;

#[test]
fn function_return_keeps_typedef_array_member() {
    assert_output(FUNCTION, &["inside=7", "after=7"]);
}

#[test]
fn resolver_keeps_typedef_array_member() {
    assert_output(NETTYPE, &["a=7"]);
}

#[test]
fn function_return_keeps_typedef_array_of_structs_member() {
    assert_output(STRUCT_ARRAY_FUNCTION, &["inside=7 2.5", "after=7 2.5"]);
}

#[test]
fn whole_struct_copy_keeps_typedef_array_member() {
    assert_output(
        ASSIGNMENT,
        &["procedural=7 proc_copy=7 continuous=2.5 cont_copy=7"],
    );
}

#[test]
fn function_pattern_keeps_typedef_array_member() {
    assert_output(FUNCTION_PATTERN, &["inside=7", "after=7"]);
}

#[test]
fn function_return_keeps_direct_array_member() {
    assert_output(&direct_real_member(FUNCTION), &["inside=7", "after=7"]);
}

#[test]
fn resolver_keeps_direct_array_member() {
    assert_output(&direct_real_member(NETTYPE), &["a=7"]);
}

#[test]
fn function_return_keeps_packed_typedef_member() {
    assert_output(PACKED_FUNCTION, &["inside=7 9", "after=7 9"]);
}

#[test]
fn function_return_keeps_direct_array_of_structs_member() {
    assert_output(
        &STRUCT_ARRAY_FUNCTION.replace(
            "  typedef I IA[2];\n  typedef struct { IA a; } S;",
            "  typedef struct { I a[2]; } S;",
        ),
        &["inside=7 2.5", "after=7 2.5"],
    );
}

#[test]
fn whole_struct_copy_keeps_direct_array_member() {
    assert_output(
        &direct_real_member(ASSIGNMENT),
        &["procedural=7 proc_copy=7 continuous=2.5 cont_copy=7"],
    );
}

#[test]
fn function_pattern_keeps_direct_array_member() {
    assert_output(
        &direct_real_member(FUNCTION_PATTERN),
        &["inside=7", "after=7"],
    );
}
