//! `--upf`: supply nets driven from the testbench through the `UPF` package,
//! a power switch, domain corruption to x, isolation clamps at the domain
//! boundary (element-specific strategy overriding `-applies_to outputs`),
//! retention exemption, `load_upf` chaining, and the missing-isolation
//! warning. The design under tests/upf/ is a renamed stand-in for a
//! switched-ALU RISC core lab.
use std::path::PathBuf;
use std::process::Command;

fn run(extra: &[&str]) -> (bool, String) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/upf");
    let upf = root.join("pwr_intent.upf");
    let core = root.join("pwr_core.sv");
    let tb = root.join("pwr_tb.sv");
    let mut args: Vec<&str> = vec![
        "--simulate", "-s", "pwr_tb", "--no-cache", "--upf", upf.to_str().unwrap(),
        core.to_str().unwrap(), tb.to_str().unwrap(),
    ];
    args.extend_from_slice(extra);
    let output = Command::new(env!("CARGO_BIN_EXE_xezim")).args(&args).output().unwrap();
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.success(), text)
}

#[test]
fn switched_domain_isolation_and_retention_follow_the_power_intent() {
    let (ok, text) = run(&[]);
    assert!(ok, "run failed:\n{text}");
    assert!(text.contains("UPF_TEST_PASS"), "power-aware checks failed:\n{text}");
    for want in [
        "[UPF] scope /pwr_tb/dut/u_core (core_blk), 4 supply nets: VMAIN, VLOW, GND, VMUL",
        "[UPF] power switch mul_sw: VMAIN -> VMUL controlled by ctrl_off=mul_off",
        "[UPF] power domain PD_MUL: elements [u_mul], power VMUL, ground GND, 2 corruptible signals",
        "[UPF] PST core_pst over [VMAIN, mul_sw/vout]: MUL_OFF={MainVolt MUL_OFF} MUL_ON={MainVolt MainVolt}",
        "Supply ON applied on '/pwr_tb/dut/u_core/VMAIN', Voltage = 1.200000",
        "Supply net '/pwr_tb/dut/u_core/VMUL' toggled to '{FULL_ON 1.20 V}'",
        "Power switch 'mul_sw': control 'ctrl_off' = 1, state OFF",
        "Power domain 'PD_MUL' is powered down",
        "Isolation enabled (strategy mul_iso_zero) on port '/pwr_tb/dut/u_core/u_mul/zero', clamp 1",
        "Isolation enabled (strategy mul_iso_all) on port '/pwr_tb/dut/u_core/u_mul/res', clamp 0",
        "Isolation control 'dut.u_core.mul_iso' (0) is not enabled when power domain 'PD_MUL' is switched OFF",
        "Power domain 'PD_LOW' is powered down",
    ] {
        assert!(text.contains(want), "missing `{want}`:\n{text}");
    }
    // No spurious power-down reports before any supply is turned on.
    assert!(!text.contains("Time: 0, Power domain"), "time-0 domain messages:\n{text}");
}

#[test]
fn explicit_scope_path_selects_the_same_instance() {
    let (ok, text) = run(&["--upf-top", "/pwr_tb/dut/u_core"]);
    assert!(ok, "run failed:\n{text}");
    assert!(text.contains("UPF_TEST_PASS"), "power-aware checks failed:\n{text}");
    assert!(text.contains("[UPF] scope /pwr_tb/dut/u_core (core_blk)"), "scope not resolved:\n{text}");
}

#[test]
fn design_without_upf_flag_is_unaffected() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/upf");
    // The bare design must still elaborate without the UPF package present
    // only when nothing imports it: compile the core alone with a stub top.
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("upf_plain");
    std::fs::create_dir_all(&dir).unwrap();
    let tb = dir.join("plain_tb.sv");
    std::fs::write(&tb, "module plain_tb; logic clk=0, rst_n=1, go=0, en=0, we=0, mul_off=0, mul_iso=0; logic [3:0] a=2, b=3; logic [7:0] d=0; logic [7:0] y, q; logic zero, busy;\n lp_soc_top dut(.*);\n always #5 clk = ~clk;\n initial begin #12 go = 1; #10 go = 0; #10 $display(\"PLAIN y=%0d\", y); $finish; end\nendmodule\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--simulate", "-s", "plain_tb", "--no-cache", root.join("pwr_core.sv").to_str().unwrap(), tb.to_str().unwrap()])
        .output()
        .unwrap();
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success() && text.contains("PLAIN y=6"), "plain run broke:\n{text}");
    assert!(!text.contains("[UPF]"), "UPF glue leaked into a run without --upf:\n{text}");
}
