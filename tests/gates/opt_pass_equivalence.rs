//! Equivalence tests for opt-in structural optimizations.
//!
//! * `XEZIM_BUF_COLLAPSE` — folds whole-net identity continuous assigns
//!   (`assign y = x;`) onto their source net, the analogue of the reference
//!   optimizer's default clock-net merging;
//! * `XEZIM_EDGE_MERGE=<N>` — merges edge blocks that share an identical
//!   sensitivity into one compiled block, the analogue of their default
//!   process-merging pass.
//! * `XEZIM_RANGE_COPY=1` — lowers a direct constant-range assignment to a
//!   single four-state slice operation.
//!
//! Both rewrite the design before compilation, so the property that matters
//! is that they change NOTHING observable. Each test runs the same source
//! twice — once stock, once with the pass on — and requires byte-identical
//! program output, plus evidence from the pass's own report that it actually
//! fired (a silently-inactive pass would make the comparison vacuous).

use std::process::Command;

fn xezim() -> String {
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("xezim").to_string_lossy().into_owned()
}

/// Run `src` and return (stdout+stderr, program-output lines only).
fn run(src: &str, tag: &str, env: &[(&str, &str)]) -> (String, Vec<String>) {
    let dir = std::env::temp_dir().join(format!("xezim_opt_eq_{tag}"));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join("dut.sv");
    std::fs::write(&path, src).expect("write");
    let mut cmd = Command::new(xezim());
    cmd.current_dir(&dir)
        .arg("--simulate")
        .arg("-s")
        .arg("tb")
        .arg("--max-time")
        .arg("500ns")
        .arg(&path);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run xezim");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    // Program output only: the `OUT` lines the design prints.
    let prog = text
        .lines()
        .filter(|l| l.starts_with("OUT "))
        .map(|l| l.to_string())
        .collect();
    (text, prog)
}

/// A buffer chain (`raw -> b1 -> b2 -> gated`) feeding real logic, so the
/// collapse has something to fold and the folded net still has to carry the
/// same values at the same times.
const BUF_DESIGN: &str = r#"
module tb;
  logic clk = 1'b0;
  logic rst_n = 1'b0;
  logic [7:0] cnt;
  // identity chain — every one of these is a collapse candidate
  wire b1, b2, b3;
  assign b1 = clk;
  assign b2 = b1;
  assign b3 = b2;
  wire [7:0] c1, c2;
  assign c1 = cnt;
  assign c2 = c1;

  always #5 clk = ~clk;
  initial begin rst_n = 1'b0; #12 rst_n = 1'b1; end

  // driven off the END of the buffer chain
  always @(posedge b3 or negedge rst_n) begin
    if (!rst_n) cnt <= 8'd0;
    else        cnt <= cnt + 8'd1;
  end

  always @(posedge b3) begin
    if (rst_n) $display("OUT t=%0t cnt=%0d c2=%0d b3=%b", $time, cnt, c2, b3);
  end

  initial begin #400; $display("OUT final cnt=%0d", cnt); $finish; end
endmodule
"#;

/// Several flops sharing ONE sensitivity list, which is what edge merging
/// groups; the values are cross-checked so a mis-ordered merge shows up.
const MERGE_DESIGN: &str = r#"
module tb;
  logic clk = 1'b0;
  logic rst_n = 1'b0;
  logic [7:0] a, b, c, d;

  always #5 clk = ~clk;
  initial begin rst_n = 1'b0; #12 rst_n = 1'b1; end

  always @(posedge clk or negedge rst_n) if (!rst_n) a <= 8'd0; else a <= a + 8'd1;
  always @(posedge clk or negedge rst_n) if (!rst_n) b <= 8'd0; else b <= a + 8'd2;
  always @(posedge clk or negedge rst_n) if (!rst_n) c <= 8'd0; else c <= b + 8'd3;
  always @(posedge clk or negedge rst_n) if (!rst_n) d <= 8'd0; else d <= c + 8'd4;

  always @(posedge clk) begin
    if (rst_n) $display("OUT t=%0t a=%0d b=%0d c=%0d d=%0d", $time, a, b, c, d);
  end

  initial begin #400; $display("OUT final %0d %0d %0d %0d", a, b, c, d); $finish; end
endmodule
"#;

const RANGE_DESIGN: &str = r#"
module tb;
  logic [127:0] source_bus;
  wire [63:0] result_bus;
  assign result_bus[55:8] = source_bus[111:64];

  task automatic show;
    #1 $display("OUT src=%h dst=%h", source_bus, result_bus);
  endtask

  initial begin
    source_bus = 128'h0123456789abcdef_fedcba9876543210; show();
    source_bus = 128'hxxxx56789abcdef0_1111222233334444; show();
    source_bus = 128'hzzzz56789abcdef0_5555666677778888; show();
    source_bus = 128'hffff0000aaaa5555_9999aaaabbbbcccc; show();
    $finish;
  end
endmodule
"#;

#[test]
fn buffer_net_collapse_is_observationally_identical() {
    // The pass is on by default; `=0` is the opt-out, and the census line
    // it prints is gated behind XEZIM_DEBUG.
    let (_, base) = run(BUF_DESIGN, "buf_base", &[("XEZIM_BUF_COLLAPSE", "0")]);
    let (text, folded) = run(BUF_DESIGN, "buf_on", &[("XEZIM_DEBUG", "1")]);
    assert!(
        !base.is_empty(),
        "baseline produced no program output — the test would be vacuous"
    );
    assert!(
        text.contains("[BUF-COLLAPSE]"),
        "the collapse never fired, so this proves nothing:\n{text}"
    );
    assert_eq!(base, folded, "buffer collapse changed observable output");
}

#[test]
fn edge_block_merge_is_observationally_identical() {
    let (_, base) = run(MERGE_DESIGN, "merge_base", &[]);
    let (text, merged) = run(MERGE_DESIGN, "merge_on", &[("XEZIM_EDGE_MERGE", "2")]);
    assert!(
        !base.is_empty(),
        "baseline produced no program output — the test would be vacuous"
    );
    assert!(
        text.contains("[EDGE-MERGE] merged"),
        "no blocks were merged, so this proves nothing:\n{text}"
    );
    assert_eq!(base, merged, "edge merging changed observable output");
}

/// The two passes compound (clock buffers fold, which enlarges the
/// same-sensitivity groups); check the combination as well, since that is
/// the configuration the benchmarks use.
#[test]
fn collapse_and_merge_together_are_identical() {
    let (_, base) = run(MERGE_DESIGN, "both_base", &[("XEZIM_BUF_COLLAPSE", "0")]);
    let (_, both) = run(
        MERGE_DESIGN,
        "both_on",
        &[("XEZIM_EDGE_MERGE", "2")],
    );
    assert!(!base.is_empty(), "baseline produced no program output");
    assert_eq!(base, both, "collapse+merge changed observable output");
}

#[test]
fn constant_range_copy_is_observationally_identical() {
    let (_, base) = run(RANGE_DESIGN, "range_base", &[]);
    let (text, lowered) = run(
        RANGE_DESIGN,
        "range_on",
        &[("XEZIM_RANGE_COPY", "1")],
    );
    assert_eq!(base.len(), 4, "baseline did not exercise every vector");
    assert!(
        text.contains("[RANGE-COPY] lowered 1 constant-range assignments"),
        "the range-copy path never fired:\n{text}"
    );
    assert_eq!(base, lowered, "range copying changed observable output");
}
