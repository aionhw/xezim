//! §20.10 / GitHub issue #107: the process exit status must reflect severity.
//! `$fatal` carries an explicit finish/status semantic and must never exit 0;
//! `$error` is opt-in via `--error-exit` so existing flows that tolerate
//! errors keep working. Parse and elaboration errors already exit 1.

use std::process::Command;

fn xezim_bin() -> std::path::PathBuf {
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("xezim")
}

fn run(src: &str, name: &str, extra: &[&str]) -> i32 {
    let dir = std::env::temp_dir().join("xezim_severity_exit");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let sv = dir.join(format!("{name}.sv"));
    std::fs::write(&sv, src).expect("write");
    let mut cmd = Command::new(xezim_bin());
    cmd.arg("--simulate").arg("-s").arg("top").arg("--max-time").arg("10");
    for a in extra {
        cmd.arg(a);
    }
    cmd.arg(&sv);
    cmd.output().expect("run xezim").status.code().unwrap_or(-1)
}

const FATAL: &str = "module top; initial begin $fatal(1, \"dead\"); end endmodule";
const ERR: &str = "module top; initial begin $error(\"boom\"); #1 $finish; end endmodule";
const OK: &str = "module top; initial begin $display(\"fine\"); #1 $finish; end endmodule";

#[test]
fn fatal_always_exits_nonzero() {
    assert_ne!(run(FATAL, "fatal", &[]), 0, "$fatal must not exit 0");
}

#[test]
fn error_exits_zero_by_default_and_nonzero_when_promoted() {
    assert_eq!(run(ERR, "err_default", &[]), 0, "$error stays non-fatal by default");
    assert_ne!(
        run(ERR, "err_promoted", &["--error-exit"]),
        0,
        "--error-exit promotes $error to a failing status"
    );
}

#[test]
fn clean_run_exits_zero_under_both_modes() {
    assert_eq!(run(OK, "ok_default", &[]), 0);
    assert_eq!(run(OK, "ok_promoted", &["--error-exit"]), 0);
}
