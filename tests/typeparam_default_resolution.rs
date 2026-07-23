//! IEEE 1800-2020 §8.25.4 — a type parameter OMITTED in a specialization
//! reference resolves to its declared DEFAULT inside the specialization's
//! method bodies.
//!
//! `uvm_callbacks #(type T=uvm_object, type CB=uvm_callback)` is routinely
//! referenced as `uvm_callbacks#(T)` (CB elided). Inside `get()`, the body
//! evaluates `uvm_typeid#(CB)::get()` to decide the if/else branch. CB must
//! resolve to its default (`uvm_callback`), not stay as the bare literal
//! "CB". Previously `resolve_type_param_with` only consulted the sig's
//! positional fragments — when the sig had fewer args than the class has type
//! params (the elided case), the trailing param came back unbound, flipped
//! the `get()` if/else, and the base-callback `typeid_map` write never ran.
//! That broke UVM callback registration for any type with a non-base callback
//! (the `09callbacks/20inherit` grandchild edge).
//!
//! This test models the elision via static-function accessors (xezim resolves
//! per-spec statics through static calls, the same path UVM's `get()` uses).

use xezim::simulate;

const SRC: &str = r#"
module top;
  class base; endclass
  class ca extends base; endclass
  class other; endclass

  // Per-type marker cell accessed ONLY through static accessors.
  class marker #(type T);
    static int id;
    static function void set_id(int v); id = v; endfunction
    static function int get_id(); return id; endfunction
  endclass

  // Mirrors uvm_callbacks#(T, CB=uvm_callback): a defaulted second type param.
  class cbh #(type T = base, type U = base);
    // Calls marker#(U)::get_id(). When the specialization elides U, U must
    // resolve to its default (base) via resolve_type_param_with's
    // type_param_defaults fallback.
    static function int u_marker();
      return marker#(U)::get_id();
    endfunction
  endclass

  int v_omitted;   // cbh#(ca) — U elided, should default to base
  int v_explicit;  // cbh#(ca, other) — U explicit

  initial begin
    marker#(base)::set_id(5);
    marker#(other)::set_id(9);
    v_omitted  = cbh#(ca)::u_marker();
    v_explicit = cbh#(ca, other)::u_marker();
  end
endmodule
"#;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(&format!("top.{}", n))
        .or_else(|| sim.get_signal(n))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able", n))
        & 0xFFFF_FFFF
}

#[test]
fn omitted_trailing_type_param_resolves_to_default() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    // cbh#(ca) elides U -> U defaults to base -> marker#(base)::get_id() == 5.
    // Before the fix this returned 0 (U unbound -> marker#(U) empty cell).
    assert_eq!(u(&sim, "v_omitted"), 5, "elided U should default to base");
    // Explicit U=other still works.
    assert_eq!(u(&sim, "v_explicit"), 9, "explicit U=other");
}
