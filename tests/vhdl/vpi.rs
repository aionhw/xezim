//! A VHDL top level driven through VPI, as a cocotb testbench drives it:
//! `--vpi-lib` exposes the top's ports by name. `vpi/vpi_top.c` is built
//! into a VPI library here (needs `cc`), writes and reads the ports, and
//! prints what it reads. Expected lines follow IEEE 1076-2008 for the VHDL
//! side (an undriven input keeps its default, §6.5.6.3) and IEEE 1800-2023
//! §38 for the VPI side.

use std::path::PathBuf;
use std::process::Command;

#[test]
fn vhdl_top_ports_through_vpi() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/vhdl/vpi");
    let tmp = std::env::temp_dir().join(format!("xezim-vhdl-vpi-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("temp directory");
    let so = tmp.join("vpi_top.so");
    let include = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("include");
    let ok = Command::new("cc")
        .args(["-shared", "-fPIC", "-I"])
        .arg(&include)
        .arg(dir.join("vpi_top.c"))
        .arg("-o")
        .arg(&so)
        .status()
        .expect("failed to launch cc")
        .success();
    assert!(ok, "cc failed");
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .current_dir(&dir)
        .args(["--no-cache", "vpi_top.vhd", "--vpi-lib"])
        .arg(&so)
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&tmp);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let actual: Vec<&str> = stdout
        .lines()
        .filter(|l| {
            !(l.starts_with("=== xezim")
                || l.starts_with("git ")
                || l.starts_with("Simulation finished at time")
                || l.starts_with("[xezim]"))
        })
        .collect();
    assert_eq!(
        actual,
        vec![
            "vpi: io not visible",
            "vpi_top.vhd:21:5:@500ps:(report note): en='1' d=UUUUUUUU",
            "vpi_top.vhd:23:5:@1500ps:(report note): en='1' d=01011010 q=01011010",
            "vpi: vpi_top.q=01011010",
            "vpi: vpi_top.en=0",
            "vpi: vpi_top.q=00000000",
        ],
        "stderr:\n{stderr}"
    );
    assert!(
        stderr.contains(
            "vpi_top.vhd:11:5: warning: VHDL: inout port 'io' cannot be connected to SystemVerilog yet; it is not visible to VPI"
        ),
        "{stderr}"
    );
    assert_eq!(out.status.code(), Some(0), "exit status");
}
