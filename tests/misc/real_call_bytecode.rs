//! Issue #129: a continuous assign calling a real-returning function now
//! compiles to bytecode (25x on the issue's Catmull-Rom reproducer; the
//! result is bit-identical to the AST path and a commercial reference
//! simulator). Admission required four pieces, each pinned here:
//!
//!  * REAL formals/return in `compile_pure_call` (§13.3.1 conversion at the
//!    register bind via `emit_to_real`; ref/output real formals still bail),
//!  * module-signal READS inside the inlined body (`fn_is_pure_in_ext`;
//!    writes to module state stay disqualifying),
//!  * small fixed-shape LOCAL ARRAYS as per-element registers with
//!    compare/branch dynamic indexing (`real row[0:3]`),
//!  * DYNAMIC 2-D unpacked element reads as a bounds-checked flat index over
//!    one Dense operand (`TBL[i][j]`).
//!
//! Also pinned: the pre-existing wrong-VALUE bug this work uncovered —
//! array ELEMENTS never inherited `is_real` in the per-signal tables, so the
//! two-state lowering admitted real arrays and a compiled `T[k] + x` read
//! raw f64 bits (4.6e18-style garbage) while the AST path was right.

use xezim::simulate;

fn msgs(src: &str) -> Vec<String> {
    simulate(src, 1000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect()
}

#[test]
fn real_array_element_read_compiled() {
    // The uncovered pre-existing bug: element realness inheritance.
    let out = msgs(
        r#"
module top;
  real T1 [0:3];
  int k;
  real x, o_comb, o_ca;
  initial begin
    for (int i=0;i<4;i++) T1[i] = 1.5*i;
    k = 2; x = 2.0;
    #1 $display("MB_%0.3f_%0.3f", o_comb, o_ca);
  end
  always_comb o_comb = T1[k] + x;
  assign o_ca = T1[k] + x;
endmodule
"#,
    );
    assert!(
        out.iter().any(|m| m == "MB_5.000_5.000"),
        "compiled real-array element read lost realness: {:?}",
        out
    );
}

#[test]
fn real_function_call_shapes_compile_correctly() {
    let out = msgs(
        r#"
module blk (input real x, output real o1, output real o2, output real o3, output real o4);
  real T1 [0:3];
  real T2 [0:3][0:2];
  initial begin
    for (int i=0;i<4;i++) begin T1[i] = 1.5*i; for (int j=0;j<3;j++) T2[i][j] = i*10.0+j; end
  end
  function automatic real f1 (input real a);
    f1 = a * 2.0 + 0.25;
  endfunction
  function automatic real f2 (input real a);
    real t; t = a / 2.0; f2 = t + 1.0;
  endfunction
  function automatic real f3 (input real a);
    int k; k = int'(a);
    f3 = T1[k] + a;
  endfunction
  function automatic real f4 (input real a);
    real row[0:1]; int k;
    k = int'(a);
    row[0] = T2[k][1];
    row[1] = T2[k][2];
    f4 = row[0] + row[1];
  endfunction
  assign o1 = f1(x);
  assign o2 = f2(x);
  assign o3 = f3(x);
  assign o4 = f4(x);
endmodule
module top;
  real x, o1, o2, o3, o4;
  blk b0 (.x(x), .o1(o1), .o2(o2), .o3(o3), .o4(o4));
  initial begin
    x = 2.0; #1;
    $display("M1_%0.3f_%0.3f_%0.3f_%0.3f", o1, o2, o3, o4);
  end
endmodule
"#,
    );
    assert!(
        out.iter().any(|m| m == "M1_4.250_2.000_5.000_43.000"),
        "inlined real-fn shapes wrong (reference: 4.25 / 2.0 / 5.0 / 43.0): {:?}",
        out
    );
}

#[test]
fn catmull_rom_surface_bit_exact() {
    // A compressed version of issue #129's reproducer (fewer iterations);
    // the accumulated sum must stay bit-identical to the AST path.
    let out = msgs(
        r#"
module blk (input real x, input real y, output real o);
  localparam int NX = 11;
  localparam int NY = 7;
  real TBL [0:NX-1][0:NY-1];
  initial begin
    for (int i = 0; i < NX; i++)
      for (int j = 0; j < NY; j++)
        TBL[i][j] = -0.0001 * i - 0.00002 * j;
  end
  function automatic int clampi (input int v, input int lo, input int hi);
    clampi = (v < lo) ? lo : ((v > hi) ? hi : v);
  endfunction
  function automatic real cr (input real p0, input real p1,
                              input real p2, input real p3, input real t);
    cr = 0.5*((2.0*p1) + (-p0 + p2)*t
            + (2.0*p0 - 5.0*p1 + 4.0*p2 - p3)*t*t
            + (-p0 + 3.0*p1 - 3.0*p2 + p3)*t*t*t);
  endfunction
  function automatic real surface (input real xx, input real yy);
    real fx, fy, tx, ty;
    real row [0:3];
    int ix, iy, n, jy;
    fx = xx / 0.045;  fy = yy / 0.05;
    if (fx < 0.0) fx = 0.0;  if (fx > real'(NX-1)) fx = real'(NX-1);
    if (fy < 0.0) fy = 0.0;  if (fy > real'(NY-1)) fy = real'(NY-1);
    ix = clampi(int'(fx), 0, NX-2);
    iy = clampi(int'(fy), 0, NY-2);
    tx = fx - real'(ix);  ty = fy - real'(iy);
    for (n = 0; n < 4; n++) begin
      jy = clampi(iy - 1 + n, 0, NY-1);
      row[n] = cr(TBL[clampi(ix-1,0,NX-1)][jy], TBL[clampi(ix,0,NX-1)][jy],
                  TBL[clampi(ix+1,0,NX-1)][jy], TBL[clampi(ix+2,0,NX-1)][jy], tx);
    end
    surface = cr(row[0], row[1], row[2], row[3], ty);
  endfunction
  assign o = surface(x, y);
endmodule
module top;
  real x, y, o, acc;
  blk b0 (.x(x), .y(y), .o(o));
  initial begin
    x = 0.0; y = 0.9; acc = 0.0;
    repeat (200) begin
      #1 x = x + 0.009;
      acc = acc + o;
    end
    $display("RNM_%.9f", acc);
  end
endmodule
"#,
    );
    assert!(
        out.iter().any(|m| m.starts_with("RNM_-")),
        "no accumulator printed: {:?}",
        out
    );
    let line = out.iter().find(|m| m.starts_with("RNM_")).unwrap().clone();
    assert_eq!(
        line, "RNM_-0.198520000",
        "Catmull-Rom accumulation drifted from the AST/reference value"
    );
}
