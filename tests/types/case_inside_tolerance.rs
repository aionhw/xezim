//! IEEE 1800-2023 §11.4.13, §12.5.3 and §12.5.4: tolerance ranges
//! are valid set members in case items and use the same bounds as `inside`.

use std::path::Path;
use std::process::Command;

fn check(fixture: &str) {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/types")
        .join(fixture);
    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--sv2023", "--no-cache"])
        .arg(source)
        .output()
        .expect("run xezim");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success() && stdout.lines().any(|line| line.trim() == "PASS"),
        "{fixture}: {stdout}\n{stderr}"
    );
}

#[test]
fn tolerance_case_items_match_their_ranges() {
    check("case_inside_tolerance.sv");
}

#[test]
fn plain_case_ranges_and_inside_operator_still_match() {
    check("case_inside_tolerance_guards.sv");
}

#[test]
fn inside_operator_evaluates_full_plain_range_bounds() {
    check("inside_operator_plain_ranges.sv");
}

#[test]
fn constant_case_inside_evaluates_range_items() {
    check("constant_case_inside_plain_range.sv");
    check("constant_case_inside_tolerance_range.sv");
}
