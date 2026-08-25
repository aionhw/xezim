//! config_db scope-matching regression tests (pure in-process).
//!
//! These exercise xezim's `uvm_config_db#(T)::set/get` interception (scope-aware
//! instance-name matching, wildcards, and misses) by running the real 1800.2
//! UVM library *in-process* via `simulate_multi` — no subprocess, no hardcoded
//! binary path, no reference simulator. Reference comparison belongs in the dev
//! workflow, not in the committed suite.

use xezim::simulate_multi;

/// Root of a 1800.2 UVM checkout (the directory holding `src/uvm_pkg.sv`), or
/// None when this machine has no copy.
///
/// The path used to be a single hardcoded `../1800.2-2020.3.1`. No CI workflow
/// provisions UVM, and developer checkouts lay it out differently, so these
/// three tests failed on EVERY push — `main` included — for a missing file
/// rather than anything about config_db. A permanently-red job is worse than no
/// job: it trained every PR's `test` check to be ignored.
///
/// Probe the known layouts (and let `$UVM_HOME` override) so the tests really
/// run wherever a library exists, and skip cleanly where none does.
fn find_uvm_root() -> Option<String> {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let mut candidates: Vec<String> = Vec::new();
    if let Ok(home) = std::env::var("UVM_HOME") {
        candidates.push(home);
    }
    for rel in [
        "../1800.2-2020.3.1",
        "../UVM/1800.2-2020",
        "../UVM/1800.2-2017",
    ] {
        candidates.push(format!("{}/{}", manifest, rel));
    }
    candidates
        .into_iter()
        .find(|root| std::path::Path::new(&format!("{}/src/uvm_pkg.sv", root)).is_file())
}

/// Run a UVM-using top module in-process and return the joined `$display`
/// output. The real UVM library (`uvm_pkg.sv`) is compiled alongside the test
/// source, with its `src/` on the include path — exactly the command shape the
/// CLI uses, but without shelling out.
///
/// None when no UVM library is available — callers skip rather than fail.
fn run_in_process(src: &str) -> Option<String> {
    let uvm_dir = find_uvm_root()?;
    let uvm_pkg = std::fs::read_to_string(format!("{}/src/uvm_pkg.sv", uvm_dir))
        .expect("uvm_pkg.sv vanished between probe and read");
    let inc = format!("{}/src", uvm_dir);

    let sim = simulate_multi(
        &[uvm_pkg, src.to_string()],
        50_000,
        Some("top"),
        &[inc],
        &[],
        None,
        false,
        None,
        None,
        &[],
        &[],
        1,
        None,
        &[],
        0,
        u64::MAX,
        None,
        &[],
        None,
        None,
        None,
        None,
        false,
        None,
    )
    .expect("simulation failed");

    Some(
        sim.output
            .iter()
            .map(|o| o.message.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

/// Emit the skip reason once so a green run cannot be mistaken for coverage.
fn skip_no_uvm(test: &str) {
    eprintln!(
        "[skip] {test}: no 1800.2 UVM library found. Set UVM_HOME=<dir containing src/uvm_pkg.sv> to run it."
    );
}

/// Specific instance name: a `get` whose context/inst_name matches a prior `set`
/// succeeds; a wildcard `set(null, "*", ...)` matches any getter.
#[test]
fn test_config_db_inst_name() {
    const TEST_NAME: &str = "test_config_db_inst_name";
    let src = r#"
`include "uvm_macros.svh"
module top;
  import uvm_pkg::*;
  initial begin
    #1;
    uvm_config_db#(int)::set(null, "tc", "my_int", 42);
    #1;
    uvm_component comp1 = new("tc");
    #1;
    int val;
    if (uvm_config_db#(int)::get(comp1, "tc", "my_int", val))
      $display("GET1_OK: %0d", val);
    else
      $display("GET1_FAIL");

    // Wildcard should match any getter path.
    #1;
    uvm_config_db#(int)::set(null, "*", "wild_int", 99);
    #1;
    if (uvm_config_db#(int)::get(comp1, "any", "wild_int", val))
      $display("GET3_OK: %0d", val);
    else
      $display("GET3_FAIL: wildcard should match");

    $finish;
  end
endmodule
"#;
    let Some(out) = run_in_process(src) else {
        skip_no_uvm(TEST_NAME);
        return;
    };
    println!("{}", out);
    // Reference-verified (2026-08-11, UVM 2020 src, same test): the set
    // scope "tc" compiles to /^tc$/ while get(comp1, "tc", ...) looks up
    // "tc.tc" (cntxt full name + "." + inst_name) — a MISS. The old
    // GET1_OK expectation pinned the unresolved-DPI always-match bug that
    // the built-in uvm_re_match replaced with real POSIX-ERE semantics.
    assert!(out.contains("GET1_FAIL"), "cross-scope get must miss: {}", out);
    assert!(out.contains("GET3_OK: 99"), "wildcard get should hit: {}", out);
}

/// A wildcard `set(null, "*", field, v)` is visible to any getter, and the
/// retrieved value is the one that was set.
#[test]
fn test_config_db_wildcard() {
    const TEST_NAME: &str = "test_config_db_wildcard";
    let src = r#"
`include "uvm_macros.svh"
module top;
  import uvm_pkg::*;
  initial begin
    #1;
    uvm_config_db#(int)::set(null, "*", "my_int", 99);
    #1;
    uvm_component comp = new("comp");
    #1;
    int val;
    if (uvm_config_db#(int)::get(comp, "any_path", "my_int", val)) begin
      if (val == 99)
        $display("TEST_PASS");
      else
        $display("TEST_FAIL: expected 99 got %0d", val);
    end else begin
      $display("TEST_FAIL: get returned 0");
    end
    $finish;
  end
endmodule
"#;
    let Some(out) = run_in_process(src) else {
        skip_no_uvm(TEST_NAME);
        return;
    };
    println!("{}", out);
    assert!(out.contains("TEST_PASS"), "wildcard value should round-trip: {}", out);
}

/// A specific-instance set hits its getter; a wildcard set hits any getter; a
/// field that was never set misses.
#[test]
fn test_config_db_hit_wildcard_and_miss() {
    const TEST_NAME: &str = "test_config_db_hit_wildcard_and_miss";
    let src = r#"
`include "uvm_macros.svh"
module top;
  import uvm_pkg::*;
  initial begin
    #1;
    uvm_config_db#(int)::set(null, "tc", "my_int", 42);
    #1;
    uvm_component comp = new("comp");
    #1;
    int val;
    if (uvm_config_db#(int)::get(comp, "tc", "my_int", val))
      $display("T1_GET: %0d", val);
    else
      $display("T1_FAIL");

    #1;
    uvm_config_db#(int)::set(null, "*", "wild_int", 77);
    #1;
    if (uvm_config_db#(int)::get(comp, "any", "wild_int", val))
      $display("T2_GET: %0d", val);
    else
      $display("T2_FAIL");

    #1;
    if (uvm_config_db#(int)::get(comp, "*", "nonexist", val))
      $display("T3_FAIL: should not exist");
    else
      $display("T3_OK: not found as expected");

    $finish;
  end
endmodule
"#;
    let Some(out) = run_in_process(src) else {
        skip_no_uvm(TEST_NAME);
        return;
    };
    println!("{}", out);
    // Reference-verified (2026-08-11): get(comp, "tc", ...) looks up
    // "comp.tc", which /^tc$/ from set(null, "tc", ...) does not match —
    // the reference prints T1_FAIL. The old T1_GET expectation pinned the
    // unresolved-DPI always-match bug.
    assert!(out.contains("T1_FAIL"), "cross-scope get must miss: {}", out);
    assert!(out.contains("T2_GET: 77"), "wildcard get: {}", out);
    assert!(out.contains("T3_OK"), "unset field should miss: {}", out);
}

/// A type argument that is a SPECIALIZATION of the enclosing class's own
/// value parameter (`comp#(N)` in a method of `comp#(N)`) or one of its local
/// type aliases (`sc_type`) must resolve through the ACTIVE specialization
/// when it becomes a generic class's type binding.
///
/// `uvm_callback_iter#(comp#(N), sc_type)` inside `comp#(N)`'s dispatch
/// previously recorded the iterator's `(T, CB)` bindings as the SYMBOLIC
/// `comp#(N)`/`sc_type` instead of the concrete `comp#(1)`/`special_cb#(1)`.
/// `uvm_callbacks#(T,CB)::get()` then keyed a DIFFERENT per-spec typeid/static
/// cell than the `uvm_callbacks#(comp#(1), special_cb#(1))::add(null, cb)`
/// that registered the callback, so the typewide callback was never seen and
/// returned zero entries. (UVM's `09callbacks/25params` and `30iterate` pull
/// exactly this pattern from the `uvm_do_callbacks` macro.)
///
/// Each value specialization must get exactly its own callback. Verified
/// byte-for-byte against a commercial simulator: `P25_PASS`. Without the fix
/// this self-test FAILs (`c1sz=0 c2sz=0`).
#[test]
fn test_params_callback_dispatch() {
    const TEST_NAME: &str = "test_params_callback_dispatch";
    let src = r#"
`include "uvm_macros.svh"
module top;
import uvm_pkg::*;

virtual class special_cb #(int N=0) extends uvm_callback;
  function new(string name="special_cb"); super.new(name); endfunction
  virtual function void fire(ref string q[$], input int n); endfunction
endclass

class comp #(int N=0) extends uvm_component;
  string q[$];
  typedef comp#(N) comp_type;
  typedef special_cb#(N) sc_type;
  function new(string n, uvm_component p=null); super.new(n,p); endfunction
  `uvm_component_utils(comp)
  `uvm_register_cb(comp_type, sc_type)
  virtual function void do_fire();
    uvm_callback_iter#(comp#(N), sc_type) it = new(this);
    special_cb#(N) cb = it.first();
    while (cb != null) begin
      cb.fire(q, N);
      cb = it.next();
    end
  endfunction
endclass

class my_sc #(int N=0) extends special_cb#(N);
  string m_id;
  function new(string id); m_id=id; endfunction
  virtual function void fire(ref string q[$], input int n);
    q.push_back($sformatf("my#(%0d)_%0d_%s", N, n, m_id));
  endfunction
endclass

initial begin
  comp#(1) c1 = new("c1");
  comp#(2) c2 = new("c2");
  begin
    my_sc#(1) s1 = new("x1");
    my_sc#(2) s2 = new("x2");
    uvm_callbacks#(comp#(1), special_cb#(1))::add(null, s1);
    uvm_callbacks#(comp#(2), special_cb#(2))::add(null, s2);
  end
  c1.do_fire();
  c2.do_fire();
  if (c1.q.size()==1 && c1.q[0]=="my#(1)_1_x1" && c2.q.size()==1 && c2.q[0]=="my#(2)_2_x2")
    $display("P25_PASS");
  else begin
    $display("P25_FAIL c1sz=%0d c2sz=%0d", c1.q.size(), c2.q.size());
    foreach(c1.q[i]) $display("  c1[%0d]=%s", i, c1.q[i]);
    foreach(c2.q[i]) $display("  c2[%0d]=%s", i, c2.q[i]);
  end
  $finish;
end
endmodule
"#;
    let Some(out) = run_in_process(src) else {
        skip_no_uvm(TEST_NAME);
        return;
    };
    println!("{}", out);
    assert!(out.contains("P25_PASS"), "parameterized callback must dispatch per-spec: {}", out);
    assert!(!out.contains("P25_FAIL"), "got P25_FAIL: {}", out);
}

/// An instance-specific EVENT callback dispatched from `uvm_event#(T)::trigger`
/// must fire exactly ONCE per trigger.
///
/// Two xezim bugs broke this together (09callbacks/90Mantis/6033):
///  1) a PARAMETERIZED static task (`tester#(T)::do_it`) ran its inlined
///     blocking body with NO active specialization (`current_spec`), so
///     `my_cb#(T)`/`uvm_event#(T)` inside the body kept `T` SYMBOLIC and the
///     callback never dispatched (pre_trigger_count stayed 0);
///  2) `foreach (cb_q[i]) skip += cb_q[i].pre_trigger(...)` ran the method
///     TWICE — `infer_width` of the `+` operand evaluated the queue-element
///     method to read its width, then the value path ran it again
///     (pre_trigger_count was 2x).
/// `my_cb::pre_trigger` returns `(count % 2)` to block every OTHER trigger, so
/// exactly ONE trigger must yield pre=1 post=0. Verified byte-for-byte against
/// a commercial simulator: `RESULT qsz=1 pre=1 post=0`.
#[test]
fn test_event_callback_fires_once() {
    const TEST_NAME: &str = "test_event_callback_fires_once";
    let src = r#"
`include "uvm_macros.svh"
module top;
  import uvm_pkg::*;

  class my_cb #(type T=uvm_object) extends uvm_event_callback#(T);
    int pre_count;
    int post_count;
    function new(string name="unnamed"); super.new(name); endfunction
    virtual function bit pre_trigger(uvm_event#(T) e, T data);
      pre_count++;
      return (pre_count % 2);
    endfunction
    virtual function void post_trigger(uvm_event#(T) e, T data);
      post_count++;
    endfunction
  endclass

  class tester #(type T=uvm_object);
    typedef uvm_event#(T) event_type;
    typedef uvm_event_callback#(T) cb_type;
    typedef my_cb#(T) my_type;
    typedef uvm_callbacks#(event_type, cb_type) cbs;
    static task do_it();
      event_type evt = new("evt");
      my_type cb = new("cb");
      cb_type cb_q[$];
      cbs::add(evt, cb);
      #1;
      evt.trigger();
      cbs::get_all(cb_q, evt);
      $display("RESULT qsz=%0d pre=%0d post=%0d", cb_q.size(), cb.pre_count, cb.post_count);
    endtask
  endclass

  initial begin
    tester#(uvm_object)::do_it();
    $finish;
  end
endmodule
"#;
    let Some(out) = run_in_process(src) else {
        skip_no_uvm(TEST_NAME);
        return;
    };
    println!("{}", out);
    assert!(
        out.contains("RESULT qsz=1 pre=1 post=0"),
        "single event trigger must run the instance callback exactly once: {}",
        out
    );
}
