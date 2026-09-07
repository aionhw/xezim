//! §7.4.1: a continuous assign to an ELEMENT of a packed 2-D signal whose
//! INNER dimension is unit width (`logic [4:0][0:0] v; assign v[p][0] = x;`).
//!
//! `compile_blocking_target`'s Index arm only accepts an `Ident` base, so the
//! nested `v[3][0]` fell through to `flattened_outer_const_signal_id`, which
//! returned the WHOLE-vector signal id and left the caller emitting
//! `BlockingAssignBitDyn(v, <inner index>)` — every `v[i][0]` wrote bit 0
//! whatever `i` was, so only index 0 was ever driven and the rest stayed x.
//!
//! That function already guarded the shape, but asked the question through
//! `packed_elem_width_of`, whose `> 1` width filter drops a unit-width
//! element — so the guard never fired for `[0:0]`. `[N-1:0][1:0]` was correct
//! precisely because its width passed the filter. The guard now asks about
//! SHAPE (`is_packed_multi_dim`), which is the property that matters here:
//! `v[i][0]` selects INSIDE element `i` regardless of how wide the element is.
//!
//! The width filter stays right for its other callers — for a single-index
//! select, element `i` of `[N-1:0][0:0]` IS physically bit `i`, so `v[i]` may
//! legitimately fuse as a bit-select, which is why the `assign v[p] = x` forms
//! below always worked.
//!
//! Expected values are the reference simulator's.

use xezim::simulate;

fn out(src: &str) -> String {
    let sim = simulate(src, 200).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Explicit indices, no generate loop — pins that the genvar is not involved.
#[test]
fn unit_inner_element_assign_explicit_indices() {
    const SRC: &str = r#"
module top;
  logic [4:0][0:0] v;
  assign v[0][0] = 1'b1;
  assign v[1][0] = 1'b0;
  assign v[2][0] = 1'b1;
  assign v[3][0] = 1'b1;
  assign v[4][0] = 1'b0;
  initial begin
    #1 $display("V %b", v);
    $finish;
  end
endmodule
"#;
    let o = out(SRC);
    assert!(o.contains("V 01101"), "every index must drive its own bit:\n{}", o);
}

/// The same shape driven from a generate loop, alongside the spellings that
/// were already correct — a regression here would mean the widened guard has
/// pulled a working form off its fast path.
#[test]
fn unit_inner_element_assign_generate_and_neighbours() {
    const SRC: &str = r#"
module top;
  localparam int N = 5;
  logic [N-1:0]      src;
  logic [N-1:0][0:0] v_elem, v_inner, v_cast;
  logic [N-1:0][1:0] v_wide;

  for (genvar p = 0; p < N; p++) begin : g1
    assign v_elem[p][0] = src[p];   // the broken shape
  end
  for (genvar p = 0; p < N; p++) begin : g2
    assign v_inner[p] = src[p];     // whole inner dim
  end
  for (genvar p = 0; p < N; p++) begin : g3
    assign v_cast[p] = 1'(src[p]);  // cast form
  end
  for (genvar p = 0; p < N; p++) begin : g4
    assign v_wide[p][0] = src[p];   // 2-bit inner dim, element
  end

  initial begin
    src = 5'b10110;
    #1 $display("E %b I %b C %b W %b", v_elem, v_inner, v_cast, v_wide);
    $finish;
  end
endmodule
"#;
    let o = out(SRC);
    assert!(o.contains("E 10110"), "unit inner dim, element assign:\n{}", o);
    assert!(o.contains("I 10110"), "whole inner dim assign:\n{}", o);
    assert!(o.contains("C 10110"), "cast form:\n{}", o);
    // The undriven upper bit of each 2-bit element stays x.
    assert!(o.contains("W x1x0x1x1x0"), "2-bit inner dim, element assign:\n{}", o);
}

/// A procedural write to the same shape always worked; it takes a different
/// path, so keep it pinned next to the continuous-assign case.
#[test]
fn unit_inner_element_procedural_write() {
    const SRC: &str = r#"
module top;
  logic [4:0][0:0] v;
  initial begin
    v = '0;
    v[3][0] = 1'b1;
    #1 $display("P %b", v);
    $finish;
  end
endmodule
"#;
    let o = out(SRC);
    assert!(o.contains("P 01000"), "procedural element write:\n{}", o);
}
