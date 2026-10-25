//! Mixed-language designs: SystemVerilog and VHDL in one simulation. Each
//! `mixed/<name>.sv` + `mixed/<name>.vhd` runs through `xezim` and its output
//! must equal `mixed/<name>.golden`, produced by Questa 2021.2
//! (`mixed/regen.sh`; GHDL does not simulate SystemVerilog). The designs
//! print only through SV `$display` and VHDL TEXTIO, whose text is the same
//! in every simulator, and keep clear of races between the languages.

use std::path::PathBuf;
use std::process::Command;

fn check(name: &str) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/vhdl/mixed");
    let sv = dir.join(format!("{}.sv", name));
    let vhd = dir.join(format!("{}.vhd", name));
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .arg("--no-cache")
        .arg(&sv)
        .arg(&vhd)
        .output()
        .expect("run xezim");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let actual: Vec<&str> = stdout
        .lines()
        .filter(|l| {
            !(l.starts_with("=== xezim")
                || l.starts_with("git ")
                || l.starts_with("Simulation finished at time")
                || l.starts_with("[xezim]")
                || l.starts_with("[warn]"))
        })
        .collect();
    let golden = std::fs::read_to_string(dir.join(format!("{}.golden", name))).expect("golden");
    assert_eq!(
        actual.join("\n").trim_end(),
        golden.trim_end(),
        "{} differs from Questa\nstderr:\n{}",
        name,
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.status.code(), Some(0), "{} exit status", name);
}

/// A VHDL entity instantiated from SystemVerilog: generics from parameter
/// expressions, ports of std_logic, std_logic_vector (descending and
/// ascending), unsigned, bit, boolean, integer and real, the nine
/// std_ulogic values leaving for 4-state logic and x/z entering, and an
/// internal signal read by a hierarchical name.
#[test]
fn mixed_sv_top() {
    check("mixed_sv_top");
}

/// SystemVerilog modules bound to the components of a VHDL testbench:
/// generics to parameters (integer, string, real), parts of signals and a
/// literal as actuals, open outputs.
#[test]
fn mixed_vhdl_top() {
    check("mixed_vhdl_top");
}

/// SystemVerilog around VHDL around SystemVerilog.
#[test]
fn mixed_sandwich() {
    check("mixed_sandwich");
}
