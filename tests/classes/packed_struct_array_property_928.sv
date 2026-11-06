`timescale 1ns/1ps
// Reporter's consolidated reproducer for #928 (header trimmed): packed arrays of
// packed structs as class properties (sections A/B), wide array reductions (C)
// and wide explicit element chains (D). Self-checking: FAIL lines + TEST_PASS.
// ----- self-check macros (tests/classes SVTEST_* style, inlined) ----
`ifndef SVTEST_DEFS_SVH
`define SVTEST_DEFS_SVH

`define SVTEST_INIT \
int failures = 0;

`define SVTEST_CHECK(expr, msg) \
if (!(expr)) begin \
  failures++; \
  $display("FAIL @%0t : %s", $time, msg); \
end

`define SVTEST_PASSFAIL \
if (failures == 0) begin \
  $display("TEST_PASS"); \
end else begin \
  $display("TEST_FAIL count=%0d", failures); \
  $fatal(1); \
end

`endif

typedef struct packed {
  logic       valid;   // MSB
  logic [7:0] mode;
} cand_t;                 // 9 bits

typedef struct packed { logic en; logic [3:0] tag; } tiny_t;   // 5 bits
typedef struct packed { cand_t cur; cand_t nxt; }      pair_t; // 18 bits
typedef struct packed { cand_t sub; logic [3:0] pad; } nest_t; // 13 bits
typedef cand_t [0:2]    carr_t;      // typedef'd packed array of structs
typedef struct        { logic valid; logic [7:0] mode; } ucand_t; // unpacked

// ===========================================================================
// SECTION A - packed-array class properties: evaluator axis
// ===========================================================================

// ---- BUG shapes: packed dims BEFORE the declarator -----------------------
class p_plain_c;  rand cand_t [0:2]    a; endclass   // ascending
class p_desc_c;   rand cand_t [2:0]    a; endclass   // descending
class p_2d_c;     rand cand_t [0:1][0:2] a; endclass // 2-D packed
class p_one_c;    rand cand_t [0:0]    a; endclass   // single element
class p_two_c;    rand cand_t [0:2]    a; endclass   // 2nd class, same shape
class p_nornd_c;      cand_t [0:2]    a; endclass    // non-rand property
class p_tdcl_c;   rand carr_t          a; endclass   // typedef'd packed array
class p_inln_c;   rand struct packed { logic valid; logic [7:0] mode; } [0:2] a;
                endclass                             // inline struct + dims
class p_tiny_c;   rand tiny_t  [0:1]   a; endclass   // different struct
class p_pair_c;   rand pair_t  [0:1]   a; endclass   // struct of structs
class p_nest_c;   rand nest_t  [0:1]   a; endclass   // struct w/ struct member
class p_meth_c;                                      // access from a method
  rand cand_t [0:2] a;
  function bit bare_ok(); return (a[0].valid === 1'b1) && (a[0].mode === 8'hFF); endfunction
  function bit this_ok(); return (this.a[0].valid === 1'b1) && (this.a[0].mode === 8'hFF); endfunction
endclass
class p_hold_c;   rand cand_t [0:2] a; endclass      // write-through check

// ---- CTRL shapes: identical, except the dims are UNPACKED (after name) ---
class u_plain_c;  rand cand_t  a[0:2];      endclass
class u_desc_c;   rand cand_t  a[2:0];      endclass
class u_2d_c;     rand cand_t  a[0:1][0:2]; endclass
class u_one_c;    rand cand_t  a[0:0];      endclass
class u_dyn_c;    rand cand_t  a[];         endclass
class u_que_c;    rand cand_t  a[$];        endclass
class u_asc_c;    rand cand_t  a[int];      endclass
class u_vec_c;    rand bit [8:0] a[0:2];    endclass   // no struct at all
class u_ust_c;    rand ucand_t a[0:2];      endclass   // unpacked struct element
class u_scat_c;   rand cand_t  s;           endclass   // scalar packed struct

// ===========================================================================
// SECTION B - packed-array class properties: constraint / randomize axis
// ===========================================================================

// ---- BUG shapes: constraint LHS is an element or an element member -------
class q_elem_c;   rand cand_t [0:2] c; constraint x { c[0] == 9'h1FF; } endclass
class q_fld_c;    rand cand_t [0:2] c; constraint x { c[0].valid == 1'b1; } endclass
class q_fe_c;     rand cand_t [0:2] c; constraint x { foreach (c[i]) c[i].valid == 1'b1; } endclass
class q_x_c;      rand cand_t [0:2] c; constraint x { c[0].valid == c[1].valid; c[0].valid == 1'b1; } endclass
class q_impl_c;   rand cand_t [0:2] c; constraint x { c[0].valid -> c[1].mode == 8'hAB; c[0].valid == 1'b1; } endclass
class q_in_c;     rand cand_t [0:2] c; constraint x { c[0].mode inside {[8'h10:8'h1F]}; } endclass
class q_ne_c;     rand cand_t [0:2] c; constraint x { c[2].valid != 1'b0; } endclass
class q_soft_c;   rand cand_t [0:2] c; constraint x { soft c[0].valid == 1'b1; } endclass
class q_2d_c;     rand cand_t [0:1][0:2] m; constraint x { m[1][2].valid == 1'b1; } endclass
class q_nest_c;   rand nest_t [0:2] n;      constraint x { n[1].sub.valid == 1'b1; } endclass

// ---- CTRL shapes: constraint paths that already work ---------------------
class q_whole_c;  rand cand_t [0:2] c; constraint x { c == '1; } endclass
class q_uq_c;     rand cand_t [0:2] c; constraint x { unique {c}; } endclass
class q_d1_c;     rand nest_t n;        constraint x { n.pad == 4'hA; } endclass
class q_scat_c;   rand cand_t s;        constraint x { s.valid == 1'b1; s.mode == 8'h21; } endclass
class u_fld_c;    rand cand_t c[0:2];   constraint x { c[0].valid == 1'b1; c[0].mode == 8'h21; } endclass

// ===========================================================================
// SECTION C - array reduction methods on wide elements
// ===========================================================================

// ---- BUG shapes: wide elements, non-sum reduction methods ----------------
class r_and96_c;  rand bit [95:0]  p[2]; constraint x { p.and()     == 96'h1_0000_0000_0000_000F; } endclass
class r_or96_c;   rand bit [95:0]  p[2]; constraint x { p.or()      == 96'h1_0000_0000_0000_000F; } endclass
class r_xor96_c;  rand bit [95:0]  p[2]; constraint x { p.xor()     == 96'h1_0000_0000_0000_000F; } endclass
class r_orsm_c;   rand bit [95:0]  p[2]; constraint x { p.or()      == 96'd15; } endclass
class r_pr65_c;   rand bit [64:0]  p[2]; constraint x { p.product() == 65'd12; } endclass
class r_pr128_c;  rand bit [127:0] p[2]; constraint x { p.product() == 128'd12; } endclass
class r_prpin_c;  rand bit [95:0]  p[2]; constraint z { p[0] == 96'd3; }
                                      constraint x { p.product() == 96'd60; } endclass

// ---- CTRL shapes: same arrays, paths that already work -------------------
class r_sum96_c;  rand bit [95:0]  p[2]; constraint x { p.sum()     == 96'h1_0000_0000_0000_0000; } endclass
class r_sum128_c; rand bit [127:0] p[2]; constraint x { p.sum()     == 128'h1_0000_0000_0000_0000; } endclass
class r_sumid_c;  rand bit [95:0]  p[2]; constraint x { p.sum() with (item) == 96'h1_0000_0000_0000_0000; } endclass
class r_sumdyn_c; rand bit [95:0]  p[];  constraint s { p.size() == 3; }
                                      constraint x { p.sum()  == 96'h1_0000_0000_0000_0000; } endclass
class r_sumq_c;   rand bit [95:0]  p[$]; constraint s { p.size() == 2; }
                                      constraint x { p.sum()  == 96'h1_0000_0000_0000_0000; } endclass
class r_and32_c;  rand bit [31:0]  p[2]; constraint x { p.and()     == 32'h0000_000F; } endclass
class r_or32_c;   rand bit [31:0]  p[2]; constraint x { p.or()      == 32'h0000_000F; } endclass
class r_xor32_c;  rand bit [31:0]  p[2]; constraint x { p.xor()     == 32'h0000_0005; } endclass
class r_pr32_c;   rand bit [31:0]  p[2]; constraint x { p.product() == 32'd12; } endclass

// ===========================================================================
// SECTION D - explicit element chains on wide elements
// ===========================================================================

// ---- BUG shapes: chains with a target above 2^64 ------------------------
class c_2term_c;  rand bit [95:0]  a[2]; constraint x { a[0]+a[1] == 96'h1_0000_0000_0000_0000; } endclass
class c_3term_c;  rand bit [95:0]  a[3]; constraint x { a[0]+a[1]+a[2] == 96'h1_0000_0000_0000_0000; } endclass
class c_4term_c;  rand bit [95:0]  a[4]; constraint x { a[0]+a[1]+a[2]+a[3] == 96'h1_0000_0000_0000_0000; } endclass
class c_sub_c;    rand bit [95:0]  a[2]; constraint x { a[0]-a[1] == 96'h1_0000_0000_0000_0000; } endclass
class c_2arr_c;   rand bit [95:0]  a[2]; rand bit [95:0] b[2];
                  constraint x { a[0]+b[0] == 96'h1_0000_0000_0000_0000; } endclass
class c_varidx_c; rand bit [95:0]  a[4]; rand bit [1:0] i;
                  constraint x { i == 2; a[0]+a[i] == 96'h1_0000_0000_0000_0000; } endclass
class c_pin_c;    rand bit [95:0]  a[3];
                  constraint x { a[0]+a[1]+a[2] == 96'h1_0000_0000_0000_0000; a[0] == 96'd100; } endclass
class c_cap_c;    rand bit [95:0]  a[2]; constraint x { a[0]+a[1] == 96'h1_FFFF_FFFF_FFFF_FFFF; } endclass
class c_65_c;     rand bit [64:0]  a[2]; constraint x { a[0]+a[1] == 65'h1_0000_0000_0000_0000; } endclass
class c_uqw_c;    rand bit [95:0]  a[4]; constraint x { unique {a}; a[0] < 96'h1_0000_0000_0000_0000; } endclass

// ---- CTRL shapes ---------------------------------------------------------
class c_pin1_c;   rand bit [95:0]  a[2]; constraint x { a[0] == 96'h1_0000_0000_0000_0005; } endclass
class c_in_c;     rand bit [95:0]  a[2];
                  constraint x { a[0] inside {[96'h1_0000_0000_0000_0000:96'h1_0000_0000_0000_0010]}; } endclass
class c_dist_c;   rand bit [95:0]  a[2];
                  constraint x { a[0] dist { 96'h1_0000_0000_0000_0000 := 1, [0:15] :/ 1 }; } endclass
class c_small_c;  rand bit [95:0]  a[3]; constraint x { a[0]+a[1]+a[2] == 96'd300; } endclass
class c_uww_c;    rand bit [95:0]  a[4]; constraint x { unique {a}; } endclass
class c_uqn_c;    rand bit [31:0]  a[4]; constraint x { unique {a}; a[0] < 32'h8000_0000; } endclass

module tb_top;
  `SVTEST_INIT

  cand_t mod_unpack [0:2];      // CTRL: module scope, unpacked
  cand_t mod_pack   [0:2];      // CTRL: module scope, packed dims
  cand_t la[0:2];               // CTRL: module-scope unpacked

  initial begin
    // ---------------- SECTION A BUG: packed-array class properties --------
    begin p_plain_c o; o = new(); o.a[0] = 9'h1FF; o.a[1] = 9'h0FF; o.a[2] = 9'h101;
      `SVTEST_CHECK(o.a[0] === 9'h1FF,       "p_plain: element write then whole read")
      `SVTEST_CHECK(o.a[1] === 9'h0FF,       "p_plain: element[1] whole read")
      `SVTEST_CHECK(o.a[0].valid === 1'b1,   "p_plain: element.valid read")
      `SVTEST_CHECK(o.a[0].mode  === 8'hFF,  "p_plain: element.mode read")
    end
    begin p_desc_c o; o = new(); o.a[0] = 9'h1FF;
      `SVTEST_CHECK(o.a[0].valid === 1'b1,   "p_desc: descending packed dims")
    end
    begin p_2d_c o; o = new(); o.a[0][0] = 9'h1FF;
      `SVTEST_CHECK(o.a[0][0].valid === 1'b1,"p_2d: 2-D packed dims")
    end
    begin p_one_c o; o = new(); o.a[0] = 9'h1FF;
      `SVTEST_CHECK(o.a[0].valid === 1'b1,   "p_one: single-element packed dims")
    end
    begin p_two_c o; o = new(); o.a[0] = 9'h1FF;
      `SVTEST_CHECK(o.a[0].valid === 1'b1,   "p_two: second class, same shape")
    end
    begin p_nornd_c o; o = new(); o.a[0] = 9'h1FF;
      `SVTEST_CHECK(o.a[0].valid === 1'b1,   "p_nornd: non-rand property")
    end
    begin p_tdcl_c o; o = new(); o.a[0] = 9'h1FF;
      `SVTEST_CHECK(o.a[0].valid === 1'b1,   "p_tdcl: typedef'd packed array")
    end
    begin p_inln_c o; o = new(); o.a[0] = 9'h1FF;
      `SVTEST_CHECK(o.a[0].valid === 1'b1,   "p_inln: inline struct + packed dims")
    end
    begin p_tiny_c o; o = new(); o.a[0] = 5'h11;
      `SVTEST_CHECK(o.a[0] === 5'h11,        "p_tiny: 5-bit struct element read")
      `SVTEST_CHECK(o.a[0].en === 1'b1,      "p_tiny: 1-bit member read")
      `SVTEST_CHECK(o.a[0].tag === 4'h1,     "p_tiny: 4-bit member read")
    end
    begin p_pair_c o; o = new(); o.a[0] = {9'h1FF, 9'h101};
      `SVTEST_CHECK(o.a[0].cur.valid === 1'b1,"p_pair: struct-of-struct cur.valid")
      `SVTEST_CHECK(o.a[0].nxt.mode  === 8'h01,"p_pair: struct-of-struct nxt.mode")
    end
    begin p_nest_c o; o = new(); o.a[0].sub.valid = 1'b1; o.a[0].sub.mode = 8'h5A;
      `SVTEST_CHECK(o.a[0].sub.valid === 1'b1,"p_nest: 2-level member write+read")
      `SVTEST_CHECK(o.a[0].sub.mode  === 8'h5A,"p_nest: 2-level member mode read")
    end
    begin p_meth_c o; o = new(); o.a[0] = 9'h1FF;
      `SVTEST_CHECK(o.bare_ok(),              "p_meth: bare a[0].<field> in method")
      `SVTEST_CHECK(o.this_ok(),              "p_meth: this.a[0].<field> in method")
    end
    begin p_hold_c o; o = new();
      o.a[1] = '0; o.a[1].valid = 1'b1; o.a[1].mode = 8'hA5;
      `SVTEST_CHECK(o.a[1].valid === 1'b1,   "p_hold: member write then read")
      `SVTEST_CHECK(o.a[1].mode  === 8'hA5,  "p_hold: member mode write then read")
      `SVTEST_CHECK(o.a[1] === {1'b1, 8'hA5},"p_hold: member write-through whole read")
    end

    // ---------------- SECTION A CTRL: unpacked arrays of the same struct ---
    begin u_plain_c o; o = new(); o.a[0] = 9'h1FF; o.a[1] = 9'h0FF; o.a[2] = 9'h101;
      `SVTEST_CHECK(o.a[1] === 9'h0FF,       "u_plain: element read")
      `SVTEST_CHECK(o.a[0].valid === 1'b1,   "u_plain: element.valid read")
      `SVTEST_CHECK(o.a[0].mode  === 8'hFF,  "u_plain: element.mode read")
    end
    begin u_desc_c o; o = new(); o.a[0] = 9'h1FF;
      `SVTEST_CHECK(o.a[0].valid === 1'b1,   "u_desc: descending unpacked dims")
    end
    begin u_2d_c o; o = new(); o.a[0][0] = 9'h1FF;
      `SVTEST_CHECK(o.a[0][0].valid === 1'b1,"u_2d: 2-D unpacked dims")
    end
    begin u_one_c o; o = new(); o.a[0] = 9'h1FF;
      `SVTEST_CHECK(o.a[0].valid === 1'b1,   "u_one: single-element unpacked")
    end
    begin u_dyn_c o; o = new(); o.a = new[2]; o.a[0] = 9'h1FF;
      `SVTEST_CHECK(o.a[0].valid === 1'b1,   "u_dyn: dynamic array element")
    end
    begin u_que_c o; o = new(); o.a.push_back(9'h1FF);
      `SVTEST_CHECK(o.a[0].valid === 1'b1,   "u_que: queue element")
    end
    begin u_asc_c o; o = new(); o.a[3] = 9'h1FF;
      `SVTEST_CHECK(o.a[3].valid === 1'b1,   "u_asc: assoc array element")
    end
    begin u_vec_c o; o = new(); o.a[0] = 9'h1FF;
      `SVTEST_CHECK(o.a[0] === 9'h1FF,       "u_vec: vector element (no struct)")
    end
    begin u_ust_c o; o = new(); o.a[0].valid = 1'b1; o.a[0].mode = 8'hFF;
      `SVTEST_CHECK(o.a[0].valid === 1'b1,   "u_ust: unpacked-struct element")
    end
    begin u_scat_c o; o = new(); o.s = 9'h1FF;
      `SVTEST_CHECK(o.s === 9'h1FF,          "u_scat: scalar packed struct write")
      `SVTEST_CHECK(o.s.valid === 1'b1,      "u_scat: scalar packed struct member")
    end

    // ---------------- SECTION A CTRL: packed shape outside a class --------
    mod_unpack[0] = 9'h1FF;
    `SVTEST_CHECK(mod_unpack[0].valid === 1'b1, "CTRL module-scope unpacked element.valid")
    mod_pack[0] = 9'h1FF;
    `SVTEST_CHECK(mod_pack[0].valid === 1'b1,   "CTRL module-scope packed-dims element.valid")
    la[0] = 9'h1FF;
    `SVTEST_CHECK(la[0].valid === 1'b1,         "CTRL module-scope array element.valid")
    begin
      cand_t [0:2] loc;                          // packed array as a LOCAL
      loc[0] = 9'h1FF;
      `SVTEST_CHECK(loc[0] === 9'h1FF,           "CTRL local packed-dims element read")
      `SVTEST_CHECK(loc[0].valid === 1'b1,       "CTRL local packed-dims element.valid")
    end

    // ---------------- SECTION B BUG: constraints over the same shape ------
    begin q_elem_c o; o = new();
      `SVTEST_CHECK(o.randomize() == 1, "elem-eq: solves")
      `SVTEST_CHECK(o.c[0] === 9'h1FF,  "elem-eq: honoured")
    end
    begin q_fld_c o; o = new();
      `SVTEST_CHECK(o.randomize() == 1, "element.member eq: solves")
      `SVTEST_CHECK(o.c[0].valid === 1'b1, "element.member eq: honoured")
    end
    begin q_fe_c o; o = new();
      `SVTEST_CHECK(o.randomize() == 1, "foreach element.member: solves")
      `SVTEST_CHECK(o.c[0].valid && o.c[1].valid && o.c[2].valid, "foreach element.member: honoured")
    end
    begin q_x_c o; o = new();
      `SVTEST_CHECK(o.randomize() == 1, "cross-element member eq: solves")
      `SVTEST_CHECK(o.c[0].valid === 1'b1 && o.c[1].valid === 1'b1, "cross-element member eq: honoured")
    end
    begin q_impl_c o; o = new();
      `SVTEST_CHECK(o.randomize() == 1, "member implication: solves")
      `SVTEST_CHECK(o.c[0].valid === 1'b1 && o.c[1].mode === 8'hAB, "member implication: honoured")
    end
    begin q_in_c o; o = new();
      `SVTEST_CHECK(o.randomize() == 1, "member inside: solves")
      `SVTEST_CHECK(o.c[0].mode >= 8'h10 && o.c[0].mode <= 8'h1F, "member inside: honoured")
    end
    begin q_ne_c o; o = new();
      `SVTEST_CHECK(o.randomize() == 1, "member != : solves")
      `SVTEST_CHECK(o.c[2].valid === 1'b1, "member != : honoured")
    end
    begin q_soft_c o; o = new();
      `SVTEST_CHECK(o.randomize() == 1, "soft member: solves")
      `SVTEST_CHECK(o.c[0].valid === 1'b1, "soft member: honoured")
    end
    begin q_2d_c o; o = new();
      `SVTEST_CHECK(o.randomize() == 1, "2-D packed dims member: solves")
      `SVTEST_CHECK(o.m[1][2].valid === 1'b1, "2-D packed dims member: honoured")
    end
    begin q_nest_c o; o = new();
      `SVTEST_CHECK(o.randomize() == 1, "2-level nested member: solves")
      `SVTEST_CHECK(o.n[1].sub.valid === 1'b1, "2-level nested member: honoured")
    end
    // ---------------- SECTION B CTRL -------------------------------------
    begin q_whole_c o; o = new();
      `SVTEST_CHECK(o.randomize() == 1, "CTRL whole-property eq")
      `SVTEST_CHECK(o.c === '1,         "CTRL whole-property eq honoured")
    end
    begin q_uq_c o; o = new();
      `SVTEST_CHECK(o.randomize() == 1, "CTRL unique {packed-array-of-struct}")
    end
    begin q_d1_c o; o = new();
      `SVTEST_CHECK(o.randomize() == 1, "CTRL depth-1 struct member")
      `SVTEST_CHECK(o.n.pad === 4'hA,   "CTRL depth-1 struct member honoured")
    end
    begin q_scat_c o; o = new();
      `SVTEST_CHECK(o.randomize() == 1, "CTRL scalar struct member")
      `SVTEST_CHECK(o.s.valid === 1'b1 && o.s.mode === 8'h21, "CTRL scalar struct member honoured")
    end
    begin u_fld_c o; o = new();
      `SVTEST_CHECK(o.randomize() == 1, "CTRL unpacked property element.member")
      `SVTEST_CHECK(o.c[0].valid === 1'b1 && o.c[0].mode === 8'h21, "CTRL unpacked element.member honoured")
    end

    // ---------------- SECTION C BUG: wide reduction methods ---------------
    begin r_and96_c o;  o = new(); `SVTEST_CHECK(o.randomize() == 1, "and()  wide target") end
    begin r_or96_c o;   o = new(); `SVTEST_CHECK(o.randomize() == 1, "or()   wide target") end
    begin r_xor96_c o;  o = new(); `SVTEST_CHECK(o.randomize() == 1, "xor()  wide target") end
    begin r_orsm_c o;   o = new(); `SVTEST_CHECK(o.randomize() == 1, "or()   small target, wide elements") end
    begin r_pr65_c o;   o = new(); `SVTEST_CHECK(o.randomize() == 1, "product() 65-bit elements") end
    begin r_pr128_c o;  o = new(); `SVTEST_CHECK(o.randomize() == 1, "product() 128-bit elements") end
    begin r_prpin_c o;  o = new();
      `SVTEST_CHECK(o.randomize() == 1, "product() pinned element")
      if (o.p[0] === 96'd3) begin
        `SVTEST_CHECK(o.p[0] * o.p[1] === 96'd60, "product() pinned result")
      end
    end
    // ---------------- SECTION C CTRL --------------------------------------
    begin r_sum96_c o;  o = new(); `SVTEST_CHECK(o.randomize() == 1, "CTRL sum() wide target") end
    begin r_sum128_c o; o = new(); `SVTEST_CHECK(o.randomize() == 1, "CTRL sum() 128-bit") end
    begin r_sumid_c o;  o = new(); `SVTEST_CHECK(o.randomize() == 1, "CTRL sum() with (item)") end
    begin r_sumdyn_c o; o = new(); `SVTEST_CHECK(o.randomize() == 1, "CTRL sum() dynamic array") end
    begin r_sumq_c o;   o = new(); `SVTEST_CHECK(o.randomize() == 1, "CTRL sum() queue") end
    begin r_and32_c o;  o = new(); `SVTEST_CHECK(o.randomize() == 1, "CTRL and()  32-bit elements") end
    begin r_or32_c o;   o = new(); `SVTEST_CHECK(o.randomize() == 1, "CTRL or()   32-bit elements") end
    begin r_xor32_c o;  o = new(); `SVTEST_CHECK(o.randomize() == 1, "CTRL xor()  32-bit elements") end
    begin r_pr32_c o;   o = new(); `SVTEST_CHECK(o.randomize() == 1, "CTRL product() 32-bit elements") end

    // ---------------- SECTION D BUG: explicit wide element chains ---------
    begin c_2term_c o;  o = new(); `SVTEST_CHECK(o.randomize() == 1, "2-term chain, wide target") end
    begin c_3term_c o;  o = new(); `SVTEST_CHECK(o.randomize() == 1, "3-term chain, wide target") end
    begin c_4term_c o;  o = new(); `SVTEST_CHECK(o.randomize() == 1, "4-term chain, wide target") end
    begin c_sub_c o;    o = new(); `SVTEST_CHECK(o.randomize() == 1, "subtraction chain") end
    begin c_2arr_c o;   o = new(); `SVTEST_CHECK(o.randomize() == 1, "chain across two arrays") end
    begin c_varidx_c o; o = new();
      `SVTEST_CHECK(o.randomize() == 1, "chain with variable index")
      `SVTEST_CHECK(o.i === 2'd2, "chain with variable index: index honoured")
    end
    begin c_pin_c o;    o = new();
      `SVTEST_CHECK(o.randomize() == 1, "chain plus pinned element")
      `SVTEST_CHECK(o.a[0] === 96'd100, "chain plus pinned element: pin honoured")
    end
    begin c_cap_c o;    o = new(); `SVTEST_CHECK(o.randomize() == 1, "chain target with carry bits") end
    begin c_65_c o;     o = new(); `SVTEST_CHECK(o.randomize() == 1, "65-bit element chain") end
    begin c_uqw_c o;    o = new();
      `SVTEST_CHECK(o.randomize() == 1, "unique {a} plus wide comparison")
      `SVTEST_CHECK(o.a[0] < 96'h1_0000_0000_0000_0000, "unique {a} plus wide comparison: honoured")
    end
    // ---------------- SECTION D CTRL --------------------------------------
    begin c_pin1_c o;  o = new();
      `SVTEST_CHECK(o.randomize() == 1, "CTRL single wide pin")
      `SVTEST_CHECK(o.a[0] === 96'h1_0000_0000_0000_0005, "CTRL single wide pin honoured")
    end
    begin c_in_c o;    o = new(); `SVTEST_CHECK(o.randomize() == 1, "CTRL wide inside{}") end
    begin c_dist_c o;  o = new(); `SVTEST_CHECK(o.randomize() == 1, "CTRL wide dist{}") end
    begin c_small_c o; o = new(); `SVTEST_CHECK(o.randomize() == 1, "CTRL 3-term chain, small target") end
    begin c_uww_c o;   o = new(); `SVTEST_CHECK(o.randomize() == 1, "CTRL unique {a} alone") end
    begin c_uqn_c o;   o = new(); `SVTEST_CHECK(o.randomize() == 1, "CTRL unique {a} + narrow comparison") end

    `SVTEST_PASSFAIL
  end
endmodule

