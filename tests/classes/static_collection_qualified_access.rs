//! §8.9 static COLLECTION properties reached through a CLASS-QUALIFIED name
//! (`ClassName::coll[i]`, `ClassName::coll.push_back(...)`) must resolve to
//! the SAME storage as the bare/in-class accessors, in every calling context.
//!
//! Two previously-broken shapes:
//!   1. A static-collection ELEMENT READ, `Class::coll[i]`, evaluated inside a
//!      class METHOD. The receiver flattens to the dotted `Class.coll`, which
//!      was not matched against the registered collection tables and fell
//!      through to a scalar bit-select — reading a null/blank element even
//!      though the same read at module scope worked.
//!   2. A static-collection BUILTIN mutation, `Class::coll.push_back(x)` /
//!      `Class::coll.size()`, reached from a non-instance context (a static
//!      function / module static initializer). The MemberAccess-call dispatch
//!      flattened the receiver to a dotted `Class.coll` that failed the
//!      dynamic-array/assoc membership tests, so the write was silently
//!      dropped (breaking `+uvm_set_severity` init, which pushes
//!      `uvm_cmdline_set_severity::settings` in `uvm_root`'s constructor).
//!
//! The storage key is per-DECLARING class for sibling collisions (the four
//! `uvm_cmdline_set_*` classes) and the BARE collection name otherwise.
//! Verified byte-for-byte against a reference simulator.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("top.{}", n)))
        .unwrap_or_else(|| panic!("signal {} not found", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able (x/z?)", n))
}

/// Reading a qualified static-collection element inside a class method must
/// return the real element (a non-null handle), not a null/blank value.
#[test]
fn qualified_static_collection_element_read_in_method() {
    let src = r#"
module top;
  class entry;
    int v;
    function new(int x); v = x; endfunction
  endclass
  class store;
    static entry q[$];
    static function void add(int x);
      entry e = new(x);
      q.push_back(e);
    endfunction
  endclass

  class reader;
    function int read0();
      entry e = store::q[0];   // qualified static-collection read inside a method
      if (e == null) return -1;
      return e.v;
    endfunction
  endclass

  int r0, r1;
  initial begin
    store::add(7);
    store::add(9);
    begin
      reader rd = new;
      r0 = rd.read0();
    end
    begin
      entry e;
      e = store::q[1];        // module-scope read (always worked)
      r1 = (e == null) ? -1 : e.v;
    end
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "r0"), 7, "qualified static-collection element read inside a method");
    assert_eq!(u(&sim, "r1"), 9, "module-scope read control");
}

/// Two SIBLING classes each declare a same-named static collection: storage is
/// per-DECLARING class, and a qualified read inside a method must find THIS
/// class's cells — not the sibling's (or null).
#[test]
fn sibling_collision_qualified_read_stays_on_own_store() {
    let src = r#"
module top;
  class a_cfg;
    static int tbl[$];
    static function void add(int x); tbl.push_back(x); endfunction
  endclass
  class b_cfg;
    static int tbl[$];
    static function void add(int x); tbl.push_back(1000 + x); endfunction
  endclass

  class svc;
    function int read_a();
      return a_cfg::tbl[0];
    endfunction
    function int read_b();
      return b_cfg::tbl[0];
    endfunction
  endclass

  int r_a, r_b;
  initial begin
    a_cfg::add(5);
    b_cfg::add(6);
    begin
      svc s = new;
      r_a = s.read_a();
      r_b = s.read_b();
    end
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "r_a"), 5, "a_cfg::tbl[0]");
    assert_eq!(u(&sim, "r_b"), 1006, "b_cfg::tbl[0] (its own store, not a_cfg's)");
}

/// A class-qualified static-collection `push_back` (module-static-init /
/// static-function context) must land on the store the later reads see —
/// reproducing the `m_do_cl_init` severity-setting path.
#[test]
fn qualified_static_collection_builtin_push_reaches_store() {
    let src = r#"
module top;
  class cfg;
    static int vals[$];
    static function int at(int i); return vals[i]; endfunction
  endclass

  // a module STATIC initializer (like `bit x = init();`) pushing via the
  // class-qualified collection name.
  function bit init();
    cfg::vals.push_back(11);
    cfg::vals.push_back(22);
    return 1;
  endfunction
  bit statically_initialized = init();

  int n, first, second;
  initial begin
    n = cfg::vals.size();
    first = cfg::vals[0];
    second = cfg::vals[1];
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "n"), 2, "size after module-static-init push_backs");
    assert_eq!(u(&sim, "first"), 11, "vals[0]");
    assert_eq!(u(&sim, "second"), 22, "vals[1]");
}