//! IEEE 1800-2023 §7.3 (Unions & Tagged Unions) semantics and legality checks,
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
fn unpacked_union_defaults_and_common_initial_sequence() {
    let out = sim_lines(
        r#"
module top;
  union { logic [31:0] l; int i; } u_lfirst;
  union { int i; logic [31:0] l; } u_ifirst;

  typedef struct { int tag; int val; byte extra; } s_a_t;
  typedef struct { int tag; int val; } s_b_t;
  typedef union { s_a_t a; s_b_t b; int first; } u_cis_t;
  u_cis_t u_cis;
  typedef union { int i; real r; } uu_t;
  uu_t uu_p;

  task automatic check_locals();
    union { logic [31:0] l; int i; } loc_lf;
    union { int i; logic [31:0] l; } loc_if;
    $display("LOC_DEF: %h %h", loc_lf.l, loc_if.i);
  endtask

  initial begin
    $display("DEF_INIT: lfirst=%h ifirst=%h", u_lfirst.l, u_ifirst.i);
    check_locals();
    u_cis.a = '{11, 22, 8'h33};
    $display("CIS: tag=%0d val=%0d first=%0d", u_cis.b.tag, u_cis.b.val, u_cis.first);
    uu_p.i = 65;
    $display("UUP=%p", uu_p);
  end
endmodule
"#,
    );
    assert_has(&out, "DEF_INIT: lfirst=xxxxxxxx ifirst=00000000");
    assert_has(&out, "LOC_DEF: xxxxxxxx 00000000");
    assert_has(&out, "CIS: tag=11 val=22 first=11");
    assert_has(&out, "UUP='{i:65, r:3.21143e-322}");
}

#[test]
fn soft_packed_unions_and_net_tagged_packed_union() {
    struct Sv2023Guard(bool);
    impl Drop for Sv2023Guard {
        fn drop(&mut self) {
            xezim::sv_parser::set_sv2023(self.0);
        }
    }
    let _guard = Sv2023Guard(xezim::sv_parser::is_sv2023());
    xezim::sv_parser::set_sv2023(true);
    let out = sim_lines(
        r#"
module top;
  typedef struct packed {
    bit [15:0] addr;
    union soft {
      bit [3:0] mode_a;
      bit [7:0] mode_b;
    } data;
  } data_u_t;
  data_u_t du;

  typedef union soft packed {
    logic [3:0] n4;
    logic [11:0] w12;
  } soft_p_t;
  soft_p_t spu;

  typedef union tagged packed {
    void A;
    logic [7:0] B;
  } tp_t;
  wire tp_t w1;
  wire union tagged packed {
    void A;
    logic [7:0] B;
  } w2;

  assign w1 = tagged B 8'h42;
  assign w2 = tagged B 8'h99;

  initial begin
    du = '0;
    du.addr = 16'hABCD;
    du.data.mode_b = 8'h5A;
    spu = '0;
    spu.w12 = 12'h345;
    #1;
    $display("FIG7_1: bits=%0d addr=%h mode_b=%h mode_a=%h",
             $bits(data_u_t), du.addr, du.data.mode_b, du.data.mode_a);
    $display("SOFT_P: bits=%0d w12=%h n4=%h", $bits(soft_p_t), spu.w12, spu.n4);
    $display("WIRE_TP: %h %h", w1.B, w2.B);
  end
endmodule
"#,
    );
    assert_has(&out, "FIG7_1: bits=24 addr=abcd mode_b=5a mode_a=a");
    assert_has(&out, "SOFT_P: bits=12 w12=345 n4=5");
    assert_has(&out, "WIRE_TP: 42 99");
}

#[test]
fn tagged_union_tag_widths_alignment_nesting_and_runtime_ops() {
    let out = sim_lines(
        r#"
module top;
  typedef union tagged packed { void A; logic [7:0] B; } ut2_t;
  typedef union tagged packed { void A; logic [7:0] B; logic [7:0] C; } ut3_t;
  typedef union tagged packed { void A; logic [7:0] B; logic [7:0] C; logic [7:0] D; logic [7:0] E; } ut5_t;

  typedef union tagged packed {
    void Invalid;
    bit [31:0] Valid;
  } v_packed_t;
  v_packed_t vp, vp_cond;

  typedef union tagged packed {
    void A;
    bit [3:0] Narrow;
    bit [7:0] Wide;
  } ut_align_t;
  ut_align_t ua;

  typedef union tagged packed {
    bit [4:0] Reg;
    bit [9:0] Addr;
    struct packed { bit [1:0] cc; bit [9:0] addr; } JmpC;
  } jmp_u_t;
  typedef union tagged packed {
    struct packed { bit [4:0] dst; bit [4:0] src; } Add;
    jmp_u_t Jmp;
  } instr_p_t;
  instr_p_t ip;

  typedef union tagged {
    void Inv;
    byte Val;
    int Num;
    string Str;
    struct { int a; int b; } Pair;
  } u_unp_t;
  u_unp_t uu, uu2, uu_nba, uu_str, uu_pair, uu_arr[2];
  bit cond;

  function automatic u_unp_t make_val(byte x);
    return tagged Val x;
  endfunction

  initial begin
    $display("UT_BITS: ut2=%0d ut3=%0d ut5=%0d", $bits(ut2_t), $bits(ut3_t), $bits(ut5_t));

    vp = tagged Valid 32'h12345678;
    $display("VP: tagbit=%b val=%h", vp[32], vp.Valid);

    ua = tagged Narrow 4'hA;
    $display("UA_ALIGN: bits=%0d tag=%b low4=%b val=%h",
             $bits(ut_align_t), ua[9:8], ua[3:0], ua.Narrow);

    ip = tagged Jmp (tagged JmpC '{cc: 2'b10, addr: 10'h155});
    $display("FIG7_3: bits=%0d t1=%b t2=%b low12=%h cc=%b addr=%h",
             $bits(ip), ip[13], ip[12], ip[11:0], ip.Jmp.JmpC.cc, ip.Jmp.JmpC.addr);
    case (ip) matches
      tagged Jmp (tagged JmpC '{cc: .c, addr: .a}): $display("IP_MATCH: cc=%b addr=%h", c, a);
      default: $display("IP_MATCH: FAIL");
    endcase

    uu = tagged Val (-8'sd5);
    $display("UU_VAL: v=%0d bits=%0d lt0=%0d p=%p", uu.Val, $bits(uu.Val), (uu.Val < 0), uu);

    uu2 = tagged Inv;
    $display("UU_INV: p=%p", uu2);
    uu2 = uu;
    $display("UU_COPY: v=%0d", uu2.Val);

    uu_str = tagged Str "hello";
    $display("UU_STR: s=%s", uu_str.Str);

    uu_pair = tagged Pair '{10, 20};
    case (uu_pair) matches
      tagged Pair '{.x, .y}: $display("PAIR_MATCH: %0d %0d", x, y);
      default: $display("PAIR_MATCH: FAIL");
    endcase

    uu = tagged Num 42;
    if (uu matches tagged Num 42)
      $display("CONST_PAT: HIT");
    else
      $display("CONST_PAT: MISS");

    uu_arr[1] = tagged Num 77;
    $display("ARR_TU: %0d", uu_arr[1].Num);

    uu = make_val(-8'sd9);
    $display("RET_TU: %0d", uu.Val);

    cond = 1'b1;
    vp_cond = cond ? tagged Valid 32'hCAFEBABE : tagged Invalid;
    $display("COND_TU: %h", vp_cond.Valid);

    uu_nba = tagged Inv;
    uu_nba <= tagged Num 99;
    #1;
    $display("NBA_TU: %0d", uu_nba.Num);
  end
endmodule
"#,
    );
    assert_has(&out, "UT_BITS: ut2=9 ut3=10 ut5=11");
    assert_has(&out, "VP: tagbit=1 val=12345678");
    assert_has(&out, "UA_ALIGN: bits=10 tag=01 low4=1010 val=a");
    assert_has(&out, "FIG7_3: bits=15 t1=1 t2=0 low12=955 cc=10 addr=155");
    assert_has(&out, "IP_MATCH: cc=10 addr=155");
    assert_has(&out, "UU_VAL: v=-5 bits=8 lt0=1 p='{tagged Val:-5}");
    assert_has(&out, "UU_INV: p='{tagged Inv:void}");
    assert_has(&out, "UU_COPY: v=-5");
    assert_has(&out, "UU_STR: s=hello");
    assert_has(&out, "PAIR_MATCH: 10 20");
    assert_has(&out, "CONST_PAT: HIT");
    assert_has(&out, "ARR_TU: 77");
    assert_has(&out, "RET_TU: -9");
    assert_has(&out, "COND_TU: cafebabe");
    assert_has(&out, "NBA_TU: 99");
}

#[test]
fn union_syntax_type_and_tagged_write_rejections() {
    for body in [
        "typedef union packed { bit [7:0] a; bit [15:0] b; } u_t; u_t u;",
        "module sub #(parameter W=8); typedef union packed { logic [W-1:0] a; logic [15:0] b; } u_t; endmodule module top; sub #(8) s(); endmodule",
        "typedef union packed tagged { void A; int B; } u_t;",
        "typedef union soft tagged { void A; int B; } u_t;",
        "typedef union tagged soft { void A; int B; } u_t;",
        "typedef union { logic [7:0] a; int b; } u_t; wire u_t w;",
        "wire union { logic [7:0] a; int b; } w;",
        "typedef union { string s; int i; } u_t; u_t u;",
        "typedef union { int d[]; int i; } u_t; u_t u;",
        "typedef union { chandle c; int i; } u_t; u_t u;",
        "typedef union { event e; int i; } u_t; u_t u;",
        "typedef struct { string s; } s_t; typedef union { s_t a; int i; } u_t; u_t u;",
        "typedef union tagged packed { void A; string B; } u_t; u_t u;",
        "typedef union tagged { void Inv; int Val; } u_t; u_t u; initial u.Val = 5;",
        "typedef union tagged { void Inv; int Val; } u_t; u_t u; initial u.Val <= 5;",
        "typedef union tagged { void Inv; int Val; } u_t; u_t u; assign u.Val = 5;",
    ] {
        let src = if body.contains("module top;") {
            body.to_string()
        } else {
            format!("module top; {body} endmodule")
        };
        let _ = rejected(&src);
    }
}

#[test]
fn tagged_union_inactive_member_read_sets_fatal_and_error() {
    let sim = simulate(
        r#"
module top;
  typedef union tagged { void Inv; int Val; } u_t;
  u_t u;
  int x;
  initial begin
    u = tagged Inv;
    x = u.Val;
  end
endmodule
"#,
        10,
    )
    .expect("simulation runs to report runtime error");
    assert!(sim.error_count > 0);
    assert!(sim.saw_fatal);
}
