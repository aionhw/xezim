//! Issue #107: the exit-code contract, end to end. CI flows key success off
//! the process status, so each row is pinned as a subprocess run:
//!
//! | condition                        | exit |
//! |----------------------------------|------|
//! | parse error                      | 1    |
//! | elaboration error                | 1    |
//! | `$fatal`                         | 1    |
//! | `$error` (default)               | 0    |
//! | `$error` with `--error-exit`     | 1    |
//! | `-s` names a missing top         | 1    | (strict by DEFAULT — `-s` is an
//! |                                  |      |  explicit assertion; silently
//! |                                  |      |  simulating a different root on a
//! |                                  |      |  typo was the reported CI trap)
//! | ... with `--no-strict-top`       | 0    | (generated-corpus leniency)
//! | valid `-s` / no `-s` auto-detect | 0    |
use std::process::Command;

fn xezim() -> String {
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("xezim").to_string_lossy().into_owned()
}

fn run(src: &str, tag: &str, args: &[&str]) -> (i32, String) {
    let path = format!("/tmp/exit_code_{tag}.sv");
    std::fs::write(&path, src).unwrap();
    let out = Command::new(xezim())
        .args(args)
        .arg(&path)
        .output()
        .expect("run xezim");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.code().unwrap_or(-1), text)
}

const CLEAN: &str = "module top; initial $display(\"ok\"); endmodule\n";

#[test]
fn error_diagnostics_exit_nonzero() {
    let (c, _) = run("module bad(; endmodule\n", "parse", &["--compile"]);
    assert_eq!(c, 1, "parse error must exit 1");
    let (c, _) = run(
        "module top; missing_mod u0(); endmodule\n",
        "elab",
        &["--simulate"],
    );
    assert_eq!(c, 1, "elaboration error must exit 1");
    let (c, _) = run(
        "module top; initial $fatal(1, \"die\"); endmodule\n",
        "fatal",
        &["--simulate"],
    );
    assert_eq!(c, 1, "$fatal must exit 1 (LRM 20.10)");
}

#[test]
fn error_severity_task_promotion_is_opt_in() {
    let src = "module top; initial $error(\"boom\"); endmodule\n";
    let (c, _) = run(src, "err_dflt", &["--simulate"]);
    assert_eq!(c, 0, "$error alone stays exit 0 by default");
    let (c, _) = run(src, "err_promo", &["--simulate", "--error-exit"]);
    assert_eq!(c, 1, "--error-exit must promote $error to a failing exit");
}

#[test]
fn missing_top_is_a_hard_error_by_default() {
    let (c, text) = run(CLEAN, "badtop", &["--simulate", "-s", "nosuchtop"]);
    assert_eq!(c, 1, "-s with an unknown top must fail by default:\n{text}");
    assert!(
        text.contains("known top-level definitions") && text.contains("no-strict-top"),
        "the error must name the known tops and the opt-out:\n{text}"
    );
    let (c, text) = run(
        CLEAN,
        "badtop_lenient",
        &["--simulate", "--no-strict-top", "-s", "nosuchtop"],
    );
    assert_eq!(c, 0, "--no-strict-top restores auto-detection:\n{text}");
    assert!(text.contains("auto-detecting"), "{text}");
    let (c, _) = run(CLEAN, "goodtop", &["--simulate", "-s", "top"]);
    assert_eq!(c, 0);
    let (c, _) = run(CLEAN, "notop", &["--simulate"]);
    assert_eq!(c, 0, "auto-detect with NO -s stays lenient");
}
