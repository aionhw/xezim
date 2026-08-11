//! Regression for `resolve_type_param_with` — the active specialization's
//! own argument must win for a type parameter it directly declares
//! (commit "fix: active specialization wins when resolving a colliding type
//! param").
//!
//! # Original UVM symptom
//!
//! `uvm_agent::build_phase` reads `is_active` from the resource pool via
//! `uvm_resource_enum_read`, whose `$cast` to `uvm_resource#(<enum>)` /
//! `uvm_resource#(uvm_integral_t)` / `uvm_resource#(uvm_bitstream_t)` must
//! succeed against the resource `uvm_config_int::set` wrote.
//! `05components/90Mantis/3167_agent_activepassive` failed: agent2 came up
//! `UVM_ACTIVE` (default) instead of the configured `UVM_PASSIVE`.
//!
//! Constructing `uvm_resource#(T)` INSIDE
//! `uvm_config_db_default_implementation_t#(T)` (a parameterized class whose
//! type-param NAME collides with the enclosing one) resolved `T` from the
//! enclosing instance's CACHED binding, which had been polluted to the full
//! implementation specialization instead of the concrete element type, so the
//! child resource recorded the wrong type and every read-side `$cast` failed.
//!
//! # Why this is a pure-SV test (no UVM library)
//!
//! The fix is a single early-return block at the top of
//! `resolve_type_param_with`: when `tn` is DIRECTLY declared in the active
//! specialization's base, return that specialization's argument (indexed by the
//! full interleaved `param_order`, not just `type_param_names`) BEFORE
//! consulting the instance binding.
//!
//! The reported pollution needs UVM's factory/`type_id` machinery to set up the
//! stale cached binding, so it cannot be reproduced in isolation without the
//! 1800.2 library *and* the DPI shared object. The early-return it depends on,
//! however, is exercised directly by a class with INTERLEAVED value+type
//! parameters (`#(int W, type T, int H)`): resolving `T` from a static context
//! has no instance binding to fall back to, and the legacy code indexed `T` by
//! `type_param_names` (slot 0) instead of `param_order` (slot 1), picking up
//! the value param's argument. That reduction fails without the fix and passes
//! with it — the same `resolve_type_param_with` block — with zero external
//! dependencies.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("top.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {n}"))
        .to_u64()
        .unwrap_or_else(|| panic!("{n} not u64-able"))
        & 0xFFFF_FFFF
}

/// A class with INTERLEAVED value+type params resolves its type parameter from
/// a static (instance-free) context by the active specialization's argument.
/// The inner `#(T)` reference mirrors the `uvm_resource#(T)`-inside-`config_db`
/// shape that broke UVM 3167.
#[test]
fn interleaved_type_param_resolves_from_active_spec() {
    let src = r#"
module top;
  class markA; static function int id(); return 11; endfunction endclass
  class markB; static function int id(); return 22; endfunction endclass

  // `inner#(T)` — stands in for `uvm_resource#(T)`; its static method must see
  // the concrete element type, not the enclosing specialization.
  class inner #(type T = markA);
    static function int tag();
      return T::id();
    endfunction
  endclass

  // A VALUE param (W) PRECEDES the TYPE param (T) followed by another value
  // param (H): param_order = [W, T, H], type_param_names = [T]. The legacy
  // index (type_param_names slot 0) wrongly mapped T onto W's argument.
  class outer #(int W = 1, type T = markA, int H = 1);
    static function int whichT();
      return inner#(T)::tag();
    endfunction
  endclass

  logic [31:0] got_a, got_b;
  initial begin
    got_a = outer#(2, markA, 3)::whichT();   // T -> markA -> 11
    got_b = outer#(2, markB, 3)::whichT();   // T -> markB -> 22
  end
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    assert_eq!(u(&sim, "got_a"), 11, "outer#(2,markA,3) must bind T -> markA");
    assert_eq!(u(&sim, "got_b"), 22, "outer#(2,markB,3) must bind T -> markB");
}
