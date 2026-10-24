//! Mixed SystemC/SystemVerilog designs: SystemVerilog instantiating
//! SystemC modules (no `sc_main`; the SystemC processes start at time 0)
//! and SystemC instantiating SystemVerilog modules through an
//! `sc_foreign_module` declaration. Each `mixed/<name>/` holds the design
//! and `reference/all_sc.cpp`, the same design written entirely in SystemC;
//! `mixed/<name>.golden` is that reference's output under the SystemC 2.3.4
//! kernel (banner removed; for a SystemVerilog top, which ends with
//! `$finish`, also its `sc_stop` message).

use std::path::PathBuf;
use std::process::Command;

fn check(name: &str) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/systemc/mixed");
    let dir = root.join(name);
    let mut files: Vec<String> = std::fs::read_dir(&dir)
        .expect("design directory")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .filter(|f| f.ends_with(".sv") || f.ends_with(".cpp"))
        .collect();
    files.sort();
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .current_dir(&dir)
        .arg("--no-cache")
        .args(&files)
        .output()
        .expect("run xezim");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let actual: Vec<&str> = stdout
        .lines()
        .filter(|l| {
            !(l.starts_with("=== xezim")
                || l.starts_with("git ")
                || l.starts_with("Simulation finished at time")
                || l.starts_with("[xezim]"))
        })
        .collect();
    let actual = actual.join("\n");
    let golden = std::fs::read_to_string(root.join(format!("{}.golden", name))).expect("golden");
    assert_eq!(
        actual.trim(),
        golden.trim(),
        "{} differs from its all-SystemC reference\nstderr:\n{}",
        name,
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A SystemVerilog testbench drives a SystemC counter (IEEE 1666-2023
/// §5.2.9 SC_METHOD on `clk.pos()`); the top is found without `-s`.
#[test]
fn sv_testbench_systemc_dut() {
    check("sv_tb");
}

/// An `sc_main` testbench drives a SystemVerilog `always_ff` shift
/// register declared to SystemC as `sc_foreign_module(nm, "shifter")`.
#[test]
fn systemc_testbench_sv_dut() {
    check("sc_tb");
}

/// SystemVerilog top → SystemC module → SystemVerilog `always_comb` leaf
/// named by the foreign module's `hdl_name()`; signal values cross both
/// language boundaries every cycle.
#[test]
fn sv_systemc_sv_hierarchy() {
    check("sandwich");
}
