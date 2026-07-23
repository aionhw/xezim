//! `XEZIM_VCD_PARAM_AS_WIRE=1` emits VCD parameters as constant-valued `$var
//! wire` instead of the (LRM-default) `$var parameter`, so viewers that shelve
//! `$var parameter` separately (Verdi/nWave) still show them in the waveform
//! pane. Values are dumped either way. Subprocess-based so the env var doesn't
//! leak into the other (parallel) VCD tests.

use std::process::Command;

fn xezim_bin() -> std::path::PathBuf {
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("xezim")
}

const SRC: &str = r#"
module core (output [7:0] q);
  parameter DEPTH = 64;
  assign q = DEPTH[7:0];
endmodule
module tb;
  wire [7:0] q;
  core #(.DEPTH(128)) u (.q(q));
  initial begin $dumpfile("{VCD}"); $dumpvars(0, tb); #5 $finish; end
endmodule
"#;

fn run_and_read(param_as_wire: bool) -> String {
    let dir = std::env::temp_dir().join("xezim_vcd_paramwire");
    std::fs::create_dir_all(&dir).unwrap();
    let vcd = dir.join(format!("p_{}.vcd", param_as_wire));
    let sv = dir.join(format!("p_{}.sv", param_as_wire));
    let _ = std::fs::remove_file(&vcd);
    std::fs::write(&sv, SRC.replace("{VCD}", vcd.to_str().unwrap())).unwrap();
    let mut cmd = Command::new(xezim_bin());
    cmd.env("XEZIM_NO_CACHE", "1");
    if param_as_wire {
        cmd.env("XEZIM_VCD_PARAM_AS_WIRE", "1");
    }
    let out = cmd
        .args(["-s", "tb", "--max-time", "100"])
        .arg(&sv)
        .output()
        .expect("run xezim");
    assert!(out.status.success(), "xezim failed: {:?}", out.status);
    std::fs::read_to_string(&vcd).expect("no VCD written")
}

fn depth_var_line(vcd: &str) -> String {
    vcd.lines()
        .find(|l| l.starts_with("$var") && l.contains(" DEPTH "))
        .unwrap_or_default()
        .to_string()
}

#[test]
fn default_dumps_param_kind() {
    let vcd = run_and_read(false);
    let l = depth_var_line(&vcd);
    assert!(l.starts_with("$var parameter 32 "), "default not `parameter`: {l}");
    // value still present
    assert!(vcd.contains("b10000000"), "DEPTH=128 value missing:\n{vcd}");
}

#[test]
fn wire_mode_dumps_param_as_wire() {
    let vcd = run_and_read(true);
    let l = depth_var_line(&vcd);
    assert!(l.starts_with("$var wire 32 "), "wire mode not `wire`: {l}");
    assert!(vcd.contains("b10000000"), "DEPTH=128 value missing:\n{vcd}");
}
