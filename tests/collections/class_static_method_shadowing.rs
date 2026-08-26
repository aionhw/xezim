//! Class STATIC methods whose name collides with a collection builtin must
//! dispatch to the method, not be swallowed by the array/queue builtin.
//!
//! Fix covered: §7.8.x static-method dispatch vs collection builtin shadowing.
//! A bare-class receiver `Class::exists(...)` (with `exists` also a collection
//! builtin method) was intercepted by `eval_builtin_method`'s `exists` handler,
//! which returns `false` (0) for any name that is not a registered collection.
//! So `uvm_config_db#(T)::exists(...)` — the config_db existence check — always
//! returned false, silently failing lookups even when the value had been set,
//! while `set`/`get` (not collection-builtin names) dispatched correctly.
//! The fix gates the identifier-collection builtin on the receiver NOT being a
//! class that declares the method, letting static methods fall through to the
//! static-dispatch path.

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

// ── Static method named `exists` must dispatch, not return false ──────
// Mirrors `uvm_config_db#(T)::exists(...)`: a class static function named
// `exists` collides with the collection builtin. Before the fix this was
// swallowed and always returned 0.
const STATIC_EXISTS_SRC: &str = r#"
module top;
  class cfgdb;
    static function bit exists(string s);
      return 1;
    endfunction
  endclass
  initial begin
    bit a;
    a = cfgdb::exists("key");
    if (a == 1) $display("TAG_PASS");
    else $display("TAG_FAIL a=%0d", a);
  end
endmodule
"#;

#[test]
fn test_static_method_named_exists() {
    let sim = simulate(STATIC_EXISTS_SRC, 200).expect("simulate failed");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "TAG_PASS"),
        "static exists() should dispatch to the method and return 1; got {:?}",
        msgs
    );
}

// ── Other collection-builtin-colliding static method names ────────────
// `size`, `num`, `first` are also collection builtin names; a class static of
// the same name must not be shadowed either.
const STATIC_SIZE_SRC: &str = r#"
module top;
  class regfile;
    static function int size();
      return 42;
    endfunction
    static function int num();
      return 7;
    endfunction
  endclass
  initial begin
    if (regfile::size() == 42 && regfile::num() == 7) $display("TAG_PASS");
    else $display("TAG_FAIL");
  end
endmodule
"#;

#[test]
fn test_static_method_named_size_and_num() {
    let sim = simulate(STATIC_SIZE_SRC, 200).expect("simulate failed");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "TAG_PASS"),
        "static size()/num() should dispatch to the methods; got {:?}",
        msgs
    );
}

// ── Genuine assoc-array `exists()` must keep working ──────────────────
// The gate must not over-block real collections: `a.exists(key)` on an actual
// associative array still routes to the builtin and returns the right result.
const ASSOC_EXISTS_STILL_WORKS_SRC: &str = r#"
module top;
  initial begin
    int a[string];
    a["k1"] = 1;
    bit e1 = a.exists("k1");
    bit e2 = a.exists("k2");
    if (e1 == 1 && e2 == 0) $display("TAG_PASS");
    else $display("TAG_FAIL e1=%0d e2=%0d", e1, e2);
  end
endmodule
"#;

#[test]
fn test_assoc_exists_still_works() {
    let sim = simulate(ASSOC_EXISTS_STILL_WORKS_SRC, 200).expect("simulate failed");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "TAG_PASS"),
        "assoc exists() builtin should still work; got {:?}",
        msgs
    );
}