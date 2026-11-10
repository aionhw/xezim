//! §9.4.2 edge terms on bits of a vector that replicates one gated clock
//! (`assign kclk[g*4 +: 4] = {4{gclk[g]}}`, the shape of a per-source
//! interrupt-controller clock vector). Every bit of such a range is written
//! by the same continuous assignment and is equal in every delta (§10.3.2),
//! so the simulator watches one alias per range instead of one per bit. Edge
//! counts and what the derived-clock flops sample (the old value of an
//! NBA-updated input, i.e. the edge lands before that time step's NBA
//! region) must stay exactly as with per-bit aliases. A vector whose bits
//! have their own clocks, and a replicated vector with a second driver on
//! one bit, must keep separate aliases.
//!
//! Expected values are the reference simulator's.
use std::path::PathBuf;
use std::process::Command;

const SRC: &str = r#"
`timescale 1ns/1ns
module kid(input kclk, input rst_b, input [7:0] d, output reg [7:0] q, output int n);
  initial n = 0;
  always @(posedge kclk or negedge rst_b)
    if (!rst_b) q <= 8'd0;
    else q <= d;
  always @(posedge kclk) n <= n + 1;
endmodule

module tb;
  logic clk = 0, rst_b = 1;
  logic [1:0] en = 2'b00, en_lat = 2'b00;
  wire [1:0] gclk;
  wire [7:0] kclk;
  wire [1:0] oclk;
  logic [7:0] d = 8'd0;
  int seen [8];
  int other [2];
  always #5 clk = ~clk;
  // Latch-based clock gate per group, as in a gated-clock cell.
  always @(*) if (!clk) en_lat = en;
  assign gclk = {2{clk}} & en_lat;
  for (genvar g = 0; g < 2; g++) begin : G
    assign kclk[g*4 +: 4] = {4{gclk[g]}};
  end
  // Not replicated: each bit has its own gated clock.
  assign oclk = {gclk[0], gclk[1]};
  wire [7:0] qa [8];
  wire [31:0] na [8];
  for (genvar i = 0; i < 8; i++) begin : K
    kid u(.kclk(kclk[i]), .rst_b(rst_b), .d(d + 8'(i)), .q(qa[i]), .n(na[i]));
    // Samples the flop output in the edge's active region: the NBA from
    // this edge has not landed yet.
    always @(posedge kclk[i]) seen[i] = seen[i] * 3 + qa[i];
  end
  // Replicated, but bit 3 has a second driver: the bits need not agree.
  wire [3:0] wv;
  assign wv = {4{gclk[0]}};
  assign wv[3] = gclk[1];
  int wired [2];
  always @(posedge wv[2]) wired[0]++;
  always @(posedge wv[3]) wired[1]++;
  always @(posedge oclk[0]) other[0]++;
  always @(posedge oclk[1]) other[1]++;
  always @(posedge clk) d <= d + 8'd5;
  initial begin
    for (int k = 0; k < 8; k++) seen[k] = 0;
    other[0] = 0; other[1] = 0;
    wired[0] = 0; wired[1] = 0;
    #1 rst_b = 0;
    #11 rst_b = 1;
    #10 en = 2'b01;
    #40 en = 2'b11;
    #30 en = 2'b10;
    #20 en = 2'b00;
    #20 en = 2'b11;
    #25;
    for (int k = 0; k < 8; k++)
      $display("T| i=%0d n=%0d q=%0d seen=%0d", k, na[k], qa[k], seen[k]);
    $display("T| other=%0d,%0d wired=%0d d=%0d t=%0t", other[0], other[1], wired[0], d, $time);
    $finish;
  end
endmodule
"#;

/// `tag` names this test's source file: the tests run in parallel, and one
/// reading a file another was still writing saw no module at all.
fn run(tag: &str, env: &[(&str, &str)]) -> String {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("replicated_clock_edges");
    std::fs::create_dir_all(&dir).unwrap();
    let sv = dir.join(format!("{tag}.sv"));
    std::fs::write(&sv, SRC).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_xezim"));
    for (k, v) in env {
        cmd.env(k, v);
    }
    let output = cmd
        .args(["--simulate", "-s", "tb", "--no-cache", sv.to_str().unwrap()])
        .output()
        .unwrap();
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(), "run failed:\n{text}");
    text
}

fn t_lines(text: &str) -> Vec<&str> {
    text.lines()
        .filter_map(|l| l.find("T|").map(|i| &l[i..]))
        .collect()
}

#[test]
fn replicated_gated_clock_bits_keep_edge_counts_and_order() {
    let o = run("counts", &[]);
    assert_eq!(
        t_lines(&o),
        vec![
            "T| i=0 n=10 q=75 seen=123070",
            "T| i=1 n=10 q=76 seen=132911",
            "T| i=2 n=10 q=77 seen=142752",
            "T| i=3 n=10 q=78 seen=152593",
            "T| i=4 n=8 q=79 seen=39917",
            "T| i=5 n=8 q=80 seen=41010",
            "T| i=6 n=8 q=81 seen=42103",
            "T| i=7 n=8 q=82 seen=43196",
            "T| other=8,10 wired=10 d=80 t=157",
        ],
        "{o}"
    );
}

#[test]
fn replicated_bits_share_one_alias_per_range() {
    let o = run("alias", &[("XEZIM_DUMP_EDGE_SENS", "1")]);
    for (bit, lo) in [(1, 0), (2, 0), (3, 0), (5, 4), (6, 4), (7, 4)] {
        assert!(
            o.contains(&format!("kclk[{bit}] shares the alias of bit {lo}")),
            "kclk[{bit}] did not share bit {lo}'s alias:\n{o}"
        );
    }
    // Separate clocks per bit, and a range with a second driver: no sharing.
    for name in ["oclk[1]", "wv[1]", "wv[2]", "wv[3]"] {
        assert!(
            !o.contains(&format!("{name} shares")),
            "{name} must keep its own alias:\n{o}"
        );
    }
}
