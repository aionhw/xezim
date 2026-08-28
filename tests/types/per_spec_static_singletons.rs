//! §8.25: per-specialization `static` members of a parameterized class when a
//! CONCRETE subclass (e.g. `ext1 extends base#(ext1)`) calls an inherited
//! static like `ext1::ID()`.
//!
//! A static call `ext1::ID()` carries no `#(spec)` on the call and has no
//! instance, so the interpreter had no way to know `ID()` belongs to the
//! `base#(ext1)` specialization: `current_spec` stayed unset and every
//! specialization keyed the shared `base::m_singleton` cell. Two sibling
//! subclasses therefore shared one singleton (`ext1::ID()` returned `ext2::ID()`),
//! and a handle-keyed associative array ("one extension per type") could never
//! tell a stored key from a different type's key — a miss returned the stored
//! element. This is the mechanism UVM's TLM generic-payload extensions rely on
//! (`uvm_tlm_extension#(T)::ID()`), so GP extension get/set/clone were all
//! broken. The fix seeds `current_spec` from the concrete subclass's extends
//! type args when dispatching the inherited static (and the instance-method
//! analogue for a concrete instance calling a parameterized base's method).
//!
//! Cross-checked against an independent tool's output.

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

/// Two concrete subclasses of a parameterized base must get DISTINCT static
/// singletons from the inherited static-ID method.
#[test]
fn per_specialization_static_singletons_are_distinct() {
    let src = r#"
package p;
  class base_t;
    int tag;
  endclass
  class keyed_t #(type T = int) extends base_t;
    static keyed_t#(T) m_singleton = null;
    static function keyed_t#(T) ID();
      if (m_singleton == null) m_singleton = new();
      return m_singleton;
    endfunction
  endclass
  class ext_a extends keyed_t#(ext_a); endclass
  class ext_b extends keyed_t#(ext_b); endclass
endpackage

module top;
  import p::*;
  keyed_t#(ext_a) a;
  keyed_t#(ext_b) b;
  initial begin
    a = ext_a::ID();
    b = ext_b::ID();
    a.tag = 111;
    b.tag = 222;
    if (a.tag == 111 && b.tag == 222 && a != b)
      $display("RESULT statics_isolated=1");
    else
      $display("RESULT statics_isolated=0");
    // Re-fetch: each specialization's ID() must still yield its own cell.
    if (ext_a::ID().tag == 111 && ext_b::ID().tag == 222)
      $display("RESULT refetch=1");
    else
      $display("RESULT refetch=0");
  end
endmodule
"#;
    let sim = simulate(src, 1000).expect("simulate failed");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "RESULT statics_isolated=1"),
        "distinct specializations must have distinct static singletons; got {:?}", msgs
    );
    assert!(
        msgs.iter().any(|m| m == "RESULT refetch=1"),
        "each specialization's static must persist; got {:?}", msgs
    );
}

/// A handle-keyed associative array: a lookup with a DIFFERENT class's ID
/// must MISS (return null), not fall through to the single stored element.
#[test]
fn handle_keyed_assoc_miss_with_sibling_specialization_ids() {
    let src = r#"
package p;
  class base_t;
  endclass
  class keyed_t #(type T = int) extends base_t;
    static keyed_t#(T) m_singleton = null;
    static function keyed_t#(T) ID();
      if (m_singleton == null) m_singleton = new();
      return m_singleton;
    endfunction
  endclass
  class ext1 extends keyed_t#(ext1); endclass
  class ext2 extends keyed_t#(ext2); endclass
  class gp extends base_t;
    base_t m_exts[base_t];
    function void set_ext(base_t hk, base_t e); m_exts[hk] = e; endfunction
    function base_t get_ext(base_t hk);
      base_t r;
      if (!m_exts.exists(hk)) r = null; else r = m_exts[hk];
      return r;
    endfunction
  endclass
endpackage

module top;
  import p::*;
  ext1 x1;
  gp g;
  int miss_is_null;
  initial begin
    x1 = new();
    g = new();
    g.set_ext(ext1::ID(), x1);
    // Storing under ext1::ID(); a lookup under ext2::ID() must MISS.
    miss_is_null = (g.get_ext(ext2::ID()) == null) ? 1 : 0;
    $display("RESULT miss_is_null=%0d", miss_is_null);
  end
endmodule
"#;
    let sim = simulate(src, 1000).expect("simulate failed");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "RESULT miss_is_null=1"),
        "assoc miss under a different specialization's ID must return null; got {:?}", msgs
    );
}