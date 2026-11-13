//! `--dpi-lib` loading (#201, #202).
//!
//! Libraries are opened `RTLD_NOW | RTLD_GLOBAL`, so one library can link
//! against another, in either command-line order. A library that cannot be
//! loaded stops the run before it starts, and calling an import that no
//! library implements is fatal; both used to finish and exit 0 with the
//! import returning 0. The reference simulator refuses to load the design in
//! the first case and stops with a fatal error in the second.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const TOP: &str = r#"
module top;
  import "DPI-C" function int dpi_a();
  initial begin
    $display("RESULT=%0d", dpi_a());
    $finish;
  end
endmodule
"#;

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("xezim_dpi_load_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("top.sv"), TOP).unwrap();
    d
}

fn shared_lib(dir: &Path, stem: &str, src: &str) -> PathBuf {
    let c = dir.join(format!("{stem}.c"));
    let so = dir.join(format!("{stem}.so"));
    std::fs::write(&c, src).unwrap();
    let ok = Command::new(std::env::var_os("CC").unwrap_or_else(|| "clang".into()))
        .args(["-shared", "-fPIC"])
        .arg(&c)
        .arg("-o")
        .arg(&so)
        .status()
        .expect("failed to launch cc")
        .success();
    assert!(ok, "cc failed for {}", c.display());
    so
}

/// A lower library and an upper one that calls into it.
fn layered_libs(dir: &Path) -> (PathBuf, PathBuf) {
    let lower = shared_lib(dir, "libb", "int other_api(void) { return 42; }\n");
    let upper = shared_lib(
        dir,
        "liba",
        "extern int other_api(void);\nint dpi_a(void) { return other_api(); }\n",
    );
    (lower, upper)
}

fn run(dir: &Path, libs: &[&Path]) -> (Output, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_xezim"));
    for l in libs {
        cmd.arg("--dpi-lib").arg(l);
    }
    let out = cmd
        .args(["--no-cache", "--error-exit", "top.sv"])
        .current_dir(dir)
        .output()
        .expect("failed to run xezim");
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out, text)
}

#[test]
fn layered_libraries_link_in_either_order() {
    let d = scratch("layered");
    let (lower, upper) = layered_libs(&d);
    for libs in [[&lower, &upper], [&upper, &lower]] {
        let (out, text) = run(&d, &[libs[0].as_path(), libs[1].as_path()]);
        assert!(out.status.success(), "{text}");
        assert!(text.contains("RESULT=42"), "{text}");
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_library_that_cannot_load_stops_the_run() {
    let d = scratch("unloadable");
    let (_, upper) = layered_libs(&d);
    // `liba` alone cannot resolve `other_api`; a missing file cannot open.
    for lib in [upper, d.join("missing.so")] {
        let (out, text) = run(&d, &[lib.as_path()]);
        assert_eq!(out.status.code(), Some(1), "{text}");
        assert!(text.contains("could not be loaded"), "{text}");
        assert!(!text.contains("RESULT="), "the run must not start:\n{text}");
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn calling_an_unimplemented_import_is_fatal() {
    let d = scratch("unimplemented");
    let other = shared_lib(&d, "libc0", "int unrelated(void) { return 1; }\n");
    let (out, text) = run(&d, &[other.as_path()]);
    assert_eq!(out.status.code(), Some(1), "{text}");
    assert!(
        text.contains("** Fatal: DPI import 'dpi_a' has no implementation"),
        "{text}"
    );
    let _ = std::fs::remove_dir_all(&d);
}
