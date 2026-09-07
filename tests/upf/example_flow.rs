//! Runs the shipped power-intent example (`examples/upf/`): supplies driven
//! through the `UPF` package, a header switch, isolation clamps released back
//! to the live drivers, corruption of the switched block, and a retained
//! configuration register.
use std::path::PathBuf;
use std::process::Command;

#[test]
fn shipped_upf_example_passes() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/upf");
    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args([
            "--simulate", "-s", "pwr_demo_tb", "--no-cache",
            "--upf", root.join("pwr_demo.upf").to_str().unwrap(),
            root.join("pwr_demo.sv").to_str().unwrap(),
            root.join("pwr_demo_tb.sv").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(), "run failed:\n{text}");
    assert!(text.contains("UPF_EXAMPLE_PASS"), "example checks failed:\n{text}");
    for want in [
        "[UPF] scope /pwr_demo_tb/dut (pwr_demo_top), 3 supply nets: VDD, VSS, VDD_ACC",
        "[UPF] power switch acc_sw: VDD -> VDD_ACC controlled by ctl=acc_on",
        "[UPF] isolation acc_iso on PD_ACC: clamp 0 control acc_iso_en sense high",
        "[UPF] power domain PD_ACC: elements [u_acc, u_cfg], power VDD_ACC, ground VSS, 3 corruptible signals, retention exempt: dut.u_cfg",
        "Power switch 'acc_sw': control 'ctl' = 0, state OFF",
        "Power domain 'PD_ACC' is powered down",
        "Isolation enabled (strategy acc_iso) on port '/pwr_demo_tb/dut/u_cfg/q', clamp 0",
        "Isolation disabled (strategy acc_iso) on port '/pwr_demo_tb/dut/u_cfg/q'",
        "ok   cfg still retained = a5",
    ] {
        assert!(text.contains(want), "missing `{want}`:\n{text}");
    }
    // The always-on domain has no retained elements of its own.
    assert!(!text.contains("PD_TOP: elements [], power VDD, ground VSS, 0 corruptible signals, retention"), "retention listed on PD_TOP:\n{text}");
}
