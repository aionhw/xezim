//! A STATIC associative array of class-handle values inside a PARAMETERIZED
//! class must persist its elements across separate static-method calls.
//!
//! Fix covered: collection-element WRITE path vs READ path storage key
//! disagreement for static collections of parameterized classes. The read
//! path rewrites a static collection's storage base via
//! `spec_static_coll_key` (e.g. `Holder#spec::arr[k]`) so each specialization
//! gets its own cells, but the element-WRITE path in `assign_value` stored
//! under the plain `arr[k]` name. Writes were therefore invisible to reads —
//! every element was silently lost between calls.
//!
//! This exact pattern is UVM's `uvm_config_db#(T)::m_rsc[uvm_component]` reuse
//! pool: each `config_db::set` must REUSE the resource already created for the
//! same (context, inst, field), only re-writing the new value. If the static
//! assoc array loses its elements between calls, every `set` creates a fresh
//! resource — `uvm_resource_pool::lookup_name` grows unboundedly and the
//! `05reuse_sets` test fails ("Got wrong queue size: expected 2, got 4").
//! Verified byte-for-byte against the reference simulator: the reference reuses
//! (queue stays at 2) and prints `** UVM TEST PASSED **`.

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn contains(sim: &xezim::compiler::Simulator, needle: &str) -> bool {
    messages(sim).iter().any(|m| m.contains(needle))
}

// ── Parameterized static class-handle assoc array must persist ────────
// Mirrors `uvm_config_db#(T)::m_rsc[uvm_component]`: a static assoc array
// keyed by a class handle whose value is a class object. Two static methods
// (`get` creates+stores; `cnt` reads back) must observe the same storage.
const PARAM_STATIC_ASSOC_PERSIST_SRC: &str = r#"
module top;
  class Ctx;
    string n;
    function new(string s); n = s; endfunction
  endclass
  class Pool;
    int count;
    function void bump(); count++; endfunction
  endclass
  class Holder #(type T = int);
    static Pool arr[Ctx];
    static function Pool get(Ctx c);
      if (!arr.exists(c)) arr[c] = new;
      return arr[c];
    endfunction
    static function int cnt(Ctx c);
      if (arr.exists(c)) return arr[c].count;
      return -1;
    endfunction
  endclass
  initial begin
    Ctx c = new("comp");
    Holder#(int)::get(c).bump();
    Holder#(int)::get(c).bump();
    int n = Holder#(int)::cnt(c);
    if (n == 2) $display("TAG_PASS n=%0d", n);
    else $display("TAG_FAIL n=%0d", n);
  end
endmodule
"#;

#[test]
fn test_param_static_assoc_persists_elements() {
    let sim = simulate(PARAM_STATIC_ASSOC_PERSIST_SRC, 200).expect("simulate failed");
    assert!(
        contains(&sim, "TAG_PASS"),
        "parameterized-class static assoc array must persist elements across calls; got {:?}",
        messages(&sim)
    );
}

// ── Sibling specs must NOT share the static collection ────────────────
// Two specializations (`Holder#(int)` vs `Holder#(string)`) each get their own
// static cell; bumping one must not leak into the other.
const SPEC_ISOLATION_SRC: &str = r#"
module top;
  class Ctx;
    string n;
    function new(string s); n = s; endfunction
  endclass
  class Pool;
    int count;
    function void bump(); count++; endfunction
  endclass
  class Holder #(type T = int);
    static Pool arr[Ctx];
    static function Pool get(Ctx c);
      if (!arr.exists(c)) arr[c] = new;
      return arr[c];
    endfunction
    static function int cnt(Ctx c);
      if (arr.exists(c)) return arr[c].count;
      return -1;
    endfunction
  endclass
  initial begin
    Ctx c = new("comp");
    // bump the int-spec pool twice, and the string-spec pool once
    Holder#(int)::get(c).bump();
    Holder#(int)::get(c).bump();
    Holder#(string)::get(c).bump();
    int ni = Holder#(int)::cnt(c);
    int ns = Holder#(string)::cnt(c);
    if (ni == 2 && ns == 1) $display("TAG_PASS ni=%0d ns=%0d", ni, ns);
    else $display("TAG_FAIL ni=%0d ns=%0d", ni, ns);
  end
endmodule
"#;

#[test]
fn test_param_specializations_are_isolated() {
    let sim = simulate(SPEC_ISOLATION_SRC, 200).expect("simulate failed");
    assert!(
        contains(&sim, "TAG_PASS"),
        "int and string specializations must have independent static assoc pools; got {:?}",
        messages(&sim)
    );
}

// ── Non-parameterized static assoc of class values must keep working ──
// Regresses the storage split: pre-fix this worked (the key had no `#spec`
// suffix, so plain-name storage was identical). The fix must not break it.
#[test]
fn test_plain_class_static_assoc_still_works() {
    let sim = simulate(
        r#"
module top;
  class Ctx;
    string n;
    function new(string s); n = s; endfunction
  endclass
  class Pool;
    int count;
    function void bump(); count++; endfunction
  endclass
  class Holder;
    static Pool arr[Ctx];
    static function Pool get(Ctx c);
      if (!arr.exists(c)) arr[c] = new;
      return arr[c];
    endfunction
    static function int cnt(Ctx c);
      if (arr.exists(c)) return arr[c].count;
      return -1;
    endfunction
  endclass
  initial begin
    Ctx c = new("comp");
    Holder::get(c).bump();
    Holder::get(c).bump();
    int n = Holder::cnt(c);
    if (n == 2) $display("TAG_PASS n=%0d", n);
    else $display("TAG_FAIL n=%0d", n);
  end
endmodule
"#,
        200,
    )
    .expect("simulate failed");
    assert!(
        contains(&sim, "TAG_PASS"),
        "plain-class static assoc of class values must persist; got {:?}",
        messages(&sim)
    );
}