//! IEEE 1800-2023 §7.6, §10.3 (#1476 follow-up): a whole continuous copy
//! between fixed arrays of one unpacked structure type pairs the elements in
//! left-to-right order and drives their members, with one top as with two:
//! a multidimensional array, a row of one, a member array, an array of a
//! child instance on either side, and a type spelled through an import on
//! one side and qualified on the other. The expected lines are the
//! reference simulator's output.

use xezim::simulate;

fn check(source: &str, expected: &[&str]) {
    for tops in [
        source.to_string(),
        format!("{source}\nmodule other; endmodule\n"),
    ] {
        let sim = simulate(&tops, 20_000).expect("simulate");
        let output: Vec<_> = sim
            .output
            .iter()
            .map(|line| line.message.as_str())
            .filter(|line| line.starts_with("v="))
            .collect();
        assert_eq!(output, expected, "{tops}");
    }
}

#[test]
fn a_whole_copy_of_a_two_dimensional_array() {
    check(
        r#"
`timescale 1ns/1ps
module top;
  typedef struct { real v; } S;
  S a[4:3][2:1], b[0:1][2:1];
  assign b = a;
  initial begin
    a[4][2].v = 1.5; a[4][1].v = 2.25; a[3][2].v = 3.5; a[3][1].v = 4.25;
    #1; $display("v=%g,%g,%g,%g", b[0][2].v, b[0][1].v, b[1][2].v, b[1][1].v);
    $finish;
  end
endmodule
"#,
        &["v=1.5,2.25,3.5,4.25"],
    );
}

#[test]
fn a_row_of_a_two_dimensional_array_on_either_side() {
    check(
        r#"
`timescale 1ns/1ps
module top;
  typedef struct { real v; int k; } S;
  S a[4:3][2:1], b[2], c[0:1][2:1];
  assign b = a[4];
  assign c[1] = a[3];
  initial begin
    a[4][2].v = 1.5; a[4][1].v = 2.25; a[3][2].k = 7; a[3][1].k = 9;
    #1 $display("v=%g,%g,%0d,%0d", b[0].v, b[1].v, c[1][2].k, c[1][1].k);
    a[4][1].v = 3.5; a[3][1].k = 11;
    #1 $display("v=%g,%g,%0d,%0d", b[0].v, b[1].v, c[1][2].k, c[1][1].k);
    $finish;
  end
endmodule
"#,
        &["v=1.5,2.25,7,9", "v=1.5,3.5,7,11"],
    );
}

#[test]
fn a_member_array_and_a_child_instance_array() {
    check(
        r#"
`timescale 1ns/1ps
typedef struct { real v; } S;
typedef struct { S arr[2]; } W;
module child; S a[2], b[2]; endmodule
module top;
  W x, y;
  S a[2], b[2];
  child u();
  assign y.arr = x.arr;
  assign b = u.a;
  assign u.b = a;
  initial begin
    x.arr[0].v = 1.5; x.arr[1].v = 2.25;
    u.a[0].v = 3.5; u.a[1].v = 4.25; a[0].v = 5.5; a[1].v = 6.25;
    #1; $display("v=%g,%g %g,%g %g,%g", y.arr[0].v, y.arr[1].v, b[0].v, b[1].v, u.b[0].v, u.b[1].v);
    $finish;
  end
endmodule
"#,
        &["v=1.5,2.25 3.5,4.25 5.5,6.25"],
    );
}

#[test]
fn one_type_spelled_through_an_import_and_qualified() {
    check(
        r#"
`timescale 1ns/1ps
package p; typedef struct { real v; } S; endpackage
module top;
  import p::*;
  S a[2]; p::S b[2];
  assign b = a;
  initial begin
    a[0].v = 1.5; a[1].v = 2.25;
    #1; $display("v=%g,%g", b[0].v, b[1].v);
    $finish;
  end
endmodule
"#,
        &["v=1.5,2.25"],
    );
}
