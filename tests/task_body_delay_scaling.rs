//! §3.14.3 — a `#d` inside a TASK body counts the module's timeunit, exactly
//! like a delay written directly in an initial/always block. Task bodies were
//! skipped by the elaborator's delay pre-scaling pass, so in a `1ns/1ps`
//! module a task's `#1` advanced 1 tick (1 ps) instead of 1 ns — clock-pulse
//! tasks ran 1000x too fast and testbench timelines diverged from the LRM.

use xezim::simulate;

#[test]
fn task_body_delay_scales_by_module_timeunit() {
    let src = r#"
`timescale 1ns/1ps
module tb;
  reg clk;
  task automatic pulse; begin #1 clk = 1'b1; #1 clk = 1'b0; end endtask
  initial begin
    clk = 1'b0;
    #1;
    pulse();
    repeat (3) pulse();
    // 1 + 4 tasks x 2ns = 9ns
    $display("T=%0d", $time);
    if ($realtime == 9.0) $display("TEST_PASS"); else $display("TEST_FAIL rt=%f", $realtime);
    $finish;
  end
endmodule
"#;
    let out: Vec<String> = simulate(src, 100_000)
        .expect("sim")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect();
    assert!(
        out.iter().any(|l| l == "TEST_PASS"),
        "task-body #1 not scaled to timeunit: {:?}",
        out
    );
}
