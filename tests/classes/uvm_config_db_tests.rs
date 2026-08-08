//! config_db scope-matching regression tests (pure in-process).
//!
//! These exercise xezim's `uvm_config_db#(T)::set/get` interception (scope-aware
//! instance-name matching, wildcards, and misses) by running the real 1800.2
//! UVM library *in-process* via `simulate_multi` — no subprocess, no hardcoded
//! binary path, no reference simulator. Reference comparison belongs in the dev
//! workflow, not in the committed suite.

use xezim::{simulate_multi, SimOptions};

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
        SimOptions {
            top_module_name: Some("top".to_string()),
            include_dirs: vec![inc],
            ..Default::default()
        },
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
    assert!(out.contains("GET1_OK: 42"), "same-instance get should hit: {}", out);
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
    assert!(out.contains("T1_GET: 42"), "specific-instance get: {}", out);
    assert!(out.contains("T2_GET: 77"), "wildcard get: {}", out);
    assert!(out.contains("T3_OK"), "unset field should miss: {}", out);
}
