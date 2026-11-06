//! Dead-clock watchdog: a process parked on a clock/reset that never changes
//! value while the design keeps firing edges is the signature of a dead-clock
//! hang (undriven net / unresolved cell / ungenerated behavioral PLL VCO). The
//! watchdog detects "no functional progress" and escalates to a prominent,
//! actionable diagnostic — abort mode fails fast (exit 3) instead of grinding.
//!
//! Uses a subprocess so the XEZIM_STUCK_CLOCK* env vars are isolated from the
//! rest of the (parallel) test run.

use std::process::Command;

fn xezim_bin() -> std::path::PathBuf {
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("xezim")
}

// A free-running clock churns edges while a process waits forever on a dead
// reference clock that never toggles. The dead clock is an undriven NET —
// the signature of a dropped instance / ungenerated clock.
const CHURN_SRC: &str = r#"
module tb;
  reg clk = 0;
  wire clk_ref;                  // dead: undriven net (implicit 1-bit) -> Z
  reg [7:0] a, b, c, d;
  reg [31:0] churn = 0;
  always #5 clk = ~clk;
  always @(posedge clk) begin a <= a + 1; b <= a; c <= b; d <= c; end
  always @(negedge clk) churn <= churn + a + b + c + d;
  initial @(posedge clk_ref) $display("ERROR: dead clk_ref woke");
  initial begin #500000 $display("reached end"); $finish; end
endmodule
"#;

// A control signal legitimately held low during a power-up: an always_ff
// asserts it only after a slow counter reaches its limit, while a free-running
// clock keeps churning edges. The awaited signal has a LIVE driver (the flop is
// clocked by the live `clk`), so this is a slow-but-healthy wait, not a dead
// clock — the watchdog must not trip.
const POWERUP_SRC: &str = r#"
module tb;
  reg clk = 0;
  always #5 clk = ~clk;            // free-running clock (churns edges)
  reg [31:0] churn = 0;
  always @(posedge clk) churn <= churn + 1;
  reg ctrl = 0;                    // driven by the always_ff below
  reg [31:0] count = 0;
  always @(posedge clk) begin
    if (count < 32'd2000) count <= count + 1;
    else ctrl <= 1'b1;             // asserted only after 2000 cycles
  end
  initial begin
    @(posedge ctrl);
    $display("power-up complete");
    $finish;
  end
endmodule
"#;

// A reset `reg` driven once by an `initial` block and held stable forever is
// NORMAL (a testbench reset/constant), not a dead clock. It has no clock
// generator, edge block or continuous assign, but it is a VARIABLE — only a
// NET with no driver is the dead-clock signature. The watchdog must not trip.
const RESET_SRC: &str = r#"
module tb;
  reg clk = 0;
  always #5 clk = ~clk;            // free-running clock (churns edges)
  reg [31:0] churn = 0;
  always @(posedge clk) churn <= churn + 1;
  reg rst_n;
  initial begin
    rst_n = 1'b1;                  // held stable forever (a constant reset)
    #500000 $display("reached end");
    $finish;
  end
  initial @(negedge rst_n) $display("ERROR: rst_n dropped");
endmodule
"#;

fn write_fixture(name: &str, src: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("xezim_dead_clock_wd");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let sv = dir.join(name);
    std::fs::write(&sv, src).expect("write sv");
    sv
}

#[test]
fn abort_mode_fails_fast_with_diagnostic() {
    let sv = write_fixture("churn_abort.sv", CHURN_SRC);
    let out = Command::new(xezim_bin())
        .env("XEZIM_STUCK_CLOCK", "abort")
        .env("XEZIM_STUCK_CLOCK_TICKS", "1000")
        .env("XEZIM_STUCK_CLOCK_EDGES", "1000")
        .env("XEZIM_STUCK_CLOCK_WALL", "0")
        .env("XEZIM_NO_CACHE", "1")
        .args(["-s", "tb"])
        .arg(&sv)
        .output()
        .expect("run xezim");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("DEAD-CLOCK WATCHDOG") && stderr.contains("clk_ref"),
        "watchdog banner missing; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("parked on 'clk_ref'"),
        "watchdog should name the dead signal; stderr:\n{stderr}"
    );
    // Abort => non-zero exit (3) so CI/regressions fail fast.
    assert_eq!(
        out.status.code(),
        Some(3),
        "abort mode must exit 3; got {:?}\nstderr:\n{stderr}",
        out.status.code()
    );
    // Must NOT have reached the end (it bailed early).
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("reached end"),
        "abort should stop before the run completes; stdout:\n{stdout}"
    );
}

#[test]
fn off_mode_is_silent() {
    let sv = write_fixture("churn_off.sv", CHURN_SRC);
    let out = Command::new(xezim_bin())
        .env("XEZIM_STUCK_CLOCK", "off")
        .env("XEZIM_STUCK_CLOCK_TICKS", "1000")
        .env("XEZIM_STUCK_CLOCK_EDGES", "1000")
        .env("XEZIM_STUCK_CLOCK_WALL", "0")
        .env("XEZIM_NO_CACHE", "1")
        .args(["-s", "tb"])
        .arg(&sv)
        .output()
        .expect("run xezim");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("DEAD-CLOCK WATCHDOG"),
        "off mode must not emit the watchdog banner; stderr:\n{stderr}"
    );
}

#[test]
fn driven_signal_held_during_powerup_is_not_dead() {
    let sv = write_fixture("powerup.sv", POWERUP_SRC);
    let out = Command::new(xezim_bin())
        .env("XEZIM_STUCK_CLOCK", "abort")
        .env("XEZIM_STUCK_CLOCK_TICKS", "1000")
        .env("XEZIM_STUCK_CLOCK_EDGES", "1000")
        .env("XEZIM_STUCK_CLOCK_WALL", "0")
        .env("XEZIM_NO_CACHE", "1")
        .args(["-s", "tb"])
        .arg(&sv)
        .output()
        .expect("run xezim");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stderr.contains("DEAD-CLOCK WATCHDOG"),
        "a driven power-up signal must not trip the watchdog; stderr:\n{stderr}"
    );
    assert!(
        stdout.contains("power-up complete"),
        "the waiting process should wake when power-up completes; stdout:\n{stdout}"
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "the run should finish cleanly, not abort; got {:?}",
        out.status.code()
    );
}

#[test]
fn stable_reset_variable_is_not_dead() {
    let sv = write_fixture("reset.sv", RESET_SRC);
    let out = Command::new(xezim_bin())
        .env("XEZIM_STUCK_CLOCK", "abort")
        .env("XEZIM_STUCK_CLOCK_TICKS", "1000")
        .env("XEZIM_STUCK_CLOCK_EDGES", "1000")
        .env("XEZIM_STUCK_CLOCK_WALL", "0")
        .env("XEZIM_NO_CACHE", "1")
        .args(["-s", "tb"])
        .arg(&sv)
        .output()
        .expect("run xezim");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stderr.contains("DEAD-CLOCK WATCHDOG"),
        "a stable reset variable must not trip the watchdog; stderr:\n{stderr}"
    );
    assert!(
        stdout.contains("reached end"),
        "the run should reach its end; stdout:\n{stdout}"
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "the run should finish cleanly, not abort; got {:?}",
        out.status.code()
    );
}
