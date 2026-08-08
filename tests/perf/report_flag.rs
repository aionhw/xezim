//! Test for --report-stats flag and +report=stats plusarg
//!
//! Verifies that the statistics footer is printed in human, JSON, and file modes.

use std::process::Command;
use std::fs;

#[test]
fn report_flag_human() {
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--report-stats", "--compile", "tests/fixtures/tiny.sv"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let combined = format!("{}{}", stdout, stderr);
    // Footer should appear in stderr
    assert!(
        combined.contains("Compilation Performance Summary"),
        "expected footer in output:\n{}",
        combined
    );
    assert!(
        combined.contains("--------------------------------------------------------"),
        "expected dashed separator in output:\n{}",
        combined
    );
}

#[test]
fn report_flag_json() {
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--report-stats=json", "--compile", "tests/fixtures/tiny.sv"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let combined = format!("{}{}", stdout, stderr);
    // JSON output should contain version field
    assert!(
        combined.contains("\"version\""),
        "expected JSON with version field:\n{}",
        combined
    );
    // Compile-only runs write nothing else to stderr, so the JSON document
    // must be the first (and only) thing there.
    assert!(
        stderr.trim_start().starts_with('{'),
        "expected JSON document starting with {{ on stderr:\n{}",
        stderr
    );
}

#[test]
fn report_flag_file() {
    let tmpfile = format!("/tmp/xezim_report_{}.json", std::process::id());
    let _ = fs::remove_file(&tmpfile);
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--report-stats", "--report-stats-file", &tmpfile, "--compile", "tests/fixtures/tiny.sv"])
        .output()
        .unwrap();
    assert!(out.status.success(), "command failed: {}", String::from_utf8_lossy(&out.stderr));
    // File should exist and contain the report
    let content = fs::read_to_string(&tmpfile).unwrap_or_default();
    assert!(
        content.contains("Compilation Performance Summary"),
        "expected report in file:\n{}",
        content
    );
    let _ = fs::remove_file(&tmpfile);
}

#[test]
fn plusarg_report_stats() {
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["+report=stats", "--compile", "tests/fixtures/tiny.sv"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let combined = format!("{}{}", stdout, stderr);
    // Footer should appear without --report-stats flag
    assert!(
        combined.contains("Compilation Performance Summary"),
        "expected footer from +report=stats:\n{}",
        combined
    );
}

#[test]
fn plusarg_report_stats_json() {
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["+report=stats:json", "--compile", "tests/fixtures/tiny.sv"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        combined.contains("\"version\""),
        "expected JSON from +report=stats:json:\n{}",
        combined
    );
}