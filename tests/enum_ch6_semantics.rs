//! IEEE 1800-2023 Chapter 6.19 & 6.24.2 enum semantics tests (Milestone 1).
//!
//! Covers:
//! - §6.8 Table 6-7: 2-state enum via typedef base defaults to 0.
//! - §6.19: anonymous enum & submodule enum signedness, ranges, 4-state X/Z, >64-bit values.
//! - §6.19: post-cast auto-increment legality (unsigned overflow vs signed wrap).
//! - §6.19.4: `++` / `--` on enum variables rejected at compile/lint time.
//! - §6.19.5: `.next(N)` / `.prev(N)` `int unsigned` step promotion, paren-less dispatch,
//!   cast-receiver `(e_t'(...)).name()`, and class-context enum receiver cache isolation.
//! - §6.24.2: `$cast` into scalar/element/struct/class/typedef enums, width/signedness/4-state/real.

use xezim::simulate;

fn rejected(src: &str) {
    if simulate(src, 100).is_ok() {
        panic!("expected compile/lint error, but simulation succeeded:\n{src}");
    }
}

fn lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 200).unwrap_or_else(|e| panic!("simulation failed: {e}\n{src}"));
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn enum_two_state_typedef_base_and_anon_signedness() {
    let src = r#"
module top;
  typedef bit [1:0] b2_t;
  typedef enum b2_t { B0 = 2'd2, B1 = 2'd3 } b2_e;
  typedef logic [1:0] l2_t;
  typedef enum l2_t { L0 = 2'd2, L1 = 2'd3 } l2_e;

  b2_e bx;
  l2_e lx;
  enum byte { ANEG = -8'sd1, APOS = 8'sd1 } ab;
  enum logic signed [3:0] { SNEG = -4'sd1, SXZ = 4'b10x1 } sb;

  initial begin
    ab = ANEG;
    sb = SXZ;
    $display("DEF|%b|%b|%0b|%0d|%0d|%0b|%0d|%b",
      bx, lx, bx === 2'b00, ANEG, ab < 0, SNEG < 0, sb === 4'b10x1, sb);
  end
endmodule
"#;
    assert_eq!(lines(src), vec!["DEF|00|xx|1|-1|1|1|1|10x1"]);
}

#[test]
fn submodule_enum_members_ranges_4state_wide_signed() {
    let src = r#"
module sub #(parameter int BASE = 10) ();
  typedef enum byte { SNEG = -8'sd2, SNEXT } se_t;
  typedef enum { RG[3] = BASE } rg_t;
  typedef enum logic [1:0] { X0 = 2'b00, XX = 2'b1x, XZ = 2'b0z } xz_t;
  typedef enum logic [79:0] { W0 = 80'h1234_5678_9abc_def0_0001, W1 } w_t;

  se_t se;
  rg_t rg;
  xz_t xz;
  w_t  wv;

  initial begin
    se = SNEXT;
    rg = RG1;
    xz = XX;
    wv = W1;
    $display("SUB|%0d|%0d|%0b|%0d|%0d|%s|%s|%h|%s",
      SNEG, se, se < 0, rg.num(), rg, xz.name(), XX === 2'b1x ? "OK" : "NO", wv, wv.name());
  end
endmodule

module top;
  sub #(10) u1();
  sub #(20) u2();
endmodule
"#;
    let out = lines(src);
    assert_eq!(out.len(), 2);
    assert!(out.contains(&"SUB|-2|-1|1|3|11|XX|OK|123456789abcdef00002|W1".to_string()));
    assert!(out.contains(&"SUB|-2|-1|1|3|21|XX|OK|123456789abcdef00002|W1".to_string()));
}

#[test]
fn enum_post_cast_auto_increment_and_incr_decr_lint() {
    // 4'd15 in 4-bit signed wraps to -1, so auto-increment produces 0 (legal).
    let legal = r#"
module top;
  enum bit signed [3:0] { VA = 4'd15, VB } v;
  initial begin
    v = VB;
    $display("POST|%0d|%0d|%s", VA, v, v.name());
  end
endmodule
"#;
    assert_eq!(lines(legal), vec!["POST|-1|0|VB"]);

    // -4'sd1 in 4-bit unsigned is 15, so auto-increment is 16 (overflow).
    rejected("module top; enum bit [3:0] { A = -4'sd1, B } c; endmodule");

    // §6.19.4: increment/decrement operators forbidden on enum variables.
    rejected("module top; typedef enum {A, B} e_t; e_t c; initial begin c = A; c++; end endmodule");
    rejected("module top; typedef enum {A, B} e_t; e_t c; initial begin c = A; ++c; end endmodule");
    rejected("module top; typedef enum {A, B} e_t; e_t c; initial begin c = B; c--; end endmodule");
    rejected("module top; typedef enum {A, B} e_t; e_t c; initial begin c = B; --c; end endmodule");
    rejected(
        "module top; typedef enum {A, B} e_t; struct packed { e_t e; } s;\n\
         initial begin s.e = A; s.e++; end endmodule",
    );
    rejected(
        "module top; typedef enum {A, B} e_t; class C; e_t e; endclass C obj;\n\
         initial begin obj = new; obj.e = A; obj.e++; end endmodule",
    );
    rejected(
        "module top; typedef enum {A, B} e_t; e_t arr[2];\n\
         initial begin arr[0] = A; arr[0]++; end endmodule",
    );
    rejected(
        "module top; typedef enum {A, B} e_t;\n\
         initial for (e_t x = A; x != B; x++) ; endmodule",
    );
}

#[test]
fn enum_methods_signed_step_parenless_cast_receiver_and_class_cache() {
    let src = r#"
package pkg_e;
  typedef enum logic [2:0] { RUN = 3'b011, STOP = 3'b100 } st_t;
endpackage

module top;
  import pkg_e::*;
  typedef enum { M0, M1, M2, M3, M4, M5, M6, M7, M8, M9, M10 } e11_t;
  typedef struct packed { st_t s; } box_t;

  class C1;
    typedef enum { X0 = 10, X1 = 11 } dup_e;
    dup_e p;
    function string get_name(); return p.name(); endfunction
  endclass

  class C2;
    typedef enum { X0 = 20, X1 = 21 } dup_e;
    dup_e p;
    function string get_name(); return p.name; endfunction
  endclass

  function automatic st_t mk_st(logic [2:0] bits);
    return st_t'(bits);
  endfunction

  function automatic string shadow_check();
    st_t c1;
    bit ok = $cast(c1, 3);
    return {ok ? "1:" : "0:", c1.name(), ":", c1.name};
  endfunction

  e11_t e11;
  byte bn;
  box_t b;
  C1 c1;
  C2 c2;
  st_t st;

  initial begin
    e11 = M0;
    bn = -1;
    b.s = RUN;
    c1 = new; c1.p = C1::X0;
    c2 = new; c2.p = C2::X1;
    st = st_t'(3'b001);
    $display("METH|%0d|%0d|%s|%s|%s|%s|%s|%s",
      e11.next(bn), e11.prev(bn),
      e11.name, b.s.name, c1.get_name(), c2.get_name(),
      mk_st(3'b001).name(), (st_t'(3'b001)).name());
    $display("METH2|%0d|%0d|%0d|%s|%s|%s|%s",
      st.num, b.s.first, b.s.last, st.name, shadow_check(), c1.p.name(), c2.p.name());
  end
endmodule
"#;
    assert_eq!(
        lines(src),
        vec![
            "METH|3|8|M0|RUN|X0|X1||",
            "METH2|2|3|4||1:RUN:RUN|X0|X1",
        ]
    );
}

#[test]
fn enum_dollar_cast_shapes_signedness_width_xz_and_real() {
    let src = r#"
package epkg;
  typedef enum byte { ENEG = -8'sd1, EP = 8'sd1 } eb_t;
endpackage

module top;
  import epkg::*;
  typedef enum bit [7:0] { U0 = 8'h00, UFF = 8'hff } eu_t;
  typedef enum bit [7:0] { U15 = 8'd15, UFF2 = 8'hff } eu15_t;
  typedef enum byte { E0_15 = 8'sd0, E15 = 8'sd15 } eb15_t;
  typedef enum int { IA = -1, IB = 1 } ei_t;
  typedef enum logic [1:0] { E0 = 2'b00, EX = 2'b1x, EZ = 2'b1z } exz_t;
  typedef struct packed { eb_t s_eb; eu_t s_eu; } st_t;
  class Holder;
    eb_t c_eb;
    eu_t c_eu;
  endclass

  eb_t eb;
  eb15_t eb15;
  eb_t arr_eb[2];
  eb_t q_eb[$];
  eb_t aa_eb[string];
  st_t s;
  Holder h;
  eu_t eu;
  eu15_t eu15;
  ei_t ei;
  exz_t exz;

  logic [7:0] u8ff = 8'hff;
  logic [8:0] u9ff = 9'h0ff;
  logic [8:0] u9_1ff = 9'h1ff;
  logic signed [8:0] s9_1ff = -9'sd1;
  logic signed [3:0] s4neg = -4'sd1;
  logic [31:0] u32ff = 32'hffff_ffff;
  logic [63:0] u64_32ff = 64'h0000_0000_ffff_ffff;
  logic [63:0] u64_allff = 64'hffff_ffff_ffff_ffff;
  int ineg = -1;
  int i255 = 255;
  int in129 = -129;
  longint lneg = -64'sd1;
  real r1 = 1.0;
  real r0 = 0.0;
  logic [1:0] vx = 2'b1x;
  logic [1:0] vbadx = 2'b0x;
  logic [3:0] v4x = 4'b001x;

  initial begin
    q_eb.push_back(EP);
    aa_eb["k"] = EP;
    h = new;

    $display("EB|%0d %0d %0d %0d %0d %0d %0d %0d %0d %0d",
      $cast(eb, u8ff), $cast(eb, u9ff), $cast(eb, u9_1ff),
      $cast(eb, s9_1ff), $cast(eb, s4neg), $cast(eb, u32ff),
      $cast(eb, ineg), $cast(eb, i255), $cast(eb, in129),
      $cast(eb15, s4neg));
    $display("EU|%0d %0d %0d %0d %0d %0d %0d",
      $cast(eu, u8ff), $cast(eu, u9ff), $cast(eu, s9_1ff),
      $cast(eu, s4neg), $cast(eu, u32ff), $cast(eu, ineg),
      $cast(eu15, s4neg));
    $display("EI|%0d %0d %0d %0d %0d %0d",
      $cast(ei, u32ff), $cast(ei, u64_32ff), $cast(ei, u64_allff),
      $cast(ei, lneg), $cast(ei, r1), $cast(ei, r0));
    $display("XZ|%0d %0d %0d %0d",
      $cast(exz, vx), $cast(exz, vbadx), $cast(exz, v4x), $cast(exz, 2'b00));
    $display("DEST|%0d %0d %0d %0d %0d %0d",
      $cast(arr_eb[0], ineg), $cast(q_eb[0], -1), $cast(aa_eb["k"], 99),
      $cast(s.s_eb, u32ff), $cast(h.c_eb, ineg), $cast(h.c_eu, ineg));
  end
endmodule
"#;
    assert_eq!(
        lines(src),
        vec![
            "EB|1 1 0 1 1 0 1 1 0 0",
            "EU|1 1 0 0 0 0 1",
            "EI|1 1 0 1 0 0",
            "XZ|1 0 1 1",
            "DEST|1 1 0 0 1 0",
        ]
    );
}
