//! A real value crossing the DPI boundary into an integral C type converts
//! by value (§35.5.6 with §6.12.2 rounding), never as its f64 bit pattern.
//! #324 (reported by Dragon-git): an exported `longint` function returning
//! `t / 1ns` handed C 0x4059000000000000, the bits of 100.0. The export's
//! own return conversion (§13.4.1) was the root cause; an imported
//! function's integral formals took the same raw bits from a real actual.
//! The expected values are the reference simulator's.

use std::path::{Path, PathBuf};
use std::process::Command;

fn manifest_path(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

/// Run `tests/dpi/<stem>_test.sv` against `tests/dpi/<stem>.c` on the
/// method-execution path `env` selects; its `T|` / PASS / FAIL lines.
fn run(stem: &str, env: &[(&str, &str)]) -> Vec<String> {
    let so = std::env::temp_dir().join(format!(
        "xezim_dpi_real_{}_{}_{}.so",
        stem,
        env.len(),
        std::process::id()
    ));
    let mut cc = Command::new(std::env::var_os("CC").unwrap_or_else(|| "clang".into()));
    cc.args(["-shared", "-fPIC"]);
    if cfg!(target_os = "macos") {
        // The exported subroutines resolve when xezim loads the library.
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
        .arg("--dpi-lib")
        .arg(&so)
        .args(["--no-cache", "--max-time", "1000", "-s", "top"])
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
        .filter(|l| l.starts_with("T|") || l.starts_with("PASS") || l.starts_with("FAIL"))
        .map(str::to_string)
        .collect()
}

const PATHS: &[&[(&str, &str)]] = &[
    &[],
    &[("XEZIM_COMPILE_METHODS", "0")],
    &[("XEZIM_METHOD_TIER", "0")],
];

/// #324 as reported.
#[test]
fn export_longint_returning_time_quotient() {
    for env in PATHS {
        assert_eq!(
            run("dpi_real_return_bug", env),
            ["PASS: get_drain_time returned 100"],
            "{env:?}"
        );
    }
}

/// Every export result type and output formal fed a real or an integral
/// value, and imported functions given real actuals for integral formals
/// (and the reverse).
#[test]
fn export_results_and_import_actuals_convert_by_value() {
    for env in PATHS {
        assert_eq!(
            run("dpi_real_int_conv", env),
            [
                "T|exp int=8",
                "T|exp long=100",
                "T|exp byte=-2",
                "T|exp short=301",
                "T|exp bit=1",
                "T|exp real=7.000000",
                "T|exp sreal=2.000000",
                "T|exp str=hi",
                "T|exp out o=3",
                "T|exp out ol=4",
                "T|exp out orl=4.000000",
                "T|exp uint=4294967295",
                "T|exp half=3",
                "T|exp mhalf=-3",
                "T|exp rvar=10",
                "T|imp add=6",
                "T|imp addv=4",
                "T|imp half=3.500000",
                "T|imp retreal_to_int=3",
                "T|imp retreal_direct=2.500000",
                "T|imp long=10000000001",
                "T|imp outs o=5 r=1.500000",
                "T|imp outs_swapped r=5.000000 i=2",
                "T|imp byte=-4",
            ],
            "{env:?}"
        );
    }
}
