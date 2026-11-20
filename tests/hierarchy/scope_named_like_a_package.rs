//! §23.6, §23.9, §26.3: a package item is named with `::`; `p.s` is a
//! hierarchical name, here through an instance or a generate block that has a
//! package's name. The type check took `p.s` for an item of package `p` and
//! failed the design ("'s' is not declared in package 'p'"). The expected
//! values are those of the reference simulator.
//!
//! Not covered: the parsed expression does not keep `::` apart from `.`, so
//! with such an instance in scope `p::K` still reads the instance, not the
//! package.
use xezim::simulate;

#[test]
fn hierarchical_name_through_a_scope_named_like_a_package() {
    let mut out: Vec<String> = simulate(
        r#"
package p;
  parameter int K = 3;
endpackage
package q;
  parameter int K = 4;
endpackage
module sub; logic [7:0] s = 8'h77; endmodule
module m;
  sub p ();
  logic [7:0] hs;
  assign hs = p.s;
  initial #1 $display("T|m hs=%h", hs);
endmodule
module m2;
  if (1) begin : q
    logic [7:0] s = 8'h66;
  end
  logic [7:0] gs;
  assign gs = q.s;
  initial #1 $display("T|m2 gs=%h", gs);
endmodule
module top; m u (); m2 v (); endmodule
"#,
        100,
    )
    .expect("simulate")
    .output
    .iter()
    .map(|l| l.message.clone())
    .filter(|m| m.starts_with("T|"))
    .collect();
    out.sort();
    assert_eq!(out, ["T|m hs=77", "T|m2 gs=66"], "{out:?}");
}
