//! IEEE 1800-2023 §11.5.1, §7.4.2, §6.19: a child module's constants are
//! checked for every instance, not only the top's (#893). The width of an
//! indexed part-select must be positive, a fixed unpacked dimension `[N]`
//! must be a positive size (at the top too, where `[N]` with a bare name used
//! to escape the check), and two enum members may not share a value. The
//! reference simulator rejects each illegal design and runs the legal ones.

use xezim::simulate;

fn err(src: &str) -> String {
    match simulate(src, 10) {
        Err(e) => e,
        Ok(_) => panic!("accepted an illegal design:\n{src}"),
    }
}

fn out(src: &str) -> Vec<String> {
    let sim = simulate(src, 10).unwrap_or_else(|e| panic!("{e}\n{src}"));
    sim.output.iter().map(|o| o.message.clone()).collect()
}

/// `x[0 +: N]` with `N = 0` in a child (§11.5.1).
#[test]
fn child_zero_width_indexed_select_is_rejected() {
    let e = err(r#"
module child;
  localparam int N = 0;
  logic [7:0] x = 8'hA5;
  logic [3:0] y;
  initial y = x[0 +: N];
endmodule
module top;
  child u();
endmodule
"#);
    assert!(e.contains("indexed part-select has zero width"), "{e}");
}

/// `int m [N]` with `N = 0` in a child and at the top (§7.4.2).
#[test]
fn zero_size_unpacked_dimension_is_rejected() {
    let e = err(r#"
module child;
  localparam int N = 0;
  int m [N];
endmodule
module top;
  child u();
endmodule
"#);
    assert!(
        e.contains("dimension size must be greater than zero"),
        "{e}"
    );
    let e = err(r#"
module top;
  localparam int N = 0;
  int m [N];
endmodule
"#);
    assert!(
        e.contains("dimension size must be greater than zero"),
        "{e}"
    );
}

/// `enum {A = 0, B = K}` with `K = 0` in a child (§6.19).
#[test]
fn child_duplicate_enum_value_is_rejected() {
    let e = err(r#"
module child;
  localparam int K = 0;
  typedef enum int {A = 0, B = K} e_t;
endmodule
module top;
  child u();
endmodule
"#);
    assert!(e.contains("have the same value"), "{e}");
}

/// The check reads each instance's own parameters: an override that makes
/// one instance illegal is reported, and the message names the clause once.
#[test]
fn an_illegal_override_of_one_instance_is_rejected() {
    let src = |n: &str| {
        format!(
            r#"
module child #(parameter int N = 2);
  logic [7:0] x = 8'hA5;
  logic [3:0] y;
  int m [N];
  initial y = x[0 +: N];
endmodule
module top;
  child #(.N(2)) a();
  child #(.N({n})) b();
  child #(.N(1)) c();
endmodule
"#
        )
    };
    let e = err(&src("0"));
    assert!(e.contains("indexed part-select has zero width"), "{e}");
    assert_eq!(
        e.matches("indexed part-select has zero width").count(),
        1,
        "{e}"
    );
    assert!(
        e.contains("dimension size must be greater than zero"),
        "{e}"
    );
    assert!(simulate(&src("3"), 10).is_ok());
}

/// The same shapes with legal constants elaborate and run in a child.
#[test]
fn legal_child_constants_run() {
    let o = out(r#"
module child #(parameter int N = 1);
  localparam int K = N;
  logic [7:0] x = 8'hA5;
  logic [N-1:0] y;
  int m [N];
  typedef enum int {A = 0, B = K} e_t;
  e_t e = B;
  initial begin
    y = x[0 +: N];
    m[0] = 7;
    $display("T|N=%0d y=%0d m=%0d size=%0d e=%0d", N, y, m[0], $size(m), e);
  end
endmodule
module top;
  child u1();
  child #(.N(4)) u4();
endmodule
"#);
    assert_eq!(o, ["T|N=1 y=1 m=7 size=1 e=1", "T|N=4 y=5 m=7 size=4 e=4"]);
}
