//! §23.8 upward references by MODULE name from bound harnesses across a
//! generate grid: a `leaf_harness` bound into every `leaf` counts through
//! `dut.v_tl_harness.req_count[leaf.ROW_ID][leaf.COL_ID]`, where `dut` and
//! `leaf` name the nearest enclosing instances of those modules, and the
//! top-level harness checks the counts through a `ref u7_t [R-1:0][C-1:0]`
//! formal. The reference simulator resolves the module names to a single
//! instance here and fails its own check; the LRM reading passes.
use std::path::PathBuf;
use std::process::Command;

#[test]
fn bound_harnesses_count_per_instance_through_module_name_references() {
    let sv = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/hierarchy/bind_grid_module_name_refs.sv");
    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--simulate", "-s", "tb", "--no-cache", sv.to_str().unwrap()])
        .output()
        .unwrap();
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(), "run failed:\n{text}");
    assert!(text.contains("TEST PASSED"), "scoreboard did not pass:\n{text}");
    assert!(!text.contains("Fatal"), "fatal in run:\n{text}");
}
