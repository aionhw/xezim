//! IEEE 1800-2023 §7.2, §7.4.2, §7.6 and §10.3: whole fixed-array
//! continuous copies pair elements in declared order and drive their members.

use xezim::simulate;

fn check_output(source: &str, expected: &[&str]) {
    let sim = simulate(source, 20_000).expect("simulate");
    let output: Vec<_> = sim
        .output
        .iter()
        .map(|line| line.message.as_str())
        .filter(|line| line.starts_with("v="))
        .collect();
    assert_eq!(output, expected);
}

#[test]
fn copy_pairs_descending_source_with_ascending_target() {
    check_output(
        r#"
`timescale 1ns/1ps
module top;
  typedef struct { real v; } S;
  S a[4:3], b[0:1];
  assign b = a;
  initial begin
    a[4].v = 1.5; a[3].v = 2.25;
    #1; $display("v=%g,%g", b[0].v, b[1].v);
    $finish;
  end
endmodule
module other; endmodule
"#,
        &["v=1.5,2.25"],
    );
}

#[test]
fn whole_copy_pairs_each_fixed_dimension() {
    check_output(
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
module other; endmodule
"#,
        &["v=1.5,2.25,3.5,4.25"],
    );
}

/// §10.3.3: each instance uses its own delay on initial and later changes.
#[test]
fn inlined_copies_keep_instance_delays_and_follow_updates() {
    check_output(
        r#"
`timescale 1ns/1ps
module copy #(parameter D = 2)(output real got);
  typedef struct { real v; } S;
  S a[1], b[1];
  assign #D b = a;
  assign got = b[0].v;
  initial begin a[0].v = 1.5; #5 a[0].v = 2.25; end
endmodule
module top;
  real fast, slow;
  copy #(.D(2)) c1(fast);
  copy #(.D(4)) c2(slow);
  initial begin
    #3 $display("v=%g,%g", fast, slow);
    #2 $display("v=%g,%g", fast, slow);
    #3 $display("v=%g,%g", fast, slow);
    #2 $display("v=%g,%g", fast, slow);
    $finish;
  end
endmodule
"#,
        &["v=1.5,0", "v=1.5,1.5", "v=2.25,1.5", "v=2.25,2.25"],
    );
}

#[test]
fn whole_copy_preserves_nested_and_array_members() {
    check_output(
        r#"
`timescale 1ns/1ps
module top;
  typedef struct { real v; } Inner;
  typedef struct packed { logic signed [3:0] n; logic [3:0] tag; } Packed;
  typedef struct { Inner inner; Packed p; real r[2:1]; logic [3:0] xz; } S;
  S a[1], b[1];
  assign b = a;
  initial begin
    a[0].inner.v = 1.5;
    a[0].p.n = -4'sd3;
    a[0].p.tag = 4'ha;
    a[0].r[2] = 2.25;
    a[0].r[1] = 3.5;
    a[0].xz = 4'b10xz;
    #1;
    $display("v=%g,%0d,%h,%g,%g,%b", b[0].inner.v, b[0].p.n, b[0].p.tag,
             b[0].r[2], b[0].r[1], b[0].xz);
    $finish;
  end
endmodule
module other; endmodule
"#,
        &["v=1.5,-3,a,2.25,3.5,10xz"],
    );
}

/// §27.5: each generate scope has independent array declarations.
#[test]
fn whole_copy_uses_the_arrays_in_its_generate_scope() {
    check_output(
        r#"
`timescale 1ns/1ps
module top;
  typedef struct { real v; } S;
  S a[1], b[1];
  assign b = a;
  if (1) begin : g
    S a[1], b[1];
    assign b = a;
    initial a[0].v = 2.25;
  end
  initial begin a[0].v = 1.5; #1 $display("v=%g,%g", b[0].v, g.b[0].v); $finish; end
endmodule
module other; endmodule
"#,
        &["v=1.5,2.25"],
    );
}

/// §6.6.7: a resolution function sees both whole-net drivers.
#[test]
fn a_resolved_nettype_still_folds_multiple_drivers() {
    check_output(
        r#"
`timescale 1ns/1ps
typedef struct { real v; } S;
function automatic S sum(input S d[]);
  S r;
  r.v = 0.0;
  foreach (d[i]) r.v += d[i].v;
  return r;
endfunction
nettype S sum_t with sum;
module top;
  sum_t a;
  assign a = '{1.5};
  assign a = '{2.25};
  initial begin #1 $display("v=%g", a.v); $finish; end
endmodule
module other; endmodule
"#,
        &["v=3.75"],
    );
}

/// §6.7.1, §28.12: a high-impedance driver cannot mask the driven 1.
#[test]
fn a_whole_net_array_copy_keeps_other_member_drivers() {
    check_output(
        r#"
`timescale 1ns/1ps
module top;
  typedef struct { logic v; } S;
  S a[1][1];
  wire S b[1][1];
  assign b[0][0].v = 1'b1;
  assign b = a;
  initial begin
    a[0][0].v = 1'bz;
    #1 $display("v=%b", b[0][0].v);
    $finish;
  end
endmodule
module other; endmodule
"#,
        &["v=1"],
    );
}

const BASIC_COPY: &str = r#"
`timescale 1ns/1ps
module top;
  typedef struct { real v; } S;
  S a[1], b[1], c[1], scalar_a, scalar_b;
  real r[4:3], q[0:1];
  assign b = a;
  assign c[0] = a[0];
  assign q = r;
  assign scalar_b = scalar_a;
  initial begin
    a[0].v = 1.5;
    r[4] = 2.25;
    r[3] = 3.5;
    scalar_a.v = 4.25;
    #1 $display("v=%g,%g,%g,%g,%g", b[0].v, c[0].v, q[0], q[1], scalar_b.v);
    $finish;
  end
endmodule
"#;

#[test]
fn whole_copy_with_two_roots_reaches_members() {
    check_output(
        &format!("{BASIC_COPY} module other; endmodule"),
        &["v=1.5,1.5,2.25,3.5,4.25"],
    );
}

#[test]
fn whole_copy_in_a_single_root_child_reaches_members() {
    check_output(
        &format!("{BASIC_COPY} module wrapper; top u(); endmodule"),
        &["v=1.5,1.5,2.25,3.5,4.25"],
    );
}

#[test]
fn a_single_root_whole_copy_still_reaches_members() {
    check_output(BASIC_COPY, &["v=1.5,1.5,2.25,3.5,4.25"]);
}
