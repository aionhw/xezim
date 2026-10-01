//! §8.25: a base class given by a TYPE PARAMETER (`class C #(type BASE)
//! extends BASE;`) must be a real ancestor for the whole class machinery —
//! `$cast`, the constructor chain, inherited properties and `super.method()`.
//!
//! The extends clause stores the parameter's NAME, which is no class key, so
//! every hierarchy walk (method lookup, inherited-property registration and
//! initialization, `super`, and the `$cast` type check) stopped one hop early
//! and the base was invisible: `$cast(base_c, c_obj)` returned 0, `super.new()`
//! never ran the base constructor, an inherited property read as 0, and
//! `super.who()` yielded nothing.
//!
//! Two resolution steps are exercised here:
//!   * the declaration's own specialization uses the parameter's declared
//!     default (§6.20.2) — `class wrap_c #(type BASE = base_c) extends BASE;`
//!     — which is what makes the default instantiation behave;
//!   * an object whose class parameter has NO default at all
//!     (`class wrap_d #(type BASE) extends BASE;`) resolves through the
//!     instance's own `type_bindings` at the `$cast` type check (§8.25).

use xezim::simulate;

/// Read a module-scope signal; x/z is a failure, not a zero.
fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able (x/z?)", n))
}

const SRC: &str = r#"
class base_c;
  int x = 5;
  int ctor_runs;
  function new(); ctor_runs = 1; endfunction
  virtual function int which(); return 1; endfunction
  function int getx(); return x; endfunction
endclass

// Base given by a type parameter, with a declared default.
class wrap_c #(type BASE = base_c) extends BASE;
  function new(); super.new(); endfunction
  virtual function int which(); return 10 + super.which(); endfunction
endclass

// Base given by a type parameter with NO default: nothing to resolve
// statically, so the object's own binding must answer the `$cast`.
class wrap_d #(type BASE) extends BASE; endclass

module tb;
  base_c b;
  wrap_c w;
  wrap_d #(base_c) wd;
  int cast_ok, ctor_ran, getx_val, super_which, no_default_cast;
  initial begin
    w  = new();
    wd = new();
    cast_ok         = $cast(b, w);      // upcast to the type-parameter base
    ctor_ran        = w.ctor_runs;      // proof that super.new() ran the base ctor
    getx_val        = w.getx();         // inherited property + inherited method
    super_which     = w.which();        // super.method() dispatch into the base
    no_default_cast = $cast(b, wd);     // no declared default: per-instance binding
  end
endmodule
"#;

#[test]
fn type_parameter_base_class_is_a_real_ancestor() {
    let sim = simulate(SRC, 10).expect("simulate failed");
    assert_eq!(
        u(&sim, "cast_ok"),
        1,
        "$cast(base_c, wrap_c_obj) must succeed for an extends-by-type-parameter base"
    );
    assert_eq!(
        u(&sim, "ctor_ran"),
        1,
        "super.new() must run the base class constructor"
    );
    assert_eq!(u(&sim, "getx_val"), 5, "inherited property and method");
    assert_eq!(
        u(&sim, "super_which"),
        11,
        "super.method() must dispatch into the type-parameter base"
    );
    assert_eq!(
        u(&sim, "no_default_cast"),
        1,
        "$cast with no declared default resolves through the instance bindings"
    );
}
