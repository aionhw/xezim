//! §22.5.1, §3.12.1 (#1512): two `-v`/`-y` library files include the same
//! guarded header that declares `$unit` subroutines. By default each library
//! file starts from the command-line defines only, so neither the guard an
//! earlier library file defined nor a source file's `` `define `` of it hides
//! the header body from a later library file. The expected lines are the
//! reference simulator's output for the same files.

use std::path::{Path, PathBuf};
use std::process::Command;

const HEADER: &str = "`ifndef WRAP_H\n`define WRAP_H\n`ifndef SYNTHESIS\n  task wrap_get(input int n, output [31:0] r);\n    r = 32'hfffffff0 | n[31:0];\n  endtask\n  function int wrap_mul(input int a, input int b);\n    return a * b;\n  endfunction\n`endif\n`endif\n";
const FIRST: &str = "`include \"wrap.h\"\nmodule leaf_first(output int q);\n  initial q = wrap_mul(2, 3);\nendmodule\n";
const MID: &str = "`include \"wrap.h\"\nmodule leaf_mid(output int q);\n  reg [31:0] r;\n  initial begin\n    wrap_get(1, r);\n    q = r[3:0];\n  end\nendmodule\n";
const TOP: &str = "module top;\n  int a, b;\n  leaf_first u1(a);\n  leaf_mid u2(b);\n  initial #1 $display(\"top a=%0d b=%0d\", a, b);\nendmodule\n";
const TOP_DEFINES_GUARD: &str = "`define WRAP_H\nmodule top;\n  int b;\n  leaf_mid u2(b);\n  initial #1 $display(\"top b=%0d\", b);\nendmodule\n";

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("xezim_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (file, text) in [
        ("inc/wrap.h", HEADER),
        ("first.v", FIRST),
        ("mid.v", MID),
        ("lib/leaf_first.v", FIRST),
        ("lib/leaf_mid.v", MID),
        ("top.sv", TOP),
        ("top_guard.sv", TOP_DEFINES_GUARD),
    ] {
        let p = dir.join(file);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    dir
}

/// Run xezim in `dir`: the `top` lines on success, else the error text.
fn xezim(dir: &Path, args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .current_dir(dir)
        .args(["--no-cache", "+incdir+inc", "-s", "top"])
        .args(args)
        .output()
        .expect("run xezim");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    if !out.status.success() {
        return text;
    }
    text.lines()
        .filter(|l| l.starts_with("top"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn each_library_file_sees_the_body_of_a_shared_guarded_header() {
    let dir = scratch("lib_guarded_header");
    assert_eq!(
        xezim(&dir, &["top.sv", "-v", "first.v", "-v", "mid.v"]),
        "top a=6 b=1"
    );
    assert_eq!(
        xezim(&dir, &["top.sv", "-y", "lib", "+libext+.v"]),
        "top a=6 b=1"
    );
    // A source file's define of the guard stays out of the library file.
    assert_eq!(xezim(&dir, &["top_guard.sv", "-v", "mid.v"]), "top b=1");
    assert_eq!(
        xezim(&dir, &["top_guard.sv", "-y", "lib", "+libext+.v"]),
        "top b=1"
    );
    // The carry switches bring the guard back, and with it the missing task.
    let carried = xezim(
        &dir,
        &[
            "top.sv",
            "-v",
            "first.v",
            "-v",
            "mid.v",
            "--carry-lib-macros",
        ],
    );
    assert!(carried.contains("wrap_get"), "{carried}");
    let carried = xezim(&dir, &["top_guard.sv", "-v", "mid.v", "--carry-src-macros"]);
    assert!(carried.contains("wrap_get"), "{carried}");
    let _ = std::fs::remove_dir_all(&dir);
}
