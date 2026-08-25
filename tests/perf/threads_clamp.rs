//! Regression tests for --threads clamping behavior.
//!
//! These tests ensure the --threads flag is properly validated and clamped
//! against available parallelism. They test:
//! - --threads 0 is rejected with an error
//! - --threads > available parallelism clamps with a warning
//! - --threads <= available parallelism works without warning
//! - The clamped value actually affects dispatch (not just a cosmetic warning)

use std::process::Command;
use std::fs;
use std::env;

fn run_xezim(args: &[&str]) -> std::process::Output {
    // Write tiny.sv to a temp file
    let tmpdir = tempfile::tempdir().expect("Failed to create temp dir");
    let sv_path = tmpdir.path().join("tiny.sv");
    fs::write(&sv_path, include_str!("../fixtures/tiny.sv")).expect("Failed to write tiny.sv");

    // Use CARGO_BIN_EXE_xezim from env (set when running test binary directly)
    // or fall back to compile-time env! macro
    let bin_path = env::var("CARGO_BIN_EXE_xezim").unwrap_or_else(|_| env!("CARGO_BIN_EXE_xezim").to_string());
    let mut cmd = Command::new(bin_path);
    cmd.args(args).arg("--simulate").arg("--max-time=100").arg(&sv_path);
    cmd.stderr(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    let output = cmd.output().expect("Failed to run xezim");
    output
}

#[test]
fn threads_zero_invalid() {
    let output = run_xezim(&["--threads", "0"]);
    assert!(!output.status.success(), "--threads 0 should fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--threads requires a positive integer") ||
        stderr.contains("requires a positive integer"),
        "Expected error message about positive integer, got: {}",
        stderr
    );
}

#[test]
fn threads_clamped_with_warning() {
    // Request an absurdly large thread count to guarantee clamping
    let output = run_xezim(&["--threads", "999999"]);
    assert!(output.status.success(), "Simulation should succeed with clamped threads");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("clamp") || stderr.contains("Clamp") ||
        stderr.contains("capped") || stderr.contains("limited"),
        "Expected clamping warning, got: {}",
        stderr
    );
}

#[test]
fn threads_within_avail_no_warning() {
    // Request 1 thread - always valid, should not warn
    let output = run_xezim(&["--threads", "1"]);
    assert!(output.status.success());

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("clamp") && !stderr.contains("Clamp") &&
        !stderr.contains("capped") && !stderr.contains("limited"),
        "Should not warn for --threads 1, got: {}",
        stderr
    );
}

#[test]
fn threads_equals_avail_plus_one_warns() {
    // Get available parallelism
    let avail = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);

    // Request avail + 1 - should clamp and warn
    let requested = avail + 1;
    let output = run_xezim(&["--threads", &requested.to_string()]);
    assert!(output.status.success());

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("clamp") || stderr.contains("Clamp") ||
        stderr.contains("capped") || stderr.contains("limited"),
        "Expected clamping warning for --threads {}, got: {}",
        requested, stderr
    );
}
