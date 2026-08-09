//! Regression test: whole-carrying of an UNPACKED class-property struct whose
//! members are ONLY a class handle + a `string` (zero integral width) into:
//!   * a module-level struct assignment, and
//!   * a FUNCTION-LOCAL struct VAR declaration whose initializer is the
//!     class-property source.
//!
//! Such a struct cannot ride a packed Value (a reference/string member has no
//! bits), so the whole copy must be decomposed member-wise. This is the exact
//! shape UVM's factory uses: `m_uvm_factory_type_pair_t match_ = override.orig;`
//! where the pair is just a `uvm_object_wrapper` handle + a type-name string.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able (x/z?)", n))
}

const SRC: &str = r#"
module tb;
  class wrapper;
    string nm;
    function new(string n = ""); nm = n; endfunction
  endclass

  typedef struct {
    wrapper hnd;       // class handle: zero integral width
    string m_type_name; // string: also zero integral width
  } tp_t;

  class holder;
    tp_t orig;
    function new(string name, wrapper w);
      orig.hnd = w; orig.m_type_name = name;
    endfunction
    // function-local var-decl init from the class property: the UVM pattern.
    function string get_orig_name();
      tp_t local_cp = orig;          // <-- match_type_pair = override.orig
      if (local_cp.hnd == null) return "NULL";
      return local_cp.m_type_name;
    endfunction
    function bit get_orig_nonnull();
      tp_t local_cp = orig;
      return local_cp.hnd != null;
    endfunction
  endclass

  wrapper w;
  holder h;
  tp_t direct;                 // module-level whole-struct copy destination
  bit direct_ok, decl_ok, decl_nonnull;

  initial begin
    w = new("BASE");
    h = new("base_class", w);
    direct = h.orig;           // module-level whole-struct copy
    direct_ok  = (direct.hnd != null) && (direct.m_type_name == "base_class");
    // Call in BOTH orders: a fresh unpacked-struct local must carry the
    // handle member no matter which method runs first.
    decl_nonnull = h.get_orig_nonnull();
    decl_ok      = (h.get_orig_name() == "base_class");
  end
endmodule
"#;

#[test]
fn struct_copy_from_class_prop_keeps_handle_and_string() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "direct_ok"), 1, "module-level whole-struct copy lost members");
    assert_eq!(u(&sim, "decl_ok"), 1, "function-local decl-init lost the string member");
    assert_eq!(u(&sim, "decl_nonnull"), 1, "class-handle member dropped in decl-init");
}