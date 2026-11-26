//! IEEE 1800-2023 §6.6.7, §6.6.8, §6.7:
//! Scoped user-defined nettypes & resolvers, nettype legality checks,
//! UDN unpacked arrays (1-D / multi-D) and unpacked-struct `force`/`release`,
//! heterogeneous `interconnect` arrays & usage/declaration prohibitions,
//! built-in net data type legality, and scoped UDN `#delay` forms.

use xezim::simulate;

fn sim_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 200)
        .unwrap_or_else(|e| panic!("expected simulation to succeed, got: {e}\n{src}"));
    sim.output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect()
}

fn expect_fail(src: &str, needles: &[&str]) {
    match simulate(src, 20) {
        Ok(sim) => panic!(
            "expected failure matching {:?}, but simulation succeeded with output {:?}\n{src}",
            needles, sim.output
        ),
        Err(e) => {
            let low = e.to_lowercase();
            assert!(
                needles.iter().any(|n| low.contains(&n.to_lowercase())),
                "expected error containing any of {:?}, got: {e}\n{src}",
                needles
            );
        }
    }
}

#[test]
fn udn_scoped_resolvers_and_class_nettypes() {
    let src = r#"
package P;
  function automatic real rsum(input real d[]);
    real s = 0.0;
    foreach (d[i]) s += d[i];
    return s;
  endfunction
  nettype real pwr;
  nettype real pwrs with rsum;
endpackage

class C;
  static function automatic real rsum(input real d[]);
    real s = 0.0;
    foreach (d[i]) s += d[i];
    return s;
  endfunction
  nettype real cwr;
  nettype real cwrs with rsum;
endclass

class Base #(int W = 8);
  typedef logic [W-1:0] T;
  static function automatic T Tsum(input T d[]);
    T s = '0;
    foreach (d[i]) s = s + d[i];
    return s;
  endfunction
endclass

typedef Base#(8) MyBase8;
typedef Base#(16) MyBase16;

nettype real wr_pkg with P::rsum;
nettype real wr_cls with C::rsum;
nettype logic [7:0] wr_td with MyBase8::Tsum;
nettype logic [7:0] wr_spec with Base#(8)::Tsum;
nettype logic [15:0] wr_td16 with MyBase16::Tsum;
nettype logic [15:0] wr_spec16 with Base#(16)::Tsum;

module drv_r #(parameter real V = 1.0) (output wr_pkg o);
  assign o = V;
endmodule

module drv_cls #(parameter real V = 1.0) (output wr_cls o);
  assign o = V;
endmodule

module drv_td #(parameter logic [7:0] V = 8'h0) (output wr_td o);
  assign o = V;
endmodule

module drv_spec #(parameter logic [7:0] V = 8'h0) (output wr_spec o);
  assign o = V;
endmodule

module drv_td16 #(parameter logic [15:0] V = 16'h0) (output wr_td16 o);
  assign o = V;
endmodule

module drv_spec16 #(parameter logic [15:0] V = 16'h0) (output wr_spec16 o);
  assign o = V;
endmodule

module drv_cwrs #(parameter real V = 1.0) (output C::cwrs o);
  assign o = V;
endmodule

module top;
  wr_pkg n_pkg;
  wr_cls n_cls;
  wr_td n_td;
  wr_spec n_spec;
  wr_td16 n_td16;
  wr_spec16 n_spec16;
  C::cwr n_cwr;
  C::cwrs n_cwrs;
  P::pwr n_pwr;
  P::pwrs n_pwrs;

  drv_r #(1.25) d1(n_pkg);
  drv_r #(2.75) d2(n_pkg);
  drv_cls #(0.5) d3(n_cls);
  drv_cls #(1.5) d4(n_cls);
  drv_td #(8'h10) d5(n_td);
  drv_td #(8'h05) d6(n_td);
  drv_spec #(8'h20) d7(n_spec);
  drv_spec #(8'h03) d8(n_spec);
  drv_td16 #(16'h1200) d11(n_td16);
  drv_td16 #(16'h0034) d12(n_td16);
  drv_spec16 #(16'h2000) d13(n_spec16);
  drv_spec16 #(16'h0056) d14(n_spec16);
  assign n_cwr = 3.25;
  drv_cwrs #(1.5) d9(n_cwrs);
  drv_cwrs #(2.25) d10(n_cwrs);
  assign n_pwr = 6.5;
  assign n_pwrs = 1.25;
  assign n_pwrs = 2.75;

  initial begin
    #1;
    $display("T|pkg=%g cls=%g td=%h spec=%h td16=%h spec16=%h cwr=%g cwrs=%g pwr=%g pwrs=%g",
      n_pkg, n_cls, n_td, n_spec, n_td16, n_spec16, n_cwr, n_cwrs, n_pwr, n_pwrs);
  end
endmodule
"#;
    assert_eq!(
        sim_lines(src),
        ["T|pkg=4 cls=2 td=15 spec=23 td16=1234 spec16=2056 cwr=3.25 cwrs=3.75 pwr=6.5 pwrs=4"]
    );
}

#[test]
fn udn_legality_checks_data_types_resolvers_and_aliases() {
    // §6.6.7 (a)-(d): illegal nettype data types
    let illegal_types = [
        "module top; nettype string ns; endmodule",
        "module top; nettype chandle nc; endmodule",
        "module top; nettype event ne; endmodule",
        "class K; int x; endclass\nmodule top; nettype K nk; endmodule",
        "module top; typedef int da_t[]; nettype da_t nda; endmodule",
        "module top; typedef int q_t[$]; nettype q_t nq; endmodule",
        "module top; typedef int aa_t[string]; nettype aa_t naa; endmodule",
        "module top; typedef struct { real r; string s; } bst_t; nettype bst_t nbst; endmodule",
    ];
    for src in illegal_types {
        expect_fail(src, &["nettype"]);
    }

    // §6.6.7 resolver signature and formal-mutation constraints
    let illegal_resolvers = [
        // Fixed-size array formal instead of dynamic array
        r#"
module top;
  function automatic real rfix(input real d[4]);
    return d[0];
  endfunction
  nettype real wr with rfix;
endmodule
"#,
        // Return type mismatch
        r#"
module top;
  function automatic int rret(input real d[]);
    return 0;
  endfunction
  nettype real wr with rret;
endmodule
"#,
        // Element type mismatch
        r#"
module top;
  function automatic real rarg(input int d[]);
    return 0.0;
  endfunction
  nettype real wr with rarg;
endmodule
"#,
        // Non-static class method resolver
        r#"
class C;
  function automatic real rinst(input real d[]);
    return d.size() > 0 ? d[0] : 0.0;
  endfunction
endclass
module top;
  nettype real wr with C::rinst;
endmodule
"#,
        // Mutating formal array element
        r#"
module top;
  function automatic real rmut(input real d[]);
    if (d.size() > 0) d[0] = 1.0;
    return 0.0;
  endfunction
  nettype real wr with rmut;
endmodule
"#,
        // Resizing formal array
        r#"
module top;
  function automatic real rnew(input real d[]);
    d = new[2];
    return 0.0;
  endfunction
  nettype real wr with rnew;
endmodule
"#,
        // Deleting formal array
        r#"
module top;
  function automatic real rdel(input real d[]);
    d.delete();
    return 0.0;
  endfunction
  nettype real wr with rdel;
endmodule
"#,
    ];
    for src in illegal_resolvers {
        expect_fail(src, &["resolver", "nettype"]);
    }

    // Alias with `with <func>` is forbidden for both unresolved and resolved base nettypes
    expect_fail(
        r#"
module top;
  function automatic real rsum(input real d[]);
    real s = 0.0;
    foreach (d[i]) s += d[i];
    return s;
  endfunction
  nettype real wr;
  nettype wr wr_alias with rsum;
endmodule
"#,
        &["alias", "nettype", "with"],
    );
    expect_fail(
        r#"
module top;
  function automatic real rsum(input real d[]);
    real s = 0.0;
    foreach (d[i]) s += d[i];
    return s;
  endfunction
  function automatic real rmax(input real d[]);
    return 0.0;
  endfunction
  nettype real wrs with rsum;
  nettype wrs wrs_alias with rmax;
endmodule
"#,
        &["alias", "nettype", "with"],
    );
}

#[test]
fn udn_unpacked_array_element_drivers_and_partial_driver_rejection() {
    let src = r#"
package P;
  function automatic real rsum(input real d[]);
    real s = 0.0;
    foreach (d[i]) s += d[i];
    return s;
  endfunction
  nettype real wr with rsum;
endpackage

class C;
  static function automatic real rsum(input real d[]);
    real s = 0.0;
    foreach (d[i]) s += d[i];
    return s;
  endfunction
  nettype real wr with rsum;
endclass

function automatic real rsum(input real d[]);
  real s = 0.0;
  foreach (d[i]) s += d[i];
  return s;
endfunction
nettype real lwr with rsum;

module top;
  lwr a[2];
  P::wr pa[2];
  C::wr ca[2];

  assign a[0] = 1.5;
  assign a[0] = 2.0;
  assign a[1] = 4.5;

  assign pa[0] = 1.25;
  assign pa[0] = 0.75;
  assign pa[1] = 3.0;

  assign ca[0] = 5.5;
  assign ca[1] = 2.25;

  initial begin
    #1;
    $display("T|a=%g,%g pa=%g,%g ca=%g,%g",
      a[0], a[1], pa[0], pa[1], ca[0], ca[1]);
  end
endmodule
"#;
    assert_eq!(sim_lines(src), ["T|a=3.5,4.5 pa=2,3 ca=5.5,2.25"]);

    // Partial driver rejection on both unscoped and scoped nettypes (§10.3)
    expect_fail(
        r#"
module top;
  typedef struct { real v; bit f; } st_t;
  nettype st_t stnet;
  stnet p;
  assign p.v = 1.0;
endmodule
"#,
        &["partial", "whole net", "user-defined nettype"],
    );
    expect_fail(
        r#"
package P;
  typedef struct { real v; bit f; } st_t;
  nettype st_t stnet;
endpackage
module top;
  P::stnet p;
  assign p.v = 1.0;
endmodule
"#,
        &["partial", "whole net", "user-defined nettype"],
    );
}

#[test]
fn udn_multid_arrays_and_unpacked_struct_force_release() {
    let src = r#"
function automatic real rsum(input real d[]);
  real s = 0.0;
  foreach (d[i]) s += d[i];
  return s;
endfunction
nettype real wr with rsum;

typedef struct {
  real v;
  logic f;
} st_t;

function automatic st_t st_sum(input st_t d[]);
  st_t res = '{0.0, 1'b0};
  foreach (d[i]) begin
    res.v += d[i].v;
    res.f |= d[i].f;
  end
  return res;
endfunction
nettype st_t stnet with st_sum;

module top;
  wr m2d[2][2];
  stnet ws;
  st_t s_var = '{3.5, 1'b1};

  assign m2d[0][0] = 1.0;
  assign m2d[0][0] = 2.5;
  assign m2d[0][1] = 4.0;
  assign m2d[1][0] = 5.25;
  assign m2d[1][1] = 7.25;

  assign ws = '{1.0, 1'b0};
  assign ws = '{2.0, 1'b1};

  initial begin
    #1;
    $display("T|m2d=%g,%g,%g,%g", m2d[0][0], m2d[0][1], m2d[1][0], m2d[1][1]);
    $display("T|init=%g/%b", ws.v, ws.f);
    force ws = '{9.0, 1'b0};
    #1;
    $display("T|fpat=%g/%b", ws.v, ws.f);
    force ws = s_var;
    #1;
    $display("T|fvar=%g/%b", ws.v, ws.f);
    release ws;
    #1;
    $display("T|rel=%g/%b", ws.v, ws.f);
  end
endmodule
"#;
    assert_eq!(
        sim_lines(src),
        [
            "T|m2d=3.5,4,5.25,7.25",
            "T|init=3/1",
            "T|fpat=9/0",
            "T|fvar=3.5/1",
            "T|rel=3/1",
        ]
    );
}

#[test]
fn interconnect_heterogeneous_array_and_prohibitions() {
    // Positive heterogeneous interconnect array (wire [7:0] + resolved realNet)
    let ok = r#"
package NetsPkg;
  function automatic real rsum(input real d[]);
    real s = 0.0;
    foreach (d[i]) s += d[i];
    return s;
  endfunction
  nettype real realNet with rsum;
endpackage

module bDrv(output wire [7:0] o);
  assign o = 8'hA5;
endmodule
module bSink(input wire [7:0] i);
endmodule

module rDrv import NetsPkg::*; #(parameter real V = 1.0) (output realNet o);
  assign o = V;
endmodule
module rSink import NetsPkg::*; (input realNet i);
endmodule

module sinkWrap(input interconnect iBus[0:1]);
  bSink s0(iBus[0]);
  rSink s1(iBus[1]);
endmodule

module top;
  interconnect iBus[0:1];
  bDrv d0(iBus[0]);
  rDrv #(1.25) d1a(iBus[1]);
  rDrv #(2.75) d1b(iBus[1]);
  sinkWrap sw(iBus);
  initial begin
    #1;
    $display("T|ic=%h/%g", sw.s0.i, sw.s1.i);
  end
endmodule
"#;
    assert_eq!(sim_lines(ok), ["T|ic=a5/4"]);

    // Negative: all 7 prohibited interconnect forms (§6.6.8)
    let bad = [
        ("module top; interconnect logic x; endmodule", &["interconnect"][..]),
        (
            "module top; interconnect (strong0, strong1) x; endmodule",
            &["interconnect"],
        ),
        (
            "module top; interconnect vectored [3:0] x; endmodule",
            &["interconnect"],
        ),
        (
            "module top; interconnect #(1, 2) x; endmodule",
            &["interconnect"],
        ),
        (
            "module top; interconnect x = 1'b0; endmodule",
            &["interconnect"],
        ),
        (
            "module top; interconnect ic; assign ic = 1'b0; endmodule",
            &["interconnect", "continuous assignment"],
        ),
        (
            "module top; wire w; interconnect ic; assign w = ic; endmodule",
            &["interconnect", "continuous assignment"],
        ),
        (
            "module top; interconnect ic; logic v; initial v = ic; endmodule",
            &["interconnect", "procedural"],
        ),
    ];
    for (src, needles) in bad {
        expect_fail(src, needles);
    }
}

#[test]
fn builtin_net_data_type_legality() {
    // Legal 4-state net declarations
    let ok = r#"
module top;
  typedef struct packed {
    logic [3:0] a;
    logic [3:0] b;
  } ps4_t;
  typedef enum logic [1:0] { E0 = 2'd0, E1 = 2'd2 } e4_t;

  wire logic [7:0] w1 = 8'hA5;
  wire integer w2 = -7;
  wire e4_t w3 = E1;
  wire ps4_t w4 = 8'h3C;
  wire vectored [3:0] w5 = 4'h9;
  initial begin
    #1;
    $display("T|nets=%h/%0d/%0d/%h/%h", w1, w2, w3, w4, w5);
  end
endmodule
"#;
    assert_eq!(sim_lines(ok), ["T|nets=a5/-7/2/3c/9"]);

    // Illegal 2-state, real, string/chandle/event, net reg, scalar vectored/scalared
    let bad = [
        "module top; wire bit x; endmodule",
        "module top; wire int x; endmodule",
        "module top; wire byte x; endmodule",
        "module top; wire shortint x; endmodule",
        "module top; wire longint x; endmodule",
        "module top; wire real x; endmodule",
        "module top; wire shortreal x; endmodule",
        "module top; wire realtime x; endmodule",
        "module top; wire enum {A, B} x; endmodule",
        "module top; typedef struct packed { bit [3:0] a; } ps2_t; wire ps2_t x; endmodule",
        "module top; typedef int my_int_t; wire my_int_t x; endmodule",
        "module top; typedef real my_real_t; wire my_real_t x; endmodule",
        "module sub; wire string s; endmodule\nmodule top; sub u(); endmodule",
        "module sub; wire chandle ch; endmodule\nmodule top; sub u(); endmodule",
        "module sub; wire event ev; endmodule\nmodule top; sub u(); endmodule",
        "module top; wire reg x; endmodule",
        "module top; tri reg x; endmodule",
        "module sub(inout wire reg x); endmodule\nmodule top; wire w; sub u(w); endmodule",
        "module top; wire vectored x; endmodule",
        "module top; tri scalared y; endmodule",
    ];
    for src in bad {
        expect_fail(
            src,
            &["net", "reg", "vectored", "scalared", "2-state", "real"],
        );
    }
}

#[test]
fn scoped_udn_delays_and_multidelay_rejection() {
    let ok = r#"
package P;
  function automatic real rsum(input real d[]);
    real s = 0.0;
    foreach (d[i]) s += d[i];
    return s;
  endfunction
  nettype real wr with rsum;
endpackage

module top;
  P::wr #5 n1 = 1.5;
  P::wr #(5) n2;
  P::wr #(1:2:3) n3;
  assign n2 = 2.5;
  assign n3 = 3.5;
  initial begin
    #1;
    $display("T|t1=%g,%g,%g", n1, n2, n3);
    #10;
    $display("T|t11=%g,%g,%g", n1, n2, n3);
  end
endmodule
"#;
    let lines = sim_lines(ok);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert_eq!(lines[1], "T|t11=1.5,2.5,3.5");

    // Multi-delay specifications are illegal on user-defined nettype declarations (§6.7.1)
    expect_fail(
        r#"
package P;
  nettype real wr;
endpackage
module top;
  P::wr #(1, 2, 3) n = 1.0;
endmodule
"#,
        &["nettype", "delay"],
    );
    expect_fail(
        r#"
module top;
  nettype real wr;
  wr #(1, 2) n = 1.0;
endmodule
"#,
        &["nettype", "delay"],
    );
}
