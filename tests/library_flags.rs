//! Commercial-style library flags: `-v <file>`, `-y <dir>`, `+libext+<ext>`.
//!
//! `-v` was previously a no-op "verbose" flag (nothing consumed it); it now
//! carries the Verilog-XL/VCS meaning: a library FILE whose modules are
//! compiled only to satisfy unresolved instantiations and are never
//! top-module candidates. `+libext+` REPLACES the `-y` extension list
//! (default .v/.sv/.V), matching commercial tools. Both also work inside
//! `-f` filelists with filelist-relative paths.

use std::path::PathBuf;
use std::process::Command;

fn xezim_bin() -> PathBuf {
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("xezim")
}

fn setup(dir: &std::path::Path) {
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    std::fs::write(
        dir.join("lib/dut.sv"),
        "module dut(output logic [3:0] y); assign y = 4'h7; endmodule\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("lib/dut3.vlib"),
        "module dut3(output logic [3:0] y); assign y = 4'h1; endmodule\n",
    )
    .unwrap();
    // Vendor-style lib file: the needed module plus an UNUSED one that
    // instantiates a missing cell (must not error, must not become a top).
    std::fs::write(
        dir.join("cells.v"),
        "module dutv(output logic o); assign o = 1; endmodule\n\
         module unused_vendor(output logic o); missing_cell u(.o(o)); endmodule\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("top.sv"),
        "module top; wire [3:0] a; wire o;\n\
         dut u1(.y(a)); dutv u2(.o(o));\n\
         initial begin #1; if (a == 7 && o) $display(\"LIB_PASS\"); \
         else $display(\"LIB_FAIL\"); $finish; end endmodule\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("top3.sv"),
        "module top3; wire [3:0] c; dut3 u(.y(c));\n\
         initial begin #1; if (c == 1) $display(\"EXT_PASS\"); $finish; end endmodule\n",
    )
    .unwrap();
}

fn run(dir: &std::path::Path, args: &[&str]) -> String {
    let out = Command::new(xezim_bin())
        .current_dir(dir)
        .args(args)
        .output()
        .expect("run xezim");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// `-v cells.v` + `-y lib`: dir module and lib-file module both resolve; the
/// unused vendor module referencing a missing cell must not break the run.
#[test]
fn v_file_and_y_dir_resolve_on_demand() {
    let dir = std::env::temp_dir().join("xezim_libflags_a");
    setup(&dir);
    let out = run(&dir, &["-v", "cells.v", "-y", "lib", "top.sv"]);
    assert!(out.contains("LIB_PASS"), "got:\n{}", out);
}

/// `+libext+.vlib` finds `.vlib` files — and REPLACES the default list, so a
/// `.sv` library file is then invisible (commercial semantics).
#[test]
fn libext_extends_and_replaces() {
    let dir = std::env::temp_dir().join("xezim_libflags_b");
    setup(&dir);
    let found = run(&dir, &["-y", "lib", "+libext+.vlib", "top3.sv"]);
    assert!(found.contains("EXT_PASS"), "got:\n{}", found);
    // .sv no longer searched when the list is replaced:
    let miss = run(&dir, &["-y", "lib", "+libext+.vlib", "-v", "cells.v", "top.sv"]);
    assert!(
        miss.contains("instantiated but not found"),
        "+libext must REPLACE the default extension list, got:\n{}",
        miss
    );
    // both extensions listed -> both resolve:
    let both = run(
        &dir,
        &["-y", "lib", "+libext+.vlib+.sv", "-v", "cells.v", "top.sv"],
    );
    assert!(both.contains("LIB_PASS"), "got:\n{}", both);
}

/// `-v` and `+libext+` inside a `-f` filelist, with filelist-relative paths.
#[test]
fn library_flags_inside_filelist() {
    let dir = std::env::temp_dir().join("xezim_libflags_c");
    setup(&dir);
    std::fs::write(
        dir.join("run.f"),
        "-v cells.v\n+libext+.sv\n-y lib\ntop.sv\n",
    )
    .unwrap();
    let out = run(&dir, &["-f", "run.f"]);
    assert!(out.contains("LIB_PASS"), "got:\n{}", out);
}

/// A missing `-v` file is a clear error, not a silent ignore.
#[test]
fn missing_v_file_errors() {
    let dir = std::env::temp_dir().join("xezim_libflags_d");
    setup(&dir);
    let out = run(&dir, &["-v", "no_such_lib.v", "top.sv"]);
    assert!(
        out.contains("library file not found"),
        "got:\n{}",
        out
    );
}

/// `--module-timescale` must apply to modules loaded from a `-v` library file,
/// same as a primary-source module. Before this, a `-v` module kept raw
/// tick-unit delays (adopted after the primary delay-rewrite), so the same
/// module scaled differently depending on how it was loaded.
#[test]
fn module_timescale_applies_to_v_library_modules() {
    let dir = std::env::temp_dir().join("xezim_libflags_ts");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("libc.v"),
        "module libcell;\n  initial #100 $display(\"CT=%0t\", $time);\nendmodule\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("t.sv"),
        "module top; libcell u(); initial #100000 $finish; endmodule\n",
    )
    .unwrap();
    // #100 in a 1ns/1ps module = 100000 ps.
    let via_v = run(&dir, &["-v", "libc.v", "--module-timescale", "1ns/1ps", "t.sv"]);
    assert!(via_v.contains("CT=100000"), "-v module must get --module-timescale:\n{}", via_v);
    // Same module as a primary source — must agree.
    let reg = run(&dir, &["--module-timescale", "1ns/1ps", "t.sv", "libc.v"]);
    assert!(reg.contains("CT=100000"), "regular source baseline:\n{}", reg);
}
