//! §13.5, §35.5.6: the actual of an imported subroutine's input formal is
//! assigned to the formal on entry, so it is context-determined by the
//! formal's width (§11.6.1, §11.8.1). It was evaluated self-determined and
//! only converted afterwards, so `f(a | (b << 1))` over one-bit operands
//! passed 0 and `f(x + y)` over 8-bit operands lost the carry, in the
//! interpreter and in the compiled lowering of an import call in a class
//! method. A formal written with neither a type nor a direction (`input int
//! a, b`) takes the previous formal's type (§13.5); it was passed to C as an
//! svLogic (#1256). The expected values are the reference simulator's.

use std::path::{Path, PathBuf};
use std::process::Command;

fn manifest_path(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

/// Run `tests/dpi/<stem>_test.sv` against `tests/dpi/<stem>.c` on the
/// method-execution path `env` selects; its PASS / FAIL lines.
fn run(stem: &str, env: &[(&str, &str)]) -> Vec<String> {
    let so = std::env::temp_dir().join(format!(
        "xezim_dpi_ctxw_{}_{}_{}.so",
        stem,
        env.len(),
        std::process::id()
    ));
    let mut cc = Command::new(std::env::var_os("CC").unwrap_or_else(|| "clang".into()));
    cc.args(["-shared", "-fPIC"]);
    if cfg!(target_os = "macos") {
        cc.arg("-Wl,-undefined,dynamic_lookup");
    }
    let ok = cc
        .arg("-I")
        .arg(manifest_path("include"))
        .arg(manifest_path(&format!("tests/dpi/{stem}.c")))
        .arg("-o")
        .arg(&so)
        .status()
        .expect("failed to launch cc")
        .success();
    assert!(ok, "cc failed for {stem}.c");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_xezim"));
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd
        .env_remove("XEZIM_METHOD_CACHE")
        .arg("--dpi-lib")
        .arg(&so)
        .args(["--no-cache", "--simulate", "-s", "top"])
        .arg(manifest_path(&format!("tests/dpi/{stem}_test.sv")))
        .output()
        .expect("failed to run xezim");
    let _ = std::fs::remove_file(&so);
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "{text}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    text.lines()
        .filter(|l| l.starts_with("PASS") || l.starts_with("FAIL"))
        .map(str::to_string)
        .collect()
}

const PATHS: &[&[(&str, &str)]] = &[
    &[],
    &[("XEZIM_COMPILE_METHODS", "0")],
    &[("XEZIM_METHOD_TIER", "0")],
];

#[test]
fn dpi_input_actual_takes_formal_width() {
    for env in PATHS {
        let lines = run("dpi_actual_width", env);
        let fails: Vec<&String> = lines.iter().filter(|l| l.starts_with("FAIL")).collect();
        assert!(
            fails.is_empty() && lines.len() == 27,
            "{env:?}: {} lines, failures {fails:?}",
            lines.len()
        );
    }
}
