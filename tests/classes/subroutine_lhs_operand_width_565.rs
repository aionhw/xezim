//! IEEE 1800-2023 §11.4.1 / §11.6: a comparison whose LEFT operand is a subroutine formal or a local declared in the body still establishes the context width for its right operand: `s === x + y` with a 9-bit formal `s` computes the sum at 9 bits; an 8-bit left operand must not widen it (#565, reported by Ganesh-TNL).
//!
//! The issue's design, run as a process: every `CHK` line must read `ok`
//! and the run must print `TEST_PASS`.

use std::path::Path;
use std::process::Command;

#[test]
fn subroutine_lhs_operand_width_565_passes() {
    let test_file = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/classes/subroutine_lhs_operand_width_565.sv");
    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .arg("--simulate")
        .arg("-s")
        .arg("top")
        .arg(test_file.to_str().unwrap())
        .output()
        .expect("Failed to execute xezim");
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!combined.contains("Parse errors"), "Parse error:\n{combined}");
    assert!(!combined.contains("  FAIL "), "{combined}");
    assert!(combined.contains("TEST_PASS"), "no TEST_PASS:\n{combined}");
}
