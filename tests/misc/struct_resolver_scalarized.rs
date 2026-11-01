//! §6.6.7 + §7.2: struct-nettype resolution functions inline over scalars.
//!
//! An electrical-equivalent net is a nettype over `struct { real V, I, R; }`
//! whose resolver takes `input T drivers[]` and returns a `T`. The nettype
//! pass emits one continuous assign per member, `n.V = res('{...}).V`, and
//! none of them compiled -- the inliner bound only scalar formals and
//! scalar-element arrays -- so every settle re-ran the resolver on the AST
//! path, three times per node. A charge-pump regulator model spent 98% of
//! its run there.
//!
//! `f(...).m` now inlines a copy of `f` rewritten over scalars for member
//! `m` (`struct_sra`). These tests pin the VALUES -- bit-identical to the AST
//! path -- across the shapes the rewrite handles and one it must refuse.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    simulate(src, 1_000_000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|l| l.starts_with("R "))
        .collect()
}

const PKG: &str = r#"
package ee;
  typedef struct { real V; real I; real R; } ee_t;
  localparam real RINF = 1.0e30;
  function automatic ee_t res(input ee_t d[]);
    ee_t r;
    real g, itot;
    g = 0.0;
    itot = 0.0;
    foreach (d[k]) begin
      if (d[k].R > 0.0 && d[k].R < RINF) begin
        g += 1.0 / d[k].R;
        itot += d[k].V / d[k].R;
      end
      itot += d[k].I;
    end
    if (g > 0.0) begin
      r.V = itot / g;
      r.R = 1.0 / g;
    end else begin
      r.V = 0.0;
      r.R = RINF;
    end
    r.I = itot;
    return r;
  endfunction
  nettype ee_t eenet with res;
endpackage
import ee::*;
"#;

/// Millman over three drivers, re-resolved as a driver changes.
#[test]
fn divider_tracks_drivers() {
    let src = format!(
        "{PKG}{}",
        r#"
module top;
  real v = 2.0, i = 0.0;
  eenet n;
  assign n = '{V: v, I: 0.0, R: 1.0e3};
  assign n = '{V: 0.0, I: 0.0, R: 1.0e3};
  assign n = '{V: 0.0, I: i, R: RINF};
  initial begin
    #1 $display("R %.4f %.6f %.1f", n.V, n.I, n.R);
    v = 4.0;
    #1 $display("R %.4f %.6f %.1f", n.V, n.I, n.R);
    i = 1.0e-3;
    #1 $display("R %.4f %.6f %.1f", n.V, n.I, n.R);
  end
endmodule
"#
    );
    assert_eq!(
        lines(&src),
        vec![
            "R 1.0000 0.002000 500.0",
            "R 2.0000 0.004000 500.0",
            "R 2.5000 0.005000 500.0",
        ]
    );
}

/// A resolver that writes its result through the function name (§13.4.1),
/// copies a whole struct local, and returns a pattern from one branch.
#[test]
fn function_name_writes_and_struct_copies() {
    let out = lines(
        r#"
package p;
  typedef struct { real a; int n; } s_t;
  function automatic s_t sum_via_name(input s_t d[]);
    sum_via_name.a = 0.0;
    foreach (d[k]) sum_via_name.a += d[k].a;
    sum_via_name.n = d.size();
  endfunction
  function automatic s_t pick(input s_t d[], input real lim);
    s_t best, tmp;
    best = '{a: 0.0, n: -1};
    foreach (d[k]) begin
      tmp = d[k];
      if (tmp.a > best.a && tmp.a < lim) best = tmp;
    end
    if (best.n < 0) return '{a: -1.0, n: 0};
    return best;
  endfunction
endpackage
module top;
  import p::*;
  real x = 1.5;
  real sa, pa, qa; int sn, pn;
  assign sa = sum_via_name('{'{x, 1}, '{2.0, 2}, '{a: 0.25, n: 3}}).a;
  assign sn = sum_via_name('{'{x, 1}, '{2.0, 2}}).n;
  assign pa = pick('{'{x, 7}, '{2.0, 8}, '{9.0, 9}}, 5.0).a;
  assign pn = pick('{'{x, 7}, '{2.0, 8}, '{9.0, 9}}, 5.0).n;
  assign qa = pick('{'{x, 7}}, 1.0).a;
  initial begin
    #1 $display("R %.2f %0d %.2f %0d %.2f", sa, sn, pa, pn, qa);
    x = 3.5;
    #1 $display("R %.2f %0d %.2f %0d %.2f", sa, sn, pa, pn, qa);
  end
endmodule
"#,
    );
    assert_eq!(out, vec!["R 3.75 2 2.00 8 -1.00", "R 5.75 2 3.50 7 -1.00"]);
}

/// A body shape the rewrite does not handle (`case`) keeps the call path and
/// still resolves correctly.
#[test]
fn unsupported_body_keeps_call_path() {
    let out = lines(
        r#"
package q;
  typedef struct { real v; real w; } t_t;
  function automatic t_t f(input t_t d[]);
    t_t r;
    case (d.size())
      1: r = d[0];
      default: begin r.v = d[0].v + d[1].v; r.w = 0.0; end
    endcase
    return r;
  endfunction
endpackage
module top;
  import q::*;
  real a = 1.0, b;
  assign b = f('{'{a, 0.0}, '{2.0, 0.0}}).v;
  initial begin
    #1 $display("R %.1f", b);
    a = 5.0;
    #1 $display("R %.1f", b);
  end
endmodule
"#,
    );
    assert_eq!(out, vec!["R 3.0", "R 7.0"]);
}

/// A body that already declares one of the scalars' names (`r__V`) keeps the
/// call path: merging the two variables would change the result.
#[test]
fn colliding_local_name_keeps_call_path() {
    let out = lines(
        r#"
package c;
  typedef struct { real V; real I; } t_t;
  function automatic t_t f(input t_t d[]);
    t_t r;
    real r__V;
    r__V = 100.0;
    r.V = d[0].V;
    r.I = 0.0;
    r.V = r.V + r__V;
    return r;
  endfunction
endpackage
module top;
  import c::*;
  real a = 1.0, b;
  assign b = f('{'{a, 0.0}}).V;
  initial begin
    #1 $display("R %.1f", b);
    a = 4.0;
    #1 $display("R %.1f", b);
  end
endmodule
"#,
    );
    assert_eq!(out, vec!["R 101.0", "R 104.0"]);
}

/// A signed integral member keeps its sign through the scalarized path: the
/// array formal's elements carry the member's declared type (§13.5).
#[test]
fn signed_member_keeps_its_sign() {
    let out = lines(
        r#"
package s;
  typedef struct { real v; int n; } t_t;
  function automatic t_t f(input t_t d[]);
    t_t r;
    r.v = 0.0;
    r.n = 0;
    foreach (d[k]) begin
      if (d[k].n < 0) r.v += d[k].v;
      r.n += d[k].n / 2;
    end
    return r;
  endfunction
endpackage
module top;
  import s::*;
  int m = -6;
  real v; int n;
  assign v = f('{'{1.5, m}, '{2.0, 4}}).v;
  assign n = f('{'{1.5, m}, '{2.0, 4}}).n;
  initial begin
    #1 $display("R %.1f %0d", v, n);
    m = 8;
    #1 $display("R %.1f %0d", v, n);
  end
endmodule
"#,
    );
    assert_eq!(out, vec!["R 1.5 -1", "R 0.0 6"]);
}
