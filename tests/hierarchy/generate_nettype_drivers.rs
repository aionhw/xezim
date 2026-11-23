//! IEEE 1800-2023 §6.5, §6.6.7: a net of a resolved nettype takes several
//! continuous drivers, and a generate block declares such a net as a module
//! does (§27.2). §10.3.1, §10.3.3: the declaration assignment is one more
//! continuous driver, and the declaration's delay belongs to it, or delays
//! every driver when there is no declaration assignment (#1484). Expected
//! values checked on the reference simulator.

use xezim::simulate;

const REAL_SUM: &str = r#"
`timescale 1ns/1ps
function automatic real rsum(input real d[]);
  rsum = 0.0;
  foreach (d[k]) rsum += d[k];
endfunction
nettype real rn with rsum;
"#;

fn lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 100).unwrap_or_else(|e| panic!("{e}\n{src}"));
    sim.output.iter().map(|l| l.message.clone()).collect()
}

fn check_real_net(design: &str, expected: &[&str]) {
    let src = format!("{REAL_SUM}{design}");
    assert_eq!(lines(&src), expected, "{src}");
}

fn rejected(src: &str) -> String {
    match simulate(src, 100) {
        Ok(_) => panic!("accepted:\n{src}"),
        Err(e) => e,
    }
}

#[test]
fn generate_blocks_resolve_multiple_drivers() {
    // §27.4, §27.5: if, for and nested case generate blocks; a hierarchical
    // driver from outside the block drives the same net (§23.6).
    check_real_net(
        r#"
module top;
  if (1) begin : G
    rn n;
    assign n = 0.5;
    assign n = 10.0;
  end
  for (genvar g = 0; g < 2; g++) begin : F
    rn n;
    assign n = g + 0.5;
    assign n = 10.0 * (g + 1);
  end
  if (1) begin : Outer
    case (1)
      1: begin : Inner
        rn n;
        assign n = 0.5;
        assign n = 10.0;
        assign n = -2.0;
      end
    endcase
  end
  if (1) begin : H
    rn n;
    assign n = 0.5;
    assign n = 1.0;
  end
  assign H.n = 10.0;
  initial #1 $display("G=%0.2f F=%0.2f %0.2f I=%0.2f H=%0.2f",
                      G.n, F[0].n, F[1].n, Outer.Inner.n, H.n);
endmodule
"#,
        &["G=10.50 F=10.50 21.50 I=8.50 H=11.50"],
    );
}

#[test]
fn generated_unpacked_struct_net() {
    let src = r#"
typedef struct { real r; int i; } sample_t;
function automatic sample_t sum(input sample_t drivers[]);
  sum = '{0.0, 0};
  foreach (drivers[k]) begin
    sum.r += drivers[k].r;
    sum.i += drivers[k].i;
  end
endfunction
nettype sample_t sn with sum;
module top;
  if (1) begin : G
    sn n;
    assign n = '{0.5, 1};
    assign n = '{10.0, 2};
  end
  initial #1 $display("n=%0.2f %0d", G.n.r, G.n.i);
endmodule
"#;
    assert_eq!(lines(src), ["n=10.50 3"]);
}

#[test]
fn generated_package_nettype_nets_in_top_and_instance() {
    let src = r#"
package p;
  function automatic logic [7:0] ror(input logic [7:0] d[]);
    ror = 0;
    foreach (d[k]) ror |= d[k];
  endfunction
  nettype logic [7:0] on with ror;
endpackage
module sub(output logic [7:0] y);
  import p::*;
  if (1) begin : G
    on w;
    assign w = 8'h01;
    assign w = 8'h40;
    assign y = w;
  end
endmodule
module top;
  wire [7:0] y;
  sub u(.y(y));
  if (1) begin : G
    p::on w;
    assign w = 8'h10;
    assign w = 8'h02;
  end
  initial #1 $display("y=%h w=%h", y, G.w);
endmodule
"#;
    assert_eq!(lines(src), ["y=41 w=12"]);
}

#[test]
fn declaration_assignment_is_a_driver() {
    // A constant and a non-constant declaration assignment, in a module-local
    // nettype's generate block.
    let src = r#"
`timescale 1ns/1ps
module top;
  function automatic real rsum(input real d[]);
    rsum = 0.0;
    foreach (d[k]) rsum += d[k];
  endfunction
  nettype real rn with rsum;
  real x = 1.5;
  if (1) begin : E
    rn n = 0.25;
    assign n = 10.0;
  end
  if (1) begin : G
    rn n = x;
    assign n = 10.0;
  end
  initial begin
    #1 $display("E=%0.2f G=%0.2f", E.n, G.n);
    x = 2.5;
    #1 $display("G=%0.2f", G.n);
  end
endmodule
"#;
    assert_eq!(lines(src), ["E=10.25 G=11.50", "G=12.50"]);
}

#[test]
fn declaration_delays() {
    // `rn #3 m = 0.5;`: the delay belongs to the declaration's driver only.
    // `rn #2 q;`: a net delay delays every driver.
    check_real_net(
        r#"
module top;
  rn #3 m = 0.5;
  assign m = 10.0;
  rn #2 q;
  assign q = 0.5;
  assign q = 10.0;
  if (1) begin : G
    rn #3 m = 0.5;
    assign m = 10.0;
    rn #2 q;
    assign q = 0.5;
    assign q = 1.0;
  end
  initial begin
    #1 $display("m=%0.2f q=%0.2f gm=%0.2f gq=%0.2f", m, q, G.m, G.q);
    #3 $display("m=%0.2f q=%0.2f gm=%0.2f gq=%0.2f", m, q, G.m, G.q);
  end
endmodule
"#,
        &[
            "m=10.00 q=0.00 gm=10.00 gq=0.00",
            "m=10.50 q=10.50 gm=10.50 gq=1.50",
        ],
    );
}

fn check_shadow(prelude: &str, header: &str, local: &str) {
    // §6.18, §6.20.3, §26.3, §26.5: the module's own type declaration hides
    // the outer nettype of the same name, so `T` declares variables.
    let src = format!(
        r#"
{prelude}
module top{header};
{local}
  if (1) begin : G
    T v = 0.5;
    T c;
    assign c = v * 2.5;
  end
  initial #1 $display("v=%0.2f c=%0.2f", G.v, G.c);
endmodule
"#
    );
    assert_eq!(lines(&src), ["v=0.50 c=1.25"], "{src}");
    let two = src.replace(
        "assign c = v * 2.5;",
        "assign c = v * 2.5;\n    assign c = 1.0;",
    );
    let err = rejected(&two);
    assert!(err.contains("multiple continuous drivers"), "{err}\n{two}");
}

#[test]
fn module_typedef_hides_unit_nettype() {
    check_shadow(
        "function automatic real rsum(input real d[]);\n  rsum = 0.0;\n  foreach (d[k]) rsum += d[k];\nendfunction\nnettype real T with rsum;",
        "",
        "  typedef real T;",
    );
}

#[test]
fn type_parameter_hides_unit_nettype() {
    check_shadow(
        "function automatic real rsum(input real d[]);\n  rsum = 0.0;\n  foreach (d[k]) rsum += d[k];\nendfunction\nnettype real T with rsum;",
        " #(parameter type T = real)",
        "",
    );
}

#[test]
fn module_typedef_hides_package_nettype() {
    check_shadow(
        "package p;\n  function automatic real rsum(input real d[]);\n    rsum = 0.0;\n    foreach (d[k]) rsum += d[k];\n  endfunction\n  nettype real T with rsum;\nendpackage",
        "",
        "  import p::*;\n  typedef real T;",
    );
}

#[test]
fn module_scope_typedef_hides_unit_nettype() {
    let src = format!(
        r#"{REAL_SUM}
nettype real T with rsum;
module top;
  typedef real T;
  T v;
  assign v = 0.5;
  assign v = 1.0;
endmodule
"#
    );
    let err = rejected(&src);
    assert!(err.contains("multiple continuous drivers"), "{err}\n{src}");
}

#[test]
fn generated_variable_and_unresolved_net_reject_two_drivers() {
    // §6.5: a variable takes one continuous assignment; §6.6.7: so does a net
    // of a nettype without a resolution function.
    let variable = r#"
module top;
  if (1) begin : G
    real v;
    assign v = 0.5;
    assign v = 10.0;
  end
endmodule
"#;
    rejected(variable);
    let unresolved = r#"
nettype real un;
module top;
  if (1) begin : G
    un n;
    assign n = 0.5;
    assign n = 10.0;
  end
endmodule
"#;
    rejected(unresolved);
}

#[test]
fn generated_wire_resolves_opposing_drivers() {
    // §6.6.1, Table 6-2: opposite drivers of equal strength resolve to x.
    let src = r#"
module top;
  if (1) begin : G
    wire w;
    assign w = 1'b0;
    assign w = 1'b1;
  end
  initial #1 $display("w=%b", G.w);
endmodule
"#;
    assert_eq!(lines(src), ["w=x"]);
}
