//! IEEE 1800-2023 §7.2 (Structures) semantics and legality checks,
//! verified byte-for-byte against reference simulators.

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

fn rejected(src: &str) -> String {
    match simulate(src, 100) {
        Ok(_) => panic!("expected design to be rejected:\n{src}"),
        Err(e) => e,
    }
}

#[test]
fn unpacked_struct_member_defaults_and_nested_shapes() {
    let out = sim_lines(
        r#"
module top;
  typedef struct {
    int a = 10;
    int b = 20;
  } def_s_t;
  def_s_t d0;
  def_s_t d1 = '{a: 99, b: 88};
  def_s_t d2 = '{a: 77, default: 0};

  typedef struct {
    int a = 42;
  } inner_t;
  typedef struct {
    inner_t in;
  } outer_t;
  outer_t outer;
  inner_t arr3d[2][2][2];

  typedef struct {
    int a[2] = '{11, 22};
  } arr_mem_t;
  arr_mem_t ais[2];

  typedef enum bit [1:0] { E0, E1 } e2_t;
  typedef struct {
    e2_t e;
    logic [1:0] l;
  } enum_mem_t;
  enum_mem_t em;

  function automatic int check_local();
    outer_t loc_out;
    arr_mem_t loc_am;
    return (loc_out.in.a == 42 && loc_am.a[0] == 11 && loc_am.a[1] == 22) ? 1 : 0;
  endfunction

  int ok_loc;
  initial begin
    ok_loc = check_local();
    $display("D0: %0d %0d", d0.a, d0.b);
    $display("D1: %0d %0d", d1.a, d1.b);
    $display("D2: %0d %0d", d2.a, d2.b);
    $display("NESTED: %0d", outer.in.a);
    $display("ARR3D: %0d", arr3d[1][1][1].a);
    $display("AIS: %0d %0d", ais[1].a[0], ais[1].a[1]);
    $display("ENUM2: %b %b", em.e, em.l);
    $display("LOC_OK: %0d", ok_loc);
  end
endmodule
"#,
    );
    assert_has(&out, "D0: 10 20");
    assert_has(&out, "D1: 99 88");
    assert_has(&out, "D2: 77 0");
    assert_has(&out, "NESTED: 42");
    assert_has(&out, "ARR3D: 42");
    assert_has(&out, "AIS: 11 22");
    assert_has(&out, "ENUM2: 00 xx");
    assert_has(&out, "LOC_OK: 1");
}

#[test]
fn mixed_packed_locals_signed_array_and_typedef_array_bits() {
    let out = sim_lines(
        r#"
module top;
  typedef struct packed {
    bit [3:0] x;
    logic [3:0] y;
  } mix_inner_t;
  typedef struct packed {
    logic [3:0] a;
    mix_inner_t b;
  } mix_outer_t;

  function automatic int check_mixed_local();
    mix_outer_t loc;
    loc.a = 4'bxxxx;
    loc.b.x = 4'bxxxx;
    loc.b.y = 4'bxxxx;
    $display("MIX_LOCAL: a=%b bx=%b by=%b", loc.a, loc.b.x, loc.b.y);
    return (loc.b.x === 4'b0000) ? 1 : 0;
  endfunction

  typedef struct packed signed { logic [3:0] v; } sp4_t;
  sp4_t [1:0] sp_arr = 8'hFF;

  typedef logic [7:0] a3_t[3];
  typedef struct {
    a3_t a;
    logic [7:0] b;
  } bits_s_t;
  bits_s_t bs;

  int ok_mix;
  initial begin
    ok_mix = check_mixed_local();
    $display("MIX_OK: %0d", ok_mix);
    $display("SP_ARR: gt0=%0d elem_lt0=%0d", (sp_arr > 0), (sp_arr[0] < 0));
    $display("BITS: type=%0d var=%0d", $bits(bits_s_t), $bits(bs));
  end
endmodule
"#,
    );
    assert_has(&out, "MIX_LOCAL: a=xxxx bx=0000 by=xxxx");
    assert_has(&out, "MIX_OK: 1");
    assert_has(&out, "SP_ARR: gt0=1 elem_lt0=1");
    assert_has(&out, "BITS: type=32 var=32");
}

#[test]
fn struct_syntax_and_member_legality_rejections() {
    for body in [
        "struct signed { int a; } s;",
        "struct tagged { int a; } s;",
        "struct { int a; } [3:0] s;",
        "struct { void a; int b; } s;",
        "struct packed { rand bit [3:0] a; } s;",
        "struct packed { real r; } [1:0] s;",
        "typedef chandle ch_t; struct packed { ch_t c; } s;",
        "typedef union { int x; } u_t; struct { int a = 1; u_t u; } s;",
        "int g; struct { int a = g; } s;",
        "int g; typedef struct { int a = g; } s_t; s_t s;",
        "struct { int a; } s1; struct { int a; } s2; initial s1 = s2;",
        "struct { int a; } s1; initial s1 = 42;",
    ] {
        let _ = rejected(&format!("module top; {body} endmodule"));
    }
}

#[test]
fn struct_nominal_compatibility_legal_cases() {
    let out = sim_lines(
        r#"
module top;
  struct { int a; } s1, s2;
  typedef struct { int a; } s_t;
  typedef s_t s2_t;
  s_t t1;
  s2_t t2;
  initial begin
    s1 = '{a: 5};
    s2 = s1;
    t1 = '{a: 9};
    t2 = t1;
    $display("OK: %0d %0d", s2.a, t2.a);
  end
endmodule
"#,
    );
    assert_has(&out, "OK: 5 9");
}
