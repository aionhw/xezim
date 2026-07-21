//! The legacy adaptive event-edge path must accelerate continuously changing
//! flops, then return to snapshot comparisons when inputs become stable.

use std::path::PathBuf;
use std::process::Command;

fn xezim_bin() -> PathBuf {
    let mut path = std::env::current_exe().expect("current_exe");
    path.pop();
    if path.ends_with("deps") {
        path.pop();
    }
    path.join("xezim")
}

fn metric(output: &str, key: &str) -> u64 {
    let start = output
        .find(key)
        .unwrap_or_else(|| panic!("missing metric {key:?} in:\n{output}"))
        + key.len();
    output[start..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .unwrap_or_else(|_| panic!("invalid metric {key:?} in:\n{output}"))
}

#[test]
fn changing_flop_uses_epochs_then_stable_flop_skips() {
    let dir = std::env::temp_dir().join(format!("xezim_adaptive_edge_skip_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp directory");
    let source = dir.join("adaptive_edge_skip.sv");
    std::fs::write(
        &source,
        r#"module top;
  reg clk = 0;
  reg d = 0;
  reg q = 0;

  always #1 clk = ~clk;
  always @(posedge clk) q <= d;

  initial begin
    repeat (40) begin
      @(negedge clk);
      d = ~d;
    end
    repeat (40) @(negedge clk);
    @(posedge clk);
    if (q === d) $display("ADAPTIVE_PASS");
    else $display("ADAPTIVE_FAIL q=%b d=%b", q, d);
    $finish;
  end
endmodule
"#,
    )
    .expect("write test source");

    let result = Command::new(xezim_bin())
        // ARMED filtering supersedes epochs by default. Keep this test focused
        // on the fallback path retained for XEZIM_ARMED_EDGE=0.
        .env("XEZIM_ARMED_EDGE", "0")
        .args(["--simulate", "-s", "top"])
        .arg(&source)
        .output()
        .expect("run xezim");
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );

    assert!(result.status.success(), "xezim failed:\n{output}");
    assert!(
        output.contains("ADAPTIVE_PASS"),
        "wrong simulation result:\n{output}"
    );
    assert!(
        metric(&output, "epoch-fast-exec=") > 0,
        "changing inputs never activated the epoch fast path:\n{output}"
    );
    assert!(
        metric(&output, "snapshot-checks=") > 0,
        "stable inputs never returned to snapshot checks:\n{output}"
    );
    assert!(
        metric(&output, "would-skip ") > 0,
        "stable flop firings were not skipped:\n{output}"
    );
}
