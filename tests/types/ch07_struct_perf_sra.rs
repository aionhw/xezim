//! Performance & edge-case tests for §7.2–§7.3:
//! - Whole unpacked struct CA single evaluation + SRA (`struct_sra`) for nested structs & control flow
//! - Bytecode lowering of unpacked struct `'{...}` patterns (swap/nested/default/NBA) & 4-state/real `==`/`!=`/`===`/`!==`
//! - Interned `StructLayout`, `clear_xz_with_mask` mixed 2-state/4-state packed structs, and wide (>128b) in-place writes
//! - `PackedPlan` cache soundness across repeated loops, distinct target types, and dynamic keys

use xezim::simulate;

fn sim_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 200).expect("simulation should succeed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn assert_has(out: &[String], expected: &str) {
    assert!(
        out.iter().any(|l| l == expected),
        "missing {expected:?} in {out:?}"
    );
}

#[test]
fn unpacked_struct_ca_single_eval_and_nested_sra() {
    let out = sim_lines(
        r#"
module top;
  typedef struct {
    logic [7:0] lo;
    logic [7:0] hi;
  } inner_t;

  typedef struct {
    inner_t pair;
    int     sum;
  } outer_t;

  int f_calls = 0;
  function automatic outer_t f_side(input int x);
    outer_t r;
    f_calls = f_calls + 1;
    r.pair.lo = x[7:0];
    r.pair.hi = x[15:8];
    r.sum = x + 100;
    return r;
  endfunction

  function automatic outer_t g_pure(input int x);
    outer_t res;
    inner_t tmp;
    int acc;
    int k;
    acc = 0;
    k = 0;
    while (k < 4) begin
      acc = acc + k;
      k = k + 1;
    end
    case (x & 3)
      0: tmp = '{lo: 8'h10, hi: 8'h20};
      1: tmp = '{lo: 8'h30, hi: 8'h40};
      default: tmp = '{lo: x[7:0], hi: x[15:8]};
    endcase
    if (x > 10)
      return '{pair: tmp, sum: acc + x};
    res.pair = tmp;
    res.sum = acc;
    return res;
  endfunction

  int in_x;
  outer_t ca_side, ca_pure;
  assign ca_side = f_side(in_x);
  assign ca_pure = g_pure(in_x);

  initial begin
    int base_calls;
    outer_t proc_b, proc_nba;

    in_x = 16'h0201;
    #5;
    base_calls = f_calls;
    in_x = 16'h0412;
    #5;
    $display("CA_CALL_DELTA %0d", f_calls - base_calls);
    $display("CA_SIDE %02x %02x %0d", ca_side.pair.lo, ca_side.pair.hi, ca_side.sum);
    $display("CA_PURE %02x %02x %0d", ca_pure.pair.lo, ca_pure.pair.hi, ca_pure.sum);

    proc_b = g_pure(16'h0501);
    proc_nba <= g_pure(16'h0602);
    #1;
    $display("PROC_B %02x %02x %0d", proc_b.pair.lo, proc_b.pair.hi, proc_b.sum);
    $display("PROC_NBA %02x %02x %0d", proc_nba.pair.lo, proc_nba.pair.hi, proc_nba.sum);
    $finish;
  end
endmodule
"#,
    );
    assert_has(&out, "CA_CALL_DELTA 1");
    assert_has(&out, "CA_SIDE 12 04 1142");
    assert_has(&out, "CA_PURE 12 04 1048");
    assert_has(&out, "PROC_B 30 40 1287");
    assert_has(&out, "PROC_NBA 02 06 1544");
}

#[test]
fn unpacked_struct_pattern_swap_and_eq_bytecode() {
    let out = sim_lines(
        r#"
module top;
  typedef struct {
    logic [7:0] a;
    logic [7:0] b;
  } ab_t;

  typedef struct {
    ab_t        inner;
    logic [7:0] tag;
  } nested_t;

  typedef struct {
    logic [7:0] tag;
    real        val;
  } real_s_t;

  ab_t ab_reg, ab_nba;
  nested_t nest;
  logic [7:0] comb_sel;
  ab_t comb_ab;
  logic eq_x, neq_x, eq_real, neq_real;
  bit ceq_diff, cneq_diff, ceq_same;

  always_comb begin
    comb_ab = '{b: comb_sel + 8'h10, a: comb_sel};
  end

  initial begin
    ab_t eq1, eq2, eqx;
    real_s_t rs1, rs2;

    comb_sel = 8'h05;
    ab_reg = '{8'h11, 8'h22};
    ab_reg = '{a: ab_reg.b, b: ab_reg.a};
    ab_nba <= '{a: ab_reg.b, b: ab_reg.a};
    nest = '{inner: '{default: 8'h5a}, tag: 8'h07};
    #1;
    $display("SWAP %02x %02x %02x %02x", ab_reg.a, ab_reg.b, ab_nba.a, ab_nba.b);
    $display("NEST %02x %02x %02x", nest.inner.a, nest.inner.b, nest.tag);
    $display("COMB_AB %02x %02x", comb_ab.a, comb_ab.b);

    eq1 = '{a: 8'h12, b: 8'hxx};
    eq2 = '{a: 8'h12, b: 8'h34};
    eqx = '{a: 8'h12, b: 8'hxx};
    eq_x = (eq1 == eq2);
    neq_x = (eq1 != eq2);
    ceq_diff = (eq1 === eq2);
    cneq_diff = (eq1 !== eq2);
    ceq_same = (eq1 === eqx);
    $display("EQ %b %b %0d %0d %0d", eq_x, neq_x, ceq_diff, cneq_diff, ceq_same);

    rs1 = '{tag: 8'h03, val: 1.5};
    rs2 = '{tag: 8'h03, val: 1.5};
    eq_real = (rs1 == rs2);
    neq_real = (rs1 !== rs2);
    $display("REAL_EQ %b %b", eq_real, neq_real);
    $finish;
  end
endmodule
"#,
    );
    assert_has(&out, "SWAP 22 11 11 22");
    assert_has(&out, "NEST 5a 5a 07");
    assert_has(&out, "COMB_AB 05 15");
    assert_has(&out, "EQ x x 0 1 1");
    assert_has(&out, "REAL_EQ 1 0");
}

#[test]
fn packed_struct_layout_mixed_and_wide_writes() {
    let out = sim_lines(
        r#"
module top;
  typedef struct packed {
    int         i;
    logic [7:0] l;
    bit [15:0]  b;
  } mixed_p_t;

  mixed_p_t mp;
  mixed_p_t mp_arr[2];

  function automatic int rec_mixed(input int depth);
    mixed_p_t loc;
    loc = 'x;
    loc.b = depth[15:0];
    if (depth == 0)
      return ($isunknown(loc.i) ? 100 : 0) + ($isunknown(loc.l) ? 1 : 0) + int'(loc.b);
    return rec_mixed(depth - 1) + ($isunknown(loc.l) ? 10 : 0);
  endfunction

  typedef struct packed {
    logic [63:0] w2;
    logic [63:0] w1;
    logic [63:0] w0;
  } wide_p_t;

  wide_p_t wp;

  initial begin
    mp = 'x;
    mp_arr[1] = '1;
    mp_arr[1].l = 8'b10xx_01zz;
    $display("MIXED %0d %b %0d %0d", mp.i, mp.l, mp.b, rec_mixed(2));
    $display("MIXED_ARR %08x %b %04x", mp_arr[1].i, mp_arr[1].l, mp_arr[1].b);

    wp = '0;
    wp.w2 = 64'hdead_beef_cafe_babe;
    wp.w0 = 64'h0123_4567_89ab_cdef;
    wp.w1[31:16] = 16'ha55a;
    $display("WIDE %016x %016x %016x", wp.w2, wp.w1, wp.w0);
    $finish;
  end
endmodule
"#,
    );
    assert_has(&out, "MIXED 0 xxxxxxxx 0 21");
    assert_has(&out, "MIXED_ARR ffffffff 10xx01zz ffff");
    assert_has(&out, "WIDE deadbeefcafebabe 00000000a55a0000 0123456789abcdef");
}

#[test]
fn packed_plan_cache_soundness() {
    let out = sim_lines(
        r#"
module top;
  typedef struct packed {
    bit [7:0] x;
    bit [7:0] y;
  } p_xy_t;

  typedef struct packed {
    logic [3:0]  m;
    logic [11:0] n;
  } p_mn_t;

  initial begin
    p_xy_t pxy;
    p_mn_t pmn;
    bit [7:0] sel;
    int i;

    for (i = 0; i < 4; i = i + 1) begin
      pxy = '{i[7:0], i[7:0] + 8'h10};
      pmn = '{i[3:0], i[11:0] + 12'h100};
      $display("LOOP %0d %02x %02x %01x %03x", i, pxy.x, pxy.y, pmn.m, pmn.n);
    end

    // Dynamic key in pattern literal evaluation and ternary condition
    for (i = 0; i < 2; i = i + 1) begin
      sel = i[7:0];
      pxy = '{x: sel ? 8'haa : 8'h55, y: sel + 8'h03};
      $display("COND %0d %02x %02x", i, pxy.x, pxy.y);
    end
    $finish;
  end
endmodule
"#,
    );
    assert_has(&out, "LOOP 0 00 10 0 100");
    assert_has(&out, "LOOP 3 03 13 3 103");
    assert_has(&out, "COND 0 55 03");
    assert_has(&out, "COND 1 aa 04");
}
