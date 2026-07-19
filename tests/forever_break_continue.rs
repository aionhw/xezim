//! IEEE 1800-2023 §9.3.3 — a `break`/`continue` inside a `forever` body that
//! can block (suspend) must be honoured when the process resumes, not silently
//! dropped while the loop re-arms for the next iteration.
//!
//! Root cause this guards: `run_process_stmts` (the suspend-aware statement
//! runner) ran a blocking `forever` via `exec_forever_sched`, which on
//! suspension re-appended a `Forever { body }` statement as the resume
//! continuation. On resume, the `Forever` arm called `exec_forever_sched`
//! again WITHOUT checking the `break_flag`/`continue_flag`/`return_flag` the
//! body had set — so a `break` was lost and the loop ran an extra iteration.
//! (Gating the `Forever` arm directly was rejected earlier: it fired on stale
//! flags before the body ran even once and deadlocked loops whose body only
//! decides to break after its first suspension. The fix is the `ForeverTail`
//! sentinel — first entry goes through `Forever` ungated; re-entry goes
//! through `ForeverTail`, which runs the loop-control gate. This mirrors the
//! `ForeachTail` design.)
//!
//! These tests pin the semantics: break exits, continue skips to the next
//! iteration, and a wait-driven forever breaks correctly under delta-sensitive
//! resumption (condition-waiter wakeup, not an edge).

use xezim::simulate;

fn lookup(sim: &xezim::compiler::Simulator, name: &str) -> u64 {
    sim.get_signal(name)
        .or_else(|| sim.get_signal(&format!("top.{}", name)))
        .unwrap_or_else(|| panic!("signal not found: {}", name))
        .to_u64()
        .unwrap_or_else(|| panic!("signal {} not u64-able", name))
}

/// A `break` inside an edge-triggered `forever` must terminate the loop. The
/// counter must reflect exactly the iteration that broke, not one more.
#[test]
fn forever_break_exits_loop() {
    let src = r#"
`timescale 1ns/1ns
module top;
  reg clk = 1'b0;
  integer count = 0;
  integer saw_break = 0;

  initial forever begin
    @(posedge clk);
    count = count + 1;
    if (count == 3) begin
      saw_break = 1;
      break;
    end
  end

  initial begin
    clk = 1'b0;
    #1 clk = 1'b1; #1 clk = 1'b0;
    #1 clk = 1'b1; #1 clk = 1'b0;
    #1 clk = 1'b1; #1 clk = 1'b0;
    #1 clk = 1'b1; #1 clk = 1'b0;  // a 4th edge the loop must NOT count
    #1;
    $finish;
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    // Correct (reference simulator): count==3, saw_break==1. Buggy behaviour
    // was count==4 — the break was dropped and the loop re-armed.
    assert_eq!(lookup(&sim, "count"), 3, "forever must stop at the break iteration");
    assert_eq!(lookup(&sim, "saw_break"), 1, "the break arm ran");
}

/// A `continue` inside a `forever` must skip the rest of the body for that
/// iteration only; `break` still terminates. Verifies both flags interact.
#[test]
fn forever_continue_skips_iteration() {
    let src = r#"
`timescale 1ns/1ns
module top;
  reg clk = 1'b0;
  integer i = 0;
  integer sum = 0;   // accumulates only ODD i (1,3,5) before break at i==6

  initial forever begin
    @(posedge clk);
    i = i + 1;
    if (i == 6) break;
    if (i[0] == 1'b0) continue;   // skip even i
    sum = sum + i;                 // odd i only
  end

  initial begin
    clk = 1'b0;
    forever #1 clk = ~clk;
  end
  initial #20 $finish;
endmodule
"#;
    let sim = simulate(src, 25).expect("simulate failed");
    // i ran 1..6 (break at 6); sum = 1+3+5 = 9.
    assert_eq!(lookup(&sim, "i"), 6, "break terminated at i==6");
    assert_eq!(lookup(&sim, "sum"), 9, "continue skipped even iterations");
}

/// A `wait`-driven `forever` (delta-sensitive: the body alternates
/// `wait(go); ...; wait(!go);`) must honour `break` when resumed via a
/// condition waiter rather than an edge.
#[test]
fn forever_wait_driven_break() {
    let src = r#"
`timescale 1ns/1ns
module top;
  reg go = 1'b0;
  integer n = 0;

  initial begin
    // five 0->1->0 pulses on go
    #2 go = 1'b1; #1 go = 1'b0;
    #2 go = 1'b1; #1 go = 1'b0;
    #2 go = 1'b1; #1 go = 1'b0;
    #2 go = 1'b1; #1 go = 1'b0;
    #2 go = 1'b1; #1 go = 1'b0;
    #1;
    $finish;
  end

  initial forever begin
    wait(go);
    n = n + 1;
    if (n == 5) break;
    wait(!go);
  end
endmodule
"#;
    let sim = simulate(src, 25).expect("simulate failed");
    // Five pulses → n reaches 5 and the forever breaks; without the break it
    // would keep waiting past the last pulse (no effect on n here, but the
    // break must not be dropped on the wait-resume path).
    assert_eq!(lookup(&sim, "n"), 5, "wait-driven forever honoured its break at n==5");
}
