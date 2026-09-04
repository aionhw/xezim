//! §35.5.4: `import "DPI-C"` / `export "DPI-C"` written at compilation-unit
//! scope are visible in every module of the unit, like a `$unit` function.
//! They were the one `$unit` declaration kind never injected, so the most
//! common DPI file layout reported every imported name as undeclared. Also
//! pinned: a `byte unsigned` / `shortint` return read at its DECLARED width
//! and sign in an expression (256 and 65530 before), and a 1-bit `logic`
//! argument carrying x as svLogic 3 / z as 2 (both were 0).
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn manifest_path(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn compile_dpi_lib(c_file: &str, stem: &str) -> PathBuf {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    let so_path = std::env::temp_dir().join(format!("{}_{}_{}.so", stem, std::process::id(), nanos));
    let status = Command::new("cc")
        .args(["-shared", "-fPIC", "-I"])
        .arg(manifest_path("include"))
        .arg(manifest_path(c_file))
        .arg("-o")
        .arg(&so_path)
        .status()
        .expect("failed to launch cc");
    assert!(status.success(), "cc failed for {}", c_file);
    so_path
}

#[test]
fn compilation_unit_dpi_imports_and_exports_reach_the_module() {
    let so = compile_dpi_lib("tests/dpi/unit_scope_dpi.c", "unit_scope_dpi");
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .arg("--dpi-lib")
        .arg(&so)
        .args(["--simulate", "-s", "unit_scope_dpi_test", "--no-cache"])
        .arg(manifest_path("tests/dpi/unit_scope_dpi_test.sv"))
        .output()
        .expect("failed to run xezim");
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    let _ = std::fs::remove_file(&so);
    assert!(out.status.success(), "run failed:\n{text}");
    for want in [
        "US add=5 sum=8 diff=6",
        "US ub=0 sh=-6",
        "US cb=1042 handle=42",
        "US logic=1 0 3 2",
    ] {
        assert!(text.contains(want), "missing `{want}`:\n{text}");
    }
}
