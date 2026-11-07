//! #938: the dead-clock watchdog fires only when the awaited signal has no
//! live driver. A signal with several drivers is live when any one of them
//! is; the first driver found used to decide alone, so a flop on a dead clock
//! declared before the live one made a healthy wait look dead.

use std::process::Command;

fn xezim_bin() -> std::path::PathBuf {
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("xezim")
}

const TWO_DRIVERS_SRC: &str = r#"
module tb;
  reg clk = 0;
  always #5 clk = ~clk;
  wire clk_ref;                     // dead: undriven net
  reg [31:0] churn = 0;
  always @(posedge clk) churn <= churn + 1;
  reg ctrl = 0;
  reg [31:0] count = 0;
  always @(posedge clk_ref) ctrl <= 1'b0;   // driver on the dead clock, found first
  always @(posedge clk) begin
    if (count < 32'd2000) count <= count + 1;
    else ctrl <= 1'b1;
  end
  initial begin
    @(posedge ctrl);
    $display("power-up complete");
    $finish;
  end
endmodule
"#;

#[test]
fn a_signal_with_one_live_driver_among_several_is_not_dead() {
    let dir = std::env::temp_dir().join("xezim_review_watchdog_938");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let sv = dir.join("two_drivers.sv");
    std::fs::write(&sv, TWO_DRIVERS_SRC).expect("write sv");
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
        "a live driver must keep the watchdog quiet; stderr:\n{stderr}"
    );
    assert!(stdout.contains("power-up complete"), "stdout:\n{stdout}");
}
