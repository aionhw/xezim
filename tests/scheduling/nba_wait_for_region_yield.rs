//! IEEE 1800-2023 §4.5: within a simulation timestep, Active region events
//! and Inactive region (`#0` delay) events run before the NBA region commits.
//!
//! A common UVM synchronization idiom (`uvm_wait_for_nba_region`) posts an NBA
//! to a procedural variable and waits on it (`nba <= next_nba; @(nba)`).
//! The intention is to yield the calling process across all delta cycles (`#0`)
//! until the NBA region commits.
//!
//! REGRESSION: xezim previously rescheduled `@(local_var)` directly into the
//! active event queue at `self.time`, causing it to resume in the active region
//! before sibling processes' `#0` continuations had finished, and advancing
//! time before pending NBAs committed.
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
    let path = format!("/tmp/nba_yield_{tag}.sv");
    std::fs::write(&path, src).unwrap();
    let out = Command::new(xezim())
        .args(["--simulate", "-s", "top", &path])
        .output()
        .expect("run xezim");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

const SV_SRC: &str = r#"module top;
  // wait_nba posts an NBA and waits on it:
  task automatic wait_nba();
    static int nba = 0;
    static int next_nba = 0;
    next_nba++;
    nba <= next_nba;
    @(nba);
    $display("WOKE_AFTER_NBA");
  endtask

  initial begin
    #1;
    fork
      wait_nba();
      begin
        // Multiple #0 delays must all complete before the NBA region wakes wait_nba
        repeat (10) #0;
        $display("ZERO_DELAY_DONE");
      end
    join
    $finish;
  end
endmodule
"#;

#[test]
fn nba_wait_yields_until_inactive_deltas_complete() {
    let out = run(SV_SRC, "test");
    let zero_pos = out.find("ZERO_DELAY_DONE").expect("ZERO_DELAY_DONE printed");
    let nba_pos = out.find("WOKE_AFTER_NBA").expect("WOKE_AFTER_NBA printed");
    assert!(
        zero_pos < nba_pos,
        "expected ZERO_DELAY_DONE before WOKE_AFTER_NBA, got:\n{out}"
    );
}
