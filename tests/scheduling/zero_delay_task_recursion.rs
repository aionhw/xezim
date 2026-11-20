//! A task that can wait (`@`, `#`, `wait`) runs inside its caller's
//! activation and resumes the caller's remaining statements from inside its
//! own, so a call chain that never actually waits nests one native frame per
//! call. A task that calls itself at one time step without taking its wait is
//! a zero-delay loop: simulated time can never advance (§4.4, §9.2). xezim
//! overflowed its native stack and aborted with SIGABRT, naming nothing in
//! the design, and `XEZIM_STALL_LIMIT` did not apply. It now reports the
//! process and its source and stops with a failing status, at the stall
//! limit or before the stack runs out.

use std::path::PathBuf;
use std::process::Command;

const SRC: &str = r#"
class C;
  int n;
  event e;
  task run();
    if (n < 0) @(e);
    n++;
    run();
  endtask
endclass
module t;
  C c = new;
  initial c.run();
  initial #10 $display("done");
endmodule
"#;

fn run(case: &str, env: &[(&str, &str)]) -> (std::process::ExitStatus, String) {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("zero_delay_task_recursion")
        .join(case);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.sv");
    std::fs::write(&path, SRC).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_xezim"));
    cmd.args(["--no-cache", "-s", "t"]).arg(&path);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().unwrap();
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status, text)
}

#[track_caller]
fn assert_reported(status: std::process::ExitStatus, text: &str) {
    assert_eq!(status.code(), Some(1), "status {status:?}:\n{text}");
    assert!(!text.contains("overflowed its stack"), "{text}");
    assert!(!text.contains("done"), "{text}");
    for needle in [
        "simulation STALLED at time 0 — a process nested",
        "initial block at",
        "c.run();",
        "zero-delay loop",
    ] {
        assert!(text.contains(needle), "no `{needle}` in:\n{text}");
    }
}

/// `XEZIM_STALL_LIMIT` bounds the nesting like the other stall checks.
#[test]
fn zero_delay_task_recursion_stops_at_the_stall_limit() {
    let (status, text) = run("limit", &[("XEZIM_STALL_LIMIT", "500")]);
    assert_reported(status, &text);
    assert!(text.contains("nested 500 task calls"), "{text}");
}

/// On a small native stack the check stops the run before the stack runs out.
/// The stack bounds are read on Linux and macOS only (`thread_stack_bounds`);
/// elsewhere the call limit is the only check. The call limit is turned off
/// (`XEZIM_STALL_LIMIT=0`), so a limit set in the environment cannot stop the
/// run first and only the stack check is tested.
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn zero_delay_task_recursion_stops_before_the_stack_runs_out() {
    let (status, text) = run(
        "stack",
        &[("XEZIM_STACK_MB", "64"), ("XEZIM_STALL_LIMIT", "0")],
    );
    assert_reported(status, &text);
    assert!(text.contains("XEZIM_STACK_MB"), "{text}");
}
