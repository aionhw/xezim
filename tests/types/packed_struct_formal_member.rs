//! A member select on a packed-struct subroutine FORMAL or local must read the
//! call frame, not the signal table.
//!
//! Inlining a non-root module flattens `s.a` into the dotted identifier `s.a`
//! (`rewrite_expr_impl`'s MemberAccess arm). That is right for a module-scope
//! aggregate — the rewrite has just prefixed it to `<inst>.s`, and
//! `packed_struct_fields` carries a layout under that scoped key. It is wrong
//! for a formal or local: those keep their bare name and live in the CALL
//! FRAME. The flattened name then took the packed-leaf path, whose base lookup
//! consulted signals ONLY, so the member read came back x — or, where a
//! same-named module-scope object existed, silently returned THAT object's
//! field instead.
//!
//! Two details made this hard to see: the WHOLE formal always read correctly
//! (only `.field` selects broke), and the same task in the ROOT module worked,
//! because root bodies are never rewritten. It needs `struct packed`
//! specifically — an unpacked struct stores members as separate `base.member`
//! frame keys, so the flattened name happens to match.

use xezim::simulate;

fn notes(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000_000).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|l| l.starts_with("NOTE:"))
        .collect()
}

/// The distilled case: a packed-struct formal of a task in a NON-root module.
#[test]
fn packed_struct_formal_members_read_in_a_nested_module() {
    let src = r#"
package sp;
  typedef struct packed { bit [7:0] hi; bit [7:0] lo; } pair_t;
endpackage
module leaf();
  import sp::*;
  task Show(input pair_t p);
    $display("NOTE: hi=%0d lo=%0d whole=%0h", p.hi, p.lo, p);
  endtask
endmodule
module top;
  import sp::*;
  leaf u_leaf();
  initial begin
    pair_t v;
    v.hi = 5;
    v.lo = 6;
    u_leaf.Show(v);
    $finish;
  end
endmodule
"#;
    assert_eq!(notes(src), vec!["NOTE: hi=5 lo=6 whole=506"]);
}

/// A same-named module-scope object must NOT capture the formal's member reads —
/// this is the silent-wrong-value form, which is worse than the x form.
#[test]
fn a_same_named_module_variable_does_not_shadow_the_formal() {
    let src = r#"
package sp2;
  typedef struct packed { bit [7:0] hi; bit [7:0] lo; } pair_t;
endpackage
module leaf();
  import sp2::*;
  pair_t p;                      // module-scope, same name as the formal
  initial begin p.hi = 1; p.lo = 2; end
  task Show(input pair_t p);
    $display("NOTE: hi=%0d lo=%0d", p.hi, p.lo);
  endtask
endmodule
module top;
  import sp2::*;
  leaf u_leaf();
  initial begin
    pair_t v;
    v.hi = 5;
    v.lo = 6;
    #1;
    u_leaf.Show(v);
    $finish;
  end
endmodule
"#;
    assert_eq!(
        notes(src),
        vec!["NOTE: hi=5 lo=6"],
        "the formal must win, not the module variable"
    );
}

/// A packed-struct LOCAL declared inside the nested task has the same shape.
#[test]
fn packed_struct_local_members_read_in_a_nested_module() {
    let src = r#"
package sp3;
  typedef struct packed { bit [7:0] hi; bit [7:0] lo; } pair_t;
endpackage
module leaf();
  import sp3::*;
  task Build();
    pair_t t;
    t.hi = 9;
    t.lo = 4;
    $display("NOTE: hi=%0d lo=%0d", t.hi, t.lo);
  endtask
endmodule
module top;
  leaf u_leaf();
  initial begin
    u_leaf.Build();
    $finish;
  end
endmodule
"#;
    assert_eq!(notes(src), vec!["NOTE: hi=9 lo=4"]);
}

/// A module-scope packed struct must still resolve through the flattened path —
/// the fix is frame-FIRST, not frame-only.
#[test]
fn module_scope_packed_struct_members_still_read() {
    let src = r#"
package sp4;
  typedef struct packed { bit [7:0] hi; bit [7:0] lo; } pair_t;
endpackage
module leaf();
  import sp4::*;
  pair_t m;
  task Show();
    $display("NOTE: hi=%0d lo=%0d", m.hi, m.lo);
  endtask
endmodule
module top;
  leaf u_leaf();
  initial begin
    u_leaf.m.hi = 7;
    u_leaf.m.lo = 8;
    #1;
    u_leaf.Show();
    $finish;
  end
endmodule
"#;
    assert_eq!(notes(src), vec!["NOTE: hi=7 lo=8"]);
}

/// §7.2.1: a packed struct is a vector with its first member most significant,
/// so each member of a formal is a fixed bit range of it. Functions called from
/// continuous assigns read direct and nested members of a module typedef, a
/// package typedef and an instance's own typedef, and every assign follows its
/// actual, x bits included.
const MEMBER_READS: &str = r#"
package pk;
  typedef struct packed { logic [1:0] hi; logic [1:0] lo; } p_t;
  typedef struct packed { logic a; p_t p; logic [2:0] b; } s_t;
  function automatic logic [1:0] lo_of(s_t c); return c.p.lo; endfunction
endpackage
module sub (input logic [7:0] i, output logic [1:0] o);
  typedef struct packed { logic [3:0] x; struct packed { logic [1:0] u, v; } y; } l_t;
  function automatic logic [1:0] v_of(l_t c); return c.y.v ^ c.x[1:0]; endfunction
  assign o = v_of(i);
endmodule
module top;
  typedef struct packed { logic a; logic [6:0] b; } s_t;
  function automatic logic a_of(s_t c); return c.a; endfunction
  function automatic logic [3:0] b_hi(s_t c); return c.b[6:3]; endfunction
  function automatic logic [1:0] hi_of(pk::s_t c); return c.p.hi; endfunction
  function automatic logic [3:0] mix(pk::s_t c, s_t d); return {c.a, c.b[0], d.a, d.b[0]}; endfunction
  s_t m [2];
  pk::s_t k;
  logic [1:0] a;
  logic [3:0] bh, mx;
  logic [1:0] hi, lo, sv;
  for (genvar g = 0; g < 2; g++) begin : gen
    assign a[g] = a_of(m[g]);
  end
  assign bh = b_hi(m[1]);
  assign hi = hi_of(k);
  assign lo = pk::lo_of(k);
  assign mx = mix(k, m[0]);
  sub u_sub(.i(k), .o(sv));
  initial begin
    m[0] = 8'h85; m[1] = 8'h3c; k = 8'b1_10_01_011;
    #1 $display("NOTE: %b %b %b %b %b %b", a, bh, hi, lo, mx, sv);
    m[0] = 8'h7f; m[1] = 8'hc3; k = 8'b0_01_10_100;
    #1 $display("NOTE: %b %b %b %b %b %b", a, bh, hi, lo, mx, sv);
    m[0] = 8'hx0; k = 8'b1_x1_0z_110;
    #1 $display("NOTE: %b %b %b %b %b %b", a, bh, hi, lo, mx, sv);
    $finish;
  end
endmodule
"#;

#[test]
fn continuous_assigns_read_packed_struct_formal_members() {
    assert_eq!(
        notes(MEMBER_READS),
        vec![
            "NOTE: 01 0111 10 01 1111 11",
            "NOTE: 10 1000 01 10 0001 11",
            "NOTE: 1x 1000 x1 0z 10x0 00",
        ]
    );
}

/// A member of a formal keeps its own width, signedness and range (§7.2.1,
/// §11.8.1, §7.4.1): `c.a >> 4` shifts all 8 bits of `a` before the 2-bit return
/// truncates, a signed member sign-extends and compares as signed, in the root
/// module and in an instance, and selects use the member's declared labels.
const WIDTH_SIGN_RANGE: &str = r#"
module sub (input logic [7:0] i, output int o, output logic l);
  typedef struct packed { logic signed [3:0] a; logic [3:0] b; } t_t;
  function automatic int g(t_t c); return c.a; endfunction
  function automatic logic neg(t_t c); return c.a < 0; endfunction
  assign o = g(i);
  assign l = neg(i);
endmodule
module top;
  typedef struct packed { logic [7:0] a; logic [3:0] b; } u_t;
  typedef struct packed { logic [1:0] x; struct packed { logic [3:0] u; logic signed [1:0] v; } y; } n_t;
  typedef struct packed { logic [7:4] a; logic [0:3] d; } r_t;
  function automatic logic [1:0] sh(u_t c); return c.a >> 4; endfunction
  function automatic int v_of(n_t c); return c.y.v; endfunction
  function automatic logic [3:0] sel(r_t c); return {c.a[6:5], c.d[1], c.a[4 +: 1]}; endfunction
  u_t s;
  n_t n;
  r_t r;
  logic [1:0] f;
  logic [3:0] rs;
  int v, so;
  logic sl;
  assign f = sh(s);
  assign v = v_of(n);
  assign rs = sel(r);
  sub u_sub(.i(s[11:4]), .o(so), .l(sl));
  initial begin
    s = 12'hf01; n = 8'b00_1100_10; r = 8'b1010_0100;
    #1 $display("NOTE: %0d %0d %0d %b %b", f, v, so, sl, rs);
    s = 12'h5a1; n = 8'b00_1100_01; r = 8'b0101_1011;
    #1 $display("NOTE: %0d %0d %0d %b %b", f, v, so, sl, rs);
    $finish;
  end
endmodule
"#;

#[test]
fn packed_struct_formal_members_keep_width_sign_and_range() {
    assert_eq!(
        notes(WIDTH_SIGN_RANGE),
        vec!["NOTE: 3 -2 -1 1 0110", "NOTE: 1 1 5 0 1001"]
    );
}

/// The member reads above compile into their continuous assigns: a fallback to
/// the interpreter keeps the output and loses the speed (`pk::lo_of`, a package
/// function, is not inlined either way).
#[test]
fn packed_struct_formal_member_reads_compile() {
    for (i, src) in [MEMBER_READS, WIDTH_SIGN_RANGE].into_iter().enumerate() {
        let sv =
            std::env::temp_dir().join(format!("xezim_formal_member_{}_{i}.sv", std::process::id()));
        std::fs::write(&sv, src).unwrap();
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_xezim"))
            .env("XEZIM_COMPILE_FAIL_STATS", "1")
            .args(["--no-cache", "-s", "top"])
            .arg(&sv)
            .output()
            .expect("run xezim");
        let _ = std::fs::remove_file(&sv);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{stderr}");
        assert!(
            !stderr.contains("Expr_MemberAccess") && !stderr.contains("_impure"),
            "{stderr}"
        );
    }
}
