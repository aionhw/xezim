//! IEEE 1800-2023 §4.6 / §10.4.2: staged evaluation preserves publication semantics.

use std::path::PathBuf;
use std::process::Command;

struct Scratch(PathBuf);
impl Scratch {
    fn new(label: &str) -> Self {
        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("multicore_{label}_{}_{time}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn run(&self, source: &str, env: &[(&str, &str)]) -> String {
        let path = self.0.join("top.sv");
        std::fs::write(&path, source).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
            .args([
                "--simulate",
                "--no-cache",
                "--error-exit",
                "--profile",
                "-s",
                "top",
            ])
            .arg(&path)
            .envs(env.iter().copied())
            .output()
            .unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.status.success(), "{env:?}\n{text}");
        text
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn partial_nbas_preserve_live_unselected_bits_and_last_write_order() {
    let scratch = Scratch::new("nba");
    let mut expression = "acc".to_string();
    for i in 0..16 {
        expression = format!(
            "(({expression} ^ seed) + ((acc >> {}) ^ 32'h9e3779b9))",
            i % 5 + 1
        );
    }
    let mut source = format!(
        "module lane(input logic clk, input logic[31:0] seed, output logic[31:0] acc); initial acc=1; always @(posedge clk) acc <= {expression}; endmodule\n"
    );
    source.push_str(r#"
module top;
  logic clk=0;
  logic[7:0] q=0, bitq=0, dynq=0, unionq=0, lastq=0, noopq=0, overlapq=0, xq=0;
  logic[127:0] wide=0;
  logic[7:0] array_q[2];
  int index=1;
  generate for(genvar i=0;i<192;i++) begin:g
    wire[31:0] unused_out;
    lane u(clk,32'h12345678+i,unused_out);
  end endgenerate
  always @(posedge clk) q[3:0] <= 4'ha;
  always @(posedge clk) q[7:4] = 4'h5;
  always @(posedge clk) begin
    bitq[2] <= 1'b1; bitq[7:4] = 4'hc;
    dynq[index+:3] <= 3'b101; dynq[7:4] = 4'hd;
    wide[67:64] <= 4'ha; wide[127:124] = 4'h5;
    unionq[1:0] <= 1; unionq[5:4] <= 2; unionq[7:6] = 3;
    lastq[3:0] <= 4'ha; lastq <= 8'h33; lastq[1:0] <= 1; lastq[7:4] = 4'hf;
    noopq[1:0] <= 0; noopq[1:0] = 3;
    overlapq[3:0] <= 4'ha; overlapq[2:1] <= 0; overlapq[7:4] = 4'h5;
    xq[3:0] <= 4'b10xz; xq[7:4] = 4'hb;
    array_q[0][3:0] <= 4'ha; array_q[0][7:4] = 4'h5;
  end
  initial begin
    array_q[0]=0; array_q[1]=0;
    #1; clk=1; #1;
    $display("T|partial|%h %h %h %h %h %h %h %b %h %h %h",q,bitq,dynq,unionq,lastq,noopq,overlapq,xq,wide[127:124],wide[67:64],array_q[0]);
    $finish;
  end
endmodule
"#);
    let modes: &[&[(&str, &str)]] = &[
        &[("XEZIM_NO_PARALLEL", "1")],
        &[("XEZIM_NO_PARALLEL", "1"), ("XEZIM_TWO_STATE", "0")],
        &[
            ("XEZIM_FORCE_PARALLEL", "1"),
            ("XEZIM_PARALLEL_SERIALIZE", "1"),
        ],
        &[("XEZIM_FORCE_PARALLEL", "1")],
        &[("XEZIM_FORCE_PARALLEL", "1"), ("XEZIM_TWO_STATE", "0")],
        &[
            ("XEZIM_FORCE_PARALLEL", "1"),
            ("XEZIM_MULTICORE", "1"),
            ("XEZIM_THREADS", "2"),
        ],
        &[
            ("XEZIM_FORCE_PARALLEL", "1"),
            ("XEZIM_MULTICORE", "1"),
            ("XEZIM_THREADS", "4"),
        ],
    ];
    for mode in modes {
        let text = scratch.run(&source, mode);
        let tagged: Vec<_> = text.lines().filter(|l| l.starts_with("T|")).collect();
        assert_eq!(
            tagged,
            ["T|partial|5a c4 da e1 31 00 58 101110xz 5 a 5a"],
            "{mode:?}\n{text}"
        );
        if mode.iter().any(|(k, _)| *k == "XEZIM_FORCE_PARALLEL") {
            assert!(
                text.contains("parallel_dispatch") || text.contains("multicore_dispatch"),
                "fixture did not exercise worker dispatch\n{text}"
            );
        }
    }
}

#[test]
fn bsp_workers_publish_independent_cones_and_observe_local_blocking_writes() {
    let scratch = Scratch::new("bsp");
    let mut source =
        "module top; logic[31:0] seed=0; bit[31:0] clean; assign clean=seed;\n".to_string();
    for i in 0..128 {
        source.push_str(&format!("logic[31:0] temp{i}, out{i}; wire[31:0] next{i}; always_comb begin temp{i}=seed+32'd{i}; out{i}=temp{i}^32'ha5a55a5a; end assign next{i}=out{i}+7;\n"));
    }
    source.push_str(
        "initial begin for(int cycle=0;cycle<8;cycle++) begin seed=cycle*32'h1234567; #1;\n",
    );
    for i in 0..128 {
        source.push_str(&format!("if(out{i} !== ((seed+32'd{i})^32'ha5a55a5a) || next{i} !== (((seed+32'd{i})^32'ha5a55a5a)+7)) $fatal(1,\"cone {i}\");\n"));
    }
    source.push_str(
        r#"
    end
    force next0=32'h1234; seed=32'h87654321; #1;
    if(next0!==32'h1234) $fatal(1,"force publication");
    release next0; #1;
    if(next0!==((seed^32'ha5a55a5a)+7)) $fatal(1,"release publication");
    seed=32'hffffffff; #1;
    if(clean!==32'hffffffff) $fatal(1,"two-state copy");
    $display("T|bsp|PASS"); $finish;
  end
endmodule
"#,
    );
    for mode in [
        vec![],
        vec![("XEZIM_BSP_SETTLE", "1")],
        vec![
            ("XEZIM_BSP_SETTLE", "1"),
            ("XEZIM_BSP_PAR", "1"),
            ("XEZIM_BSP_PAR_THRESHOLD", "2"),
        ],
        vec![
            ("XEZIM_BSP_SHADOW", "1"),
            ("XEZIM_BSP_PAR", "1"),
            ("XEZIM_BSP_PAR_THRESHOLD", "2"),
        ],
    ] {
        let text = scratch.run(&source, &mode);
        assert!(text.lines().any(|l| l == "T|bsp|PASS"), "{mode:?}\n{text}");
    }
}
