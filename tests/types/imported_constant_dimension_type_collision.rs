//! §26.3, §7.4.2, §7.8: `[N]` parses alike as an associative dimension keyed
//! by a type N and as a fixed size N. In a scope that imports a package
//! declaring a parameter N, it is the fixed size, even when another package,
//! which the scope does not import, declares a type named N: package types
//! are hoisted design-wide under their bare names, and that type kept the
//! dimension associative (`logic [27:0] a [N]` gave `$high(a)` 27). A scope
//! that imports the package of the type still gets the associative array, and
//! a local parameter still sizes its own arrays. The expected values are those
//! of the reference simulator.
use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    let mut out: Vec<String> = simulate(src, 100)
        .expect("simulate")
        .output
        .iter()
        .map(|l| l.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect();
    out.sort();
    out
}

#[test]
fn imported_parameter_sizes_arrays_next_to_a_foreign_type() {
    let out = lines(
        r#"
package p; localparam int unsigned N = 2; localparam int unsigned M = 3; endpackage
package tp; typedef int N; endpackage
interface ifc import p::*; ();
  logic [7:0] a [N];
endinterface
module child import p::*; ();
  logic [27:0] a [N];
  logic [3:0] b [N][M];
  initial begin
    a[0] = 28'h1; a[1] = 28'h2; b[1][2] = 4'h7;
    #1 $display("T|%m a=%0d:%0d b=%0d:%0d b12=%h", $low(a), $high(a), $size(b, 1), $size(b, 2), b[1][2]);
  end
endmodule
module child2 import p::*; ();
  localparam int N = 4;
  logic [7:0] a [N];
  initial #1 $display("T|%m own a=%0d:%0d", $low(a), $high(a));
endmodule
module child3 import tp::*; ();
  int a [N];
  initial begin
    a[5] = 9;
    #1 $display("T|%m assoc num=%0d", a.num());
  end
endmodule
module top;
  child u();
  child u2();
  child2 v();
  child3 w();
  ifc i();
  initial #1 $display("T|ifc a=%0d:%0d", $low(i.a), $high(i.a));
endmodule
"#,
    );
    assert_eq!(
        out,
        [
            "T|ifc a=0:1",
            "T|top.u a=0:1 b=2:3 b12=7",
            "T|top.u2 a=0:1 b=2:3 b12=7",
            "T|top.v own a=0:3",
            "T|top.w assoc num=1",
        ],
        "{out:?}"
    );
}

/// An explicit import of the parameter, an `int` array, a local parameter,
/// and the top module itself importing the package.
#[test]
fn explicit_import_and_top_module() {
    let out = lines(
        r#"
package p; localparam int unsigned N = 2; endpackage
package tp; typedef int N; endpackage
module child import p::*; ();
  logic [27:0] a [N];
  int c [N];
  initial begin
    #1 $display("T|%m a=%0d:%0d size=%0d c=%0d", $low(a), $high(a), $size(a), $size(c));
  end
endmodule
module child4 ();
  import p::N;
  logic [7:0] e [N];
  initial #1 $display("T|%m e=%0d", $size(e));
endmodule
module child5 ();
  localparam int N = 5;
  logic [7:0] e [N];
  initial #1 $display("T|%m e=%0d", $size(e));
endmodule
module top import p::*; ();
  logic [7:0] t [N];
  child u();
  child4 v();
  child5 x();
  initial #1 $display("T|top t=%0d", $size(t));
endmodule
"#,
    );
    assert_eq!(
        out,
        [
            "T|top t=2",
            "T|top.u a=0:1 size=2 c=2",
            "T|top.v e=2",
            "T|top.x e=5",
        ],
        "{out:?}"
    );
}
