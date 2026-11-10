`timescale 1ns / 1ps
`ifndef SVTEST_DEFS_SVH
`define SVTEST_DEFS_SVH
`define SVTEST_INIT \
int failures = 0;
`define SVTEST_CHECK(expr, msg) \
if (!(expr)) begin \
failures++; \
$display("FAIL: %s at time %0t ps", msg, $time); \
end
`define SVTEST_PASSFAIL \
if (failures == 0) begin \
$display("TEST_PASS"); \
end else begin \
$display("TEST_FAIL count=%0d", failures); \
$fatal(1); \
end
`endif

// Combined MWE for one simulator bug (I) distilled from a proprietary
// testbench failure ("[bs] dut lookup expect 2 got 0").  Every check is
// verified against a reference simulator (the reference simulator 24.09-s006: TEST_PASS);
// under the simulator under test the 25 bug checks fail while the 19
// control checks pass.  The cousin checks (marked "I-cousin", from
// probes 12-13) show the same root cause reached through other containers
// — a 2D packed-typedef LOCAL, an unpacked array of structs (in-function
// AND at module scope) — and other index shapes — an index into the
// element, a 4-D member, ascending / non-zero-based declared ranges, an
// if-condition, a continuous assign.
//
// Bug I — an INLINED function's chained index into a multi-dimensional
// packed-array member of a register-bound struct formal keeps only the
// FIRST index: every index after it degrades to a ONE-BIT bit-select of
// the first-level slice.
//
//   grid.vtab[by][ux]   on  u3_t [3:0][7:0] vtab  (24-bit rows of eight
//                                             3-bit elements) compiles as
//     [by]  ->  correct 24-bit row select (member offset + by*24)
//     [ux]  ->  bit ux of that row register  (ONE bit, not the 3-bit
//               element at 3*ux — i.e. the middle bit of vtab[by][0])
//   so the read returns 0 over a clean seed, and 1 when vtab[by][0] is
//   seeded 3'b010 (bit 1 of the row) — never the addressed element.
//
// Trigger (all required, pinned by probes 1-11 in probes/):
//   * a packed-struct member that is a 2D-or-deeper packed array with a
//     MULTI-BIT element type (u3_t [3:0][7:0]); 1D members are fine
//   * the read happens INSIDE a function body that the caller's
//     always_comb gets INLINED (compile_pure_call); module-scope reads
//     of the very same expression are correct
//   * the function's struct FORMAL name collides with a module-scope
//     signal of the calling module (the port `grid`), which is what makes
//     the member-shape metadata visible to the inliner so the inline
//     SUCCEEDS — without the collision the inline bails to the (correct)
//     AST interpreter.  ANSI and non-ANSI ports are hit equally.
//   * const, dynamic and mixed indices are all affected; on a 3D member
//     every depth past the first is affected, and a two-level select
//     (grid.ctab[1][bx]) returns 1 bit instead of the 24-bit sub-row.
//
// Controls (must keep working): single-index row read through the same
// formal, 1D member through the same formal, 2D member with 1-BIT
// elements through the same formal (the degraded select is accidentally
// the element select), module-scope reads of the same expressions, and
// the same function behind a RENAMED port (no collision -> interpreter).

package mwe_pkg;
  typedef logic [15:0] u16_t;
  typedef logic [2:0]  u3_t;
  typedef struct packed {
    u16_t                pad;
    u3_t [3:0][7:0]      vtab;  // 2D member, 3-bit elements (bug I target)
    u3_t [3:0][3:0][7:0] ctab;  // 3D member, 3-bit elements (bug I target)
    u3_t [15:0]          v1d;   // 1D member (control)
    logic [3:0][7:0]     btab;  // 2D member, 1-bit elements (control)
  } grid_t;

  // ---- bug I shapes: two or more indices into vtab/ctab ----
  function automatic u3_t tab_pick(input grid_t grid,
                                   input logic [1:0] by, input logic [1:0] bx);
    tab_pick = grid.vtab[by][bx];            // dynamic + dynamic
  endfunction

  function automatic u3_t cst_pick(input grid_t grid);
    cst_pick = grid.vtab[1][1];              // constant + constant
  endfunction

  function automatic u3_t mix_pick(input grid_t grid, input logic hor,
                                   input logic [1:0] by, input logic [1:0] bx);
    if (hor) mix_pick = grid.vtab[1][bx];    // constant row, dynamic column
    else     mix_pick = grid.vtab[by][1];    // dynamic row, constant column
  endfunction

  function automatic u3_t cube_pick(input grid_t grid,
                                    input logic [1:0] by, input logic [1:0] bx,
                                    input logic [2:0] kx);
    cube_pick = grid.ctab[by][bx][kx];       // 3D, all dynamic
  endfunction

  function automatic u3_t [7:0] sub_pick(input grid_t grid,
                                         input logic [1:0] bx);
    sub_pick = grid.ctab[1][bx];             // 3D, two levels -> 24-bit sub-row
  endfunction


  // ---- controls: single index, or members the degraded select gets right ----
  function automatic u3_t [7:0] row_pick(input grid_t grid,
                                         input logic [1:0] by);
    row_pick = grid.vtab[by];                // single (first) index only
  endfunction

  function automatic u3_t vec_pick(input grid_t grid,
                                   input logic [3:0] ix);
    vec_pick = grid.v1d[ix];                 // 1D member, dynamic index
  endfunction

  function automatic logic bit_pick(input grid_t grid,
                                    input logic [1:0] by, input logic [1:0] bx);
    bit_pick = grid.btab[by][bx];            // 2D member, 1-bit elements
  endfunction

  // ---- cousin shapes (probe12/probe13): same root cause, other containers
  // and index ranges.  The chained-select helpers flatten Index chains to an
  // IDENT root, so a chain rooted at a typedef local or an unpacked-array
  // element degrades the same way.
  typedef u3_t [3:0][7:0] tab_t;             // 2D packed typedef (cousin C)
  typedef struct packed { grid_t g; u16_t pad2; } outer_t;   // nested (control)
  typedef struct packed {
    u3_t [0:3][7:0]      vasc;               // ASCENDING outer dim
    u3_t [4:1][7:0]      vnz;                // NON-ZERO-BASED outer dim
    u3_t [1:0][3:0][7:0] cube;               // 4-D member
    u16_t                pad;
  } g2_t;

  function automatic u3_t loctab_pick(input tab_t tab,
                                      input logic [1:0] by, input logic [1:0] bx);
    tab_t t;                                 // C: 2D packed-typedef LOCAL
    t = tab;
    loctab_pick = t[by][bx];
  endfunction

  function automatic u3_t arr_pick(input grid_t a[4],
                                   input logic [1:0] by, input logic [1:0] bx);
    arr_pick = a[2].vtab[by][bx];            // D: unpacked array of structs
  endfunction

  function automatic logic bit3_pick(input grid_t grid,
                                     input logic [1:0] by, input logic [1:0] bx);
    bit3_pick = grid.vtab[by][bx][0];        // index INTO the 3-bit element
  endfunction

  function automatic u3_t cube_pick4(input g2_t g2, input logic ix,
                                     input logic [1:0] by, input logic [1:0] bx);
    cube_pick4 = g2.cube[ix][by][bx];        // 4-D member, three indices
  endfunction

  function automatic u3_t asc_pick(input g2_t g2,
                                   input logic [1:0] by, input logic [1:0] bx);
    asc_pick = g2.vasc[by][bx];              // ascending declared outer dim
  endfunction

  function automatic u3_t nz_pick(input g2_t g2,
                                  input logic [2:0] by, input logic [1:0] bx);
    nz_pick = g2.vnz[by][bx];                // non-zero-based outer dim
  endfunction

  function automatic logic ifc_pick(input grid_t grid,
                                    input logic [1:0] by, input logic [1:0] bx);
    if (grid.vtab[by][bx] === 3'd2) ifc_pick = 1'b1;   // read in an if-cond
    else                            ifc_pick = 1'b0;
  endfunction

  // ---- cousin CONTROLS: containers the mis-compiled path never reaches ----
  function automatic u3_t nested_pick(input outer_t outer,
                                      input logic [1:0] by, input logic [1:0] bx);
    nested_pick = outer.g.vtab[by][bx];      // nested struct member
  endfunction

  function automatic u3_t locstr_pick(input grid_t grid,
                                      input logic [1:0] by, input logic [1:0] bx);
    grid_t g;                                // struct-typed LOCAL
    g = grid;
    locstr_pick = g.vtab[by][bx];
  endfunction

  function automatic u3_t wr_pick(input grid_t grid,
                                  input logic [1:0] by, input logic [1:0] bx);
    grid_t g;                                // LHS chained write, read back
    g = grid;
    g.vtab[by][bx] = 3'd7;
    wr_pick = g.vtab[by][bx];
  endfunction

  function automatic grid_t mk(input grid_t src); mk = src; endfunction
  function automatic u3_t callbase_pick(input grid_t grid,
                                        input logic [1:0] by, input logic [1:0] bx);
    callbase_pick = mk(grid).vtab[by][bx];   // call-result base (bails)
  endfunction
endpackage

// bug I DUT: the port `grid` collides with every formal above, so every
// call inlines.  The last three assignments are the module-scope controls.
module dut_collide (grid, by, bx, kx,
                    o_dyn, o_cst, o_mixc, o_mixr, o_cube, o_sub,
                    o_row, o_vec, o_bit, o_mdyn, o_mrow, o_mcst);
  import mwe_pkg::*;
  input  grid_t        grid;
  input  logic [1:0]   by, bx;
  input  logic [2:0]   kx;
  output logic [2:0]   o_dyn, o_cst, o_mixc, o_mixr, o_cube, o_vec, o_mcst;
  output logic [23:0]  o_sub, o_row, o_mrow;
  output logic         o_bit;
  output logic [2:0]   o_mdyn;

  always_comb begin
    o_dyn  = tab_pick(grid, by, bx);        // I: vtab[by][bx]   dyn + dyn
    o_cst  = cst_pick(grid);                // I: vtab[1][1]     const + const
    o_mixc = mix_pick(grid, 1'b1, by, bx);  // I: vtab[1][bx]    const + dyn
    o_mixr = mix_pick(grid, 1'b0, by, bx);  // I: vtab[by][1]    dyn + const
    o_cube = cube_pick(grid, by, bx, kx);   // I: ctab[by][bx][kx]  3D all dyn
    o_sub  = sub_pick(grid, bx);            // I: ctab[1][bx]    24-bit sub-row
    o_row  = row_pick(grid, by);            // control: vtab[by] single index
    o_vec  = vec_pick(grid, {by, bx});      // control: v1d[{by,bx}] 1D member
    o_bit  = bit_pick(grid, by, bx);        // control: btab[by][bx] 1-bit elems
    o_mdyn = grid.vtab[by][bx];             // control: module scope, dyn + dyn
    o_mrow = grid.vtab[by];                 // control: module scope, row
    o_mcst = grid.vtab[1][1];               // control: module scope, const
  end
endmodule

// bug I control: RENAMED port — no module-scope signal is named `grid`, so
// the inline bails and the AST interpreter reads the same expression right.
module dut_safe (grid_in, by, bx, o_dyn);
  import mwe_pkg::*;
  input  grid_t      grid_in;
  input  logic [1:0] by, bx;
  output logic [2:0] o_dyn;
  always_comb o_dyn = tab_pick(grid_in, by, bx);
endmodule

// cousin DUTs: same root cause reached through other containers (2D typedef
// local, unpacked array of structs) and other index shapes (bit-into-element,
// 4-D member, ascending / non-zero-based ranges, if-condition, assign).
module dut_scope (grid, outer, tab, g2, by, bx, bynz, ix,
                  o_loctab, o_arrf, o_arrm, o_bit3, o_cube, o_asc, o_nz,
                  o_ifc, o_assign, o_nested, o_locstr, o_wr, o_callbase);
  import mwe_pkg::*;
  input  grid_t      grid;
  input  outer_t     outer;
  input  tab_t       tab;
  input  g2_t        g2;
  input  logic [1:0] by, bx, bynz;
  input  logic       ix;
  output logic [2:0] o_loctab, o_arrf, o_arrm, o_bit3, o_cube, o_asc, o_nz,
                     o_assign, o_nested, o_locstr, o_wr, o_callbase;
  output logic       o_ifc;

  grid_t carr[4];
  always_comb begin
    for (int k = 0; k < 4; k++) carr[k] = grid;
    o_loctab   = loctab_pick(tab, by, bx);   // C: 2D typedef local
    o_arrf     = arr_pick(carr, by, bx);     // D: array of structs, in fn
    o_arrm     = carr[2].vtab[by][bx];       // D: array of structs, module scope
    o_bit3     = bit3_pick(grid, by, bx);    // index into the element
    o_cube     = cube_pick4(g2, ix, by, bx); // 4-D member
    o_asc      = asc_pick(g2, by, bx);       // ascending outer dim
    o_nz       = nz_pick(g2, bynz, bx);      // non-zero-based outer dim
    o_ifc      = ifc_pick(grid, by, bx);     // read in an if-condition
    o_nested   = nested_pick(outer, by, bx); // control: nested struct
    o_locstr   = locstr_pick(grid, by, bx);  // control: struct-typed local
    o_wr       = wr_pick(grid, by, bx);      // control: LHS chained write
    o_callbase = callbase_pick(grid, by, bx);// control: call-result base
  end
  assign o_assign = tab_pick(grid, by, bx);  // control of context: assign stmt
endmodule


module tb;
  import mwe_pkg::*;
  logic [1:0]  by, bx;
  logic [2:0]  kx;
  grid_t       st;

  logic [2:0]  dyn, cst, mixc, mixr, cube, vec, mcst, mdyn, sdyn;
  logic [23:0] sub, row, mrow;
  logic        bit1;

  dut_collide u (.grid(st), .by(by), .bx(bx), .kx(kx),
                 .o_dyn(dyn), .o_cst(cst), .o_mixc(mixc), .o_mixr(mixr),
                 .o_cube(cube), .o_sub(sub), .o_row(row), .o_vec(vec),
                 .o_bit(bit1), .o_mdyn(mdyn), .o_mrow(mrow), .o_mcst(mcst));
  dut_safe us (.grid_in(st), .by(by), .bx(bx), .o_dyn(sdyn));

  outer_t      ot;
  tab_t        tt;
  g2_t         g2;
  logic [1:0]  bynz;
  logic        ix;
  logic [2:0]  loctab, arrf, arrm, bit3, cube4, asc, nz, asgn, nested, locstr,
               wrv, callb;
  logic        ifc;

  dut_scope usc (.grid(st), .outer(ot), .tab(tt), .g2(g2), .by(by), .bx(bx),
                 .bynz(bynz), .ix(ix),
                 .o_loctab(loctab), .o_arrf(arrf), .o_arrm(arrm), .o_bit3(bit3),
                 .o_cube(cube4), .o_asc(asc), .o_nz(nz), .o_ifc(ifc),
                 .o_assign(asgn), .o_nested(nested), .o_locstr(locstr),
                 .o_wr(wrv), .o_callbase(callb));

  `SVTEST_INIT
  initial begin
    by = 2'd1; bx = 2'd1; kx = 3'd1;

    // ============ pass 1: clean seed — only the addressed elements ========
    st = '0;
    st.vtab[1][1]     = 3'd2;
    st.ctab[1][1][1]  = 3'd2;
    st.v1d[5]         = 3'd2;   // v1d[{by,bx}] = v1d[5]
    st.btab[1][1]     = 1'b1;
    // cousin seeds (discriminator bits set from the start where needed)
    bynz = 2'd2; ix = 1'b0;
    tt = '0;  tt[1][1] = 3'd2;  tt[1][0] = 3'b010;
    g2 = '0;
    g2.vasc[1][1]    = 3'd2;  g2.vasc[1][0] = 3'b010;
    g2.vnz[2][1]     = 3'd2;  g2.vnz[2][0]  = 3'b010;
    g2.cube[0][1][1] = 3'd2;  g2.cube[0][1][0] = 3'b010;
    ot = '0;  ot.g = st;
    #1;
    `SVTEST_CHECK(dyn === 3'd2,
       "I: vtab[by][bx] dyn+dyn gave 0, expect 2")
    `SVTEST_CHECK(cst === 3'd2,
       "I: vtab[1][1] const+const gave 0, expect 2")
    `SVTEST_CHECK(mixc === 3'd2,
       "I: vtab[1][bx] const row + dyn col gave 0, expect 2")
    `SVTEST_CHECK(mixr === 3'd2,
       "I: vtab[by][1] dyn row + const col gave 0, expect 2")
    `SVTEST_CHECK(cube === 3'd2,
       "I: ctab[by][bx][kx] 3D all-dyn gave 0, expect 2")
    `SVTEST_CHECK(sub === 24'h000010,
       "I: ctab[1][bx] two-level select gave 0, expect 24'h000010")
    // controls
    `SVTEST_CHECK(row === 24'h000010,
       "I: vtab[by] single index through the formal (control, must keep working)")
    `SVTEST_CHECK(vec === 3'd2,
       "I: v1d[ix] 1D member through the formal (control, must keep working)")
    `SVTEST_CHECK(bit1 === 1'b1,
       "I: btab[by][bx] 1-bit elements through the formal (control, must keep working)")
    `SVTEST_CHECK(mdyn === 3'd2,
       "I: module-scope vtab[by][bx] (control, must keep working)")
    `SVTEST_CHECK(mrow === 24'h000010,
       "I: module-scope vtab[by] (control, must keep working)")
    `SVTEST_CHECK(mcst === 3'd2,
       "I: module-scope vtab[1][1] (control, must keep working)")
    `SVTEST_CHECK(sdyn === 3'd2,
       "I: same function behind a renamed port (control, must keep working)")
    // ---- cousin bug checks (probe12 / probe13) ----
    `SVTEST_CHECK(loctab === 3'd2,
       "I-cousin: 2D packed-typedef local t[by][bx] gave 1 (bit-select), expect 2")
    `SVTEST_CHECK(arrf === 3'd2,
       "I-cousin: a[2].vtab[by][bx] in fn gave 0 (bit-select), expect 2")
    `SVTEST_CHECK(arrm === 3'd2,
       "I-cousin: carr[2].vtab[by][bx] module scope gave 0 (bit-select), expect 2")
    `SVTEST_CHECK(cube4 === 3'd2,
       "I-cousin: 4-D member cube[ix][by][bx] gave X, expect 2")
    `SVTEST_CHECK(asc === 3'd2,
       "I-cousin: ascending-dims vasc[by][bx] gave 1 (bit-select), expect 2")
    `SVTEST_CHECK(nz === 3'd2,
       "I-cousin: non-zero-based vnz[by][bx] gave 1 (bit-select), expect 2")
    `SVTEST_CHECK(ifc === 1'b1,
       "I-cousin: chained read in an if-condition gave 0 (guard never fires), expect 1")
    `SVTEST_CHECK(asgn === 3'd2,
       "I-cousin: chained read in a continuous assign gave 0 (bit-select), expect 2")
    // ---- cousin controls ----
    `SVTEST_CHECK(nested === 3'd2,
       "I: outer.g.vtab[by][bx] nested struct (control, must keep working)")
    `SVTEST_CHECK(locstr === 3'd2,
       "I: struct-typed local g.vtab[by][bx] (control, must keep working)")
    `SVTEST_CHECK(wrv === 3'd7,
       "I: LHS chained write g.vtab[by][bx]=7 (control, must keep working)")
    `SVTEST_CHECK(callb === 3'd2,
       "I: mk(grid).vtab[by][bx] call-result base (control, must keep working)")

    // ============ pass 2: discriminator seed ==============================
    // vtab[1][0] = 3'b010 sets bit 1 of row 1: a read that degrades to a
    // bit-select of the row returns 1; the addressed element is still 2.
    // ctab[1][1][0] = 3'b010 does the same inside the 3D sub-row.
    st.vtab[1][0]    = 3'b010;
    st.ctab[1][1][0] = 3'b010;
    ot.g             = st;
    #1;
    `SVTEST_CHECK(dyn === 3'd2,
       "I: vtab[by][bx] gave 1 = bit 1 of vtab[1][0], expect 2")
    `SVTEST_CHECK(cst === 3'd2,
       "I: vtab[1][1] gave 1 = bit 1 of vtab[1][0], expect 2")
    `SVTEST_CHECK(mixc === 3'd2,
       "I: vtab[1][bx] gave 1 = bit 1 of vtab[1][0], expect 2")
    `SVTEST_CHECK(mixr === 3'd2,
       "I: vtab[by][1] gave 1 = bit 1 of vtab[1][0], expect 2")
    `SVTEST_CHECK(cube === 3'd2,
       "I: ctab[by][bx][kx] still 0 past two bit-selects, expect 2")
    `SVTEST_CHECK(sub === 24'h000012,
       "I: ctab[1][bx] gave 0 (1 bit, not the 24-bit sub-row), expect 24'h000012")
    // controls over the re-seeded struct
    `SVTEST_CHECK(row === 24'h000012,
       "I: vtab[by] single index, discriminator seed (control, must keep working)")
    `SVTEST_CHECK(mrow === 24'h000012,
       "I: module-scope vtab[by], discriminator seed (control, must keep working)")
    `SVTEST_CHECK(vec === 3'd2,
       "I: v1d[ix], discriminator seed (control, must keep working)")
    `SVTEST_CHECK(bit1 === 1'b1,
       "I: btab[by][bx], discriminator seed (control, must keep working)")
    `SVTEST_CHECK(sdyn === 3'd2,
       "I: renamed port, discriminator seed (control, must keep working)")
    // ---- cousin bug checks over the re-seeded containers ----
    `SVTEST_CHECK(loctab === 3'd2,
       "I-cousin: t[by][bx] gave 1 = bit 1 of t[1][0], expect 2")
    `SVTEST_CHECK(arrf === 3'd2,
       "I-cousin: a[2].vtab[by][bx] gave 1 = bit 1 of carr[2].vtab[1][0], expect 2")
    `SVTEST_CHECK(arrm === 3'd2,
       "I-cousin: module-scope carr[2].vtab[by][bx] gave 1, expect 2")
    `SVTEST_CHECK(cube4 === 3'd2,
       "I-cousin: 4-D member still X past three bit-selects, expect 2")
    `SVTEST_CHECK(bit3 === 3'd0,
       "I-cousin: vtab[by][bx][0] gave 1 = bit 1 of the row, expect 0")
    // ---- cousin controls over the re-seeded containers ----
    `SVTEST_CHECK(nested === 3'd2,
       "I: nested struct, discriminator seed (control, must keep working)")
    `SVTEST_CHECK(locstr === 3'd2,
       "I: struct-typed local, discriminator seed (control, must keep working)")
    `SVTEST_CHECK(callb === 3'd2,
       "I: call-result base, discriminator seed (control, must keep working)")

    `SVTEST_PASSFAIL
    $finish;
  end
endmodule

