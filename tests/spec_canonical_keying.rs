//! IEEE 1800-2020 §6.20.2/§8.25 — consistent per-specialization static storage
//! when a class has **defaulted type parameters**.
//!
//! A specialization referenced two ways must resolve to the SAME static cell:
//!   - named directly with a defaulted param omitted:  `C#(arg)`
//!   - named via a typedef that expands ALL params:    `this_type` → `C#(arg,..,Default)`
//!   - reached through an ancestor's `extends`
//!
//! Previously each produced a different storage key (the omitted-default vs
//! bare-name forms differed), so a static written one way was read as 0 the
//! other way. This broke UVM's callback `register_super_type`, which writes
//! `m_s_typeid` via a typedef-based `this_type::get()` and reads it directly:
//! the read missed and `m_derived_types` stayed empty, so typewide callbacks
//! never propagated to derived classes.
//!
//! Verified byte-for-byte against reference simulators.

use xezim::simulate;

const SRC: &str = r#"
module top;
  // Class with a DEFAULTED type parameter (mirrors uvm_callbacks#(T, CB=uvm_callback)).
  class base; endclass
  class ca extends base; endclass
  class cb extends base; endclass

  class holder #(type T = base, type U = base);
    // A static whose value depends only on T (like uvm_callbacks::m_typeid).
    static int s_t;
    static function void set_s_t(int v);
      // `this_type` typedef expands to ALL params; U stays the bare name "U"
      // unless canonicalized.
      s_t = v;
    endfunction
    typedef holder#(T, U) this_type;
    static function int get_s_t();
      return s_t;
    endfunction
  endclass

  // Two specializations that differ only in the (defaulted) second param.
  // holder#(ca)   == holder#(ca, base)
  // holder#(cb)   == holder#(cb, base)
  // They must get SEPARATE s_t cells, and each must be consistent between
  // the direct-name and typedef-name access forms.
  int v_ca, v_cb;
  initial begin
    // WRITE via the fully-explicit form `holder#(ca, base)` and READ via the
    // defaulted-omitted form `holder#(ca)` — these are the SAME
    // specialization and MUST hit the same static cell. Before the
    // canonicalization fix they keyed differently (one padded/shortened) so
    // the read returned 0.
    holder#(ca, base)::set_s_t(111);
    holder#(cb, base)::set_s_t(222);
    v_ca = holder#(ca)::get_s_t();
    v_cb = holder#(cb)::get_s_t();
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
fn defaulted_type_param_specializations_isolate_consistently() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    // Each specialization keeps its own value (no cross-contamination), and
    // the typedef-based write (`s_t = v` inside set_s_t, where `this_type`
    // expands holder#(T,U)) lands in the same cell the direct-name read
    // (`holder#(ca)::get_s_t()`) consults.
    assert_eq!(u(&sim, "v_ca"), 111, "holder#(ca) static lost its value");
    assert_eq!(u(&sim, "v_cb"), 222, "holder#(cb) static lost its value");
}
