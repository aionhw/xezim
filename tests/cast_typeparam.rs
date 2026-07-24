//! IEEE 1800-2020 §6.20.2/§8.25 — `$cast` with a **type-parameter-typed**
//! destination. Inside a parameterized-class method, `$cast(me, obj)` where
//! `me` is declared `T me;` (T a type parameter) must check the source against
//! T's *concrete* specialization, not a too-loose ancestor.
//!
//! Previously `class_of_var("me")` returned None (the recorded type "T" is not
//! a real class), so `cast_type_ok` fell through to the permissive "unknown
//! dest type" branch and EVERY such cast returned 1. This corrupted UVM's
//! callback type filtering: `m_add_tw_cbs` does `T me; if ($cast(me, obj))`
//! and adds a typewide callback to every instance whose `$cast` succeeds — so
//! a `b_comp` callback leaked into `a_comp` instances' queues.
//!
//! Verified byte-for-byte against reference simulators.

use xezim::simulate;

const SRC: &str = r#"
module top;
  class Cbase; endclass
  class Ca extends Cbase; endclass
  class Cb extends Cbase; endclass

  class cklass #(type T = Cbase);
    static function bit cast_ck(Cbase obj);
      T me;
      if ($cast(me, obj)) return 1;
      return 0;
    endfunction
  endclass

  int tp_wrong, tp_right_same, tp_right_diff;
  initial begin
    Ca a; Cb b;
    a = new(); b = new();
    // cklass#(Cb) on a Ca instance: must FAIL (0) — siblings, not subtype.
    tp_wrong      = cklass#(Cb)::cast_ck(a);
    // cklass#(Ca) on a Ca instance: must succeed (1).
    tp_right_same = cklass#(Ca)::cast_ck(a);
    // cklass#(Cb) on a Cb instance: must succeed (1).
    tp_right_diff = cklass#(Cb)::cast_ck(b);
  end
endmodule
"#;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("top.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able", n))
        & 0xFFFF_FFFF
}

#[test]
fn type_param_typed_cast_uses_concrete_specialization() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(
        u(&sim, "tp_wrong"),
        0,
        "$cast(Cb, Ca-instance) must fail for siblings"
    );
    assert_eq!(
        u(&sim, "tp_right_same"),
        1,
        "$cast(Ca, Ca-instance) must succeed"
    );
    assert_eq!(
        u(&sim, "tp_right_diff"),
        1,
        "$cast(Cb, Cb-instance) must succeed"
    );
}
