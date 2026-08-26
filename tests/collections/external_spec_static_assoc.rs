//! EXTERNAL class-qualified access to a per-specialization STATIC associative
//! array of a parameterized class.
//!
//! The static collection is stored PER-SPECIALIZATION (`Class#spec::member`,
//! rewritten by the internal element/read paths via `spec_static_coll_key`),
//! so an EXTERNAL qualified access — `Class#(spec)::member.size()`,
//! `Class#(spec)::member[k]`, and `Class#(spec)::member.size()` — must resolve
//! to that same key. Before, the external paths either returned the bare
//! global name (reading the EMPTY bare cell) or a dotted `Class.member` that
//! neither the size nor element lookups recognised, so the reuse pool reported
//! size()==0 and element reads came back null.
//!
//! Reference-verified (IEEE 1800-2023 §8.9 static members / §8.25
//! specialization): each specialization's static collection is independent,
//! a fresh local static assoc gets its first element from a static-method
//! write visible through a qualified external read, and setting the same key
//! again REUSES the stored element (size stays 1).

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able (x/z?)", n))
}

// ── External `.size()` over a spec-keyed static assoc reports the count ──
#[test]
fn external_spec_static_assoc_size() {
    let src = r#"
module tb;
  class Ctx; endclass
  class Pool; int count; function void bump(); count++; endfunction endclass
  class Holder#(type T=int);
    static Pool arr[Ctx];
    static function void bump(Ctx c);
      if (!arr.exists(c)) arr[c] = new;
      arr[c].bump();
    endfunction
  endclass
  int sz, cnt;
  initial begin
    Ctx c = new;
    Holder#(int)::bump(c);          // writes through the static method
    Holder#(int)::bump(c);
    sz = Holder#(int)::arr.size();  // external qualified .size()
    cnt = Holder#(int)::arr[c].count; // external qualified element read
  end
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    assert_eq!(u(&sim, "sz"), 1, "external Class#spec::arr.size() sees the stored element");
    assert_eq!(u(&sim, "cnt"), 2, "external Class#spec::arr[c] reads the stored member");
}

// ── External element read returns the object stored by the static method ──
#[test]
fn external_spec_static_assoc_element() {
    let src = r#"
module tb;
  class C; endclass
  class P; int n; function void bump(); n++; endfunction endclass
  class cfg#(type T=int);
    static P m_rsc[C];
    static function void set(C h);
      if (!m_rsc.exists(h)) m_rsc[h] = new;
      m_rsc[h].bump();
    endfunction
  endclass
  int nonnull, bumpcnt;
  initial begin
    C h = new;
    cfg#(int)::set(h);
    cfg#(int)::set(h);
    P m = cfg#(int)::m_rsc[h];     // external qualified element fetch
    nonnull = (m == null) ? 0 : 1;
    bumpcnt = m.n;
  end
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    assert_eq!(u(&sim, "nonnull"), 1, "external element fetch returns the stored object (not null)");
    assert_eq!(u(&sim, "bumpcnt"), 2, "both static-method writes hit the same fetched element");
}

// ── Reuse-pool semantics (uvm_config_db::m_rsc pattern): set() twice ─────
// Reusing the same context creates ONE pool entry (size 1) with two settings
// (num 2). Mirrors the external `.size()` + element-fetch 4172 read pattern.
#[test]
fn external_spec_static_assoc_reuse_pool() {
    let src = r#"
module tb;
  class Ctx; endclass
  class P; int n; function void setn(int x); n = x; endfunction endclass
  class cfg#(type T=int);
    static P m_rsc[Ctx];
    static function void set(Ctx c, int v);
      if (!m_rsc.exists(c)) m_rsc[c] = new;
      m_rsc[c].setn(v);
    endfunction
  endclass
  int sz, nv;
  initial begin
    Ctx c = new;
    cfg#(int)::set(c, 3);
    cfg#(int)::set(c, 4);           // overwrite; REUSES the same pool
    sz = cfg#(int)::m_rsc.size();   // must be 1 (one context reused)
    nv = cfg#(int)::m_rsc[c].n;     // latest value wins
  end
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    assert_eq!(u(&sim, "sz"), 1, "reusing the context must NOT grow m_rsc (reuse pool)");
    assert_eq!(u(&sim, "nv"), 4, "overwritten value in the reused pool is visible externally");
}