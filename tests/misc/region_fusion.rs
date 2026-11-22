//! Opt-in comb-region fusion (XEZIM_REGIONS=1): dependency-connected
//! compiled comb entries merge into one larger block — registers renumbered,
//! branch targets offset, members topo-ordered, order-satisfied link reads
//! dropped from the fused sensitivity. Values must match the unfused run
//! exactly. (Benchmarked net-negative on ibex/c906 — recompute waste beats
//! dispatch savings — so the pass stays opt-in; this test keeps it correct.)

use std::process::Command;

#[test]
fn region_fusion_matches_unfused_values() {
    let src = r#"
module tb;
  reg [7:0] a, b;
  wire [7:0] s1, s2, s3, s4, s5;
  wire [7:0] t1, t2;
  reg  [7:0] c1, c2;
  assign s1 = a + b;
  assign s2 = s1 ^ 8'h5a;
  assign s3 = s2 & a;
  assign s4 = s3 | b;
  assign s5 = s4 - s1;
  always @* c1 = a ^ s5;
  always @* c2 = c1 + s2;
  assign t1 = s1 & s3;
  assign t2 = t1 ^ s5;
  reg [7:0] q;
  always @(posedge a[0]) q <= s5;
  initial begin
    a = 8'h11; b = 8'h22; #1;
    $display("R1 s5=%h t2=%h c2=%h", s5, t2, c2);
    a = 8'h80; #1;
    $display("R2 s5=%h t2=%h c2=%h q=%h", s5, t2, c2, q);
    a = 8'h81; b = 8'h7f; #1;
    $display("R3 s5=%h t2=%h c2=%h q=%h", s5, t2, c2, q);
    $finish;
  end
endmodule
"#;
    let dir = std::env::temp_dir().join(format!("xezim_region_fusion_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let f = dir.join("t.sv");
    std::fs::write(&f, src).unwrap();
    let run = |fused: bool| -> Vec<String> {
        let mut c = Command::new(env!("CARGO_BIN_EXE_xezim"));
        c.args(["--no-cache", "-s", "tb", "--max-time", "1000"])
            .arg(&f);
        if fused {
            c.env("XEZIM_REGIONS", "1");
        } else {
            c.env_remove("XEZIM_REGIONS");
        }
        let out = c.output().expect("run xezim");
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| l.starts_with("R"))
            .map(|l| l.to_string())
            .collect()
    };
    let plain = run(false);
    let fused = run(true);
    assert_eq!(plain.len(), 3, "expected 3 result lines: {:?}", plain);
    assert_eq!(fused, plain, "fused values must match the unfused run");
}

#[test]
fn shared_region_follows_its_input_through_unknowns_and_known_values() {
    let dir = std::env::temp_dir().join(format!("xezim_shared_region_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("shared.sv");
    std::fs::write(
        &path,
        r#"
module tb;
  reg [7:0] a;
  wire [7:0] plus, minus, other;
  assign plus = a + 8'd7;
  assign minus = a - 8'd3;
  assign other = {4'b0, a[7:4]};
  initial begin
    a = 8'h20; #1; $display("R %b %b %b", plus, minus, other);
    a = 8'b0101xz01; #1; $display("R %b %b %b", plus, minus, other);
    a = 8'h51; #1; $display("R %b %b %b", plus, minus, other);
    $finish;
  end
endmodule
"#,
    )
    .unwrap();
    let run = |mode| {
        let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
            .args(["--no-cache", "-s", "tb"])
            .arg(&path)
            .env("XEZIM_REGIONS", mode)
            .env("XEZIM_REGION_STATS", "1")
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{stderr}");
        if mode == "shared" {
            assert!(
                stderr.contains("fused 2 members into 1 regions"),
                "{stderr}"
            );
        }
        String::from_utf8(out.stdout)
            .unwrap()
            .lines()
            .filter(|line| line.starts_with("R "))
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    let plain = run("0");
    assert_eq!(
        plain,
        [
            "R 00100111 00011101 00000010",
            "R xxxxxxxx xxxxxxxx 00000101",
            "R 01011000 01001110 00000101",
        ]
    );
    assert_eq!(run("shared"), plain);
    std::fs::remove_dir_all(dir).unwrap();
}
