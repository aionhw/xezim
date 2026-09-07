//! IEEE 1800-2023 §4.5: within each timestep, the simulation cycle processes
//! Active-region events, followed by Inactive-region (`#0` delay) events, then
//! NBA events, then Reactive-region events.
//!
//! When an active-region write satisfies a level-sensitive `wait(cond)`, that
//! condition waiter runs in the active region. However, if already-pending `#0`
//! continuations exist in the Inactive region, a multi-generation cascade of
//! newly-awakened condition waiters must not loop indefinitely and starve the
//! Inactive region; `#0` continuations must be promoted and given their turn
//! in each delta cycle before subsequent generations of condition-waiter cascades.
use std::process::Command;

fn xezim() -> String {
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("xezim").to_string_lossy().into_owned()
}

fn run(src: &str, tag: &str) -> String {
    let path = format!("/tmp/cond_inactive_{tag}.sv");
    std::fs::write(&path, src).unwrap();
    let out = Command::new(xezim())
        .args(["--simulate", "-s", "top", &path])
        .output()
        .expect("run xezim");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

const SV_SRC: &str = r#"module top;
  int stage = 0;
  int queue[$];

  task automatic background_worker();
    while (1) begin
      wait (queue.size() != 0);
      void'(queue.pop_front());
      // When 1st generation runs, push 2nd generation
      if (stage == 0) begin
        queue.push_back(1);
      end
      #0;
    end
  endtask

  initial begin
    fork
      background_worker();
    join_none

    queue.push_back(0); // wake 1st generation of worker
    #0;                 // Inactive region continuation
    stage = 1;          // Must run before 2nd generation
    $display("STAGE_COMPLETED stage=%0d", stage);
    #1;
    $finish;
  end
endmodule
"#;

#[test]
fn condition_waiter_cascade_yields_to_inactive_zero_delay() {
    let out = run(SV_SRC, "cond_yield");
    assert!(
        out.contains("STAGE_COMPLETED stage=1"),
        "expected #0 continuation to run before 2nd generation condition waiter, got:\n{out}"
    );
}
