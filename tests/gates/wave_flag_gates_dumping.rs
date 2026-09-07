//! §21.7: waveform dumping is opt-in at model-compile time via `--wave`.
//!
//! An active dump is not free — it forces loops that would otherwise compile
//! onto the AST path so every iteration is visible, and it builds a
//! per-traced-signal table. A run that never dumps should not pay for either,
//! and a design that happens to call `$dumpvars` should not silently start
//! writing a file.
//!
//! Without `--wave` the `$dump*` family warns once per task and is ignored;
//! the simulation itself must still run to completion, which is the property
//! that keeps this from breaking existing testbenches. `--fst`/`--xtrace` are
//! explicit dump requests and turn waveform support on by themselves.

use std::process::Command;

fn xezim() -> String {
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("xezim").to_string_lossy().into_owned()
}

const SRC: &str = r#"
module tb;
  logic clk = 0;
  logic [7:0] cnt = 0;
  always #5 clk = ~clk;
  always @(posedge clk) cnt <= cnt + 1;
  initial begin
    $dumpfile("{VCD}");
    $dumpvars(0, tb);
    repeat (10) @(posedge clk);
    $display("CNT=%0d", cnt);
    $finish;
  end
endmodule
"#;

/// Returns (combined output, whether the VCD file exists).
fn run(tag: &str, extra: &[&str]) -> (String, bool) {
    let dir = std::env::temp_dir().join(format!("xezim_wave_gate_{tag}"));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let vcd = dir.join("out.vcd");
    let _ = std::fs::remove_file(&vcd);
    let sv = dir.join("dut.sv");
    std::fs::write(&sv, SRC.replace("{VCD}", vcd.to_str().unwrap())).expect("write");

    let mut cmd = Command::new(xezim());
    cmd.current_dir(&dir)
        .env("XEZIM_NO_CACHE", "1")
        .arg("--simulate")
        .arg("--max-time")
        .arg("200")
        .arg("-s")
        .arg("tb");
    for a in extra {
        cmd.arg(a);
    }
    let out = cmd.arg(&sv).output().expect("run xezim");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (text, vcd.is_file())
}

#[test]
fn without_wave_the_dump_tasks_are_ignored_but_the_run_still_completes() {
    let (text, wrote) = run("off", &[]);
    assert!(
        !wrote,
        "a VCD was written without --wave:\n{text}"
    );
    assert!(
        text.contains("CNT=9"),
        "ignoring the dump must not stop the simulation:\n{text}"
    );
    // One warning per task, so the user knows why there is no waveform.
    assert!(
        text.contains("$dumpfile ignored") && text.contains("$dumpvars ignored"),
        "expected a one-time note per ignored dump task:\n{text}"
    );
}

#[test]
fn with_wave_the_vcd_is_written() {
    let (text, wrote) = run("on", &["--wave"]);
    assert!(wrote, "--wave did not produce a VCD:\n{text}");
    assert!(
        !text.contains("ignored"),
        "--wave must not warn about ignored dump tasks:\n{text}"
    );
    assert!(text.contains("CNT=9"), "simulation did not complete:\n{text}");
}

#[test]
fn fst_implies_wave_without_an_explicit_flag() {
    // An explicit dump request should not also demand --wave.
    let dir = std::env::temp_dir().join("xezim_wave_gate_fst");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let fst = dir.join("out.fst");
    let _ = std::fs::remove_file(&fst);
    let sv = dir.join("dut.sv");
    // No $dump* at all: the FST comes from the CLI alone.
    std::fs::write(
        &sv,
        r#"
module tb;
  logic clk = 0;
  logic [7:0] cnt = 0;
  always #5 clk = ~clk;
  always @(posedge clk) cnt <= cnt + 1;
  initial begin repeat (10) @(posedge clk); $display("CNT=%0d", cnt); $finish; end
endmodule
"#,
    )
    .expect("write");
    let out = Command::new(xezim())
        .current_dir(&dir)
        .env("XEZIM_NO_CACHE", "1")
        .args(["--simulate", "--max-time", "200", "-s", "tb", "--fst"])
        .arg(&fst)
        .arg(&sv)
        .output()
        .expect("run xezim");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(fst.is_file(), "--fst alone did not produce a dump:\n{text}");
}
