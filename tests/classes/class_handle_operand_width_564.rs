//! IEEE 1800-2023 §11.6.1 / §11.8.2: an operand reached by walking DOWN from a class handle in module scope — a packed-struct member (`e.st.sa`) or an element of an array property (`e.arr[0]`) — is sized from the declared property types, so `s === e.st.sa + e.st.sb` computes the sum at 9 bits (#564, reported by Ganesh-TNL).
//!
//! The issue's design, run as a process: every `CHK` line must read `ok`
//! and the run must print `TEST_PASS`.

use std::path::Path;
use std::process::Command;

#[test]
fn class_handle_operand_width_564_passes() {
    let test_file = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/classes/class_handle_operand_width_564.sv");
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
