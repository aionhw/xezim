//! IEEE 1800-2023 §7.4–§7.7 array semantics: OOB reads, X/Z writes,
//! multi-dimensional/mixed fixed+dynamic arrays, `new[]` sizing/init,
//! and array argument passing/writeback, verified byte-for-byte against
//! reference simulators.

use xezim::simulate;

fn sim_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 100).expect("simulation should succeed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn assert_has(out: &[String], expected: &str) {
    assert!(
        out.iter().any(|l| l == expected),
        "missing {expected:?} in {out:?}"
    );
}

#[test]
fn oob_struct_and_multidim_reads_writes_and_continuous_assign() {
    let out = sim_lines(
        r#"
module sub_mod;
  int d[][2];
  int q[$:2];
  typedef bit [7:0] u8_t;
  u8_t m2[2][2];
  initial begin
    d = new[2];
    d[1][0] = 22; d[1][1] = 33;
    $display("S8_D|%0d %0d", d[1][0], d[1][1]);
    q.push_back(1); q.push_back(2); q.push_back(3); q.push_back(4);
    $display("S8_Q|%0d %p", q.size(), q);
    $display("S8_M2|%h%h%h%h", m2[0][0], m2[0][1], m2[1][0], m2[1][1]);
  end
endmodule

module top;
  typedef struct { int x = 7; } st_t;
  st_t arr_st[2];
  st_t tmp_s;

  int a2[2][2];
  bit [7:0] arr3[2][2][2];

  int cm_src[2][2];
  int cm_dst[2][2];
  assign cm_dst = cm_src;

  int m23[2][][3];

  sub_mod u();

  initial begin
    int loc3[2][2][2];
    logic [31:0] idx;

    // Unpacked-struct OOB whole copy yields member default (7);
    // OOB member read yields member type default (0, not 7 and not x).
    tmp_s = arr_st[5];
    $display("S7_ST|%0d %0d", tmp_s.x, arr_st[5].x);

    // 2-state multi-D OOB reads yield 0.
    a2[0][1] = 42;
    $display("S7_2D_OOB|%0d %0d", a2[5][0], a2[0][5]);

    // 3-D writes reject X/Z and OOB indices without folding to slot 0.
    arr3[0][0][0] = 8'hAA;
    idx = 32'bx;
    arr3[idx][0][0] = 8'h55;
    arr3[5][0][0] = 8'h33;
    $display("S7_3D_XZ|%0x", arr3[0][0][0]);

    // Continuous assignment on 2-D unpacked array.
    cm_src[0][0] = 11; cm_src[0][1] = 22;
    cm_src[1][0] = 33; cm_src[1][1] = 44;

    // Mixed fixed + dynamic + fixed array.
    m23[1] = new[2];
    m23[1][1][2] = 77;
    $display("S8_M23|%0d", m23[1][1][2]);
    $display("S8_LOC|%0d", loc3[1][1][1]);

    #1;
    $display("S7_CM|%0d %0d", cm_dst[0][0], cm_dst[1][1]);
  end
endmodule
"#,
    );
    assert_has(&out, "S7_ST|7 0");
    assert_has(&out, "S7_2D_OOB|0 0");
    assert_has(&out, "S7_3D_XZ|aa");
    assert_has(&out, "S8_M23|77");
    assert_has(&out, "S8_M2|00000000");
    assert_has(&out, "S8_LOC|0");
    assert_has(&out, "S7_CM|11 44");
    assert_has(&out, "S8_D|22 33");
    assert_has(&out, "S8_Q|3 '{1, 2, 3}");
}

#[test]
fn dynamic_array_new_sizing_sources_and_errors() {
    let sim = simulate(
        r#"
module top;
  typedef int dyn_i_t[];
  typedef logic [7:0] dyn_b_t[];

  int mat[2][];
  int mat2[2][] = '{'{100, 200}, '{300, 400}};
  int f_desc[5:2] = '{10, 20, 30, 40};
  int d_from_desc[];
  int d_dflt[4] = '{default: 7};
  dyn_b_t d_cat;
  dyn_i_t d_fn;
  int a_slice[5] = '{10, 20, 30, 40, 50};
  int d_sl[];
  int d_err[];

  int sz_calls = 0;
  int idx_calls = 0;

  function automatic int f_sz();
    sz_calls++;
    return 2;
  endfunction

  function automatic int f_idx();
    idx_calls++;
    return 0;
  endfunction

  function automatic dyn_i_t make_arr();
    dyn_i_t r;
    r = new[4];
    r[0] = 11; r[1] = 22; r[2] = 33; r[3] = 44;
    return r;
  endfunction

  initial begin
    logic signed [31:0] xz_n;

    // Single evaluation of src_expr before mutating target.
    mat[0] = new[2];
    mat[0][0] = 10; mat[0][1] = 20;
    mat[0] = new[f_sz()](mat[f_idx()]);
    $display("S10_MAT|%0d%0d %p", sz_calls, idx_calls, mat[0]);

    // Mixed fixed + dynamic initializer.
    $display("S10_MAT2|%p", mat2[1]);

    // Descending / non-zero fixed array source in new[](src).
    d_from_desc = new[4](f_desc);
    $display("S10_FDESC|%0d %0d %p", d_from_desc[0], d_from_desc[3], d_from_desc);

    // '{default: 7}
    $display("S10_DFLT|%p", d_dflt);

    // §10.10.3 nested concatenation preserved in dynamic array init.
    d_cat = {{4'hA, 4'hB}, 8'hCC};
    $display("S10_CONCAT|%0x %0x", d_cat[0], d_cat[1]);

    // Function-call source and array slice source.
    d_fn = new[4](make_arr());
    $display("S10_FUNC|%0d %0d %p", d_fn[0], d_fn[3], d_fn);
    d_sl = new[3](a_slice[1:3]);
    $display("S10_SLICE|%p", d_sl);

    // Negative and X/Z sizes report runtime errors without clobbering target.
    d_err = new[2];
    d_err[0] = 5; d_err[1] = 6;
    d_err = new[-3];
    $display("S10_NEG|size=%0d v0=%0d", d_err.size(), d_err[0]);
    xz_n = 32'bx;
    d_err = new[xz_n];
    $display("S10_XZ|size=%0d v1=%0d", d_err.size(), d_err[1]);
  end
endmodule
"#,
        100,
    )
    .expect("simulation should succeed");
    let out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert_has(&out, "S10_MAT|11 '{10, 20}");
    assert_has(&out, "S10_MAT2|'{300, 400}");
    assert_has(&out, "S10_FDESC|10 40 '{10, 20, 30, 40}");
    assert_has(&out, "S10_DFLT|'{7, 7, 7, 7}");
    assert_has(&out, "S10_CONCAT|ab cc");
    assert_has(&out, "S10_FUNC|11 44 '{11, 22, 33, 44}");
    assert_has(&out, "S10_SLICE|'{20, 30, 40}");
    assert_has(&out, "S10_NEG|size=2 v0=5");
    assert_has(&out, "S10_XZ|size=2 v1=6");
    let errs: Vec<&str> = out.iter().map(String::as_str).filter(|s| s.contains("** Error:")).collect();
    assert!(errs.len() >= 2, "expected negative and X/Z new[] errors, got {out:?}");
}

#[test]
fn fixed_dynamic_assignment_and_formal_writeback() {
    let sim = simulate(
        r#"
module top;
  int fix3[3] = '{11, 22, 33};
  int d2[];
  int f_desc[5:2] = '{10, 20, 30, 40};
  int f_off[5:2] = '{1, 2, 3, 4};

  function automatic int first_f(input int x[0:3]);
    return x[0];
  endfunction

  task automatic write_f(output int x[0:3]);
    x[0] = 101; x[1] = 102; x[2] = 103; x[3] = 104;
  endtask

  task automatic write_d(output int d[]);
    d = new[4];
    d[0] = 201; d[1] = 202; d[2] = 203; d[3] = 204;
  endtask

  task automatic write_d_mismatch(output int d[]);
    d = new[2];
    d[0] = 301; d[1] = 302;
  endtask

  initial begin
    d2 = new[2];
    d2[0] = 99; d2[1] = 88;
    fix3 = d2;
    $display("S12_FIX_DYN|%p", fix3);
    $display("S12_CALL_MISMATCH|%0d", first_f(d2));
    $display("S12_FIRST_F|%0d", first_f(f_desc));
    write_f(f_off);
    $display("S12_WRITE_F|%0d %0d %p", f_off[5], f_off[2], f_off);
    write_d(f_off);
    $display("S12_WRITE_D|%0d %0d left=%0d %p", f_off[5], f_off[2], $left(f_off), f_off);
    write_d_mismatch(f_off);
    $display("S12_WRITE_D_MIS|%0d %0d left=%0d", f_off[5], f_off[2], $left(f_off));
  end
endmodule
"#,
        100,
    )
    .expect("simulation should succeed");
    let out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert_has(&out, "S12_FIX_DYN|'{11, 22, 33}");
    assert_has(&out, "S12_CALL_MISMATCH|0");
    assert_has(&out, "S12_FIRST_F|10");
    assert_has(&out, "S12_WRITE_F|101 104 '{101, 102, 103, 104}");
    assert_has(&out, "S12_WRITE_D|201 204 left=5 '{201, 202, 203, 204}");
    assert_has(&out, "S12_WRITE_D_MIS|201 204 left=5");
    let errs: Vec<&str> = out.iter().map(String::as_str).filter(|s| s.contains("** Error:")).collect();
    assert!(errs.len() >= 3, "expected size-mismatch errors, got {out:?}");
}
