//! IEEE 1800-2023 §6.20.2: an element of an aggregate parameter can be
//! used in an expression, including a dependent constant expression.

use xezim::simulate;

fn assert_output(src: &str, expected: &[&str]) {
    let sim = simulate(src, 10).expect("simulate failed");
    let mut actual: Vec<_> = sim.output.iter().map(|out| out.message.trim()).collect();
    let mut expected = expected.to_vec();
    actual.sort_unstable();
    expected.sort_unstable();
    assert_eq!(actual, expected);
}

#[test]
fn ordered_override_supplies_element_to_localparam_and_width() {
    let src = r#"
module child #(parameter int A[2] = '{7,9})();
  localparam int LAST = A[1];
  logic [LAST-1:0] data;
endmodule
module top;
  child #(.A('{3,5})) u();
  initial begin
    #1;
    $display("control=%0d", u.A[1]);
    $display("constant=%0d bits=%0d", u.LAST, $bits(u.data));
    $finish;
  end
endmodule
"#;
    // [2] is [0:1], so the ordered pattern sets A[1] to 5 (§7.4.2, §10.9.1).
    assert_output(src, &["control=5", "constant=5 bits=5"]);
}

#[test]
fn header_initializers_supply_elements_to_dependent_constants() {
    let src = r#"
module leaf #(parameter int N = 1) ();
  initial #1 $display("%m N=%0d", N);
endmodule
module child #(parameter int A[2] = '{7,9}, parameter int HEAD = A[1]) ();
  localparam int LAST = A[1];
  logic [A[1]-1:0] packed_data;
  byte unpacked_data[A[1]+1];
  if (A[1] >= 5) begin : selected
    initial #1 $display("%m selected");
  end
  leaf #(.N(A[1])) grandchild();
  initial #1 $display("%m runtime=%0d head=%0d last=%0d packed=%0d unpacked_bits=%0d",
                      A[1], HEAD, LAST, $bits(packed_data), $bits(unpacked_data));
endmodule
module top;
  child dflt();
  child #(.A('{3,5})) ordered();
  child #(.A('{1:6, default:2})) keyed();
  child #(.A('{default:7})) fill();
  child #(.A('{2{8}})) replicated();
  child #('{3,11}) positional();
  initial #2 $finish;
endmodule
"#;
    // The override replaces the initializer (§23.10.2–3). An element N
    // gives N packed bits, 8*(N+1) unpacked bits and selects the branch
    // for N >= 5 (§7.4.1–2, §20.6.2, §27.5).
    assert_output(
        src,
        &[
            "top.dflt runtime=9 head=9 last=9 packed=9 unpacked_bits=80",
            "top.dflt.grandchild N=9",
            "top.dflt.selected selected",
            "top.ordered runtime=5 head=5 last=5 packed=5 unpacked_bits=48",
            "top.ordered.grandchild N=5",
            "top.ordered.selected selected",
            "top.keyed runtime=6 head=6 last=6 packed=6 unpacked_bits=56",
            "top.keyed.grandchild N=6",
            "top.keyed.selected selected",
            "top.fill runtime=7 head=7 last=7 packed=7 unpacked_bits=64",
            "top.fill.grandchild N=7",
            "top.fill.selected selected",
            "top.replicated runtime=8 head=8 last=8 packed=8 unpacked_bits=72",
            "top.replicated.grandchild N=8",
            "top.replicated.selected selected",
            "top.positional runtime=11 head=11 last=11 packed=11 unpacked_bits=96",
            "top.positional.grandchild N=11",
            "top.positional.selected selected",
        ],
    );
}

#[test]
fn descending_signed_elements_keep_their_index_and_type() {
    let src = r#"
module child #(parameter byte signed A[1:0] = '{9,-2}) ();
  localparam int LEFT = A[1];
  localparam int RIGHT = A[0];
  initial #1 $display("runtime=%0d,%0d constant=%0d,%0d", A[1], A[0], LEFT, RIGHT);
endmodule
module top;
  localparam int K = 3;
  child #(.A('{K,-2})) u();
  initial #2 $finish;
endmodule
"#;
    // Patterns follow the declared order and assign to the element type
    // (§7.4.2, §10.9.1). The first index is 1 and -2 sign-extends to int.
    assert_output(src, &["runtime=3,-2 constant=3,-2"]);
}

#[test]
fn four_state_and_real_elements_keep_their_values() {
    let src = r#"
module child #(
  parameter logic [3:0] A[-1:-2] = '{4'b1x0z, 4'h6},
  parameter real R[2] = '{1.5, 2.5}
) ();
  localparam logic [3:0] FOUR = A[-1];
  localparam real REAL_VALUE = R[1];
  initial #1 $display("four=%b/%b real=%0.1f/%0.1f", A[-1], FOUR, R[1], REAL_VALUE);
endmodule
module top;
  child u();
  initial #2 $finish;
endmodule
"#;
    // An element read preserves the four-state and real types (§6.11–12, §10.9.1).
    assert_output(src, &["four=1x0z/1x0z real=2.5/2.5"]);
}

#[test]
fn multidimensional_and_struct_elements_supply_dependent_constants() {
    let src = r#"
package pk;
  typedef struct packed { logic [3:0] width; logic [1:0] mode; } entry_t;
endpackage
module child #(parameter int M[2][2] = '{'{1,2},'{3,4}},
               parameter pk::entry_t S[2] = '{'{5,1},'{6,2}}) ();
  localparam int MATRIX = M[1][1];
  localparam pk::entry_t ENTRY = S[1];
  localparam int FIELD = S[1].width;
  initial #1 $display("%m matrix=%0d/%0d field=%0d/%0d/%0d",
                      M[1][1], MATRIX, S[1].width, ENTRY.width, FIELD);
endmodule
module top;
  child dflt();
  child #(.M('{'{1,2},'{3,8}}), .S('{default:pk::entry_t'{7,1}})) overridden();
  initial #2 $finish;
endmodule
"#;
    // M[1][1] selects the second entry of the second row, and width is
    // the first field of the selected structure (§7.4.4, §7.2.1, §10.9.1–2).
    assert_output(
        src,
        &[
            "top.dflt matrix=4/4 field=6/6/6",
            "top.overridden matrix=8/8 field=7/7/7",
        ],
    );
}

#[test]
fn ordered_override_supplies_elements_to_child_pattern() {
    let src = r#"
module leaf #(parameter int B[2] = '{1,2}, parameter int N = 1) ();
  localparam int LAST = B[1];
  initial #1 $display("runtime=%0d constant=%0d N=%0d", B[1], LAST, N);
endmodule
module parent #(parameter int A[2] = '{7,9}) ();
  leaf #(.B('{A[0], A[1]+1}), .N(A[1])) pattern();
endmodule
module top;
  parent #(.A('{3,5})) u();
  initial #2 $finish;
endmodule
"#;
    // Override expressions use the instantiating scope (§23.10.2), so
    // the pattern puts the parent's A[1] + 1 at B[1] (§10.9.1).
    assert_output(src, &["runtime=6 constant=6 N=5"]);
}

#[test]
fn nested_module_uses_outer_default_and_ordered_override() {
    let src = r#"
module outer #(parameter int A[2] = '{3,5}) ();
  module inner ();
    logic [A[1]-1:0] data;
    initial #1 $display("%m runtime=%0d bits=%0d", A[1], $bits(data));
  endmodule
  inner i();
endmodule
module top;
  outer dflt();
  outer #(.A('{6,8})) ordered();
  initial #2 $finish;
endmodule
"#;
    // The nested declaration and instantiation share the outer scope (§23.4).
    assert_output(
        src,
        &[
            "top.dflt.i runtime=5 bits=5",
            "top.ordered.i runtime=8 bits=8",
        ],
    );
}

#[test]
fn nested_module_keeps_outer_reference_override_elements() {
    let src = r#"
package pk;
  localparam int B[2] = '{4,7};
  localparam logic [7:0] T[2][2] = '{'{1,2},'{3,6}};
endpackage
module leaf #(parameter int N = 1) ();
  initial #1 $display("%m N=%0d", N);
endmodule
module outer #(parameter int A[2] = '{3,5},
               parameter logic [7:0] M[2][2] = '{'{9,9},'{9,9}}) ();
  module inner ();
    logic [A[1]-1:0] data;
    byte unpacked_data[A[1]];
    logic [M[1][1]-1:0] matrix_data;
    leaf #(.N(A[1])) l();
    initial #1 $display("runtime=%0d bits=%0d usize=%0d mbits=%0d",
                        A[1], $bits(data), $size(unpacked_data), $bits(matrix_data));
  endmodule
  inner i();
endmodule
module top;
  outer #(.A(pk::B), .M(pk::T)) r();
  initial #2 $finish;
endmodule
"#;
    // The inner module sees A[1]=7 and M[1][1]=6, including when it
    // evaluates the expression supplied to its child (§23.4, §23.10.2).
    assert_output(src, &["runtime=7 bits=7 usize=7 mbits=6", "top.r.i.l N=7"]);
}
