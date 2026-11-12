//! Options that read their values from a file (src/cli_files.rs):
//! `-fst_scope_file <file>` adds FST dump scopes, and `-xezim_env <file>`
//! sets `XEZIM_*` variables before xezim reads any of them.

use std::path::{Path, PathBuf};
use std::process::Command;

#[allow(dead_code)]
#[path = "../../src/cli_files.rs"]
mod cli_files;

use cli_files::{env_files_in_args, option_value, parse_env_file, parse_scope_file};

fn xezim() -> String {
    // Cargo names the binary it built for the integration tests.
    env!("CARGO_BIN_EXE_xezim").to_string()
}

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("xezim_cli_files_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Run xezim in `dir` with XEZIM_VERBOSE removed from the inherited
/// environment unless `env` sets it; returns (exit code, stdout + stderr).
fn run(dir: &Path, env: &[(&str, &str)], args: &[&str]) -> (i32, String) {
    let mut c = Command::new(xezim());
    c.current_dir(dir).args(args).env_remove("XEZIM_VERBOSE");
    for (k, v) in env {
        c.env(k, v);
    }
    let out = c.output().expect("run xezim");
    (
        out.status.code().unwrap_or(-1),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

const DESIGN: &str = "module top;\n  sub u1(); sub u2();\n  reg a = 0;\n  initial #10 $finish;\nendmodule\nmodule sub;\n  reg x = 0, y = 0;\nendmodule\n";

#[test]
fn scope_file_parsing() {
    let text =
        "# scopes\ntop.u1   // the first\n\ntop.u2, top.a\n  top.u3 top.u4  # two on a line\n";
    assert_eq!(
        parse_scope_file(text),
        vec!["top.u1", "top.u2", "top.a", "top.u3", "top.u4"]
    );
    assert!(parse_scope_file("# only comments\n\n// none\n").is_empty());
}

#[test]
fn env_file_parsing() {
    let text = r#"
# every accepted form
XEZIM_A=1
export XEZIM_B=two words
setenv XEZIM_C 3
XEZIM_D "quoted value"
XEZIM_E='single'   // trailing comment
XEZIM_F=
unsetenv XEZIM_G
unset XEZIM_H
"#;
    let got = parse_env_file(text).unwrap();
    let want: Vec<(String, Option<String>)> = vec![
        ("XEZIM_A".into(), Some("1".into())),
        ("XEZIM_B".into(), Some("two words".into())),
        ("XEZIM_C".into(), Some("3".into())),
        ("XEZIM_D".into(), Some("quoted value".into())),
        ("XEZIM_E".into(), Some("single".into())),
        ("XEZIM_F".into(), Some("".into())),
        ("XEZIM_G".into(), None),
        ("XEZIM_H".into(), None),
    ];
    assert_eq!(got, want);
    let e = parse_env_file("XEZIM_OK=1\nPATH=/tmp\n").unwrap_err();
    assert!(e.contains("line 2") && e.contains("PATH"), "{}", e);
}

#[test]
fn option_spellings() {
    for a in ["-xezim_env", "--xezim_env", "-xezim-env", "--xezim-env"] {
        assert_eq!(
            option_value(a, cli_files::XEZIM_ENV_NAMES),
            Some(None),
            "{}",
            a
        );
    }
    assert_eq!(
        option_value("--xezim-env=f.env", cli_files::XEZIM_ENV_NAMES),
        Some(Some("f.env"))
    );
    assert_eq!(
        option_value("-fst_scope_file=s.txt", cli_files::FST_SCOPE_FILE_NAMES),
        Some(Some("s.txt"))
    );
    assert_eq!(option_value("-f", cli_files::FST_SCOPE_FILE_NAMES), None);
    let args: Vec<String> = ["a.sv", "-xezim_env", "one.env", "--xezim-env=two.env"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(
        env_files_in_args(&args).unwrap(),
        vec!["one.env", "two.env"]
    );
    assert!(env_files_in_args(&["-xezim_env".to_string()]).is_err());
}

#[test]
fn xezim_env_file_sets_variables_before_the_run() {
    let d = scratch("env");
    std::fs::write(d.join("tb.sv"), DESIGN).unwrap();
    // XEZIM_VERBOSE=1 makes the run print its [PHASE] timing lines.
    std::fs::write(d.join("v.env"), "# verbose run\nsetenv XEZIM_VERBOSE 1\n").unwrap();
    let (rc, out) = run(&d, &[], &["-s", "top", "tb.sv"]);
    assert_eq!(rc, 0, "{}", out);
    assert!(
        !out.contains("[PHASE]"),
        "verbose without the file:\n{}",
        out
    );
    let (rc, out) = run(&d, &[], &["-xezim_env", "v.env", "-s", "top", "tb.sv"]);
    assert_eq!(rc, 0, "{}", out);
    assert!(out.contains("[PHASE]"), "verbose with the file:\n{}", out);
    // The file overrides the shell, and `unsetenv` removes a variable.
    std::fs::write(d.join("off.env"), "unsetenv XEZIM_VERBOSE\n").unwrap();
    let (rc, out) = run(
        &d,
        &[("XEZIM_VERBOSE", "1")],
        &["--xezim-env=off.env", "-s", "top", "tb.sv"],
    );
    assert_eq!(rc, 0, "{}", out);
    assert!(!out.contains("[PHASE]"), "unsetenv must win:\n{}", out);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn xezim_env_file_errors_stop_the_run() {
    let d = scratch("envbad");
    std::fs::write(d.join("tb.sv"), DESIGN).unwrap();
    std::fs::write(d.join("bad.env"), "HOME=/tmp\n").unwrap();
    let (rc, out) = run(&d, &[], &["-xezim_env", "bad.env", "-s", "top", "tb.sv"]);
    assert_eq!(rc, 1, "{}", out);
    assert!(out.contains("not an XEZIM_* variable"), "{}", out);
    let (rc, out) = run(
        &d,
        &[],
        &["-xezim_env", "missing.env", "-s", "top", "tb.sv"],
    );
    assert_eq!(rc, 1, "{}", out);
    assert!(out.contains("missing.env"), "{}", out);
    let _ = std::fs::remove_dir_all(&d);
}

/// The `[FST] dumping N signals (scopes=K)` line of a run.
fn fst_line(out: &str) -> String {
    out.lines()
        .find(|l| l.starts_with("[FST] dumping"))
        .unwrap_or_default()
        .to_string()
}

#[test]
fn fst_scope_file_adds_scopes() {
    let d = scratch("fst");
    std::fs::write(d.join("tb.sv"), DESIGN).unwrap();
    std::fs::write(d.join("scopes.txt"), "# only the first instance\ntop.u1\n").unwrap();
    let (rc, out) = run(
        &d,
        &[],
        &[
            "--fst",
            "a.fst",
            "--fst-scope",
            "top.u1",
            "-s",
            "top",
            "tb.sv",
        ],
    );
    assert_eq!(rc, 0, "{}", out);
    let want = fst_line(&out);
    assert!(want.contains("scopes=1"), "{}", out);
    for args in [
        vec!["-fst_scope_file", "scopes.txt"],
        vec!["--fst-scope-file=scopes.txt"],
    ] {
        let mut all = vec!["--fst", "b.fst"];
        all.extend(args.iter().copied());
        all.extend(["-s", "top", "tb.sv"]);
        let (rc, out) = run(&d, &[], &all);
        assert_eq!(rc, 0, "{:?}: {}", args, out);
        assert_eq!(fst_line(&out), want, "{:?}", args);
    }
    // Inside an args file: a relative path resolves as given, else next to the
    // args file, like every other path in it.
    std::fs::create_dir_all(d.join("cfg")).unwrap();
    std::fs::write(d.join("cfg/both_scopes.txt"), "top.u1\ntop.u2\n").unwrap();
    std::fs::write(d.join("cfg/run.f"), "-fst_scope_file both_scopes.txt\n").unwrap();
    let (rc, out) = run(
        &d,
        &[],
        &["--fst", "c.fst", "-f", "cfg/run.f", "-s", "top", "tb.sv"],
    );
    assert_eq!(rc, 0, "{}", out);
    assert!(fst_line(&out).contains("scopes=2"), "{}", out);
    let _ = std::fs::remove_dir_all(&d);
}

/// Three levels, one register each: `top.a`, `top.m1.b`, `top.m1.l1.c`.
const NESTED: &str = "module top;\n  mid m1();\n  reg a = 0;\n  initial #10 $finish;\nendmodule\nmodule mid;\n  leaf l1();\n  reg b = 0;\nendmodule\nmodule leaf;\n  reg c = 0;\nendmodule\n";

/// The number of signals the `[FST] dumping N signals` line reports.
fn fst_count(out: &str) -> usize {
    let l = fst_line(out);
    l.split_whitespace()
        .nth(2)
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("no [FST] line:\n{}", out))
}

#[test]
fn fst_scope_depth_limits_levels() {
    // IEEE 1800-2023 §21.7.1.4: 0 = every level below the scope, N = N
    // levels starting at the scope.
    let d = scratch("depth");
    std::fs::write(d.join("tb.sv"), NESTED).unwrap();
    let count = |scopes: &[&str]| {
        let mut args = vec!["--fst", "d.fst"];
        for s in scopes {
            args.extend(["--fst-scope", s]);
        }
        args.extend(["-s", "top", "tb.sv"]);
        let (rc, out) = run(&d, &[], &args);
        assert_eq!(rc, 0, "{:?}: {}", scopes, out);
        fst_count(&out)
    };
    let all = count(&["top.m1"]);
    assert_eq!(count(&["0:top.m1"]), all, "0: means every level");
    assert_eq!(count(&["1:top.m1"]), 1, "1: is the scope's own signals (b)");
    assert_eq!(count(&["2:top.m1"]), 2, "2: adds one level of children (c)");
    assert_eq!(all, 2);
    // Mixed depths in one list, and from a scope file.
    assert_eq!(count(&["1:top.m1", "top.m1.l1"]), 2);
    std::fs::write(d.join("depth.txt"), "1:top.m1   # only b\n").unwrap();
    let (rc, out) = run(
        &d,
        &[],
        &[
            "--fst",
            "e.fst",
            "-fst_scope_file",
            "depth.txt",
            "-s",
            "top",
            "tb.sv",
        ],
    );
    assert_eq!(rc, 0, "{}", out);
    assert_eq!(fst_count(&out), 1, "{}", out);
    let _ = std::fs::remove_dir_all(&d);
}
