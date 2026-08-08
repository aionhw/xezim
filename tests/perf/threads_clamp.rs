//! CLI `--threads` clamp behaviour.
//!
//! Verifies that passing `--threads` with a value exceeding available
//! parallelism prints a warning and clamps to the available count, and that
//! `--threads 0` is rejected. These assert on deterministic stderr text, so
//! they are safe to run on any machine size.

use std::process::Command;

#[test]
fn threads_clamped_with_warning() {
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--sv2017", "--threads", "999999", "--compile"])
        .arg("tests/fixtures/tiny.sv")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("[warning]") && stderr.contains("clamping to"),
        "stderr: {}",
        stderr
    );
}

#[test]
fn threads_zero_invalid() {
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--sv2017", "--threads", "0", "--compile"])
        .arg("tests/fixtures/tiny.sv")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("[warning]") && stderr.contains("invalid"),
        "stderr: {}",
        stderr
    );
}
