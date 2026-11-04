//! §9.2.2.4, §9.4.2: an edge block that reads or writes one element of an
//! unpacked array by a constant index (`q[i] <= ...` in a generate loop) is
//! armed for re-execution by writes to that element only. Every element of
//! the array armed it, so N blocks each writing one element of a shared array
//! marked N blocks per element write, N^2 marks per clock: 2,000 such blocks
//! spent 1 µs per nonblocking assignment in the NBA region. The census counts
//! the element inputs; the values are the reference simulator's.
use std::process::Command;

const DESIGN: &str = r#"
module top;
  logic clk = 0;
  logic [15:0] cnt = 0;
  logic [15:0] q [64];
  logic [15:0] r [64];
  logic [15:0] sum;
  always @(posedge clk) cnt <= cnt + 16'd3;
  genvar i;
  generate for (i = 0; i < 64; i++) begin : g
    always @(posedge clk) q[i] <= cnt ^ 16'(i * 37);
    always @(posedge clk) if (q[i][0]) r[i] <= q[i]; else r[i] <= r[i] + 16'd1;
  end endgenerate
  always @(posedge clk) sum <= q[3] + r[7] + q[63];
  initial begin
    repeat (40) begin #5 clk = 1; #5 clk = 0; end
    $display("T|%h %h %h %h", sum, q[3], r[7], r[63]);
    $finish;
  end
endmodule
"#;

#[test]
fn constant_index_element_arms_only_itself() {
    let dir = std::env::temp_dir().join(format!("xezim_const_arm_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temporary directory");
    let path = dir.join("design.sv");
    std::fs::write(&path, DESIGN).expect("write design");
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args([
            "--simulate",
            "-s",
            "top",
            path.to_str().unwrap(),
            "--no-cache",
        ])
        .env("XEZIM_EVENT_EDGE_CENSUS", "1")
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "simulation failed:\n{text}");
    let lines: Vec<&str> = text.lines().filter(|l| l.starts_with("T|")).collect();
    assert_eq!(lines, ["T|0af6 001a 0171 0969"]);
    let elems: u64 = text
        .lines()
        .find_map(|l| l.split("element inputs=").nth(1))
        .and_then(|rest| rest.split(')').next())
        .and_then(|n| n.trim().parse().ok())
        .unwrap_or_else(|| panic!("missing element inputs:\n{text}"));
    // 64 writes of q, 64 writes and 64 reads of r, three reads in `sum`;
    // whole-array arming gave 12,416.
    assert!(elems <= 300, "element inputs={elems}");
}
