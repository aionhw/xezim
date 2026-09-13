//! FFI integration test: compile the C and Python probes against
//! `libxezim.so`, run a real simulation to emit the sidecars, then
//! exercise both probes end-to-end. The test is skipped if `cc` or
//! `python3` is missing, so it runs in CI but doesn't block local
//! builds without a C toolchain.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Path to the built `libxezim.so` (cdylib from `crate-type = ["rlib","cdylib"]`).
///
/// `cargo test` does not build the cdylib, so the probe link target only
/// exists after `cargo build --lib` (CI does this explicitly). Resolve the
/// target directory through `CARGO_TARGET_DIR` so a custom target dir works
/// too, not just a repo-root `target/`.
fn libxezim_path() -> PathBuf {
    let target = std::env::var("CARGO_TARGET_DIR").map(PathBuf::from).unwrap_or_else(|_| {
        let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.push("target");
        p
    });
    target.join("debug").join("libxezim.so")
}

/// Path to the xezim binary next to the test binary.
fn xezim_bin() -> PathBuf {
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("xezim")
}

/// Minimal design whose trace outputs are analytically known.
const DESIGN: &str = r#"
`timescale 1ns/1ns
module top;
  reg clk = 0;
  reg a = 0, b = 0;
  wire d = a & b;
  wire e = d | a;
  wire [1:0] w = {d, e};
  always #5 clk = ~clk;
  initial begin
    #10 a = 1;   // t=10: e goes 1
    #10 b = 1;   // t=20: d goes 1
    #10 a = 0;   // t=30: d goes 0 (e stays 1)
    #10 b = 0;   // t=40: e goes 0
    #10 $finish; // t=50
  end
endmodule
"#;

/// Build the C probe: `cc -I c-api xezim_trace_probe.c -L target/debug -lxezim -o <out>`.
fn build_c_probe(out: &Path) -> Result<(), String> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let status = Command::new("cc")
        .args([
            "-I",
            manifest.join("c-api").to_str().unwrap(),
            manifest.join("c-api/xezim_trace_probe.c").to_str().unwrap(),
            "-L",
            manifest.join("target/debug").to_str().unwrap(),
            "-lxezim",
            "-o",
            out.to_str().unwrap(),
        ])
        .status()
        .map_err(|e| format!("failed to run cc: {e}"))?;
    if !status.success() {
        return Err("cc failed".into());
    }
    Ok(())
}

/// Run the C probe against the generated sidecars.
fn run_c_probe(probe: &Path, graph: &Path, census: &Path) -> Result<(), String> {
    let lib_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/debug");
    let mut env_ld = std::env::var("LD_LIBRARY_PATH").unwrap_or_default();
    if !env_ld.is_empty() {
        env_ld.push(':');
    }
    env_ld.push_str(lib_dir.to_str().unwrap());
    let status = Command::new(probe)
        .args([
            graph.to_str().unwrap(),
            census.to_str().unwrap(),
            "d",
            "a",
            "e",
            "6",
            "2",
        ])
        .env("LD_LIBRARY_PATH", &env_ld)
        .status()
        .map_err(|e| format!("failed to run probe: {e}"))?;
    if !status.success() {
        return Err("C probe exited non-zero".into());
    }
    Ok(())
}

/// Run the Python probe against the generated sidecars.
fn run_python_probe(graph: &Path, census: &Path) -> Result<(), String> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let probe_py = manifest.join("c-api/xezim_trace_probe.py");
    let status = Command::new("python3")
        .args([
            probe_py.to_str().unwrap(),
            graph.to_str().unwrap(),
            census.to_str().unwrap(),
            "d",
            "a",
            "e",
            "6",
            "2",
        ])
        .status()
        .map_err(|e| format!("failed to run python3: {e}"))?;
    if !status.success() {
        return Err("Python probe exited non-zero".into());
    }
    Ok(())
}

/// Check if `cc` and `python3` exist in PATH.
fn have_toolchain() -> bool {
    Command::new("cc")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
        && Command::new("python3")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok()
}

#[test]
fn ffi_probes_work() {
    if !have_toolchain() {
        eprintln!("skipping FFI test: cc or python3 not in PATH");
        return;
    }
    if !libxezim_path().exists() {
        eprintln!(
            "skipping FFI test: {} not built (run `cargo build --lib`); C ABI not exercised",
            libxezim_path().display()
        );
        return;
    }

    // 1. Run xezim to produce the sidecars.
    let dir = std::env::temp_dir().join(format!("xezim_ffi_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let sv = dir.join("a.sv");
    std::fs::write(&sv, DESIGN).expect("write sv");
    let graph = dir.join("graph.json");
    let census = dir.join("census.json");

    let bin = xezim_bin();
    let out = Command::new(bin)
        .current_dir(&dir)
        .env("XEZIM_ACT_TRACE_CENSUS_FILE", &census)
        .args([
            "--simulate",
            "--max-time",
            "100",
            "-s",
            "top",
            "--debug+all",
            "--debug-graph-file",
        ])
        .arg(&graph)
        .arg(&sv)
        .output()
        .expect("run xezim");
    assert!(
        out.status.success(),
        "xezim run failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(graph.exists(), "graph sidecar missing");
    assert!(census.exists(), "census sidecar missing");

    // 2. Build the C probe.
    let probe_bin = dir.join("xezim_trace_probe");
    build_c_probe(&probe_bin).expect("C probe build failed");
    assert!(probe_bin.exists(), "C probe binary not produced");

    // 3. Run the C probe.
    run_c_probe(&probe_bin, &graph, &census).expect("C probe run failed");

    // 4. Run the Python probe.
    run_python_probe(&graph, &census).expect("Python probe run failed");

    // 5. Negative: a missing sidecar must give NULL + non-empty last_error.
    // (Already covered inside the probes themselves.)
}
