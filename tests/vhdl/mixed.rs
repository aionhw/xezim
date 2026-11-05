//! Mixed-language designs: SystemVerilog and VHDL in one simulation. Each
//! `mixed/<name>.sv` + `mixed/<name>.vhd` runs through `xezim` and its output
//! must equal `mixed/<name>.golden`, produced by Questa 2021.2
//! (`mixed/regen.sh`; GHDL does not simulate SystemVerilog). A design whose
//! ports Questa cannot connect (a VHDL record or array of vectors at the
//! boundary) has a golden of the layout xezim defines instead, and a
//! `mixed/<name>.lrm` saying why. The designs print only through SV
//! `$display` and VHDL TEXTIO, whose text is the same in every simulator,
//! and keep clear of races between the languages.

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
        "{} differs from its golden\nstderr:\n{}",
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

/// VHDL and SV registers on one clock edge sample each other's old values.
#[test]
fn mixed_clocked() {
    check("mixed_clocked");
}

/// A record port in both directions, and three-dimensional arrays of
/// records in both directions. Each crosses as one packed logic vector: the
/// left index and the first field are the most significant bits, and each
/// std_logic is one bit. Every field and element is read on its own; the
/// golden is that layout (`mixed_record.lrm`).
#[test]
fn mixed_record() {
    check("mixed_record");
}

/// A record signal whose field is an array, instantiated from
/// SystemVerilog and initialised from a constant: its initial value, then
/// the values written on a clock edge.
#[test]
fn mixed_record_field() {
    check("mixed_record_field");
}

/// A signal of a record subtype constraining an array field, in an entity
/// instantiated from SystemVerilog: the field written whole and by a slice.
#[test]
fn mixed_record_subtype() {
    check("mixed_record_subtype");
}

/// A slice of an array of vectors as the actual of a SystemVerilog
/// component's port: only its elements cross (`mixed_slice_actual.lrm`).
#[test]
fn mixed_slice_actual() {
    check("mixed_slice_actual");
}

/// An array of arrays of vectors in both directions: one packed vector,
/// the left index of each level first (`mixed_array_of_arrays.lrm`); input
/// elements read one by one.
#[test]
fn mixed_array_of_arrays() {
    check("mixed_array_of_arrays");
}
