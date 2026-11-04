//! A run that ends on a run-time error xezim reports exits nonzero, as one
//! that ends on `$fatal` does (IEEE 1800-2023 §20.10: a fatal error
//! terminates the simulation with an error code). Each of these printed its
//! error, stopped the simulation and exited 0, so a script or harness that
//! checks the exit status took the run for a pass:
//!
//! - reading a property through a null handle in a process body, calling a
//!   method on a null handle in a loop, and waiting on a mailbox that was
//!   never constructed (§8.4: an access through a null handle is illegal);
//! - a loop or delta cycle that can never let time advance: a `forever`
//!   with no timing control, `#0` re-arming, an edge ping-pong between two
//!   blocks, a nonblocking-assignment cascade (§4.4, §9.4);
//! - `$sdf_annotate` on a file it cannot read (Clause 32).
//!
//! A run that ends at `$finish` or `--max-time`, or with only an `$error`
//! and no `--error-exit`, still exits 0.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Run `src` in a fresh directory; return the exit code and the output. A
/// run that is still going after a minute (a livelock check that no longer
/// fires) is killed, and the test fails with what it printed.
fn run(tag: &str, src: &str, args: &[&str]) -> (Option<i32>, String) {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("runtime_error_exit_{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("t.sv"), src).unwrap();
    let log = std::fs::File::create(dir.join("out.txt")).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .current_dir(&dir)
        // Stall limits small enough that the livelocks trip in milliseconds.
        .env("XEZIM_STALL_LIMIT", "2000")
        .args(args)
        .arg("t.sv")
        .stdout(log.try_clone().unwrap())
        .stderr(log)
        .stdin(Stdio::null())
        .spawn()
        .expect("run xezim");
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break Some(status);
        }
        if start.elapsed() > Duration::from_secs(60) {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let text = std::fs::read_to_string(dir.join("out.txt")).unwrap_or_default();
    let status = status.unwrap_or_else(|| panic!("{tag}: still running after 60 s:\n{text}"));
    (status.code(), text)
}

/// The run reports `message`, stops before `post`, and exits 1.
fn fails(tag: &str, src: &str, message: &str) {
    let (code, text) = run(tag, src, &[]);
    assert!(text.contains(message), "{tag}: no `{message}` in:\n{text}");
    assert!(!text.contains("post"), "{tag}: the run went on:\n{text}");
    assert_eq!(code, Some(1), "{tag}: exit status\n{text}");
}

#[test]
fn null_handle_read_in_a_process_exits_nonzero() {
    fails(
        "null_read",
        "class C;\n  int v = 7;\nendclass\nmodule t;\n  C c;\n  initial begin\n    $display(\"v=%0d\", c.v);\n    $display(\"post\");\n  end\nendmodule\n",
        "null object dereference: 'c.v'",
    );
}

#[test]
fn null_handle_call_in_a_loop_exits_nonzero() {
    fails(
        "null_call",
        "class C;\n  int n;\n  task go();\n    n++;\n    #1;\n  endtask\nendclass\nmodule t;\n  C c;\n  initial forever c.go();\n  initial #5 $display(\"post\");\nendmodule\n",
        "null receiver at time 0",
    );
}

#[test]
fn null_mailbox_wait_exits_nonzero() {
    fails(
        "null_mailbox",
        "module t;\n  mailbox #(int) m;\n  int v;\n  initial begin\n    m.get(v);\n    $display(\"post\");\n  end\nendmodule\n",
        "null mailbox handle at time 0",
    );
}

#[test]
fn livelocks_exit_nonzero() {
    // A `forever` with no timing control.
    fails(
        "forever",
        "module t;\n  int n = 0;\n  initial forever n++;\n  initial #1 $display(\"post\");\nendmodule\n",
        "body with no timing control ran 2000 times",
    );
    // A process that re-arms itself with `#0`, with nothing scheduled after
    // it to defer to.
    fails(
        "zero_delay",
        "module t;\n  int n = 0;\n  initial forever begin\n    #0;\n    n++;\n  end\nendmodule\n",
        "process activations at this timestamp",
    );
    // Two blocks that trigger each other at one time.
    fails(
        "ping_pong",
        "module t;\n  reg a = 0, b = 0;\n  always @(a) b = ~b;\n  always @(b) a = ~a;\n  initial #0 a = 1;\n  initial #10 $display(\"post\");\nendmodule\n",
        "edge-cascade delta cycles",
    );
    // A nonblocking assignment that retriggers its own block at one time.
    fails(
        "nba_cascade",
        "module t;\n  reg c = 0;\n  always @(c) c <= ~c;\n  initial #1 c = 1;\n  initial #10 $display(\"post\");\nendmodule\n",
        "edge/NBA cascade did not converge",
    );
}

#[test]
fn unreadable_sdf_file_exits_nonzero() {
    fails(
        "sdf",
        "module t;\n  initial begin\n    $sdf_annotate(\"missing.sdf\");\n    $display(\"post\");\n  end\nendmodule\n",
        "cannot read SDF file 'missing.sdf'",
    );
}

#[test]
fn runs_without_a_fatal_error_still_exit_zero() {
    let finish =
        "module t;\n  initial begin\n    $display(\"done\");\n    $finish;\n  end\nendmodule\n";
    assert_eq!(run("finish", finish, &[]).0, Some(0));
    // `--max-time` ends a free-running clock.
    let clock = "module t;\n  reg c = 0;\n  always #5 c = ~c;\nendmodule\n";
    let (code, text) = run("max_time", clock, &["--max-time", "100"]);
    assert_eq!(code, Some(0), "{text}");
    // `$error` fails the run only under `--error-exit`.
    let error = "module t;\n  initial begin\n    $error(\"e\");\n    $finish;\n  end\nendmodule\n";
    assert_eq!(run("error", error, &[]).0, Some(0));
    assert_eq!(run("error_exit", error, &["--error-exit"]).0, Some(1));
}
