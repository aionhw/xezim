//! GitHub #108: DPI imports inside CHILD modules (parameterized or not) must
//! dispatch to the C function. Only top-module and package DPI items reached
//! `register_dpi_import`; every call in an instantiated module silently
//! resolved to nothing and "returned" 0/null.

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

#[test]
fn dpi_import_in_child_modules_dispatches() {
    let so_path = unique_so_path("child_module_import");
    let status = Command::new("cc")
        .arg("-shared")
        .arg("-fPIC")
        .arg(manifest_path("tests/dpi/child_module_import.c"))
        .arg("-o")
        .arg(&so_path)
        .status()
        .expect("failed to launch cc");
    assert!(status.success(), "cc failed");

    let bin = env!("CARGO_BIN_EXE_xezim");
    let out = Command::new(bin)
        .arg("--simulate")
        .arg("--sv2017")
        .arg("-s")
        .arg("top")
        .arg("--dpi-lib")
        .arg(&so_path)
        .arg("--max-time")
        .arg("1000")
        .arg(manifest_path("tests/dpi/child_module_import_test.sv"))
        .output()
        .expect("failed to run xezim");
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    let _ = std::fs::remove_file(&so_path);

    for want in [
        "[probe] open 'no-param-child'",
        "[probe] open 'int-param-child'",
        "nchild: null=0",
        "ichild: null=0",
    ] {
        assert!(text.contains(want), "missing {want:?} in:\n{text}");
    }
}
