//! `-l` / `--log <file>`: everything the run prints goes to the file.
//!
//! This was previously a lie: the CLI accepted the flag and called a
//! `set_log_file` that was a stub returning `Ok(())`, so the run reported
//! success and wrote nothing. The redirection is done at the file-descriptor
//! level (dup2), which is what makes it catch ALL of the output — the
//! simulator prints through `println!`, and DPI/VPI C models `printf()`
//! straight to fd 1, so a Rust-writer-based logger would miss both.

use std::path::PathBuf;
use std::process::Command;

fn xezim_bin() -> PathBuf {
    // Cargo names the binary it built for the integration tests.
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_xezim"))
}

const SRC: &str = r#"
module top;
  initial begin
    $display("MARKER_STDOUT");
    $fdisplay(32'h8000_0002, "MARKER_STDERR");
    #1 $finish;
  end
endmodule
"#;

fn run_with_log(flag: &[&str], log: &std::path::Path) -> (String, String) {
    let dir = log.parent().unwrap();
    let sv = dir.join(format!(
        "log_redirect_{}.sv",
        log.file_stem().unwrap().to_string_lossy()
    ));
    std::fs::write(&sv, SRC).expect("write sv");

    let out = Command::new(xezim_bin())
        .args(flag)
        .arg(&sv)
        .output()
        .expect("run xezim");

    let terminal = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let logged = std::fs::read_to_string(log).unwrap_or_default();
    (terminal, logged)
}

/// The log file must actually be written, and must capture BOTH streams — and
/// the terminal must then be silent, because this is a redirect, not a tee.
#[test]
fn log_flag_redirects_both_streams_to_the_file() {
    let dir = std::env::temp_dir().join("xezim_log_test");
    std::fs::create_dir_all(&dir).expect("mkdir");

    for (i, flag) in [vec!["-l"], vec!["--log"]].into_iter().enumerate() {
        let log = dir.join(format!("run{}.log", i));
        let _ = std::fs::remove_file(&log);
        let mut args = flag.clone();
        let log_str = log.to_string_lossy().to_string();
        args.push(&log_str);

        let (terminal, logged) = run_with_log(&args, &log);

        assert!(
            logged.contains("MARKER_STDOUT"),
            "{:?}: stdout not captured in the log file. log:\n{}",
            flag,
            logged
        );
        assert!(
            logged.contains("MARKER_STDERR"),
            "{:?}: stderr not captured in the log file. log:\n{}",
            flag,
            logged
        );
        assert!(
            !terminal.contains("MARKER_STDOUT") && !terminal.contains("MARKER_STDERR"),
            "{:?}: output still reached the terminal — this is a redirect, not a tee. \
             terminal:\n{}",
            flag,
            terminal
        );
    }
}

/// `--log=<file>` (the `=` form) must work too.
#[test]
fn log_flag_accepts_equals_form() {
    let dir = std::env::temp_dir().join("xezim_log_test");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let log = dir.join("eq.log");
    let _ = std::fs::remove_file(&log);

    let arg = format!("--log={}", log.display());
    let (_, logged) = run_with_log(&[&arg], &log);
    assert!(
        logged.contains("MARKER_STDOUT"),
        "--log=<file> did not capture output. log:\n{}",
        logged
    );
}

/// A log path that cannot be opened must FAIL the run, not silently "succeed"
/// while dropping every line the user asked to keep.
#[test]
fn unopenable_log_path_is_an_error() {
    let dir = std::env::temp_dir().join("xezim_log_test");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let sv = dir.join("bad.sv");
    std::fs::write(&sv, SRC).expect("write sv");

    let out = Command::new(xezim_bin())
        .arg("--log")
        .arg("/nonexistent-dir-xyz/run.log")
        .arg(&sv)
        .output()
        .expect("run xezim");

    assert!(
        !out.status.success(),
        "an unopenable log path must fail the run"
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("cannot open log file"),
        "expected a clear error, got:\n{}",
        err
    );
}

/// The result line must appear exactly ONCE. It used to be printed to stdout by
/// the CLI *and* to stderr by the library, so it showed up twice in any terminal
/// or merged log.
#[test]
fn finish_line_is_not_printed_twice() {
    let dir = std::env::temp_dir().join("xezim_log_test");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let log = dir.join("once.log");
    let _ = std::fs::remove_file(&log);

    let log_str = log.to_string_lossy().to_string();
    let (_, logged) = run_with_log(&["--log", &log_str], &log);

    let n = logged
        .lines()
        .filter(|l| l.contains("Simulation finished at time"))
        .count();
    assert_eq!(
        n, 1,
        "result line must appear once, saw {}x:\n{}",
        n, logged
    );
}
