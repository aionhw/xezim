//! `-uvm` with `XEZIM_UVM_DIR` (src/uvm_dir.rs): the UVM library is added to
//! the design only when `-uvm` is given, in every mode. The path logic is
//! tested directly by including the binary's module source; the command line
//! is tested on a small stand-in UVM tree and once on the real library.

use std::path::{Path, PathBuf};
use std::process::Command;

#[allow(dead_code)]
#[path = "../../src/uvm_dir.rs"]
mod uvm_dir;

use uvm_dir::{UvmDirAction, apply_uvm_dir, resolve_uvm_src};

fn xezim() -> String {
    // Cargo names the binary it built for the integration tests.
    env!("CARGO_BIN_EXE_xezim").to_string()
}

/// A fresh scratch directory per test (tests run in parallel).
fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("xezim_uvm_dir_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write(p: &Path, text: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

/// A stand-in UVM `src` directory at `src`: a `uvm_pkg` with one function
/// and a `uvm_macros.svh` with one macro, enough to tell that both the
/// package and the include directory were added.
fn mini_uvm_src(src: &Path, tag: &str) {
    write(
        &src.join("uvm_pkg.sv"),
        &format!(
            "package uvm_pkg;\n  function automatic string uvm_hello();\n    return \"{}\";\n  endfunction\nendpackage\n",
            tag
        ),
    );
    write(&src.join("uvm_macros.svh"), "`define UVM_MINI_MACRO 7\n");
}

const TB: &str = r#"`include "uvm_macros.svh"
module tb;
  import uvm_pkg::*;
  initial $display("T|%s|%0d", uvm_hello(), `UVM_MINI_MACRO);
endmodule
"#;

/// Run xezim in `dir` with `XEZIM_UVM_DIR` = `uvm_dir` (unset when None)
/// and no `XEZIM_UVM_VERSION`; returns (exit code, stdout, stderr).
fn run(dir: &Path, uvm_dir: Option<&Path>, args: &[&str]) -> (i32, String, String) {
    let mut c = Command::new(xezim());
    c.current_dir(dir)
        .args(args)
        .env_remove("XEZIM_UVM_VERSION");
    match uvm_dir {
        Some(u) => c.env("XEZIM_UVM_DIR", u),
        None => c.env_remove("XEZIM_UVM_DIR"),
    };
    let out = c.output().expect("run xezim");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn resolver_accepts_src_release_root_and_multi_release_checkout() {
    let d = scratch("resolve");
    // A `src` directory and a release root.
    mini_uvm_src(&d.join("rel/src"), "rel");
    assert_eq!(
        resolve_uvm_src(&d.join("rel/src"), None).unwrap(),
        d.join("rel/src")
    );
    assert_eq!(
        resolve_uvm_src(&d.join("rel"), None).unwrap(),
        d.join("rel/src")
    );
    // A multi-release checkout: newest present by default, or the one asked.
    mini_uvm_src(&d.join("multi/1.2/src"), "1.2");
    mini_uvm_src(&d.join("multi/1800.2-2017/src"), "2017");
    assert_eq!(
        resolve_uvm_src(&d.join("multi"), None).unwrap(),
        d.join("multi/1800.2-2017/src")
    );
    assert_eq!(
        resolve_uvm_src(&d.join("multi"), Some("1.2")).unwrap(),
        d.join("multi/1.2/src")
    );
    // Errors name where they looked.
    let e = resolve_uvm_src(&d.join("multi"), Some("1.1d")).unwrap_err();
    assert!(e.contains("1.1d"), "{}", e);
    let e = resolve_uvm_src(&d.join("nowhere"), None).unwrap_err();
    assert!(e.contains("no uvm_pkg.sv"), "{}", e);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn apply_adds_only_with_the_flag() {
    let d = scratch("apply");
    mini_uvm_src(&d.join("u/src"), "u");
    let u = d.join("u");
    let us = u.to_str().unwrap();
    let base = || (vec!["tb.sv".to_string()], vec!["inc".to_string()]);

    // No -uvm: nothing, even with the variable set.
    let (mut f, mut i) = base();
    assert_eq!(
        apply_uvm_dir(false, Some(us), None, &mut f, &mut i),
        UvmDirAction::NotRequested
    );
    assert_eq!((f, i), base());

    // -uvm: the package first, the include directory after the user's.
    let (mut f, mut i) = base();
    let src = u.join("src");
    assert_eq!(
        apply_uvm_dir(true, Some(us), None, &mut f, &mut i),
        UvmDirAction::Added(src.clone())
    );
    assert_eq!(
        f,
        vec![
            src.join("uvm_pkg.sv").to_string_lossy().into_owned(),
            "tb.sv".to_string()
        ]
    );
    assert_eq!(
        i,
        vec!["inc".to_string(), src.to_string_lossy().into_owned()]
    );

    // -uvm with uvm_pkg.sv already in the list: left alone.
    let mut f = vec!["/x/uvm_pkg.sv".to_string(), "tb.sv".to_string()];
    let mut i = Vec::new();
    assert_eq!(
        apply_uvm_dir(true, Some(us), None, &mut f, &mut i),
        UvmDirAction::AlreadySupplied
    );
    assert_eq!(f.len(), 2);
    assert!(i.is_empty());

    // -uvm without a usable variable: an error that says why.
    for v in [None, Some("")] {
        let (mut f, mut i) = base();
        match apply_uvm_dir(true, v, None, &mut f, &mut i) {
            UvmDirAction::Error(e) => assert!(e.contains("XEZIM_UVM_DIR"), "{}", e),
            other => panic!("expected an error, got {:?}", other),
        }
    }
    let (mut f, mut i) = base();
    match apply_uvm_dir(
        true,
        Some(d.join("nowhere").to_str().unwrap()),
        None,
        &mut f,
        &mut i,
    ) {
        UvmDirAction::Error(e) => assert!(e.contains("no uvm_pkg.sv"), "{}", e),
        other => panic!("expected an error, got {:?}", other),
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn uvm_flag_adds_the_library_for_compile_and_simulation() {
    let d = scratch("cli");
    mini_uvm_src(&d.join("uvm/src"), "mini");
    write(&d.join("tb.sv"), TB);
    let u = d.join("uvm");

    // Simulation, with -uvm on the command line and with --uvm.
    for flag in ["-uvm", "--uvm"] {
        let (rc, out, err) = run(&d, Some(&u), &[flag, "tb.sv"]);
        assert_eq!(rc, 0, "{}: stdout:\n{}\nstderr:\n{}", flag, out, err);
        assert!(
            out.contains("T|mini|7"),
            "{}: stdout:\n{}\nstderr:\n{}",
            flag,
            out,
            err
        );
    }
    // Compile only.
    let (rc, out, err) = run(&d, Some(&u), &["--compile", "-uvm", "tb.sv"]);
    assert_eq!(rc, 0, "stdout:\n{}\nstderr:\n{}", out, err);
    // -uvm inside an args file.
    write(&d.join("run.f"), "-uvm\ntb.sv\n");
    let (rc, out, err) = run(&d, Some(&u), &["-f", "run.f"]);
    assert_eq!(rc, 0, "stdout:\n{}\nstderr:\n{}", out, err);
    assert!(
        out.contains("T|mini|7"),
        "stdout:\n{}\nstderr:\n{}",
        out,
        err
    );
    // --dump-files-list shows the added package and include directory.
    let (_, out, _) = run(
        &d,
        Some(&u),
        &["--compile", "--dump-files-list", "-uvm", "tb.sv"],
    );
    let src = u.join("src");
    assert!(
        out.contains(&src.join("uvm_pkg.sv").to_string_lossy().into_owned()),
        "{}",
        out
    );
    assert!(out.contains("include dir(s)"), "{}", out);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn without_the_flag_or_the_variable_nothing_is_added() {
    let d = scratch("noflag");
    mini_uvm_src(&d.join("uvm/src"), "mini");
    write(&d.join("tb.sv"), TB);
    let u = d.join("uvm");

    // The variable alone does nothing: the include cannot be found.
    let (rc, out, err) = run(&d, Some(&u), &["tb.sv"]);
    assert_ne!(rc, 0, "stdout:\n{}\nstderr:\n{}", out, err);
    assert!(!out.contains("T|mini"), "stdout:\n{}", out);

    // -uvm without the variable is an error that names it.
    let (rc, _, err) = run(&d, None, &["-uvm", "tb.sv"]);
    assert_eq!(rc, 1, "stderr:\n{}", err);
    assert!(err.contains("-uvm needs XEZIM_UVM_DIR"), "stderr:\n{}", err);

    // -uvm with a directory that holds no UVM.
    let (rc, _, err) = run(&d, Some(&d.join("nowhere")), &["-uvm", "tb.sv"]);
    assert_eq!(rc, 1, "stderr:\n{}", err);
    assert!(err.contains("no uvm_pkg.sv"), "stderr:\n{}", err);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn uvm_flag_respects_a_user_supplied_package() {
    let d = scratch("own");
    mini_uvm_src(&d.join("uvm/src"), "from-env");
    mini_uvm_src(&d.join("own"), "own");
    write(&d.join("tb.sv"), TB);
    let (rc, out, err) = run(
        &d,
        Some(&d.join("uvm")),
        &["-uvm", "-I", "own", "own/uvm_pkg.sv", "tb.sv"],
    );
    assert_eq!(rc, 0, "stdout:\n{}\nstderr:\n{}", out, err);
    assert!(
        out.contains("T|own|7"),
        "stdout:\n{}\nstderr:\n{}",
        out,
        err
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// The real UVM checkout, found as the UVM integration tests find it.
fn real_uvm_checkout() -> PathBuf {
    if let Ok(d) = std::env::var("XEZIM_UVM_DIR") {
        return PathBuf::from(d);
    }
    let sibling = Path::new("../UVM");
    if sibling.join("1.2/src/uvm_pkg.sv").exists() {
        return sibling.canonicalize().expect("canonicalize ../UVM");
    }
    let target = Path::new("target/uvm-checkout");
    if !target.join("1.2/src/uvm_pkg.sv").exists() {
        let ok = Command::new("git")
            .args(["clone", "--depth", "1", "https://github.com/nitronis/UVM"])
            .arg(target)
            .status()
            .is_ok_and(|s| s.success());
        assert!(
            ok,
            "cloning https://github.com/nitronis/UVM failed (set XEZIM_UVM_DIR)"
        );
    }
    target
        .canonicalize()
        .expect("canonicalize target/uvm-checkout")
}

#[test]
fn uvm_flag_runs_a_real_uvm_test() {
    let d = scratch("real");
    write(
        &d.join("tb.sv"),
        r#"`include "uvm_macros.svh"
import uvm_pkg::*;
class hello_test extends uvm_test;
  `uvm_component_utils(hello_test)
  function new(string name, uvm_component parent);
    super.new(name, parent);
  endfunction
  task run_phase(uvm_phase phase);
    phase.raise_objection(this);
    #10 `uvm_info("HELLO", "hello from -uvm", UVM_LOW)
    phase.drop_objection(this);
  endtask
endclass
module tb;
  initial run_test();
endmodule
"#,
    );
    let mut c = Command::new(xezim());
    c.current_dir(&d)
        .args([
            "-uvm",
            "-D",
            "UVM_NO_DPI",
            "tb.sv",
            "+UVM_TESTNAME=hello_test",
        ])
        .env("XEZIM_UVM_DIR", real_uvm_checkout())
        .env("XEZIM_UVM_VERSION", "1800.2-2017");
    let out = c.output().expect("run xezim");
    let (so, se) = (
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "stdout:\n{}\nstderr:\n{}",
        so,
        se
    );
    assert!(
        so.contains("hello from -uvm"),
        "stdout:\n{}\nstderr:\n{}",
        so,
        se
    );
    assert!(so.contains("UVM_ERROR :    0"), "stdout:\n{}", so);
    let _ = std::fs::remove_dir_all(&d);
}
