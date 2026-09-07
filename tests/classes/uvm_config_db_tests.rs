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

/// A value set via `uvm_config_db::set` is visible to `uvm_config_db::exists`
/// at the same scope and through a `*` wildcard; a never-set field reports
/// missing.
/// Regression: the static `uvm_config_db#(T)::exists` (whose name collides
/// with the collection `exists` builtin) was routed to the associative-array
/// query and answered 0 without ever running the class method, so every
/// `exists` reported false even when the matching resource was present.
#[test]
fn test_config_db_exists_after_set() {
    const TEST_NAME: &str = "test_config_db_exists_after_set";
    let src = r#"
`include "uvm_macros.svh"
module top;
  import uvm_pkg::*;
  initial begin
    #1;
    // Same scope: set scope "uut" and look it up with a matching concrete
    // scope that the resource's stored scope must satisfy.
    #1;
    uvm_config_db#(int)::set(null, "uut", "spec_int", 1);
    #1;
    if (uvm_config_db#(int)::exists(null, "uut", "spec_int"))
      $display("E1_OK: specific exists");
    else
      $display("E1_FAIL: specific exists returned 0");

    // Wildcard: a set with `*` inst_name is visible to exists at a deeper,
    // concretely-named scope.
    #1;
    uvm_config_db#(int)::set(null, "*", "wild_int", 2);
    #1;
    if (uvm_config_db#(int)::exists(null, "deep/path", "wild_int"))
      $display("E2_OK: wildcard exists");
    else
      $display("E2_FAIL: wildcard exists returned 0");

    // A field never set must miss even against the wildcard.
    #1;
    if (uvm_config_db#(int)::exists(null, "any", "never_set"))
      $display("E3_FAIL: unset field reported present");
    else
      $display("E3_OK: unset field missing");
    $finish;
  end
endmodule
"#;
    let Some(out) = run_in_process(src) else {
        skip_no_uvm(TEST_NAME);
        return;
    };
    println!("{}", out);
    // Reference-verified (2026-08-28, UVM 2020 src): a set visible to get is
    // also visible to exists at the same scope and through a `*` wildcard,
    // while a never-set field reports missing.
    assert!(out.contains("E1_OK"), "specific exists: {}", out);
    assert!(out.contains("E2_OK"), "wildcard exists: {}", out);
    assert!(out.contains("E3_OK"), "unset field must miss: {}", out);
}
