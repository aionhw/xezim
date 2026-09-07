//! §6.19 — an uninitialized enum whose base defaults to `int` is 2-state and
//! initializes to 0, and enum methods (`first`/`last`/`next`/`prev`/`num`/
//! `name`) resolve correctly on a local typed with a CLASS TYPE PARAMETER.
//!
//! Regression for a UVM enum-name/from_name test:
//! `uvm_enum_wrapper#(e_t)::from_name()` builds a `protected static
//! T map[string]` of `<name, value>` by walking `T e = e.first()`. Two bugs
//! made every `from_name` / `w_t::map` lookup fail:
//!   1. A bare `enum {...}` (no base) was treated as 4-state, so an
//!      uninitialized `e_t` read `x`, `e.first()` never gained a concrete
//!      value, and `e.next()` never advanced (the walk stopped after 1).
//!   2. A local declared with a class type parameter (`T e;` in a
//!      parameterized class) wasn't bound to the concrete enum type, so
//!      `e.first()/.next()/.num()` resolved to no enum members and answered
//!      the generic-default 0 (first==last==num==0).
//!
//! Two further STATIC-collection bugs on a param'd class (`wrapper#(T)`):
//!   3. `map.size()/num()/exists()` on the per-spec storage key
//!      `wrapper#e_t::map` returned garbage, so `from_name`'s `size()==0`
//!      guard never (correctly) gated the one-time build.
//!   4. A `static T map[string]` stored its string keys as hashed integers
//!      (the per-spec key wasn't recognized as STRING-keyed) — set/get
//!      stayed self-consistent so lookups worked, but `foreach (map[k])` /
//!      `map.first()/next()` decoded garbage (sorted numeric hashes).

use xezim::simulate;

fn output_of(sim: &xezim::compiler::Simulator) -> String {
    sim.output
        .iter()
        .map(|o| o.message.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn uninit_enum_defaults_to_first_member() {
    // A `typedef`'d enum with no base type default-initializes to 0 (its
    // implied `int` base), so the first()/next() walk visits all members.
    const SRC: &str = r#"
typedef enum {ALPHA, BETA, GAMMA, DELTA} e_t;
module top;
  e_t val;
  int count;
  initial begin
    count = 0;
    // `val` is uninitialized: a bare-enum default base (int) is 2-state,
    // so it is 0 = ALPHA, not x. `first()` still arms the walk boundary.
    void'(val.first());
    do begin
      count++;
      val = val.next();
    end while (val != val.first());
    $display("COUNT=%0d", count);
  end
endmodule
"#;
    let out = output_of(&simulate(SRC, 100).expect("sim"));
    assert_eq!(out, "COUNT=4", "enum walk must visit all 4 members:\n{}", out);
}

#[test]
fn enum_methods_resolve_type_param_local() {
    // Inside a parameterized class, a local `T e;` must bind `T` to the
    // concrete enum type so first/last/num/next resolve its member list.
    const SRC: &str = r#"
typedef enum {ALPHA, BETA, GAMMA, DELTA} e_t;
class wrapper#(type T);
  static function void probe();
    T e;
    $display("FIRST=%0d", e.first());
    $display("LAST=%0d", e.last());
    $display("NUM=%0d", e.num());
    e = e.first();
    e = e.next();
    $display("NEXT=%0d", e);
  endfunction
endclass
typedef wrapper#(e_t) w_t;
module top;
  initial begin
    w_t::probe();
  end
endmodule
"#;
    let out = output_of(&simulate(SRC, 100).expect("sim"));
    for (tag, want) in [("FIRST", 0), ("LAST", 3), ("NUM", 4), ("NEXT", 1)] {
        let line = format!("{}={}", tag, want);
        assert!(out.contains(&line), "missing `{}`:\n{}", line, out);
    }
}

#[test]
fn param_wrapper_from_name_populates_static_map() {
    // Mirrors `uvm_enum_wrapper#(T)::from_name`: a static `T map[string]`
    // keyed by `e.name()` must resolve names after the first lookup arms
    // `m_init_map()`. `exists` (the UC-compiling member lookup) must hit.
    const SRC: &str = r#"
typedef enum {ALPHA, BETA, GAMMA, DELTA} e_t;
class wrapper#(type T);
  static T map[string];
  static function bit from_name(string name, ref T value);
    if (map.size() == 0) m_init_map();
    if (map.exists(name)) begin
      value = map[name];
      return 1;
    end
    else return 0;
  endfunction
  static function void m_init_map();
    T e = e.first();
    do begin
      map[e.name()] = e;
      e = e.next();
    end while (e != e.first());
  endfunction
endclass
typedef wrapper#(e_t) w_t;
module top;
  initial begin
    e_t v;
    if (w_t::from_name("ALPHA", v)) $display("A_OK %0d", v);
    else $display("A_MISS");
    if (w_t::from_name("DELTA", v)) $display("D_OK %0d", v);
    else $display("D_MISS");
    if (w_t::from_name("NOPE", v)) $display("N_BAD");
    else $display("N_MISS");
  end
endmodule
"#;
    let out = output_of(&simulate(SRC, 100).expect("sim"));
    assert!(out.contains("A_OK 0"), "ALPHA must resolve to 0:\n{}", out);
    assert!(out.contains("D_OK 3"), "DELTA must resolve to 3:\n{}", out);
    assert!(out.contains("N_MISS"), "unknown name must miss:\n{}", out);
    assert!(!out.contains("A_MISS"), "ALPHA must not miss:\n{}", out);
    assert!(!out.contains("N_BAD"), "unknown name must not resolve:\n{}", out);
}

#[test]
fn static_param_assoc_size_counts_populated_keys() {
    // Regression: `size()`/`num()` (and `exists`) on a STATIC ASSOC array of
    // a PARAMETERIZED class returned garbage when accessed inside a method.
    // `uvm_enum_wrapper#(T)::from_name` gates its one-time name-map build on
    // `map.size() == 0`; a garbage size returned a garbage non-zero so the
    // map was never (properly) built and name lookups failed. The per-spec
    // static storage key (`wrapper#e_t::map`) was not recognized as an
    // associative array, so the assoc-counting `size()` path was missed.
    const SRC: &str = r#"
typedef enum {ALPHA, BETA, GAMMA, DELTA} e_t;
class wrapper#(type T);
  static T map[string];
  static function void build();
    T e = e.first();
    do begin
      map[e.name()] = e;
      e = e.next();
    end while (e != e.first());
  endfunction
  static function void probe();
    $display("EMPTY=%0d", map.size());
    build();
    $display("FULL=%0d", map.size());
    $display("NUM=%0d", map.num());
    $display("HIT=%0d", map.exists("ALPHA"));
    $display("MISS=%0d", map.exists("NOPE"));
  endfunction
endclass
typedef wrapper#(e_t) w_t;
module top;
  initial w_t::probe();
endmodule
"#;
    let out = output_of(&simulate(SRC, 100).expect("sim"));
    // Reference-verified (2026-08-28, reference simulator): EMPTY=0, FULL=4, NUM=4,
    // HIT=1, MISS=0.
    for (tag, want) in [
        ("EMPTY", 0),
        ("FULL", 4),
        ("NUM", 4),
        ("HIT", 1),
        ("MISS", 0),
    ] {
        let line = format!("{}={}", tag, want);
        assert!(out.contains(&line), "missing `{}`:\n{}", line, out);
    }
}

#[test]
fn static_param_assoc_foreach_first_next_string_keys() {
    // Regression: a `static T map[string]` of a PARAMETERIZED class stored
    // its STRING keys as hashed integers (the per-spec key `wrapper#e_t::map`
    // wasn't recognized as string-keyed, so `assoc_key_str` took the numeric
    // branch; set/get were self-consistent but `foreach`/`first()`/`next()`
    // decoded garbage). Straight lookups were byte-for-byte right, so this
    // only shows in ITERATION: `foreach (map[k])`, `map.first(k)`,
    // `map.next(k)` must visit each key with its real string text.
    const SRC: &str = r#"
typedef enum {ALPHA, BETA, GAMMA, DELTA} e_t;
class wrapper#(type T);
  static T map[string];
  static function void build();
    T e = e.first();
    do begin
      map[e.name()] = e;
      e = e.next();
    end while (e != e.first());
  endfunction
  static function void go();
    string k;
    build();
    foreach (map[k]) $display("FE %s=%0d", k, map[k]);
    if (map.first(k)) do begin
      $display("FN %s=%0d", k, map[k]);
    end while (map.next(k));
    $display("SZ %0d", map.size());
  endfunction
endclass
typedef wrapper#(e_t) w_t;
module top;
  initial w_t::go();
endmodule
"#;
    let out = output_of(&simulate(SRC, 100).expect("sim"));
    // Reference-verified (2026-08-28, reference simulator): FE/BE and FN/FN each run
    // ALPHA=0, BETA=1, DELTA=3, GAMMA=2 (lexicographic assoc order), and
    // SZ=4.
    for (tag, k, val) in [("FE", "ALPHA", 0), ("FE", "BETA", 1), ("FE", "DELTA", 3), ("FE", "GAMMA", 2)] {
        let line = format!("{} {}={}", tag, k, val);
        assert!(out.contains(&line), "missing `{}`:\n{}", line, out);
    }
    for (tag, k, val) in [("FN", "ALPHA", 0), ("FN", "BETA", 1), ("FN", "DELTA", 3), ("FN", "GAMMA", 2)] {
        let line = format!("{} {}={}", tag, k, val);
        assert!(out.contains(&line), "missing `{}`:\n{}", line, out);
    }
    assert!(out.contains("SZ 4"), "missing SZ 4:\n{}", out);
}

#[test]
fn static_assoc_foreach_knows_signed_int_key_width() {
    // Regression: `foreach (m[i])` over a STATIC assoc array with a SIGNED
    // integer key bound the index as UNSIGNED 32-bit (the per-spec key
    // `wrapper#e_t::imap` — and a non-parameterized class's bare `imap` —
    // weren't in `assoc_index_width_for`), so a `-3` key read back as
    // `4294967293` and `imap[that]` missed the stored `imap[-3]`. `int`,
    // `shortint` and `byte` keys must all iterate with their real sign.
    const SRC: &str = r#"
class wrapper#(type T);
  static int imap[int];
  static shortint smap[shortint];
  static byte bmap[byte];
  static function void fill();
    imap[3]=6; imap[-1]=7; imap[-100]=8;
    smap[7]=1; smap[-2]=2; smap[300]=3;
    bmap[-5]=4; bmap[100]=5;
  endfunction
  static function void go();
    int i; shortint s; byte b;
    fill();
    foreach (imap[i]) $display("II %0d=%0d", i, imap[i]);
    foreach (smap[s]) $display("SS %0d=%0d", s, smap[s]);
    foreach (bmap[b]) $display("BB %0d=%0d", b, bmap[b]);
  endfunction
endclass
typedef wrapper#(int) w_t;
module top;
  initial w_t::go();
endmodule
"#;
    let out = output_of(&simulate(SRC, 100).expect("sim"));
    // Reference-verified (2026-08-28, reference simulator) — numeric ASCending: II -100, -1, 3;
    // SS -2, 7, 300; BB -5, 100 — each with its stored value.
    for (tag, k, val) in [
        ("II", -100, 8),
        ("II", -1, 7),
        ("II", 3, 6),
        ("SS", -2, 2),
        ("SS", 7, 1),
        ("SS", 300, 3),
        ("BB", -5, 4),
        ("BB", 100, 5),
    ] {
        let line = format!("{} {}={}", tag, k, val);
        assert!(out.contains(&line), "missing `{}` (signed key must not wrap):\n{}", line, out);
    }
    // A wrapped unsigned key must NOT appear.
    assert!(
        !out.contains("4294967293") && !out.contains("II 4294") && !out.contains("SS 4294")
            && !out.contains("BB 4294"),
        "signed key wrapped to unsigned:\n{}",
        out
    );
}

#[test]
fn static_assoc_module_read_after_inmethod_write() {
    // Regression: a STATIC collection of a PARAMETERIZED class written INSIDE
    // a method (`wrapper#e_t::map[K]=v`) and then READ at MODULE SCOPE by its
    // typedef-specialization alias (`w_t::map[K]`, `w_t::map.size()`) used a
    // DIFFERENT storage key. `resolve_hier_name` collapsed `w_t::map` to the
    // bare `map` (no class context at module scope), while the in-method write
    // stored per-spec `wrapper#e_t::map` — so the module read saw an EMPTY
    // map (`size()==0`, `map[K]` returns x). Both forms of access to the same
    // per-specialization static must resolve to the identical storage key.
    const SRC: &str = r#"
typedef enum {ALPHA, BETA, GAMMA, DELTA} e_t;
class wrapper#(type T);
  static T map[string];
  static function void fill();
    map["ALPHA"]=ALPHA; map["BETA"]=BETA; map["GAMMA"]=GAMMA; map["DELTA"]=DELTA;
  endfunction
endclass
typedef wrapper#(e_t) w_t;
module top;
  initial begin
    w_t::fill();
    $display("SZ %0d", w_t::map.size());
    foreach (w_t::map[k]) $display("FE %s=%0d", k, w_t::map[k]);
  end
endmodule
"#;
    let out = output_of(&simulate(SRC, 100).expect("sim"));
    // Reference-verified (2026-08-28, reference simulator): SZ=4; foreach visits the
    // string keys ALPHA/BETA/GAMMA/DELTA with their stored values.
    assert!(out.contains("SZ 4"), "module read saw an empty map:\n{}", out);
    for (tag, k, val) in [
        ("FE", "ALPHA", 0),
        ("FE", "BETA", 1),
        ("FE", "GAMMA", 2),
        ("FE", "DELTA", 3),
    ] {
        let line = format!("{} {}={}", tag, k, val);
        assert!(out.contains(&line), "missing `{}` (module read must match in-method write):\n{}", line, out);
    }
}