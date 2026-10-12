//! §27.5 / §13.4.3 — a user CONSTANT-FUNCTION call used as a generate-if
//! condition (or generate-case selector/value) must be evaluated at
//! elaboration.
//!
//! `is_const_expr`/`eval_const_expr` do not fold a Call, so a condition like
//! `if (has_neighbor(r, dir))` silently read 0 and the else/default branch was
//! taken for every iteration. In an INSTANTIATED CHILD module the generate is
//! flattened through `collect_effective_items_scoped`, which used the raw
//! `eval_const_expr`; at TOP level it went through `elaborate_generate_if`,
//! which rejected the Call outright ("Generate if condition must be a constant
//! expression"). The machinery to fold a user const-fn call already existed
//! (`const_eval_i64_with_params` via the FUNCS_TLS table, and
//! `substitute_const_fn_calls`); these paths just didn't use it. The same
//! call at runtime was always correct — the problem is elaboration-only.

use xezim::simulate;

fn i(sim: &xezim::compiler::Simulator, n: &str) -> i64 {
    let v = sim
        .get_signal(n)
        .or_else(|| sim.get_signal(&format!("top.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able (x/z?)", n));
    v as u32 as i32 as i64
}

/// The bug: a const-fn call as a generate-if condition inside an INSTANTIATED
/// CHILD module (the `collect_effective_items_scoped` flatten path).
#[test]
fn const_fn_call_selects_generate_if_branch_in_child() {
    let src = r#"
package p;
  function automatic bit pick(int unsigned i); return (i % 2) == 0; endfunction
endpackage
module child(output logic [3:0] o);
  import p::*;
  for (genvar k = 0; k < 4; k++) begin : g
    if (pick(k)) begin : y assign o[k] = 1'b1; end
    else         begin : n assign o[k] = 1'b0; end
  end
endmodule
module top;
  logic [3:0] res;
  child u(res);
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    // pick(k) true for even k -> bits 0 and 2 set -> 4'b0101.
    assert_eq!(i(&sim, "res"), 0b0101, "const-fn generate-if condition in a child");
}

/// The same, at TOP level (the `elaborate_generate_if` path, which used to
/// reject a Call condition as non-constant).
#[test]
fn const_fn_call_selects_generate_if_branch_at_top() {
    let src = r#"
package p;
  function automatic bit on(int unsigned i); return i > 0; endfunction
endpackage
module top;
  import p::*;
  logic a, b;
  if (on(3)) begin : y1 assign a = 1'b1; end else begin : n1 assign a = 1'b0; end
  if (on(0)) begin : y2 assign b = 1'b1; end else begin : n2 assign b = 1'b0; end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(i(&sim, "a"), 1, "on(3) -> then branch");
    assert_eq!(i(&sim, "b"), 0, "on(0) -> else branch");
}

/// A const-fn call as a generate-CASE selector.
#[test]
fn const_fn_call_selects_generate_case_arm() {
    let src = r#"
package p;
  function automatic int unsigned sel(int unsigned i); return i + 1; endfunction
endpackage
module child(output logic [2:0] o);
  import p::*;
  for (genvar k = 0; k < 3; k++) begin : g
    case (sel(k))
      1: begin : a assign o[k] = 1'b0; end
      2: begin : b assign o[k] = 1'b1; end
      default: begin : d assign o[k] = 1'b0; end
    endcase
  end
endmodule
module top;
  logic [2:0] res;
  child u(res);
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    // sel(0)=1 ->0, sel(1)=2 ->1, sel(2)=3 -> default ->0  => 3'b010.
    assert_eq!(i(&sim, "res"), 0b010, "const-fn generate-case selector");
}

/// A const-fn over an ENUM value bound to a generate-body localparam, in a
/// doubly-nested for-generate in a child: neighbour links of a 2x2 grid,
/// checked as a connectivity mask.
#[test]
fn const_fn_over_localparam_enum_in_nested_generate_in_child() {
    let src = r#"
package meshp;
  typedef enum logic [2:0] { North=3'd0, East=3'd1, South=3'd2, West=3'd3 } dir_e;
  localparam int unsigned MeshX = 2, MeshY = 2, NumRouters = 4;
  function automatic int unsigned rx(int unsigned r); return r % MeshX; endfunction
  function automatic int unsigned ry(int unsigned r); return r / MeshX; endfunction
  function automatic bit has_neighbor(int unsigned r, dir_e d);
    case (d)
      North: return ry(r) + 1 < MeshY;
      South: return ry(r) > 0;
      East:  return rx(r) + 1 < MeshX;
      West:  return rx(r) > 0;
      default: return 1'b0;
    endcase
  endfunction
endpackage
module mesh_child(output logic [NumRouters-1:0][3:0] conn);
  import meshp::*;
  for (genvar r = 0; r < NumRouters; r++) begin : gr
    for (genvar dp = 0; dp < 4; dp++) begin : gd
      localparam dir_e Dir = dir_e'(dp);
      if (has_neighbor(r, Dir)) begin : gc assign conn[r][dp] = 1'b1; end
      else                      begin : gb assign conn[r][dp] = 1'b0; end
    end
  end
endmodule
module top;
  import meshp::*;
  logic [NumRouters-1:0][3:0] c;
  mesh_child u(c);
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    // conn[r] = {West, South, East, North}. Router 0=(0,0): E,N. 1=(1,0): W,N.
    // 2=(0,1): E,S. 3=(1,1): W,S. Bit order dp: N=0,E=1,S=2,W=3.
    // r0: E,N -> 0011  r1: W,N -> 1001  r2: E,S -> 0110  r3: W,S -> 1100
    let c = i(&sim, "c") as u64;
    assert_eq!((c >> 0) & 0xf, 0b0011, "router 0 connectivity");
    assert_eq!((c >> 4) & 0xf, 0b1001, "router 1 connectivity");
    assert_eq!((c >> 8) & 0xf, 0b0110, "router 2 connectivity");
    assert_eq!((c >> 12) & 0xf, 0b1100, "router 3 connectivity");
}

/// A struct-typed generate-body localparam whose init is an assignment pattern
/// containing const-fn calls in field positions.  The `add_params_from_items`
/// sub-module body localparam path was calling `eval_const_expr_val` directly
/// on the whole pattern (which has no AssignmentPattern arm → returns 0) instead
/// of routing through `pack_struct_const_value`.  Combined with the
/// `ExprKind::Call` arm added to `eval_const_expr_val`, this makes
/// `'{x: x_t'(f(r)), y: y_t'(g(r))}` produce the correct field values.
///
/// For example `localparam xy_id_t Id = '{x: x_bits_t'(xcoord(r)),
/// y: y_bits_t'(ycoord(r)), port_id: '0}` inside a generate-for body in an
/// instantiated child module.
///
/// NOTE: function bodies must use plain arithmetic (no typedef-cast return like
/// `coord_t'(r % N)`) because `const_fn_expr_supported` rejects `$__xz_type_cast`
/// to prevent silent wrong-width evaluations.  The typedef casts in the STRUCT
/// PATTERN fields (`coord_bits_t'(xcoord(r))`) go through `eval_const_expr_val`'s
/// `$__xz_type_cast` arm, which then delegates the inner Call to the new
/// `ExprKind::Call` arm — that path is what this test exercises.
#[test]
fn struct_localparam_in_generate_body_with_const_fn_fields_in_child() {
    let src = r#"
package p;
  localparam int unsigned N = 2;
  typedef logic [1:0] coord_bits_t;
  typedef struct packed { coord_bits_t x; coord_bits_t y; } coord_t;
  // Plain-arithmetic bodies (no typedef cast).
  function automatic int unsigned xcoord(int unsigned r); return r % N; endfunction
  function automatic int unsigned ycoord(int unsigned r); return r / N; endfunction
endpackage
module child(output logic [3:0][3:0] xy_out);
  import p::*;
  for (genvar r = 0; r < 4; r++) begin : gr
    // Typedef casts live in the STRUCT PATTERN, not in the function bodies.
    localparam coord_t C = '{x: coord_bits_t'(xcoord(r)), y: coord_bits_t'(ycoord(r))};
    assign xy_out[r] = C;
  end
endmodule
module top;
  import p::*;
  logic [3:0][3:0] out;
  child u(out);
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    // Router r: x = r % 2, y = r / 2. Packed struct coord_t {x[1:0], y[1:0]}.
    // r0=(x=0,y=0)->4'b0000  r1=(x=1,y=0)->4'b0100  r2=(x=0,y=1)->4'b0001  r3=(x=1,y=1)->4'b0101
    // Packed layout: x is the HIGHER field (declared first in packed struct),
    // y is lower.  coord_t = {x[1:0], y[1:0]} -> bits[3:2]=x, bits[1:0]=y.
    let out = i(&sim, "out") as u64;
    assert_eq!((out >>  0) & 0xf, 0b0000, "router 0 coord (x=0,y=0)");
    assert_eq!((out >>  4) & 0xf, 0b0100, "router 1 coord (x=1,y=0)");
    assert_eq!((out >>  8) & 0xf, 0b0001, "router 2 coord (x=0,y=1)");
    assert_eq!((out >> 12) & 0xf, 0b0101, "router 3 coord (x=1,y=1)");
}

/// The guard: a genuinely NON-constant generate-if condition (a signal, not a
/// const-fn call) is still rejected. Folding const-fn calls must not loosen the
/// constant-expression requirement.
#[test]
fn a_nonconstant_generate_if_condition_is_still_rejected() {
    let src = r#"
module top;
  logic s;
  if (s) begin : y logic dummy; assign dummy = 1'b1; end
endmodule
"#;
    let err = match simulate(src, 20) {
        Ok(_) => panic!("must reject a non-constant generate-if condition"),
        Err(e) => e.to_string(),
    };
    assert!(
        err.contains("constant"),
        "the error should mention the constant requirement: {err}"
    );
}
