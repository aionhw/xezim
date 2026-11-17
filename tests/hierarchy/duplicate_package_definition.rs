//! §3.13, §26.2: the package name space holds one declaration per name across
//! all compilation units, so a second `package P` in the same file or in
//! another file is an error (both in simulation and `--compile`), while
//! distinct name spaces (module, interface, class, typedef), distinct inner
//! names across packages, and `` `ifndef ``-guarded includes stay legal.

use std::path::PathBuf;
use std::process::Command;
use xezim::simulate;

fn error_of(src: &str) -> String {
    match simulate(src, 10) {
        Ok(sim) => panic!("accepted; output {:?}", sim.output),
        Err(e) => e,
    }
}

fn simulate_files(sources: &[&str], paths: &[&str]) -> Result<xezim::compiler::Simulator, String> {
    let sources: Vec<String> = sources.iter().map(|s| s.to_string()).collect();
    let paths: Vec<String> = paths.iter().map(|s| s.to_string()).collect();
    xezim::simulate_multi(
        &sources,
        10,
        Some("tb"),
        &[],
        &paths,
        None,
        false,
        None,
        None,
        &[],
        &[],
        None,
        &[],
        0,
        u64::MAX,
        None,
        &[],
        None,
        None,
        None,
        None,
        false,
    )
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("xezim_dup_pkg_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    dir
}

#[test]
fn duplicate_package_in_one_file() {
    let e = error_of(
        r#"
package p; localparam int K = 1; function int f(); return 10; endfunction endpackage
package p; localparam int K = 2; function int g(); return 20; endfunction endpackage
module tb; initial $display("f=%0d", p::f()); endmodule
"#,
    );
    assert!(e.contains("duplicate definition of package 'p'"), "{e}");
    assert!(e.contains(":3:9: error:"), "{e}");
    assert!(
        e.contains("already defined at") && e.contains(":2:9"),
        "{e}"
    );
    assert!(e.contains("§3.13, §26.2"), "{e}");
}

#[test]
fn duplicate_package_across_files_and_compile_mode() {
    let e = match simulate_files(
        &[
            "package p; localparam int K = 1; endpackage\n",
            "package p; localparam int K = 2; endpackage\nmodule tb; initial $display(\"K=%0d\", p::K); endmodule\n",
        ],
        &["first.sv", "second.sv"],
    ) {
        Ok(sim) => panic!("accepted; output {:?}", sim.output),
        Err(e) => e,
    };
    assert!(
        e.contains("second.sv:1:9: error: duplicate definition of package 'p'"),
        "{e}"
    );
    assert!(e.contains("already defined at first.sv:1:9"), "{e}");

    let dir = scratch("compile");
    let f1 = dir.join("first.sv");
    let f2 = dir.join("second.sv");
    std::fs::write(&f1, "package p; localparam int K = 1; endpackage\n").unwrap();
    std::fs::write(
        &f2,
        "package p; localparam int K = 2; endpackage\nmodule tb; initial $display(\"K=%0d\", p::K); endmodule\n",
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--compile", "--no-cache"])
        .arg(&f1)
        .arg(&f2)
        .output()
        .expect("run xezim --compile");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        !out.status.success(),
        "--compile accepted duplicate packages"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("duplicate definition of package 'p'"),
        "{stderr}"
    );
}

#[test]
fn package_with_same_named_other_kinds_and_inner_names_is_legal() {
    let src = r#"
package pm; localparam int K = 1; endpackage
module pm; endmodule
package pi; localparam int K = 2; endpackage
interface pi; logic a; endinterface
package pc; localparam int K = 3; endpackage
class pc; int v; endclass
package pt; localparam int K = 4; endpackage
typedef int pt;
package p1; localparam int K = 10; class c; int v = 1; endclass endpackage
package p2; localparam int K = 20; class c; int v = 2; endclass endpackage
module tb;
  pm um();
  pi ui();
  pc oc = new;
  pt vt = 5;
  initial begin
    p1::c a = new;
    p2::c b = new;
    $display("R|%0d %0d %0d %0d %0d", p1::K, p2::K, a.v, b.v, vt);
  end
endmodule
"#;
    let sim = simulate_files(&[src], &[]).expect("separate name spaces must pass");
    assert!(
        sim.output.iter().any(|o| o.message == "R|10 20 1 2 5"),
        "{:?}",
        sim.output
    );
}

#[test]
fn ifndef_guarded_package_include_from_two_files_is_legal() {
    let dir = scratch("guarded");
    let hdr = dir.join("pkg.svh");
    let fa = dir.join("a.sv");
    let fb = dir.join("b.sv");
    std::fs::write(
        &hdr,
        "`ifndef PKG_SVH\n`define PKG_SVH\npackage gp; localparam int K = 9; endpackage\n`endif\n",
    )
    .unwrap();
    std::fs::write(
        &fa,
        "`include \"pkg.svh\"\nmodule a; initial $display(\"a=%0d\", gp::K); endmodule\n",
    )
    .unwrap();
    std::fs::write(
        &fb,
        "`include \"pkg.svh\"\nmodule tb; a ua(); initial $display(\"b=%0d\", gp::K); endmodule\n",
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--simulate", "--no-cache", "-I"])
        .arg(&dir)
        .arg(&fa)
        .arg(&fb)
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "rejected:\n{stdout}\n{stderr}");
    assert!(stdout.contains("a=9") && stdout.contains("b=9"), "{stdout}");
}
