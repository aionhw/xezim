use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn manifest_path(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn unique_so_path(stem: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!("{}_{}_{}.so", stem, std::process::id(), nanos))
}

fn compile_dpi_lib(c_file: &str, stem: &str) -> PathBuf {
    let c_path = manifest_path(c_file);
    let so_path = unique_so_path(stem);
    let status = Command::new("cc")
        .arg("-shared")
        .arg("-fPIC")
        .arg(&c_path)
        .arg("-o")
        .arg(&so_path)
        .status()
        .expect("failed to launch cc");
    assert!(status.success(), "cc failed for {}", c_path.display());
    so_path
}

fn run_xezim_with_dpi(so_path: &Path, sv_file: &str) -> String {
    let bin = std::env::var("CARGO_BIN_EXE_xezim").unwrap_or_else(|_| "xezim".to_string());
    let sv_path = manifest_path(sv_file);
    let out = Command::new(bin)
        .arg("--dpi-lib")
        .arg(so_path)
        .arg("--max-time")
        .arg("1000")
        .arg(&sv_path)
        .output()
        .expect("failed to run xezim");
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    assert!(
        out.status.success(),
        "xezim failed for {}:\n{}",
        sv_path.display(),
        text
    );
    text
}

fn assert_dpi_pass(c_file: &str, stem: &str, sv_file: &str) {
    let so = compile_dpi_lib(c_file, stem);
    let log = run_xezim_with_dpi(&so, sv_file);
    assert!(
        log.contains("TEST_PASS"),
        "missing TEST_PASS for {}:\n{}",
        sv_file,
        log
    );
}

#[test]
fn dpi_simple_test() {
    assert_dpi_pass(
        "tests/dpi/simple_dpi.c",
        "simple_dpi",
        "tests/dpi/simple_dpi_test.sv",
    );
}

#[test]
fn dpi_extended_test() {
    assert_dpi_pass(
        "tests/dpi/extended_dpi.c",
        "extended_dpi",
        "tests/dpi/extended_dpi_test.sv",
    );
}

#[test]
fn dpi_logic_vec_test() {
    assert_dpi_pass(
        "tests/dpi/logic_vec_dpi.c",
        "logic_vec_dpi",
        "tests/dpi/logic_vec_dpi_test.sv",
    );
}

#[test]
fn dpi_shortreal_string_test() {
    assert_dpi_pass(
        "tests/dpi/shortreal_string_dpi.c",
        "shortreal_string_dpi",
        "tests/dpi/shortreal_string_dpi_test.sv",
    );
}

#[test]
fn dpi_open_array_test() {
    assert_dpi_pass(
        "tests/dpi/open_array_dpi.c",
        "open_array_dpi",
        "tests/dpi/open_array_dpi_test.sv",
    );
}
