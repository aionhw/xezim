//! IEEE 1800-2023 §6.18 (typedefs, forward typedefs, interface-based typedefs),
//! §6.22.1 (matching built-in class specializations), and §6.25 / §8.25.1
//! (parameterized data types & unspecialized parameterized class checks).

use xezim::simulate;

fn sim_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 100)
        .unwrap_or_else(|e| panic!("expected simulation to succeed, got: {e}"));
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn expect_fail(src: &str, needle: &str) {
    match simulate(src, 20) {
        Ok(sim) => panic!(
            "expected failure matching {needle:?}, but simulation succeeded with output {:?}",
            sim.output
        ),
        Err(e) => {
            let low = e.to_lowercase();
            assert!(
                low.contains(&needle.to_lowercase()),
                "expected error containing {needle:?}, got: {e}"
            );
        }
    }
}

#[test]
fn interface_based_typedefs_scalar_param_forwarded_and_indexed() {
    let src = r#"
interface simple_if;
  typedef logic [15:0] data_t;
endinterface

interface param_if #(parameter int W = 8);
  typedef logic [W-1:0] pdata_t;
endinterface

module sub_iface(simple_if p, param_if qp, simple_if arr[2]);
  typedef p.data_t my_t;
  typedef qp.pdata_t my_pt;
  typedef arr[0].data_t idx0_t;
  typedef arr[1].data_t idx1_t;
  my_t v;
  my_pt pv;
  idx0_t a0;
  idx1_t a1;
  initial begin
    #1;
    v = 16'hA55A;
    pv = '1;
    a0 = 16'h1234;
    a1 = 16'h5678;
    $display("IF|%0d %0h %0d %0h %0d %0h %0d %0h",
      $bits(v), v, $bits(pv), pv, $bits(a0), a0, $bits(a1), a1);
  end
endmodule

module fwd_iface_wrap(simple_if wp, param_if qp);
  typedef wp.data_t wrap_t;
  typedef qp.pdata_t wrap_pt;
  wrap_t wt;
  wrap_pt wpt;
  initial begin
    #1;
    wt = 16'h5A5A;
    wpt = 12'h3C3;
    $display("FW|%0d %0h %0d %0h", $bits(wt), wt, $bits(wpt), wpt);
  end
endmodule

module top;
  simple_if u_sif();
  param_if #(12) u_pif();
  simple_if u_arr[2]();
  sub_iface u_sub(.p(u_sif), .qp(u_pif), .arr(u_arr));
  fwd_iface_wrap u_fwd(.wp(u_sif), .qp(u_pif));
endmodule
"#;
    let lines = sim_lines(src);
    assert!(lines.contains(&"IF|16 a55a 12 fff 16 1234 16 5678".to_string()), "{lines:?}");
    assert!(lines.contains(&"FW|16 5a5a 12 3c3".to_string()), "{lines:?}");
}

#[test]
fn forward_typedefs_positive_forms_and_deferred_aliases() {
    let src = r#"
package p_fwd;
  typedef class pc_t;
  typedef pc_t::pinner_t p_alias_t;
  p_alias_t pv;
  class pc_t;
    typedef logic [9:0] pinner_t;
  endclass
endpackage

module top;
  import p_fwd::*;
  typedef enum e_fwd_t;
  typedef struct s_fwd_t;
  typedef union u_fwd_t;
  typedef class c_fwd_t;
  typedef interface class ic_fwd_t;
  typedef bare_fwd_t;
  typedef bare_fwd_t; // duplicate forward typedef is legal
  typedef c_fwd_t::inner_t c_alias_t;
  parameter type P_FWD = c_fwd_t::inner_t;

  e_fwd_t ev;
  s_fwd_t sv;
  c_alias_t ca;
  P_FWD pf;

  typedef enum logic [1:0] { E0 = 2'd1, E1 = 2'd2 } e_fwd_t;
  typedef struct packed { logic [7:0] hi; logic [7:0] lo; } s_fwd_t;
  typedef union packed { logic [7:0] a; logic [7:0] b; } u_fwd_t;
  typedef logic [23:0] bare_fwd_t;
  typedef bare_fwd_t; // post-definition forward typedef is legal

  interface class ic_fwd_t;
    pure virtual function int get_id();
  endclass

  class c_fwd_t implements ic_fwd_t;
    typedef logic [11:0] inner_t;
    virtual function int get_id();
      return 42;
    endfunction
  endclass

  initial begin
    c_fwd_t cinst = new;
    ev = E1;
    sv.hi = 8'h12;
    sv.lo = 8'h34;
    ca = 12'hABC;
    pf = 12'h123;
    pv = 10'h2AA;
    $display("FT|%0d %0h %0d %0d %0d %0h %0d %0h %0d %0h",
      ev, sv, $bits(u_fwd_t), $bits(bare_fwd_t), cinst.get_id(),
      ca, $bits(ca), pf, $bits(pf), pv);
  end
endmodule
"#;
    let lines = sim_lines(src);
    assert!(
        lines.contains(&"FT|2 1234 8 24 42 abc 12 123 12 2aa".to_string()),
        "{lines:?}"
    );
}

#[test]
fn matching_and_distinct_builtin_specializations_and_typename() {
    let src = r#"
class StatBox #(type T = int);
  static int cnt = 0;
  static function string tn();
    return $typename(T);
  endfunction
endclass

module top;
  initial begin
    StatBox#(int)::cnt = 10;
    StatBox#(bit signed [31:0])::cnt += 1;
    StatBox#(byte)::cnt = 20;
    StatBox#(bit signed [7:0])::cnt += 2;
    StatBox#(shortint)::cnt = 30;
    StatBox#(bit signed [15:0])::cnt += 3;
    StatBox#(longint)::cnt = 40;
    StatBox#(bit signed [63:0])::cnt += 4;
    StatBox#(integer)::cnt = 50;
    StatBox#(logic signed [31:0])::cnt += 5;
    StatBox#(logic [31:0])::cnt = 60;
    StatBox#(reg [31:0])::cnt += 6;
    StatBox#(integer unsigned)::cnt += 1;
    StatBox#(real)::cnt = 70;
    StatBox#(realtime)::cnt += 7;
    StatBox#(bit)::cnt = 80;
    StatBox#(bit unsigned)::cnt += 8;

    StatBox#(bit [0:0])::cnt = 101;
    StatBox#(bit [3:0])::cnt = 102;
    StatBox#(bit [4:1])::cnt = 103;
    StatBox#(bit [0:3])::cnt = 104;
    StatBox#(int unsigned)::cnt = 105;
    StatBox#(logic [3:0])::cnt = 106;

    $display("SP|%0d %0d %0d %0d %0d %0d %0d %0d|%0d %0d %0d %0d %0d %0d",
      StatBox#(int)::cnt, StatBox#(byte)::cnt, StatBox#(shortint)::cnt, StatBox#(longint)::cnt,
      StatBox#(integer)::cnt, StatBox#(logic [31:0])::cnt, StatBox#(real)::cnt, StatBox#(bit)::cnt,
      StatBox#(bit [0:0])::cnt, StatBox#(bit [3:0])::cnt, StatBox#(bit [4:1])::cnt,
      StatBox#(bit [0:3])::cnt, StatBox#(int unsigned)::cnt, StatBox#(logic [3:0])::cnt);

    $display("TN|%s|%s|%s",
      StatBox#(bit signed [31:0])::tn(),
      StatBox#(bit unsigned)::tn(),
      StatBox#(bit [3:0])::tn());
  end
endmodule
"#;
    let lines = sim_lines(src);
    assert!(
        lines.contains(&"SP|11 22 33 44 55 67 77 88|101 102 103 104 105 106".to_string()),
        "{lines:?}"
    );
    let tn_line = lines.iter().find(|l| l.starts_with("TN|")).expect("TN line");
    assert!(tn_line.starts_with("TN|int|bit|bit[3:0]") || tn_line.starts_with("TN|int|bit|bit [3:0]"), "{tn_line}");
}

#[test]
fn parameterized_data_types_ports_nets_casts_and_extends() {
    let src = r#"
package p38;
  class PC #(int W = 8);
    typedef logic [W-1:0] w_t;
  endclass
endpackage

virtual class C38 #(type T = bit, int N = 4);
  typedef T [N-1:0] vec_t;
  typedef struct packed {
    vec_t a;
    logic [3:0] b;
  } pkt_t;
endclass

module sub_port38(input p38::PC#(10)::w_t in_v, output p38::PC#(10)::w_t out_v);
  assign out_v = ~in_v;
endmodule

module top;
  class Base #(int W = 4);
    typedef struct packed {
      bit [W-1:0] payload;
      bit flag;
    } word;
  endclass
  class Child #(int N = 3) extends Base#(N + 2);
  endclass

  typedef C38#(bit [7:0], 3) c3_t;
  typedef C38#() c0_t;

  p38::PC#(10)::w_t port_in;
  p38::PC#(10)::w_t port_out;
  sub_port38 u_sp(.in_v(port_in), .out_v(port_out));

  C38#(logic [3:0], 2)::vec_t w_vec;
  assign w_vec = 8'h3C;

  C38#(byte, 2)::vec_t v_b2;
  C38#(byte, 2)::pkt_t p_b2;
  C38#()::vec_t v_def;
  c3_t::vec_t v_c3;
  c0_t::vec_t v_c0;
  p38::PC#(12)::w_t pkg_w12;
  p38::PC#()::w_t pkg_wdef;
  Child#(8)::word ch_w;

  initial begin
    v_b2 = C38#(byte, 2)::vec_t'(32'h1_ABCD);
    v_c3 = c3_t::vec_t'(32'h12_3456);
    v_def = 4'h9;
    v_c0 = 4'h5;
    p_b2.a = 16'h1234;
    p_b2.b = 4'hF;
    pkg_w12 = 12'hFED;
    pkg_wdef = 8'hA5;
    ch_w = '1;
    port_in = 10'h155;
    #2;
    $display("P38|%0d %0h %0d %0d %0d %0d %0h %0d %0d %0h %0h %0d %0h",
      $bits(v_b2), v_b2, $bits(p_b2), $bits(v_def), $bits(v_c0),
      $bits(v_c3), v_c3, $bits(pkg_w12), $bits(pkg_wdef), w_vec, port_out,
      $bits(ch_w), ch_w.payload);
  end
endmodule
"#;
    let lines = sim_lines(src);
    assert!(
        lines.contains(&"P38|16 abcd 20 4 4 24 123456 12 8 3c 2aa 11 3ff".to_string()),
        "{lines:?}"
    );
}

#[test]
fn forward_typedef_negative_cases_are_rejected() {
    // Unresolved forward typedefs in module, package, generate, and $unit.
    expect_fail("module top; typedef class Missing; endmodule", "does not resolve");
    expect_fail("module top; typedef enum Missing; endmodule", "does not resolve");
    expect_fail("module top; typedef interface class Missing; endmodule", "does not resolve");
    expect_fail("module top; typedef Missing; endmodule", "does not resolve");
    expect_fail("package p; typedef class Missing; endpackage module top; endmodule", "does not resolve");
    expect_fail(
        "module top; if (1) begin : g typedef class Missing; end endmodule",
        "does not resolve",
    );
    expect_fail("typedef class Missing; module top; endmodule", "does not resolve");
    expect_fail(
        "typedef class Missing; module inner; class Missing; endclass endmodule module top; endmodule",
        "does not resolve",
    );

    // Kind mismatches.
    expect_fail(
        "module top; typedef enum T; typedef struct packed { int a; } T; endmodule",
        "does not match",
    );
    expect_fail(
        "module top; typedef struct S; typedef int S; endmodule",
        "does not match",
    );
    expect_fail(
        "module top; typedef class IC; interface class IC; endclass endmodule",
        "does not match",
    );
    expect_fail(
        "module top; typedef interface class C; class C; endclass endmodule",
        "does not match",
    );
    expect_fail(
        "package p; typedef union U; typedef struct packed { int a; } U; endpackage module top; endmodule",
        "does not match",
    );

    // Premature C::T variable declaration vs legal typedef/type-param uses.
    expect_fail(
        "module top; typedef C; C::T x; class C; typedef int T; endclass endmodule",
        "before forward typedef",
    );
    let legal_typedef = r#"
module top;
  typedef C;
  typedef C::T c_t;
  parameter type P = C::T;
  c_t y;
  P z;
  class C;
    typedef logic [7:0] T;
  endclass
  initial begin
    y = 8'h5A;
    z = 8'hA5;
    $display("OK|%0d %0h %0d %0h", $bits(y), y, $bits(z), z);
  end
endmodule
"#;
    assert!(sim_lines(legal_typedef).contains(&"OK|8 5a 8 a5".to_string()));

    // Premature interface class implements / extends vs legal order after definition.
    expect_fail(
        "module top; typedef interface class IC; class D implements IC; endclass interface class IC; endclass endmodule",
        "before its definition",
    );
    expect_fail(
        "module top; typedef interface class IC; interface class D extends IC; endclass interface class IC; endclass endmodule",
        "before its definition",
    );
    let legal_ic = r#"
module top;
  typedef interface class IC;
  interface class IC;
    pure virtual function int f();
  endclass
  class D implements IC;
    virtual function int f(); return 7; endfunction
  endclass
  initial begin D d = new; $display("IC=%0d", d.f()); end
endmodule
"#;
    assert!(sim_lines(legal_ic).contains(&"IC=7".to_string()));
}

#[test]
fn unspecialized_parameterized_class_scope_negative_and_positive_controls() {
    // Negative cases: unspecialized parameterized class before `::`.
    expect_fail(
        "class PC #(int W = 4); typedef int T; endclass module top; PC::T x; endmodule",
        "needs a parameter value list",
    );
    expect_fail(
        "class PC #(int W = 4); typedef int T; endclass module top; typedef PC::T u_t; endmodule",
        "needs a parameter value list",
    );
    expect_fail(
        "class PC #(int W = 4); typedef logic [W-1:0] T; endclass module sub(input PC::T x); endmodule module top; endmodule",
        "needs a parameter value list",
    );
    expect_fail(
        "class PC #(int W = 4); static int K = 1; endclass module top; int x; initial x = PC::K; endmodule",
        "needs a parameter value list",
    );
    expect_fail(
        "class PC #(int W = 4); typedef logic [W-1:0] T; endclass module top; int x; initial x = PC::T'(5); endmodule",
        "needs a parameter value list",
    );
    expect_fail(
        "package pkg; class PC #(int W = 4); typedef int T; endclass endpackage module top; pkg::PC::T x; endmodule",
        "needs a parameter value list",
    );
    expect_fail(
        "package pkg; class PC #(int W = 4); typedef logic [W-1:0] T; endclass endpackage module top; int x; initial x = pkg::PC::T'(5); endmodule",
        "needs a parameter value list",
    );

    // Positive controls: PC#()::, pkg::PC#()::, self-scope PC::, out-of-block PC::m, and unrelated package.
    let positive_controls = r#"
package p_other;
  class PC #(int W = 4);
    typedef logic [W-1:0] T;
  endclass
endpackage

package p_plain;
  class PC;
    typedef logic [15:0] T;
    static int K = 9;
  endclass
  class Gen #(int W = 8);
    typedef logic [W-1:0] T;
    function T self_ok();
      Gen::T v = '1;
      return v;
    endfunction
    extern function T ext_m(input T x);
  endclass
  function Gen::T Gen::ext_m(input Gen::T x);
    return x;
  endfunction
endpackage

module top;
  import p_plain::*;
  PC::T plain_v;
  Gen#()::T g_def;
  p_other::PC#()::T o_def;
  initial begin
    Gen#(8) g = new;
    plain_v = 16'hBEEF;
    g_def = g.self_ok();
    o_def = 4'hA;
    $display("CTRL|%0d %0h %0d %0h %0d %0h %0d",
      $bits(plain_v), plain_v, $bits(g_def), g_def, $bits(o_def), o_def, PC::K + g.ext_m(8'h11));
  end
endmodule
"#;
    assert!(sim_lines(positive_controls).contains(&"CTRL|16 beef 8 ff 4 a 26".to_string()));
}
