//! IEEE 1801 §6.11 / SV §10.6.2: a root domain powers down and recovers.
use std::process::Command;

#[test]
fn root_domain_corrupts_and_releases_constant_child_outputs() {
    let dir = std::env::temp_dir().join(format!("xezim_upf_root_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("top.sv");
    let intent = dir.join("root.upf");
    std::fs::write(
        &source,
        r#"
module leaf(output wire [3:0] q); assign q=4'h9; endmodule
module top;
  import UPF::*;
  wire [3:0] q; leaf u(q);
  initial begin
    void'(supply_on("VDD",1.0)); void'(supply_on("VSS",0.0));
    #2; if(q!==4'h9) $fatal(1,"root power-up q=%h",q);
    void'(supply_off("VDD"));
    #2; if(q!==4'bxxxx) $fatal(1,"root power-down q=%h",q);
    void'(supply_on("VDD",1.0));
    #2; if(q!==4'h9) $fatal(1,"root recovery q=%h",q);
    $display("PASS root domain"); $finish;
  end
endmodule
"#,
    )
    .unwrap();
    std::fs::write(&intent, "set_design_top top\ncreate_power_domain PD -elements {.}\ncreate_supply_net VDD -domain PD\ncreate_supply_net VSS -domain PD\nset_domain_supply_net PD -primary_power_net VDD -primary_ground_net VSS\n").unwrap();
    for override_top in [false, true] {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_xezim"));
        cmd.args([
            "--simulate",
            "--no-cache",
            "--error-exit",
            "-s",
            "top",
            "--upf",
        ])
        .arg(&intent)
        .arg(&source);
        if override_top {
            cmd.args(["--upf-top", "/top"]);
        }
        let output = cmd.output().unwrap();
        let text = String::from_utf8_lossy(&output.stdout).into_owned()
            + &String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success() && text.contains("PASS root domain"),
            "{text}"
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}
