//! IEEE 1800-2023 §26.3, §11.5.1, §6.19 (#721): a child's constants
//! fold against its own parameters and the package constants every scope
//! reads (`pkg::N`). The package constants are one table shared by the
//! instances; each instance's view used to hold a copy of all of them.
//! Expected lines are the reference simulator's, which also rejects the
//! illegal designs.

use xezim::simulate;

const PKG: &str = "package cfg; parameter int Z = 0; parameter int W = 4; endpackage\n";

fn err(body: &str) -> String {
    let src = format!("{PKG}{body}");
    match simulate(&src, 10) {
        Err(e) => e,
        Ok(_) => panic!("accepted an illegal design:\n{src}"),
    }
}

fn out(body: &str) -> Vec<String> {
    let src = format!("{PKG}{body}");
    let sim = simulate(&src, 10).unwrap_or_else(|e| panic!("{e}\n{src}"));
    sim.output.iter().map(|o| o.message.clone()).collect()
}

/// `x[0 +: cfg::Z]` with `Z = 0`, read through the package in a child.
#[test]
fn zero_width_select_by_a_package_constant_is_rejected_in_a_child() {
    let e = err(r#"
module child #(parameter int N = 2);
  logic [7:0] x = 8'hA5;
  logic [3:0] y;
  initial y = x[0 +: cfg::Z];
endmodule
module top; child u(); child #(.N(3)) v(); endmodule
"#);
    assert!(e.contains("indexed part-select has zero width"), "{e}");
    assert_eq!(
        e.matches("indexed part-select has zero width").count(),
        1,
        "{e}"
    );
}

/// A child's own parameter is not the package constant of the same name:
/// `Z` is 2 and 3 here, whatever `cfg::Z` is.
#[test]
fn a_child_parameter_is_read_before_a_package_constant_of_its_name() {
    let o = out(r#"
module child #(parameter int Z = 2);
  logic [7:0] x = 8'hA5;
  logic [3:0] y;
  initial begin
    y = x[0 +: Z];
    $display("T|%m y=%h", y);
  end
endmodule
module top; child u1(); child #(.Z(3)) u3(); endmodule
"#);
    assert_eq!(o, ["T|top.u1 y=1", "T|top.u3 y=5"]);
}

/// Legal uses of package constants beside each instance's own parameter.
#[test]
fn package_constants_fold_beside_each_instance_parameter() {
    let o = out(r#"
module child #(parameter int N = 2);
  import cfg::*;
  logic [7:0] x = 8'hA5;
  logic [3:0] y;
  int m [cfg::W];
  typedef enum logic [cfg::W-1:0] {A = 4'h1, B = 4'h2} e_t;
  e_t e = B;
  initial begin
    y = x[N +: cfg::W];
    m[N] = 7;
    $display("T|%m N=%0d y=%h m=%0d size=%0d e=%0d w=%0d", N, y, m[N], $size(m), e, W);
  end
endmodule
module top; child u1(); child #(.N(3)) u3(); endmodule
"#);
    assert_eq!(
        o,
        [
            "T|top.u1 N=2 y=9 m=7 size=4 e=2 w=4",
            "T|top.u3 N=3 y=4 m=7 size=4 e=2 w=4"
        ]
    );
}
