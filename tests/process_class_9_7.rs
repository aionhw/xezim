//! IEEE 1800-2023 §9.7 fine-grain process control — `process` built-in class.
//!
//! Pure-SystemVerilog tests for `process::self()`, `status()`, `kill()`,
//! `await()`, `suspend()`, and `resume()`. Each test is a minimal program
//! whose expected output was cross-checked against reference simulators
//! (golden outputs in `tests/lrm_9_7/ref/`).
//!
//! No UVM library, no DPI — runs in-process via `simulate`.

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

// ── status(): RUNNING vs WAITING ──────────────────────────────────────

const STATUS_RUNNING_SRC: &str = r#"
module top;
  process job;
  int seen_waiting;
  initial begin
    fork
      begin
        job = process::self();
        seen_waiting = job.status();   // executing now -> RUNNING(1)
        #50;                            // now blocks -> WAITING(2)
      end
    join_none
    #10;
    $display("RESULT blocking_status=%0d", job.status());  // 2 WAITING
    $display("RESULT running_status=%0d", seen_waiting);   // 1 RUNNING
    #100;
  end
endmodule
"#;

#[test]
fn status_running_and_waiting() {
    let sim = simulate(STATUS_RUNNING_SRC, 1000).expect("simulate failed");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "RESULT blocking_status=2"),
        "process blocked in #50 should report WAITING(2); got {:?}",
        msgs
    );
    assert!(
        msgs.iter().any(|m| m == "RESULT running_status=1"),
        "process actively executing should report RUNNING(1); got {:?}",
        msgs
    );
}

// ── status(): FINISHED after completion ───────────────────────────────

const STATUS_FINISHED_SRC: &str = r#"
module top;
  process job;
  initial begin
    fork
      begin job = process::self(); #5; end
    join_none
    #20;
    $display("RESULT finished_status=%0d", job.status());  // 0 FINISHED
  end
endmodule
"#;

#[test]
fn status_finished() {
    let sim = simulate(STATUS_FINISHED_SRC, 1000).expect("simulate failed");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "RESULT finished_status=0"),
        "completed process should report FINISHED(0); got {:?}",
        msgs
    );
}

// ── kill(): terminates a process before it completes ──────────────────

const KILL_SRC: &str = r#"
module top;
  process job;
  initial begin
    fork
      begin
        job = process::self();
        #100;
        $display("RESULT VICTIM_RAN");   // must NOT print
      end
    join_none
    #10;
    job.kill();
    $display("RESULT killed_status=%0d", job.status());  // 4 KILLED
    #100;
    $display("RESULT done");
  end
endmodule
"#;

#[test]
fn kill_terminates_process() {
    let sim = simulate(KILL_SRC, 1000).expect("simulate failed");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "RESULT killed_status=4"),
        "killed process should report KILLED(4); got {:?}",
        msgs
    );
    assert!(
        !msgs.iter().any(|m| m == "RESULT VICTIM_RAN"),
        "killed process must not run its continuation; got {:?}",
        msgs
    );
}

// ── await(): blocks until the target process terminates ───────────────

const AWAIT_SRC: &str = r#"
module top;
  process job;
  initial begin
    fork
      begin job = process::self(); #30; end
    join_none
    #0;                                // let the forked child run first
    wait(job != null);                 // guard against null handle
    job.await();                       // blocks until child finishes at #30
    $display("RESULT await_done_at=%0t", $time);
    $display("RESULT status_after=%0d", job.status());  // 0 FINISHED
  end
endmodule
"#;

#[test]
fn await_blocks_until_termination() {
    let sim = simulate(AWAIT_SRC, 1000).expect("simulate failed");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "RESULT await_done_at=30"),
        "await() should resume at t=30 when child finishes; got {:?}",
        msgs
    );
    assert!(
        msgs.iter().any(|m| m == "RESULT status_after=0"),
        "after await, status should be FINISHED(0); got {:?}",
        msgs
    );
}

// ── suspend()/resume(): pauses then restarts a blocked process ────────

const SUSPEND_RESUME_SRC: &str = r#"
module top;
  process job;
  initial begin
    fork
      begin
        job = process::self();
        #10;  $display("RESULT step_a_at=%0t", $time);
        #40;  $display("RESULT step_b_at=%0t", $time);
      end
    join_none
    #5;
    job.suspend();
    $display("RESULT suspended_at=%0t status=%0d", $time, job.status());
    #50;
    job.resume();
    $display("RESULT resumed_at=%0t", $time);
    #100;
  end
endmodule
"#;

#[test]
fn suspend_then_resume() {
    let sim = simulate(SUSPEND_RESUME_SRC, 1000).expect("simulate failed");
    let msgs = messages(&sim);
    // After suspend, status is SUSPENDED(3).
    assert!(
        msgs.iter().any(|m| m == "RESULT suspended_at=5 status=3"),
        "suspended process should report SUSPENDED(3); got {:?}",
        msgs
    );
    // After resume at t=55, the original #10 delay (from t=0, expiry t=10)
    // has transpired, so step_a fires immediately at t=55.
    assert!(
        msgs.iter().any(|m| m == "RESULT step_a_at=55"),
        "resumed process should continue at t=55 (original delay transpired); got {:?}",
        msgs
    );
    // step_b is #40 after step_a: t=55+40=95.
    assert!(
        msgs.iter().any(|m| m == "RESULT step_b_at=95"),
        "second delay after resume should fire at t=95; got {:?}",
        msgs
    );
}

// ── await() inside a forever loop body must block, not spin ─────────────
// A `process_handle.await()` call nested in a `forever` body is routed
// through `exec_forever_sched` → `run_process_stmts` (because `await` is
// recognized as blocking). Without the detection, `exec_forever_sched`
// ran the await via the synchronous `exec_statement`, which treated it as a
// void method call returning immediately — the forever body busy-spun,
// repeatedly re-checking state that hadn't changed.

const AWAIT_IN_FOREVER_SRC: &str = r#"
module top;
  process captured_proc;
  int target_started;
  int await_loops;

  // The "target" task: captures its own process handle, signals it started,
  // then blocks forever (never terminates during the test window).
  task automatic the_target();
    captured_proc = process::self();
    target_started = 1;
    wait (0);
  endtask

  // The "monitor": forever awaits the target. If await() works, the loop
  // body never completes a full iteration (the target never terminates).
  task automatic the_monitor();
    forever begin
      captured_proc.await();
      await_loops = await_loops + 1;
    end
  endtask

  initial begin
    fork the_target(); join_none
    wait (target_started == 1);
    fork the_monitor(); join_none
    #5;
    $display("RESULT await_loops=%0d", await_loops);
    $finish;
  end
endmodule
"#;

#[test]
fn await_in_forever_body_blocks() {
    let sim = simulate(AWAIT_IN_FOREVER_SRC, 1000).expect("simulate failed");
    let msgs = messages(&sim);
    // The target is permanently blocked (wait(0)), so await() must never
    // return — await_loops stays 0. Without the fix, await() fell through to
    // exec_statement (treated as a void call), the forever busy-spun, and
    // await_loops ran to ~100000.
    assert!(
        msgs.iter().any(|m| m == "RESULT await_loops=0"),
        "await() inside a forever body must block while the target is alive; got {:?}",
        msgs
    );
}
