//! Enum introspection on TYPE-PARAMETER-typed locals, and 4-state enum local
//! defaults.
//!
//! Reference-verified (IEEE 1800-2023 §6.19):
//!  * `enum` methods `first()/next()/name()/...` on a local whose DECLARED
//!    type is a class TYPE PARAMETER (`T e;` inside `holder#(e_t)::f()`) must
//!    resolve against the BOUND enum. Before, a `TypeReference` naming `T`
//!    (not a declared typedef) was never registered in `var_typedef_types`, so
//!    the receiver had no enum type: `first()` returned the default 0 by luck
//!    but `num()` was 0, `last()` was 0, and `next()` never advanced.
//!  * A fresh LOCAL 4-state `enum` variable defaults to its FIRST member, not
//!    X — so `void'(v.first()); do { m[v.name()] = v; v = v.next(); } while
//!    (v != v.first())` builds the whole mapping. With the X default the
//!    `v != v.first()` guard read X (taken as FALSE), the do-while ran once,
//!    and the assoc map held a single corrupt entry.
//!  * Together these let the `uvm_enum_wrapper#(T)::from_name` name-map be
//!    populated and consulted correctly.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able (x/z?)", n))
}

// ── Enum methods on a TYPE-PARAMETER-typed local return real members ──
#[test]
fn typeparam_enum_methods_iterate() {
    let src = r#"
module tb;
  typedef enum {ALPHA, BETA, GAMMA, DELTA} e_t;
  class holder#(type T=int);
    static function void probe(ref int nv, ref int lv, ref int nx, ref int nm);
      T e;
      e = e.first();      // ALPHA
      nv = e.num();       // 4
      lv = e.last();      // DELTA = 3
      e = e.next();       // BETA = 1
      nx = e;
      nm = e.num();
    endfunction
  endclass
  int nv, lv, nx, nm;
  initial begin
    holder#(e_t)::probe(nv, lv, nx, nm);
  end
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    assert_eq!(u(&sim, "nv"), 4, "e.num() of a T-typed enum local");
    assert_eq!(u(&sim, "lv"), 3, "e.last() of a T-typed enum local (DELTA)");
    assert_eq!(u(&sim, "nx"), 1, "e.next() from ALPHA of a T-typed enum local -> BETA");
    assert_eq!(u(&sim, "nm"), 4, "e.num() still 4 after next()");
}

// ── A fresh local 4-state enum defaults to its FIRST member, not X ───
#[test]
fn local_enum_defaults_to_first_member() {
    let src = r#"
module tb;
  typedef enum {ALPHA, BETA, GAMMA, DELTA} e_t;
  int is_first, next_is_beta;
  initial begin
    e_t v;                 // local, uninitialized
    is_first = (v == ALPHA); // must be 1 (default = first member), not X
    next_is_beta = (v.next() == BETA);
  end
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    assert_eq!(u(&sim, "is_first"), 1, "fresh local 4-state enum defaults to first member (not X)");
    assert_eq!(u(&sim, "next_is_beta"), 1, "next() of first member is second member");
}

// ── Full name-map build+lookup via a type-param static assoc (3693) ───
#[test]
fn typeparam_static_name_map() {
    let src = r#"
module tb;
  typedef enum {ALPHA, BETA, GAMMA, DELTA} e_t;
  class wrapper#(type T=int);
    static T map[string];
    static function bit from_name(string name, ref T value);
      if (map.size() == 0) m_init_map();
      if (map.exists(name)) begin value = map[name]; return 1; end
      else return 0;
    endfunction
    protected static function void m_init_map();
      T e = e.first();
      do begin
        map[e.name()] = e;
        e = e.next();
      end while (e != e.first());
    endfunction
  endclass
  e_t val;
  bit ok_alpha, ok_beta, ok_gamma, ok_delta, bad;
  initial begin
    ok_alpha = wrapper#(e_t)::from_name("ALPHA", val);
    ok_beta  = wrapper#(e_t)::from_name("BETA", val);
    ok_gamma = wrapper#(e_t)::from_name("GAMMA", val);
    ok_delta = wrapper#(e_t)::from_name("DELTA", val);
    bad      = wrapper#(e_t)::from_name("FOO", val);
  end
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    assert_eq!(u(&sim, "ok_alpha"), 1, "ALPHA resolvable by name");
    assert_eq!(u(&sim, "ok_beta"), 1, "BETA resolvable by name");
    assert_eq!(u(&sim, "ok_gamma"), 1, "GAMMA resolvable by name");
    assert_eq!(u(&sim, "ok_delta"), 1, "DELTA resolvable by name");
    assert_eq!(u(&sim, "bad"), 0, "unknown name is rejected");
}