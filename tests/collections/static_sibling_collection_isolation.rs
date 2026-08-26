//! Sibling classes declaring a same-named STATIC collection must not alias to
//! one shared storage cell.
//!
//! Fix covered: static collections were registered in the module array tables
//! under their BARE name (a "single shared global store"), so `cA::q` and
//! `cB::q` (sibling classes, each with `static int q[$]`, sharing a common
//! base) read and wrote the SAME storage — every `push_back` to one class
//! appeared in the others. This silently broke UVM's `uvm_cmdline_set_*`
//! classes (all extending `uvm_cmdline_setting_base`, each with a static
//! `settings` queue): a `+UVM_VERBOSITY=` value written to
//! `uvm_cmdline_verbosity::settings` leaked into
//! `uvm_cmdline_set_action::settings` / `set_severity` / `set_verbosity`,
//! emitting spurious `+uvm_set_*=UVM_* "never took effect"` warnings and a
//! null-deref, and the verbosity was never applied.
//!
//! The fix detects SIBLING-COLLIDING static collections (a bare name declared
//! by >1 class with distinct qualified keys) and gives them class-qualified
//! storage (`cA::q`, `cB::q`) while leaving non-colliding statics
//! (e.g. `uvm_domain::m_domains`) on their original bare-name storage — the
//! latter broad rewrite split storage across the phase-graph access paths.
//! Verified byte-for-byte against the reference simulator, which yields
//! `cA=2 cB=1 cC=0`.

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

// ── Sibling classes, unqualified access inside static methods ──────────
// Mirrors UVM's `uvm_cmdline_set_*` classes: each static method touches the
// class's OWN `q` by bare name. Pre-fix all three classes shared one cell so
// cA=3 cB=3 cC=3; the reference (and post-fix xezim) gives cA=2 cB=1 cC=0.
const SIBLING_STATIC_QUEUE_SRC: &str = r#"
module top;
  class base_c; endclass
  class cA extends base_c;
    static int q[$];
    static function void add(int v); q.push_back(v); endfunction
    static function int cnt(); return q.size(); endfunction
  endclass
  class cB extends base_c;
    static int q[$];
    static function void add(int v); q.push_back(v); endfunction
    static function int cnt(); return q.size(); endfunction
  endclass
  class cC extends base_c;
    static int q[$];
    static function int cnt(); return q.size(); endfunction
  endclass
  initial begin
    cA::add(1); cA::add(2);
    cB::add(100);
    if (cA::cnt() == 2 && cB::cnt() == 1 && cC::cnt() == 0)
      $display("TAG_PASS");
    else
      $display("TAG_FAIL cA=%0d cB=%0d cC=%0d", cA::cnt(), cB::cnt(), cC::cnt());
  end
endmodule
"#;

#[test]
fn test_sibling_static_queues_are_isolated() {
    let sim = simulate(SIBLING_STATIC_QUEUE_SRC, 200).expect("simulate failed");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "TAG_PASS"),
        "sibling-class static queues must be independent; got {:?}",
        msgs
    );
}

// ── Same, via a direct ClassName::q receiver (module scope) ────────────
// The qualified form must also not alias when the static collection is a
// sibling collision.
const SIBLING_QUALIFIED_RECEIVER_SRC: &str = r#"
module top;
  class base_c; endclass
  class cA extends base_c; static int q[$]; endclass
  class cB extends base_c; static int q[$]; endclass
  class cC extends base_c; static int q[$]; endclass
  initial begin
    cA::q.push_back(1); cA::q.push_back(2);
    cB::q.push_back(100);
    if (cA::q.size() == 2 && cB::q.size() == 1 && cC::q.size() == 0)
      $display("TAG_PASS");
    else
      $display("TAG_FAIL cA=%0d cB=%0d cC=%0d", cA::q.size(), cB::q.size(), cC::q.size());
  end
endmodule
"#;

#[test]
fn test_sibling_static_queue_qualified_receiver() {
    let sim = simulate(SIBLING_QUALIFIED_RECEIVER_SRC, 200).expect("simulate failed");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "TAG_PASS"),
        "qualified Class::q receivers must be isolated; got {:?}",
        msgs
    );
}

// ── A NON-colliding static collection must stay in its bare-name cell ──
// Regresses the broad-rewrite hazard: a static assoc array declared by a
// SINGLE class (like `uvm_domain::m_domains`) must keep working. cX::mm is
// the only static collection named `mm`, so it stays on the global bare cell
// and both the qualified and in-class accesses see the same data.
#[test]
fn test_singleton_static_collection_stays_bare() {
    let sim = simulate(
        r#"
module top;
  class solo;
    static int mm[int];
    static function void set_val(int k, int v); mm[k] = v; endfunction
    static function int get_val(int k);
      if (mm.exists(k)) return mm[k];
      return -1;
    endfunction
  endclass
  initial begin
    solo::set_val(5, 42);
    int v = solo::get_val(5);
    if (v == 42) $display("TAG_PASS");
    else $display("TAG_FAIL v=%0d", v);
  end
endmodule
"#,
        200,
    )
    .expect("simulate failed");
    assert!(
        messages(&sim).iter().any(|m| m == "TAG_PASS"),
        "non-colliding static assoc must keep working; got {:?}",
        messages(&sim)
    );
}