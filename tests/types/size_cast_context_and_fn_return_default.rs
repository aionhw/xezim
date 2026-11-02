//! Round-13 ivtest fixes, both reference-validated:
//!
//! 1. §6.24.1 (br_gh220): a size cast's width is the operand's evaluation
//!    CONTEXT — `5'(3'd7 + 3'd6)` computes the sum at 5 bits (13), not at the
//!    self-determined 3 bits (5).
//! 2. §13.4.1 (br_gh337): the implicit return variable starts at the return
//!    TYPE's default — x for a 4-state type, 0 for a 2-state one. An empty
//!    `function integer f();` returns 'bx; a partially-assigning function
//!    returns x on the untaken path.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able (x/z?)", n))
}

#[test]
fn size_cast_width_is_operand_context() {
    let src = r#"
module tb;
  logic [4:0] r5;
  logic [2:0] a3 = 7, b3 = 6;
  logic [7:0] r8;
  int c1, c2, c3, c4;
  initial begin
    r5 = 5'(3'd7 + 3'd6);
    c1 = (r5 == 5'b01101);          // 13, not (7+6)&7
    r5 = 5'(a3 + b3);
    c2 = (r5 == 5'b01101);
    r5 = 5'(3'sd3 * 3'sd3);
    c3 = (r5 == 5'b01001);          // 9 at 5 bits, not 3-bit wrap
    r8 = 3'(8'hFF) + 8'd0;
    c4 = (r8 == 8'd7);              // truncating cast still truncates
  end
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    assert_eq!(u(&sim, "c1"), 1, "unsized-op sum widens to the cast width");
    assert_eq!(u(&sim, "c2"), 1, "variable operands too");
    assert_eq!(u(&sim, "c3"), 1, "signed multiply in wider context");
    assert_eq!(u(&sim, "c4"), 1, "narrowing cast unchanged");
}

/// A cast's own width sizes the expression it sits in (§6.24.1, §11.6.1).
/// `9'(cnt) + step > 8'hff` with cnt = 255 and step = 1 is a 9-bit sum
/// (256), so the compare is true. Reporting the cast as 0 bits wide wrapped
/// the sum to 0. The parenthesized spelling `(9)'(x)` is the parser's
/// half of the same bug.
#[test]
fn size_cast_widens_the_parent_sum() {
    let src = r#"
module tb;
  logic [7:0] cnt;
  logic [7:0] step;
  logic [7:0] result;
  logic sat, sat_param;
  logic [15:0] wide;
  localparam int W = 9;
  always_comb begin
    automatic logic [7:0] next_c;
    next_c = cnt;
    if ((9'(next_c) + step) > 8'hff)
      result = 8'hff;
    else
      result = next_c + step;
    sat = (9'(next_c) + step) > 8'hff;
    sat_param = (W'(next_c) + step) > 8'hff;
    wide = {9'(next_c) + step};
  end
  int held, flagged, param, carried, exact;
  initial begin
    cnt = 8'hfe;
    step = 8'h01;
    #1;
    exact = (result === 8'hff) && (sat === 1'b0) && (wide === 16'h00ff);
    cnt = 8'hff;
    #1;
    held = (result === 8'hff);
    flagged = (sat === 1'b1);
    param = (sat_param === 1'b1);
    carried = (wide === 16'h0100);
  end
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    assert_eq!(u(&sim, "exact"), 1, "254+1 is 255 and does not saturate");
    assert_eq!(u(&sim, "held"), 1, "255+1 stays saturated");
    assert_eq!(u(&sim, "flagged"), 1, "the compare sees the 9-bit sum");
    assert_eq!(u(&sim, "param"), 1, "a parameter cast widens the sum too");
    assert_eq!(u(&sim, "carried"), 1, "the sum itself keeps the carry");
}

#[test]
fn function_return_defaults_to_type_default() {
    let src = r#"
module tb;
  function integer f4(integer x); endfunction
  function int f2(int x); endfunction
  function logic [7:0] fpart(input b); if (b) fpart = 8'h5A; endfunction
  int r4_is_x, r2_is_0, part_x, part_set;
  initial begin
    r4_is_x  = (f4(3) === 'bx);
    r2_is_0  = (f2(3) === 0);
    part_x   = (fpart(1'b0) === 8'bx);
    part_set = (fpart(1'b1) === 8'h5A);
  end
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    assert_eq!(u(&sim, "r4_is_x"), 1, "empty 4-state function returns x");
    assert_eq!(u(&sim, "r2_is_0"), 1, "empty 2-state function returns 0");
    assert_eq!(u(&sim, "part_x"), 1, "untaken assign path leaves x");
    assert_eq!(u(&sim, "part_set"), 1, "taken path returns the value");
}

/// br_gh277b: inside an INLINED INSTANCE, a function formal that shares its
/// name with another module-scope object (here function `y`) was rewritten to
/// the prefixed module name, so the body read a nonexistent signal (x). The
/// formal must shadow (§13.4). Also covers nested function calls reading
/// module vars — the always_comb sensitivity follows scoped callees.
#[test]
fn instance_function_formal_shadows_module_scope_name() {
    let src = r#"
module duti;
  reg a, b, c, d;
  function z(input x, input y);
    z = x + y;
  endfunction
  function y(input x);
    y = z(x, b) + z(x, c);
  endfunction
  always_comb d = y(a);
endmodule
module tb;
  duti u();
  int direct_z, direct_y, comb_d;
  initial begin
    #1 u.a = 0;
    #1 u.b = 0;
    #1 u.c = 1;
    #1;
    direct_z = (u.z(1'b0, 1'b1) === 1'b1);
    direct_y = (u.y(1'b0) === 1'b1);
    comb_d = (u.d === 1'b1);
  end
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    assert_eq!(
        u(&sim, "direct_z"),
        1,
        "formal y must shadow module function y"
    );
    assert_eq!(u(&sim, "direct_y"), 1, "nested scoped call");
    assert_eq!(
        u(&sim, "comb_d"),
        1,
        "always_comb refires on vars read in scoped callees"
    );
}
