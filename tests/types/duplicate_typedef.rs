//! §3.13, §6.18: a scope's type-name space holds one typedef or class per
//! name. Duplicate typedefs in the compilation unit, a module, a package, or
//! colliding with a class are errors; identical `$unit` typedefs in different
//! command-line files emit a warning and keep the first; forward typedefs,
//! separate scopes, shadowing, and `` `ifndef ``-guarded includes pass; and
//! `--dump-merged-sv` suppresses repeated `$unit` typedefs across adopted `-v`
//! libraries so the merged file recompiles standalone.

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
    let dir = std::env::temp_dir().join(format!("xezim_dup_td_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    dir
}

#[test]
fn duplicate_typedefs_in_same_scope_or_with_class_are_errors() {
    let e = error_of(
        "typedef logic [7:0] t;\ntypedef logic [15:0] t;\nmodule tb; t v; initial $display(\"%0d\", $bits(v)); endmodule\n",
    );
    assert!(
        e.contains("duplicate definition of typedef 't' in the compilation unit"),
        "{e}"
    );
    assert!(e.contains(":2:22: error:"), "{e}");

    // Identical text in the SAME command-line file is still an error.
    let e = error_of("typedef logic [7:0] t;\ntypedef logic [7:0] t;\nmodule tb; t v; endmodule\n");
    assert!(
        e.contains("duplicate definition of typedef 't' in the compilation unit"),
        "{e}"
    );

    let e = error_of(
        "module tb;\n  typedef logic [7:0] t;\n  typedef logic [15:0] t;\n  t v;\nendmodule\n",
    );
    assert!(
        e.contains("duplicate definition of typedef 't' in module 'tb'"),
        "{e}"
    );

    let e = error_of(
        "package p;\n  typedef logic [7:0] t;\n  typedef logic [15:0] t;\nendpackage\nmodule tb; p::t v; endmodule\n",
    );
    assert!(
        e.contains("duplicate definition of typedef 't' in package 'p'"),
        "{e}"
    );

    let e = error_of("typedef int c;\nclass c; int x; endclass\nmodule tb; c v; endmodule\n");
    assert!(
        e.contains("duplicate definition of class 'c' in the compilation unit"),
        "{e}"
    );

    let e = error_of("class c; int x; endclass\ntypedef int c;\nmodule tb; c v; endmodule\n");
    assert!(
        e.contains("duplicate definition of typedef 'c' in the compilation unit"),
        "{e}"
    );
}

#[test]
fn differing_cross_file_unit_typedefs_are_an_error() {
    let e = match simulate_files(
        &[
            "typedef logic [7:0] t;\nmodule a; t v; initial $display(\"a=%0d\", $bits(v)); endmodule\n",
            "typedef logic [15:0] t;\nmodule b; t v; initial $display(\"b=%0d\", $bits(v)); endmodule\nmodule tb; a ua(); b ub(); endmodule\n",
        ],
        &["td_a.sv", "td_b.sv"],
    ) {
        Ok(sim) => panic!("accepted; output {:?}", sim.output),
        Err(e) => e,
    };
    assert!(
        e.contains("td_b.sv:1:22: error: duplicate definition of typedef 't'"),
        "{e}"
    );
    assert!(e.contains("already defined at td_a.sv:1:21"), "{e}");
}

#[test]
fn identical_cross_file_unit_typedefs_warn_and_pass() {
    let dir = scratch("cross_warn");
    let fa = dir.join("same_a.sv");
    let fb = dir.join("same_b.sv");
    std::fs::write(
        &fa,
        "typedef enum {RED, GREEN} color_t;\ntypedef struct packed { logic [3:0] a; } s_t;\nmodule a; color_t c; s_t s; endmodule\n",
    )
    .unwrap();
    std::fs::write(
        &fb,
        "typedef enum { RED, GREEN } color_t;\ntypedef struct packed {\n  logic [3:0] a;\n} s_t;\nmodule tb; color_t c = GREEN; s_t s; a ua(); initial $display(\"c=%0d s=%0d\", c, $bits(s)); endmodule\n",
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--simulate", "--no-cache"])
        .arg(&fa)
        .arg(&fb)
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "rejected:\n{stdout}\n{stderr}");
    assert!(stdout.contains("c=1 s=4"), "{stdout}");
    assert!(
        stderr.contains("warning: duplicate definition of typedef 'color_t'"),
        "{stderr}"
    );
    assert!(
        stderr.contains("warning: duplicate definition of typedef 's_t'"),
        "{stderr}"
    );
}

#[test]
fn forward_typedefs_distinct_scopes_shadowing_and_guarded_includes_pass() {
    let src = r#"
typedef class c;
typedef class c;
typedef c;
class c; int x = 3; endclass
typedef struct s_t;
typedef struct { int a; } s_t;
typedef s_t;
typedef int tt;
package p1; typedef logic [3:0] tt; endpackage
package p2; typedef logic [5:0] tt; endpackage
module tb;
  import p1::*;
  typedef logic [8:0] tt;
  tt v;
  c o = new;
  s_t s;
  function automatic int f(); typedef byte tt; tt b; return $bits(b); endfunction
  if (1) begin : g1 typedef logic [1:0] tt; tt w; end
  else begin : g2 typedef logic [2:0] tt; tt w; end
  for (genvar i = 0; i < 2; i++) begin : g typedef logic [i:0] tt; tt w; end
  initial begin
    s.a = 4;
    $display("R|%0d %0d %0d %0d %0d %0d %0d", o.x, s.a, $bits(v), f(), $bits(g1.w), $bits(g[1].w), $bits(p2::tt'(0)));
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("legal scopes and forward typedefs must pass");
    assert!(
        sim.output.iter().any(|o| o.message == "R|3 4 9 8 2 2 6"),
        "{:?}",
        sim.output
    );

    let dir = scratch("guarded");
    let hdr = dir.join("guarded.svh");
    let fa = dir.join("inc_a.sv");
    let fb = dir.join("inc_b.sv");
    std::fs::write(
        &hdr,
        "`ifndef G\n`define G\ntypedef logic [7:0] gt;\n`endif\n",
    )
    .unwrap();
    std::fs::write(&fa, "`include \"guarded.svh\"\nmodule a; gt v; endmodule\n").unwrap();
    std::fs::write(
        &fb,
        "`include \"guarded.svh\"\nmodule tb; gt v; a ua(); initial $display(\"gt=%0d\", $bits(v)); endmodule\n",
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
    assert!(stdout.contains("gt=8"), "{stdout}");
    assert!(
        !stderr.contains("warning: duplicate definition"),
        "{stderr}"
    );
}

#[test]
fn dump_merged_sv_strips_duplicate_unit_typedefs_across_libraries() {
    let dir = scratch("merged_td");
    let tb = dir.join("tb.sv");
    let lx = dir.join("libx.sv");
    let ly = dir.join("liby.sv");
    let merged = dir.join("merged.sv");
    std::fs::write(
        &tb,
        "module tb;\n  int a, b;\n  mod_x u1(.o(a));\n  mod_y u2(.o(b));\n  initial begin #1; $display(\"V=%0d,%0d\", a, b); $finish; end\nendmodule\n",
    )
    .unwrap();
    std::fs::write(
        &lx,
        "typedef struct packed {\n  logic [7:0] byte_v;\n} helper_t;\nmodule mod_x(output int o);\n  assign o = 11;\nendmodule\n",
    )
    .unwrap();
    std::fs::write(
        &ly,
        "typedef logic [15:0] helper_t;\nmodule mod_y(output int o);\n  assign o = 22;\nendmodule\n",
    )
    .unwrap();
    let dump = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--simulate", "--no-cache", "-s", "tb"])
        .arg(&tb)
        .arg("-v")
        .arg(&lx)
        .arg("-v")
        .arg(&ly)
        .arg("--dump-merged-sv")
        .arg(&merged)
        .output()
        .expect("run xezim --dump-merged-sv");
    let dump_out = String::from_utf8_lossy(&dump.stdout).to_string();
    assert!(dump.status.success(), "dump failed:\n{dump_out}");
    assert!(
        dump_out.contains("suppressed 1 duplicate"),
        "expected suppressed duplicate typedef:\n{dump_out}"
    );
    let rerun = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--simulate", "--no-cache", "-s", "tb"])
        .arg(&merged)
        .output()
        .expect("rerun merged.sv");
    let _ = std::fs::remove_dir_all(&dir);
    let stdout = String::from_utf8_lossy(&rerun.stdout);
    let stderr = String::from_utf8_lossy(&rerun.stderr);
    assert!(rerun.status.success(), "merged failed:\n{stdout}\n{stderr}");
    assert!(stdout.contains("V=11,22"), "{stdout}");
}
