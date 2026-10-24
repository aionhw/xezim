//! The VHDL front end: `.vhd`/`.vhdl` files reach the VHDL parser, syntax
//! errors are reported at their source location, `--work=` names the
//! design library of the files that follow, and `--std=` accepts VHDL-2008.

use std::path::PathBuf;
use std::process::{Command, Output};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/vhdl")
}

fn xezim(args: &[&str], env: &[(&str, &str)]) -> (Output, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .current_dir(dir())
        .arg("--no-cache")
        .args(args)
        .envs(env.iter().copied())
        .output()
        .expect("run xezim");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out, text)
}

/// A syntax error fails the run with `file:line:col: error:` at the
/// offending token, the source line and a caret.
#[test]
fn syntax_error_is_located() {
    let (out, text) = xezim(&["errors/syntax.vhd"], &[]);
    assert!(!out.status.success(), "{}", text);
    assert!(
        text.contains("errors/syntax.vhd:2:19: error: VHDL: syntax error: expected ')'"),
        "{}",
        text
    );
    assert!(text.contains("2 |   port (x : in bit"), "{}", text);
}

/// §13.2: `--work=<lib>` puts the following files' design units into that
/// library (`XEZIM_VHDL_DUMP` lists them).
#[test]
fn work_library_of_design_units() {
    let (_, text) = xezim(
        &["--work=mylib", "designs/clock_report.vhd"],
        &[("XEZIM_VHDL_DUMP", "1")],
    );
    assert!(
        text.contains("designs/clock_report.vhd [mylib]: entity clock_report"),
        "{}",
        text
    );
    assert!(
        text.contains("designs/clock_report.vhd [mylib]: architecture a of clock_report"),
        "{}",
        text
    );
}

/// Only VHDL-2008 is supported; another `--std=` is rejected.
#[test]
fn std_other_than_2008_is_rejected() {
    let (out, text) = xezim(&["--std=93", "designs/clock_report.vhd"], &[]);
    assert!(!out.status.success());
    assert!(
        text.contains("--std=93: only VHDL-2008 (--std=08) is supported"),
        "{}",
        text
    );
    let (_, text) = xezim(
        &["--std=08", "designs/clock_report.vhd"],
        &[("XEZIM_VHDL_DUMP", "1")],
    );
    assert!(text.contains("[work]: entity clock_report"), "{}", text);
}
