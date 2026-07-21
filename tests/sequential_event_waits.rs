//! IEEE 1800-2023 §9.4.2 — a process that performs SEVERAL sequential event
//! controls (`@(posedge clk); …; @(posedge clk); …`) must resume on EACH edge.
//! A re-armed waiter (the continuation after the first `@` resumes) is a fresh
//! waiter whose `captured_prev` baseline is taken when it re-arms — and that
//! baseline must track the signal's running level so the NEXT qualifying edge
//! is detected, not stay frozen at the re-arm value.
//!
//! Root cause this guards: the event-waiter firing loop compared the signal's
//! current value against the `captured_prev` snapshot taken at arm time, but
//! never refreshed that snapshot when an edge did NOT fire. So a waiter that
//! re-armed while its signal sat at the edge-target level (clk=1 right after a
//! posedge resumed the process) had `captured_prev=1` (pb_one=true); when the
//! clock later went 1→0→1, the Posedge test `!pb_one && cb_one` stayed false
//! forever — the second (and every subsequent) posedge was lost and the process
//! stranded. This silently broke `bind`-monitor assertions and any testbench
//! pattern counting multiple clock edges in one `initial`.
//!
//! The fix refreshes a non-firing waiter's `captured_prev` to each sensitivity
//! signal's current value on every check, so a qualifying transition across
//! multiple time-steps is caught. The same-tick NBA-region semantics
//! `captured_prev` was introduced for are preserved: a waiter still does not
//! fire on an edge that completed before it armed (at arm time captured_prev
//! already equals current). These tests pin the cross-tick sequential case
//! (posedge, negedge, and a combined count) and guard the same-tick NBA case
//! against regression.

fn out(src: &str, prefix: &str) -> String {
    let sim = xezim::simulate(src, 10_000).expect("simulate");
    sim.output
        .iter()
        .filter(|o| o.message.starts_with(prefix))
        .map(|o| o.message.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Three sequential `@(posedge clk)` in one initial each resume at t=5/15/25.
#[test]
fn sequential_posedge_waits_each_resume() {
    let src = r#"
`timescale 1ns/1ns
module tb;
  bit clk = 0;
  int cnt = 0;
  initial begin
    @(posedge clk); cnt += 1;
    @(posedge clk); cnt += 10;
    @(posedge clk); cnt += 100;
    $display("DONE cnt=%0d", cnt);
  end
  always #5 clk = ~clk;
  initial #40 $finish;
endmodule
"#;
    assert_eq!(out(src, "DONE"), "DONE cnt=111");
}

/// Sequential `@(negedge clk)` waits resume on each falling edge (t=10, t=20).
#[test]
fn sequential_negedge_waits_each_resume() {
    let src = r#"
`timescale 1ns/1ns
module tb;
  bit clk = 0;
  int n = 0;
  initial begin
    @(negedge clk); n += 1;   // t=10
    @(negedge clk); n += 1;   // t=20
    @(negedge clk); n += 1;   // t=30
    $display("NEGS=%0d", n);
  end
  always #5 clk = ~clk;
  initial #40 $finish;
endmodule
"#;
    assert_eq!(out(src, "NEGS"), "NEGS=3");
}

/// A `bind`-monitor style process: an `initial` with interleaved
/// `@(posedge clk)` and `assert` statements. Pre-fix only the first assert
/// ran (the process stalled after the first `@`); post-fix all three run.
/// (bind_directive_basic.rs covers the full bind elaboration; this is the
/// minimal sequential-edge core.)
#[test]
fn interleaved_event_control_and_assertions() {
    let src = r#"
`timescale 1ns/1ns
module tb;
  bit clk = 0;
  int pc = 0;
  int hits = 0;
  initial begin
    @(posedge clk); assert (pc == 10) else $display("A1 fail %0d", pc); if (pc == 10) hits++;
    @(posedge clk); assert (pc == 20) else $display("A2 fail %0d", pc); if (pc == 20) hits++;
    @(posedge clk); assert (pc == 30) else $display("A3 fail %0d", pc); if (pc == 30) hits++;
    $display("HITS=%0d", hits);
  end
  always #5 clk = ~clk;
  initial begin
    pc = 10; #10; pc = 20; #10; pc = 30; #10;
  end
  initial #40 $finish;
endmodule
"#;
    // All three assertions run and pass → 3 hits, no "A* fail" messages.
    assert_eq!(out(src, "HITS"), "HITS=3");
    assert_eq!(out(src, "A"), "");
}

/// Same-tick NBA-region waiter (the scenario `captured_prev` was added for)
/// must STILL wake — regression guard for the refresh fix.
#[test]
fn same_tick_nba_waiter_still_wakes() {
    let src = r#"
`timescale 1ns/1ns
module tb;
  bit nba = 0;
  initial begin
    nba <= 1;       // NBA: commits this tick
    @(nba);         // armed while nba=0; must wake when NBA applies nba=1
    $display("WOKEN nba=%0d", nba);
  end
  initial #10 $finish;
endmodule
"#;
    assert_eq!(out(src, "WOKEN"), "WOKEN nba=1");
}
