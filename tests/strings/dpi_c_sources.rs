//! C/C++ DPI sources named among the design files (IEEE 1800-2023
//! §35.5). A file list that names `add.c` next to `t.sv` used to have the C
//! file parsed as SystemVerilog. Such a source is now compiled with the host
//! C/C++ compiler, with xezim's `svdpi.h` on the include path, into a cached
//! shared library loaded like `--dpi-lib`; `.o`/`.a` files are linked into
//! it and a `.so` is loaded as it is.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("xezim_dpi_csrc_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("cache")).unwrap();
    d
}

fn write(dir: &Path, name: &str, text: &str) {
    std::fs::write(dir.join(name), text).unwrap();
}

/// Run xezim in `dir` with its DPI cache under `dir/cache`.
fn run(dir: &Path, args: &[&str]) -> (Output, String) {
    run_with_cache(dir, &dir.join("cache"), args)
}

/// Run xezim in `dir` with its DPI cache under `cache`.
fn run_with_cache(dir: &Path, cache: &Path, args: &[&str]) -> (Output, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--no-cache"])
        .args(args)
        .current_dir(dir)
        .env("XEZIM_CACHE_DIR", cache)
        .output()
        .expect("failed to run xezim");
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out, text)
}

/// The libraries built into the scratch cache.
fn built_libraries(dir: &Path) -> usize {
    std::fs::read_dir(dir.join("cache").join("xezim").join("dpi"))
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.file_name().to_string_lossy().ends_with(".so"))
                .count()
        })
        .unwrap_or(0)
}

fn cc() -> String {
    std::env::var("CC").unwrap_or_else(|_| "clang".to_string())
}

/// Build `dir/libmodel.a` holding `int model(int a, int b) { return <body>; }`.
fn model_archive(dir: &Path, body: &str) {
    write(dir, "model.c", &format!("int model(int a, int b) {{ return {body}; }}\n"));
    let ok = |c: &mut Command| c.current_dir(dir).status().is_ok_and(|s| s.success());
    assert!(ok(Command::new(cc()).args(["-fPIC", "-c", "model.c", "-o", "model.o"])));
    let _ = std::fs::remove_file(dir.join("libmodel.a"));
    let ar = std::env::var("AR").unwrap_or_else(|_| "ar".to_string());
    assert!(ok(Command::new(ar).args(["rcs", "libmodel.a", "model.o"])));
}

/// A C source that calls `model` from an archive named in `-LDFLAGS`.
const MODEL_ADD_C: &str =
    "int model(int a, int b);\nint c_add(int a, int b) { return model(a, b); }\n";

const ADD_SV: &str = r#"
module t;
  import "DPI-C" function int c_add(input int a, input int b);
  initial begin $display("sum=%0d", c_add(2, 3)); $finish; end
endmodule
"#;

/// The issue's case: `add.c` before `t.sv` in a `-f` list.
#[test]
fn c_source_in_a_file_list_is_compiled_and_loaded() {
    let d = scratch("flist");
    write(&d, "add.c", "int c_add(int a, int b) { return a + b; }\n");
    write(&d, "t.sv", ADD_SV);
    write(&d, "files.f", "add.c\nt.sv\n");
    let (out, text) = run(&d, &["-f", "files.f", "-s", "t"]);
    assert!(out.status.success(), "{text}");
    assert!(text.contains("sum=5"), "{text}");
    assert!(!text.contains("Parse error"), "{text}");
    assert_eq!(built_libraries(&d), 1, "{text}");
    let _ = std::fs::remove_dir_all(&d);
}

/// A C++ source on the command line: compiled and linked as C++ (it uses the
/// standard library), with `svdpi.h` found without an `-I`, `-CFLAGS`
/// reaching the compiler, and a call back into an exported SV function.
#[test]
fn cxx_source_with_svdpi_cflags_and_an_export() {
    let d = scratch("cxx");
    write(
        &d,
        "model.cpp",
        r#"#include "svdpi.h"
#include <vector>
extern "C" int sv_twice(int a);
extern "C" int c_scale(int a) {
  std::vector<int> v{a, SCALE};
  return v[0] * v[1];
}
extern "C" svBit c_odd(int a) { return (svBit)(sv_twice(a) / 2 & 1); }
"#,
    );
    write(
        &d,
        "t.sv",
        r#"
module t;
  import "DPI-C" function int c_scale(input int a);
  import "DPI-C" function bit c_odd(input int a);
  export "DPI-C" function sv_twice;
  function int sv_twice(input int a); return 2 * a; endfunction
  initial begin $display("scaled=%0d odd=%0d", c_scale(7), c_odd(7)); $finish; end
endmodule
"#,
    );
    let (out, text) = run(&d, &["-CFLAGS", "-DSCALE=3 -Wall", "model.cpp", "t.sv"]);
    assert!(out.status.success(), "{text}");
    assert!(text.contains("scaled=21 odd=1"), "{text}");
    let _ = std::fs::remove_dir_all(&d);
}

/// The library is reused while the sources and the headers they include are
/// unchanged, and rebuilt when either changes.
#[test]
fn library_is_cached_by_preprocessed_source() {
    let d = scratch("cache");
    write(&d, "k.h", "#define K 0\n");
    write(&d, "add.c", "#include \"k.h\"\nint c_add(int a, int b) { return a + b + K; }\n");
    write(&d, "t.sv", ADD_SV);
    let (_, text) = run(&d, &["add.c", "t.sv"]);
    assert!(text.contains("sum=5"), "{text}");
    let (_, text) = run(&d, &["add.c", "t.sv"]);
    assert!(text.contains("sum=5"), "{text}");
    assert_eq!(built_libraries(&d), 1, "an unchanged build was not reused");
    write(&d, "k.h", "#define K 100\n");
    let (_, text) = run(&d, &["add.c", "t.sv"]);
    assert!(text.contains("sum=105"), "an edited header was not rebuilt: {text}");
    assert_eq!(built_libraries(&d), 2);
    let _ = std::fs::remove_dir_all(&d);
}

/// An object file and a static archive are linked into the library, and a
/// shared library in the list is loaded as `--dpi-lib` is.
#[test]
fn objects_archives_and_shared_libraries_in_a_file_list() {
    let d = scratch("objs");
    write(&d, "one.c", "int c_one(void) { return 1; }\n");
    write(&d, "two.c", "int c_two(void) { return 2; }\n");
    write(&d, "four.c", "int c_four(void) { return 4; }\n");
    write(&d, "eight.c", "int c_eight(void) { return 8; }\n");
    let ok = |c: &mut Command| c.current_dir(&d).status().is_ok_and(|s| s.success());
    assert!(ok(Command::new(cc()).args(["-fPIC", "-c", "two.c", "-o", "two.o"])));
    assert!(ok(Command::new(cc()).args(["-fPIC", "-c", "four.c", "-o", "four.o"])));
    let ar = std::env::var("AR").unwrap_or_else(|_| "ar".to_string());
    assert!(ok(Command::new(ar).args(["rcs", "libfour.a", "four.o"])));
    assert!(ok(Command::new(cc()).args(["-shared", "-fPIC", "eight.c", "-o", "libeight.so"])));
    write(
        &d,
        "t.sv",
        r#"
module t;
  import "DPI-C" function int c_one();
  import "DPI-C" function int c_two();
  import "DPI-C" function int c_four();
  import "DPI-C" function int c_eight();
  initial begin $display("total=%0d", c_one() + c_two() + c_four() + c_eight()); $finish; end
endmodule
"#,
    );
    write(&d, "files.f", "one.c\ntwo.o\nlibfour.a\nlibeight.so\nt.sv\n");
    let (out, text) = run(&d, &["--error-exit", "-f", "files.f"]);
    assert!(out.status.success(), "{text}");
    assert!(text.contains("total=15"), "{text}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn precompiled_cxx_object_links_its_runtime() {
    let dir = scratch("cxx_object");
    write(&dir, "model.cpp", r#"
#include <string>
extern "C" int c_add(int a, int b) {
    return std::stoi(std::to_string(a + b));
}
"#);
    let cxx = std::env::var("CXX").unwrap_or_else(|_| "c++".into());
    assert!(Command::new(cxx)
        .current_dir(&dir)
        .args(["-fPIC", "-c", "model.cpp", "-o", "model.o"])
        .status()
        .unwrap()
        .success());
    write(&dir, "t.sv", ADD_SV);
    let (out, text) = run(&dir, &["--error-exit", "model.o", "t.sv"]);
    assert!(out.status.success(), "{text}");
    assert!(text.lines().any(|line| line == "sum=5"), "{text}");
    std::fs::remove_dir_all(dir).unwrap();
}

/// A library named in `-LDFLAGS` is not in the cache key, so the library is
/// linked again on every run: an edit to that archive reaches the next run,
/// in a library of its own. The C source's object is compiled once.
#[test]
fn ldflags_archive_edit_reaches_the_next_run() {
    let d = scratch("ldflags");
    write(&d, "add.c", MODEL_ADD_C);
    write(&d, "t.sv", ADD_SV);
    let archive = |body: &str| model_archive(&d, body);
    let objects = || -> Vec<(PathBuf, std::time::SystemTime)> {
        let rd = std::fs::read_dir(d.join("cache").join("xezim").join("dpi")).unwrap();
        rd.flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("obj-"))
            .map(|e| (e.path(), e.metadata().unwrap().modified().unwrap()))
            .collect()
    };
    archive("a + b");
    let (out, text) = run(&d, &["-LDFLAGS", "./libmodel.a", "add.c", "t.sv"]);
    assert!(out.status.success(), "{text}");
    assert!(text.contains("sum=5"), "{text}");
    let first = objects();
    archive("a + b + 100");
    let (out, text) = run(&d, &["-LDFLAGS", "./libmodel.a", "add.c", "t.sv"]);
    assert!(out.status.success(), "{text}");
    assert!(text.contains("sum=105"), "the edited archive was not linked: {text}");
    assert_eq!(built_libraries(&d), 2, "{text}");
    assert_eq!(first.len(), 1);
    assert_eq!(objects(), first, "the unchanged source was compiled again");
    let _ = std::fs::remove_dir_all(&d);
}

/// Two directories with the same C source and `-LDFLAGS` text but their own
/// `./libmodel.a` have the same cache key. The second run must not replace
/// the library the first one loads, which a concurrent first run would then
/// load with the other model in it: each linked library has its own path,
/// and linking the same bytes again reuses it.
#[test]
fn ldflags_builds_of_one_key_keep_their_own_libraries() {
    let base = scratch("ldflags_dirs");
    let cache = base.join("cache");
    let (a, b) = (base.join("a"), base.join("b"));
    for (dir, body) in [(&a, "a + b"), (&b, "a + b + 100")] {
        std::fs::create_dir_all(dir).unwrap();
        write(dir, "add.c", MODEL_ADD_C);
        write(dir, "t.sv", ADD_SV);
        model_archive(dir, body);
    }
    let libraries = || -> Vec<(String, Vec<u8>)> {
        let rd = std::fs::read_dir(cache.join("xezim").join("dpi")).unwrap();
        let mut libs: Vec<(String, Vec<u8>)> = rd
            .flatten()
            .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
            .filter(|(name, _)| name.ends_with(".so"))
            .map(|(name, path)| (name, std::fs::read(path).unwrap()))
            .collect();
        libs.sort();
        libs
    };
    let args = ["-LDFLAGS", "./libmodel.a", "add.c", "t.sv"];
    let (out, text) = run_with_cache(&a, &cache, &args);
    assert!(out.status.success(), "{text}");
    assert!(text.contains("sum=5"), "{text}");
    let first = libraries();
    assert_eq!(first.len(), 1);
    let (out, text) = run_with_cache(&b, &cache, &args);
    assert!(out.status.success(), "{text}");
    assert!(text.contains("sum=105"), "{text}");
    let both = libraries();
    assert_eq!(both.len(), 2, "the second run replaced the first run's library");
    assert!(both.contains(&first[0]), "the first run's library changed");
    let (_, text) = run_with_cache(&a, &cache, &args);
    assert!(text.contains("sum=5"), "{text}");
    assert_eq!(libraries(), both, "the same link was not reused");
    let _ = std::fs::remove_dir_all(&base);
}

/// With no cache directory set (`$XEZIM_CACHE_DIR`, `$XDG_CACHE_HOME`,
/// `$HOME`), the library is built in `xezim-<uid>` in the temporary
/// directory, created private. One that other users can write to, where
/// anyone could have put headers or a library of their own, is refused.
#[cfg(unix)]
#[test]
fn temporary_cache_directory_must_be_private() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let d = scratch("tmpcache");
    write(&d, "add.c", "int c_add(int a, int b) { return a + b; }\n");
    write(&d, "t.sv", ADD_SV);
    let tmp = d.join("tmp");
    std::fs::create_dir(&tmp).unwrap();
    let uid = std::fs::metadata(&d).unwrap().uid();
    let private = tmp.join(format!("xezim-{uid}"));
    let run_in_tmp = || {
        let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
            .args(["--no-cache", "add.c", "t.sv"])
            .current_dir(&d)
            .env_remove("XEZIM_CACHE_DIR")
            .env_remove("XDG_CACHE_HOME")
            .env_remove("HOME")
            .env("TMPDIR", &tmp)
            .output()
            .expect("failed to run xezim");
        let mut text = String::from_utf8_lossy(&out.stdout).to_string();
        text.push_str(&String::from_utf8_lossy(&out.stderr));
        (out, text)
    };
    std::fs::create_dir(&private).unwrap();
    std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o777)).unwrap();
    let (out, text) = run_in_tmp();
    assert!(!out.status.success(), "{text}");
    assert!(text.contains("can be written by other users"), "{text}");
    assert!(!text.contains("sum="), "{text}");
    std::fs::remove_dir(&private).unwrap();
    let (out, text) = run_in_tmp();
    assert!(out.status.success(), "{text}");
    assert!(text.contains("sum=5"), "{text}");
    assert_eq!(std::fs::metadata(&private).unwrap().mode() & 0o777, 0o700);
    assert!(private.join("xezim").join("dpi").is_dir(), "{text}");
    let _ = std::fs::remove_dir_all(&d);
}

/// A source that does not compile stops the run before it starts.
#[test]
fn compile_error_stops_the_run() {
    let d = scratch("error");
    write(&d, "add.c", "int c_add(int a, int b) { return a + ; }\n");
    write(&d, "t.sv", ADD_SV);
    let (out, text) = run(&d, &["add.c", "t.sv"]);
    assert!(!out.status.success(), "{text}");
    assert!(text.contains("DPI source 'add.c'"), "{text}");
    assert!(!text.contains("sum="), "{text}");
    let _ = std::fs::remove_dir_all(&d);
}

/// `--dump-files-list` shows the DPI files apart from the HDL sources.
#[test]
fn files_list_shows_dpi_sources() {
    let d = scratch("list");
    write(&d, "add.c", "int c_add(int a, int b) { return a + b; }\n");
    write(&d, "t.sv", ADD_SV);
    write(&d, "files.f", "add.c\nt.sv\n");
    let (_, text) = run(&d, &["--dump-files-list", "-f", "files.f"]);
    assert!(text.contains("1 source file(s)"), "{text}");
    assert!(text.contains("--- DPI C/C++ file(s): 1 ---"), "{text}");
    let _ = std::fs::remove_dir_all(&d);
}
