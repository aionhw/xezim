//! §22.5.1, §3.12.1: with `--carry-lib-macros`, a text macro defined in one
//! `-y`/`-v` library file, directly or through an `include`, is visible in
//! the library files read after it, in the order they are resolved, so a
//! library module that includes the design's headers shares them with the
//! library modules it instantiates. Without the switch each library file
//! starts from the command-line defines only (library_file_macro_carry.rs).

use std::path::Path;
use std::process::Command;

const MID: &str = "`include \"defs.vh\"\nmodule mid;\n  LEAF l();\nendmodule\n";
const LEAF: &str = "module LEAF;\n  initial $display(\"W=%0d\", `W);\nendmodule\n";

fn write(dir: &Path, name: &str, text: &str) {
    let p = dir.join(name);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

/// Run `top2.v` with the library arguments `lib` in `dir`.
fn run(dir: &Path, lib: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .current_dir(dir)
        .args(["--no-cache", "--carry-lib-macros", "top2.v"])
        .args(lib)
        .args(["+libext+.v", "+incdir+.", "-s", "top"])
        .output()
        .expect("run xezim");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{lib:?}:\n{text}");
    text
}

#[test]
fn macro_from_one_library_file_reaches_the_next() {
    let dir = std::env::temp_dir().join(format!("xezim_libmacro_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    write(&dir, "defs.vh", "`define W 5\n");
    write(
        &dir,
        "top2.v",
        "module top;\n  mid u();\n  initial #1 $finish;\nendmodule\n",
    );
    // The leaf's file sorts before (`aleaf`) and after (`zleaf`) the file
    // that includes the header, and lives in another `-y` directory.
    write(&dir, "lib2/mid.v", &MID.replace("LEAF", "aleaf"));
    write(&dir, "lib2/aleaf.v", &LEAF.replace("LEAF", "aleaf"));
    write(&dir, "lib3/mid.v", &MID.replace("LEAF", "zleaf"));
    write(&dir, "lib3/zleaf.v", &LEAF.replace("LEAF", "zleaf"));
    write(&dir, "lib4a/aleaf.v", &LEAF.replace("LEAF", "aleaf"));
    write(&dir, "lib4b/mid.v", &MID.replace("LEAF", "aleaf"));
    for lib in [
        &["-y", "lib2"][..],
        &["-y", "lib3"],
        &["-y", "lib4a", "-y", "lib4b"],
        &["-v", "lib2/mid.v", "-v", "lib2/aleaf.v"],
    ] {
        let text = run(&dir, lib);
        assert!(text.contains("W=5"), "{lib:?}:\n{text}");
        assert!(!text.contains("undefined"), "{lib:?}:\n{text}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Run xezim in `dir` with `args`; the run must succeed.
fn xezim(dir: &Path, args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .current_dir(dir)
        .arg("--no-cache")
        .args(args)
        .output()
        .expect("run xezim");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{args:?}:\n{text}");
    text
}

/// With `--relax-library-packages`, a `-v` file's packages are compiled in a
/// pass of their own, which reads the files as the module pass does: under
/// `--carry-lib-macros` the module a macro of an earlier library file selects
/// imports the package compiled beside it, and by default both passes take
/// the other branch. And `--dump-merged-sv` keeps a library file named
/// twice behind an include guard: its text is that of the read its module
/// came from, not of the second read, which the guard leaves empty.
#[test]
fn library_file_macros_reach_packages_and_the_merged_dump() {
    let dir = std::env::temp_dir().join(format!("xezim_libmacro_pkg_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    write(&dir, "feature.v", "`define FEATURE\nmodule unused_cell; endmodule\n");
    write(
        &dir,
        "leaf.v",
        "`ifdef FEATURE\n\
         package pk_on; localparam int V = 7; endpackage\n\
         module leaf; import pk_on::*; initial $display(\"V=%0d\", V); endmodule\n\
         `else\n\
         package pk_off; localparam int V = 3; endpackage\n\
         module leaf; import pk_off::*; initial $display(\"V=%0d\", V); endmodule\n\
         `endif\n",
    );
    write(&dir, "top3.v", "module top;\n  leaf u();\n  initial #1 $finish;\nendmodule\n");
    let args = ["--relax-library-packages", "top3.v", "-v", "feature.v", "-v", "leaf.v", "-s", "top"];
    let text = xezim(&dir, &[&["--carry-lib-macros"][..], &args].concat());
    assert!(text.contains("V=7"), "{text}");
    let text = xezim(&dir, &args);
    assert!(text.contains("V=3"), "{text}");

    write(
        &dir,
        "guard.v",
        "`ifndef GUARD_V\n`define GUARD_V\nmodule gmid;\n  initial $display(\"GMID\");\nendmodule\n`endif\n",
    );
    write(&dir, "top4.v", "module top;\n  gmid u();\n  initial #1 $finish;\nendmodule\n");
    let args = ["top4.v", "-v", "guard.v", "-v", "guard.v", "-s", "top", "--dump-merged-sv", "merged.sv"];
    let text = xezim(&dir, &args);
    assert!(text.contains("GMID"), "{text}");
    let merged = std::fs::read_to_string(dir.join("merged.sv")).expect("merged.sv");
    assert!(merged.contains("module gmid"), "{merged}");
    let text = xezim(&dir, &["merged.sv", "-s", "top"]);
    assert!(text.contains("GMID"), "the merged file: {text}");
    let _ = std::fs::remove_dir_all(&dir);
}
