//! Test for report data content correctness
//!
//! Verifies that the JSON report contains expected non-empty fields.

use std::process::Command;
use std::fs;

#[test]
fn report_data_has_required_fields() {
    let tmpfile = format!("/tmp/xezim_report_data_{}.json", std::process::id());
    let _ = fs::remove_file(&tmpfile);
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--report-stats=json", "--report-stats-file", &tmpfile, "tests/fixtures/tiny.sv"])
        .output()
        .unwrap();
    assert!(out.status.success(), "command failed: {}", String::from_utf8_lossy(&out.stderr));

    let content = fs::read_to_string(&tmpfile).unwrap_or_default();
    let _ = fs::remove_file(&tmpfile);

    // Parse JSON and check required fields
    let json: serde_json::Value = serde_json::from_str(&content)
        .expect("report should be valid JSON");

    // version
    assert!(json.get("version").is_some(), "missing version field");
    let version = json["version"].as_str().unwrap_or("");
    assert!(!version.is_empty(), "version should be non-empty");

    // phases.total_ms
    assert!(json.get("phases").is_some(), "missing phases object");
    let total_ms = json["phases"]["total_ms"].as_f64().unwrap_or(-1.0);
    assert!(total_ms >= 0.0, "phases.total_ms should be numeric and >= 0, got {}", total_ms);

    // cpu.total_s
    assert!(json.get("cpu").is_some(), "missing cpu object");
    let cpu_total = json["cpu"]["total_s"].as_f64().unwrap_or(-1.0);
    assert!(cpu_total >= 0.0, "cpu.total_s should be numeric and >= 0, got {}", cpu_total);

    // mem.peak_rss_kb
    assert!(json.get("mem").is_some(), "missing mem object");
    let peak_rss = json["mem"]["peak_rss_kb"].as_u64().unwrap_or(u64::MAX);
    assert!(peak_rss != u64::MAX, "mem.peak_rss_kb should be present");
    assert!(peak_rss > 0, "mem.peak_rss_kb should be > 0, got {}", peak_rss);

    // threads_used
    assert!(json.get("threads_used").is_some(), "missing threads_used field");
    let threads_used = json["threads_used"].as_u64().unwrap_or(0);
    assert!(threads_used >= 1, "threads_used should be >= 1, got {}", threads_used);
}