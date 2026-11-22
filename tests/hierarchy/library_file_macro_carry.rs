//! §22.5.1, §3.12.1: the macros a `-v`/`-y` library file is preprocessed
//! with. By default, as in the reference simulator, a library file starts
//! from the command-line defines (`+define+`, `-D`) only: no macro of a
//! source file or of another library file reaches it. `--carry-src-macros`
//! adds the macros the source files left defined; `--carry-lib-macros` adds
//! as well those of the library files read before it, in read order. No
//! library macro reaches a source file. The default expectations are the
//! reference simulator's output for the same files and command lines.

use std::path::{Path, PathBuf};
use std::process::Command;

const LEAF: &str = "module leaf;\n  initial begin\n`ifdef W\n    $display(\"leaf sees W=%0d\", `W);\n`else\n    $display(\"leaf: W undefined\");\n`endif\n  end\nendmodule\n";
const TOP: &str = "module top;\n  mid u();\n  initial #1 $finish;\nendmodule\n";
const TOP_LEAF: &str = "module top;\n  leaf l();\n  initial #1 $finish;\nendmodule\n";

fn write(dir: &Path, name: &str, text: &str) {
    let p = dir.join(name);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("xezim_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run xezim in `dir`; the run must succeed. The `leaf`/`top` lines it
/// printed.
fn xezim(dir: &Path, args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .current_dir(dir)
        .args(args)
        .output()
        .expect("run xezim");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{args:?}:\n{text}");
    text.lines()
        .filter(|l| l.starts_with("leaf") || l.starts_with("top"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn library_file_macros_follow_the_carry_switch() {
    let dir = scratch("libmac_modes");
    write(&dir, "leaf.v", LEAF);
    write(
        &dir,
        "mid.v",
        "`define W 5\nmodule mid;\n  leaf l();\nendmodule\n",
    );
    write(&dir, "plain_mid.v", "module mid;\n  leaf l();\nendmodule\n");
    write(&dir, "top.sv", TOP);
    write(&dir, "top_w7.sv", &format!("`define W 7\n{TOP_LEAF}"));
    write(&dir, "top_leaf.sv", TOP_LEAF);
    write(&dir, "lib/defs.vh", "`define W 9\n");
    write(
        &dir,
        "lib/mid.v",
        "`include \"defs.vh\"\nmodule mid;\n  leaf l();\nendmodule\n",
    );
    write(&dir, "lib/leaf.v", LEAF);
    write(&dir, "lib2/leaf.v", LEAF);
    write(
        &dir,
        "top_sees.sv",
        "module top;\n  mid u();\n  initial begin\n`ifdef W\n    #1 $display(\"top sees W=%0d\", `W);\n`else\n    #1 $display(\"top: W undefined\");\n`endif\n    $finish;\n  end\nendmodule\n",
    );
    write(&dir, "lone_mid.v", "`define W 5\nmodule mid;\nendmodule\n");
    const U: &str = "leaf: W undefined";
    // (files and switches, then the output by default, with
    // `--carry-src-macros` and with `--carry-lib-macros`).
    let cases: &[(&[&str], [&str; 3])] = &[
        // A `-v` file's macro, for a `-v` file read after it and before it.
        (
            &["top.sv", "-v", "mid.v", "-v", "leaf.v"],
            [U, U, "leaf sees W=5"],
        ),
        (&["top.sv", "-v", "leaf.v", "-v", "mid.v"], [U, U, U]),
        // `-y`: the macro of a header the instantiating module includes.
        (
            &["top.sv", "-y", "lib", "+libext+.v", "+incdir+lib"],
            [U, U, "leaf sees W=9"],
        ),
        // A source file's macro, into a `-v` file and a `-y` file.
        (
            &["top_w7.sv", "-v", "leaf.v"],
            [U, "leaf sees W=7", "leaf sees W=7"],
        ),
        (
            &["top_w7.sv", "-y", "lib2", "+libext+.v"],
            [U, "leaf sees W=7", "leaf sees W=7"],
        ),
        // A command-line define reaches every library file, and is the
        // value a source file's redefinition leaves by default.
        (
            &["top_leaf.sv", "-v", "leaf.v", "+define+W=3"],
            ["leaf sees W=3"; 3],
        ),
        (
            &["top_leaf.sv", "-v", "leaf.v", "-DW=3"],
            ["leaf sees W=3"; 3],
        ),
        (
            &["top_leaf.sv", "-y", "lib2", "+libext+.v", "+define+W=4"],
            ["leaf sees W=4"; 3],
        ),
        (
            &["top_w7.sv", "-v", "leaf.v", "+define+W=3"],
            ["leaf sees W=3", "leaf sees W=7", "leaf sees W=7"],
        ),
        // A library file's macro never reaches a source file, even one
        // named after the `-v`.
        (
            &["top_sees.sv", "-v", "lone_mid.v"],
            ["top: W undefined"; 3],
        ),
        (
            &["-v", "lone_mid.v", "top_sees.sv"],
            ["top: W undefined"; 3],
        ),
    ];
    for (files, want) in cases {
        for (switches, want) in [
            (&[][..], want[0]),
            (&["--carry-src-macros"][..], want[1]),
            (&["--carry-lib-macros"][..], want[2]),
            // Both switches, in either order: `--carry-lib-macros`.
            (&["--carry-src-macros", "--carry-lib-macros"][..], want[2]),
            (&["--carry-lib-macros", "--carry-src-macros"][..], want[2]),
        ] {
            let args = [&["--no-cache", "-s", "top"][..], switches, files].concat();
            assert_eq!(xezim(&dir, &args), want, "{args:?}");
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The switches inside an args file, also one nested in another, and
/// beside `-v` files named in it.
#[test]
fn carry_switch_in_an_args_file() {
    let dir = scratch("libmac_args");
    write(&dir, "leaf.v", LEAF);
    write(
        &dir,
        "mid.v",
        "`define W 5\nmodule mid;\n  leaf l();\nendmodule\n",
    );
    write(&dir, "top.sv", TOP);
    write(&dir, "top_w7.sv", &format!("`define W 7\n{TOP_LEAF}"));
    write(&dir, "lib.f", "--carry-lib-macros\n-v mid.v\n-v leaf.v\n");
    write(&dir, "outer.f", "-f lib.f\ntop.sv\n");
    write(&dir, "src.f", "--carry-src-macros -v leaf.v\n");
    write(&dir, "plain.f", "-v leaf.v\n");
    for (args, want) in [
        (&["-f", "lib.f", "top.sv"][..], "leaf sees W=5"),
        (&["-f", "outer.f"][..], "leaf sees W=5"),
        (&["-f", "src.f", "top_w7.sv"][..], "leaf sees W=7"),
        (&["-f", "plain.f", "top_w7.sv"][..], "leaf: W undefined"),
        (
            &["-f", "plain.f", "top_w7.sv", "--carry-src-macros"][..],
            "leaf sees W=7",
        ),
    ] {
        let args = [&["--no-cache", "-s", "top"][..], args].concat();
        assert_eq!(xezim(&dir, &args), want, "{args:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A design the cache stored under one switch is not served to a run under
/// another, and a compiled artifact keeps the switch it was compiled with.
#[test]
fn carry_switch_keys_the_design_cache_and_artifacts() {
    let dir = scratch("libmac_cache");
    write(&dir, "leaf.v", LEAF);
    write(
        &dir,
        "mid.v",
        "`define W 5\nmodule mid;\n  leaf l();\nendmodule\n",
    );
    write(&dir, "top.sv", TOP);
    write(&dir, "top_w7.sv", &format!("`define W 7\n{TOP_LEAF}"));
    let lib: &[&str] = &["top.sv", "-v", "mid.v", "-v", "leaf.v", "-s", "top"];
    let src: &[&str] = &["top_w7.sv", "-v", "leaf.v", "-s", "top"];
    // Each run twice: the second one from the cache the first one stored.
    for (switch, files, want) in [
        (None, lib, "leaf: W undefined"),
        (Some("--carry-lib-macros"), lib, "leaf sees W=5"),
        (None, lib, "leaf: W undefined"),
        (Some("--carry-src-macros"), lib, "leaf: W undefined"),
        (None, src, "leaf: W undefined"),
        (Some("--carry-src-macros"), src, "leaf sees W=7"),
        (Some("--carry-lib-macros"), src, "leaf sees W=7"),
        (None, src, "leaf: W undefined"),
    ] {
        let args = [&["--cache-dir", "cache"][..], switch.as_slice(), files].concat();
        assert_eq!(xezim(&dir, &args), want, "{args:?}");
        assert_eq!(xezim(&dir, &args), want, "{args:?}, from the cache");
    }
    for (switch, want) in [
        (None, "leaf: W undefined"),
        (Some("--carry-lib-macros"), "leaf sees W=5"),
    ] {
        let args = [
            &["--compile", "--no-cache"][..],
            switch.as_slice(),
            lib,
            &["-o", "d.xez"],
        ]
        .concat();
        xezim(&dir, &args);
        assert_eq!(xezim(&dir, &["d.xez"]), want, "{args:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
