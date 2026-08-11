//! Opt-in `--report-stats` footer (src/report.rs).
//!
//! The renderers are pure functions of a `RunStats` value, and the module
//! lives in the CLI binary (not the library), so the exact text is asserted
//! by including that file directly. Two end-to-end cases then run the real
//! binary (same pattern as tests/classes/typename_param_class.rs) to pin the
//! flag/env semantics: footer present only when asked for, on stderr, with
//! stdout unchanged.

use std::process::Command;

// The footer module is part of the xezim binary, not the xezim library, so
// include its source directly for the pure-function tests. It only depends
// on std + libc.
#[path = "../../src/report.rs"]
mod report;

use report::{ReportMode, RunStats};

fn sample_stats() -> RunStats {
    RunStats {
        version: "0.9.8".to_string(),
        git_rev: "abc1234".to_string(),
        wall_ms: 42,
        cpu_user_ms: 30,
        cpu_sys_ms: 7,
        peak_rss_kb: Some(12_345),
        hostname: Some("buildhost".to_string()),
        sim_time_ns: Some(1_000),
    }
}

#[test]
fn human_rendering() {
    let text = report::render_human(&sample_stats());
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "--- xezim run statistics ---");
    assert!(lines.contains(&"version     : 0.9.8"), "got:\n{text}");
    assert!(lines.contains(&"git_rev     : abc1234"), "got:\n{text}");
    assert!(lines.contains(&"wall_ms     : 42"), "got:\n{text}");
    assert!(lines.contains(&"cpu_user_ms : 30"), "got:\n{text}");
    assert!(lines.contains(&"cpu_sys_ms  : 7"), "got:\n{text}");
    assert!(lines.contains(&"peak_rss_kb : 12345"), "got:\n{text}");
    assert!(lines.contains(&"hostname    : buildhost"), "got:\n{text}");
    assert!(lines.contains(&"sim_time_ns : 1000"), "got:\n{text}");
    // Scripts grep stdout for this phrase; the footer must never emit it.
    assert!(!text.contains("Simulation finished"), "got:\n{text}");
}

#[test]
fn human_rendering_omits_unmeasured_fields() {
    let mut stats = sample_stats();
    stats.peak_rss_kb = None;
    stats.hostname = None;
    stats.sim_time_ns = None;
    let text = report::render_human(&stats);
    assert!(!text.contains("peak_rss_kb"), "got:\n{text}");
    assert!(!text.contains("hostname"), "got:\n{text}");
    assert!(!text.contains("sim_time_ns"), "got:\n{text}");
}

#[test]
fn json_rendering() {
    let json = report::render_json(&sample_stats());
    // Single flat line, newline-terminated.
    assert!(json.ends_with("}\n"), "got: {json}");
    assert_eq!(json.lines().count(), 1, "got: {json}");
    assert_eq!(
        json,
        "{\"schema_version\":1,\"version\":\"0.9.8\",\"git_rev\":\"abc1234\",\
         \"wall_ms\":42,\"cpu_user_ms\":30,\"cpu_sys_ms\":7,\
         \"peak_rss_kb\":12345,\"hostname\":\"buildhost\",\"sim_time_ns\":1000}\n"
    );
}

#[test]
fn json_rendering_omits_unmeasured_fields() {
    let mut stats = sample_stats();
    stats.peak_rss_kb = None;
    stats.hostname = None;
    stats.sim_time_ns = None;
    let json = report::render_json(&stats);
    assert!(!json.contains("peak_rss_kb"), "got: {json}");
    assert!(!json.contains("hostname"), "got: {json}");
    assert!(!json.contains("sim_time_ns"), "got: {json}");
    assert!(json.contains("\"schema_version\":1"), "got: {json}");
}

#[test]
fn json_hostname_escaping() {
    let mut stats = sample_stats();
    stats.hostname = Some("we\"ird\\host\nname\x01".to_string());
    let json = report::render_json(&stats);
    assert!(
        json.contains("\"hostname\":\"we\\\"ird\\\\host\\nname\\u0001\""),
        "got: {json}"
    );
    // Direct checks on the escaper itself.
    assert_eq!(report::escape_json("plain-host"), "plain-host");
    assert_eq!(report::escape_json("a\"b\\c"), "a\\\"b\\\\c");
    assert_eq!(report::escape_json("a\r\n\tb"), "a\\r\\n\\tb");
    assert_eq!(report::escape_json("\x02"), "\\u0002");
}

#[test]
fn env_value_mapping() {
    assert_eq!(report::mode_from_env_value(None), ReportMode::Off);
    assert_eq!(report::mode_from_env_value(Some("")), ReportMode::Off);
    assert_eq!(report::mode_from_env_value(Some("0")), ReportMode::Off);
    assert_eq!(report::mode_from_env_value(Some("1")), ReportMode::Human);
    assert_eq!(report::mode_from_env_value(Some("json")), ReportMode::Json);
    assert_eq!(report::mode_from_env_value(Some("yes")), ReportMode::Off);
}

#[test]
fn collectors_return_plausible_values() {
    let (user_ms, sys_ms) = report::cpu_times_ms();
    // getrusage cannot go backwards; just pin that the pair is well-formed
    // (no panic, plausible magnitude for a test process).
    assert!(user_ms < 3_600_000 && sys_ms < 3_600_000);
    #[cfg(target_os = "linux")]
    {
        let peak = report::peak_rss_kb().expect("VmHWM readable on Linux");
        assert!(peak > 0);
        let host = report::hostname().expect("hostname readable on Linux");
        assert!(!host.is_empty());
    }
}

/// Write a trivial design and run the built binary on it, returning
/// (stdout, stderr). `envs` is applied on top of a scrubbed
/// XEZIM_REPORT_STATS so the ambient environment cannot leak in.
fn run_binary(extra_args: &[&str], envs: &[(&str, &str)]) -> (String, String) {
    let dir = std::env::temp_dir().join(format!(
        "xezim_report_stats_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let sv = dir.join("smoke.sv");
    std::fs::write(&sv, "module report_stats_smoke;\n  initial $finish;\nendmodule\n")
        .expect("write smoke.sv");

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_xezim"));
    cmd.arg(sv.to_str().unwrap())
        .args(extra_args)
        .env_remove("XEZIM_REPORT_STATS");
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("failed to execute xezim");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.status.success(), "xezim exited nonzero");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn binary_emits_json_footer_on_stderr_when_asked() {
    let (stdout, stderr) = run_binary(&["--report-stats=json"], &[]);
    // Normal stdout is untouched.
    assert!(stdout.contains("Simulation finished at time"), "stdout:\n{stdout}");
    assert!(!stdout.contains("schema_version"), "stdout:\n{stdout}");
    // The footer is one JSON line on stderr with the expected keys.
    let line = stderr
        .lines()
        .find(|l| l.contains("\"schema_version\":1"))
        .unwrap_or_else(|| panic!("no JSON footer on stderr:\n{stderr}"));
    assert!(line.starts_with('{') && line.ends_with('}'), "footer: {line}");
    for key in [
        "\"version\":\"",
        "\"git_rev\":\"",
        "\"wall_ms\":",
        "\"cpu_user_ms\":",
        "\"cpu_sys_ms\":",
        "\"sim_time_ns\":",
    ] {
        assert!(line.contains(key), "missing {key} in footer: {line}");
    }
}

#[test]
fn binary_emits_no_footer_by_default() {
    let (stdout, stderr) = run_binary(&[], &[]);
    assert!(stdout.contains("Simulation finished at time"), "stdout:\n{stdout}");
    for stream in [&stdout, &stderr] {
        assert!(!stream.contains("xezim run statistics"), "got:\n{stream}");
        assert!(!stream.contains("schema_version"), "got:\n{stream}");
    }
}

#[test]
fn binary_env_switch_and_cli_precedence() {
    // Env alone turns the human footer on.
    let (_, stderr) = run_binary(&[], &[("XEZIM_REPORT_STATS", "1")]);
    assert!(stderr.contains("--- xezim run statistics ---"), "stderr:\n{stderr}");
    // CLI flag wins over the env: human text despite XEZIM_REPORT_STATS=json.
    let (_, stderr) = run_binary(&["--report-stats"], &[("XEZIM_REPORT_STATS", "json")]);
    assert!(stderr.contains("--- xezim run statistics ---"), "stderr:\n{stderr}");
    assert!(!stderr.contains("schema_version"), "stderr:\n{stderr}");
}
