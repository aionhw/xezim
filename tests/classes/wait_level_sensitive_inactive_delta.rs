//! IEEE 1800-2023 §9.7.4 / §4.5: `wait(cond)` is LEVEL-sensitive — a process
//! resumes the MOMENT its condition is true, including in the SAME timestep
//! via a delta-cycle handoff. If a blocking write makes the condition true and
//! then a `#0`-resumed continuation OVERWRITES the watched value within the
//! same timestep, the waiter must observe the intermediate value and proceed on
//! it — not skip straight to the clobbered one.
//!
//! REGRESSION: xezim parked `wait(expr)` on non-signal state in a deferred
//! end-of-tick fixpoint, which ran AFTER the INACTIVE (`#0`) region. So in
//!
//!     x = 1;  #0;  x = 0;      // peer process
//!     wait(x != 0)             // waiter, must see x==1
//!
//! the waiter woke after the `#0` had already overwritten x to 0 and observed
//! the wrong value. The UVM TLM2 nonblocking (AT) passthrough handshake trips
//! this: the slave's `wait(state != BEGIN_RESP)` is released by the master
//! sending END_RESP, but the master's trailing `#0` immediately re-drives a
//! new request, so the slave saw the next request's phase instead of END_RESP
//! and never advanced its state machine (its completed-transaction count
//! stayed 0 while the master's climbed — a hard UVM_ERROR mismatch).
use std::process::Command;

fn xezim() -> String {
    // Resolve the sibling CLI binary from the test binary's own location so
    // this works for both debug and release profiles.
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("xezim").to_string_lossy().into_owned()
}

fn run(src: &str, tag: &str) -> String {
    let path = format!("/tmp/wait_inactive_{tag}.sv");
    std::fs::write(&path, src).unwrap();
    let out = Command::new(xezim())
        .args(["--simulate", "-s", "top", &path])
        .output()
        .expect("run xezim");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A waiter on `st` must wake at the delta where `st` becomes 4, BEFORE the
/// writer's `#0` (inactive region) overwrites it to 1 in the same timestep.
const WAIT_DELTA: &str = r#"module top;
  int st = 3;
  task sla();
    wait(st != 3);
    if (st == 4) $display("RESULT PASS");
    else         $display("RESULT FAIL st=%0d", st);
  endtask
  task mst();
    #8;
    st = 4;   // first change: wait(st!=3) must wake here
    #0;       // yield one delta (inactive region)
    st = 1;   // clobber within same timestep
  endtask
  initial begin
    fork sla(); mst(); join_none
    #20;
    $finish;
  end
endmodule
"#;

#[test]
fn wait_wakes_on_intermediate_delta_value_before_inactive_clobber() {
    let out = run(WAIT_DELTA, "delta");
    assert!(
        out.contains("RESULT PASS"),
        "a level-sensitive wait must observe the intermediate value written\n\
         before a #0-resumed overwrite (not the clobbered one)\noutput:\n{out}"
    );
}