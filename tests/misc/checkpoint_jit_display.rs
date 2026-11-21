//! A run restored from a checkpoint continues a process that displays a
//! signal value, also when the restoring run compiles natively
//! (`XEZIM_JIT=1`). IEEE 1800-2023 §9.4.2 wakes `always @(i)` on a change of
//! `i`, §21.2.1 defines `$display`, and Annex D.9 says a saved state is
//! reloaded "so that the simulation state can be reloaded at a later time and
//! processing can continue where it left off". The user guide says native
//! compilation may differ between the saving and the restoring run.

use std::path::{Path, PathBuf};
use std::process::Command;

const FINISHED: &str = "Simulation finished at time 6000 ($finish called)";

const INT_DISPLAY: &str = r#"`timescale 1ns/1ps
module top;
  int i;
  always @(i) $display("%0t i=%0d", $time, i);
  initial begin
    #2 i = 1;
    #3 i = 2;
    #1 $display("PASS");
    $finish;
  end
endmodule
"#;

const REAL_DISPLAY: &str = r#"`timescale 1ns/1ps
module top;
  real v;
  always @(v) $display("%0t v=%g", $realtime, v);
  initial begin
    #2 v = 1.5;
    #3 v = 2.5;
    #1 $display("PASS");
    $finish;
  end
endmodule
"#;

const NO_ARGUMENT: &str = r#"`timescale 1ns/1ps
module top;
  int i;
  always @(i) $display("changed");
  initial begin
    #2 i = 1;
    #3 i = 2;
    #1 $display("PASS");
    $finish;
  end
endmodule
"#;

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("xezim_ckpt_display_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Run `design` with `args`, native compilation on or off; the lines of
/// stdout without the version banner, and stderr. A run that does not exit
/// with success (for example a segmentation fault) fails the test.
fn run(dir: &Path, design: &Path, args: &[&str], jit: bool) -> (Vec<String>, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .current_dir(dir)
        .arg("--no-cache")
        .args(args)
        .arg(design)
        .env("XEZIM_JIT", if jit { "1" } else { "0" })
        .env("XEZIM_AOT", "0")
        .env("XEZIM_PROC_FSM", "1")
        .output()
        .expect("run xezim");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        out.status.success(),
        "xezim {args:?} (XEZIM_JIT={}) exited with {}\nstdout:\n{stdout}\nstderr:\n{stderr}",
        jit as u8,
        out.status
    );
    let lines = stdout
        .lines()
        .filter(|l| {
            !l.starts_with("=== xezim ") && !l.starts_with("git ") && !l.starts_with("tag ")
        })
        .map(str::to_string)
        .collect();
    (lines, stderr)
}

/// Save `src` at `save_at` in one run, restore it in another, and expect the
/// restored run to print exactly `tail`, which is also the end of the saving
/// run's output.
fn check_restore(
    tag: &str,
    src: &str,
    save_jit: bool,
    restore_jit: bool,
    artifact: bool,
    save_at: &str,
    tail: &[&str],
) {
    let d = dir(tag);
    let source = d.join("top.sv");
    std::fs::write(&source, src).unwrap();
    let design = if artifact {
        let path = d.join("top.xezbc");
        run(
            &d,
            &source,
            &["--compile", "-o", path.to_str().unwrap()],
            false,
        );
        path
    } else {
        source
    };
    let ck = d.join("top.ckpt");
    let spec = format!("{save_at},{}", ck.display());
    let (saved, err) = run(&d, &design, &["--save-at", &spec], save_jit);
    assert!(err.contains("saved"), "{err}");
    assert!(
        saved.ends_with(&tail.iter().map(|s| s.to_string()).collect::<Vec<_>>()),
        "the saving run printed {saved:?}, expected it to end with {tail:?}"
    );
    let (restored, err) = run(
        &d,
        &design,
        &["--restore", ck.to_str().unwrap()],
        restore_jit,
    );
    assert!(err.contains("restored"), "{err}");
    assert_eq!(restored, tail, "{err}");
    let _ = std::fs::remove_dir_all(&d);
}

/// The checkpoint is taken at 2000 ps, after the first change of `i`.
const AFTER_FIRST: [&str; 3] = ["5000 i=2", "PASS", FINISHED];
/// The checkpoint is taken at time 0, before the process ran.
const FROM_START: [&str; 4] = ["2000 i=1", "5000 i=2", "PASS", FINISHED];

#[test]
fn default_restore_displays_the_signal_value() {
    check_restore(
        "def",
        INT_DISPLAY,
        false,
        false,
        false,
        "2500ps",
        &AFTER_FIRST,
    );
}

#[test]
fn default_restore_displays_a_real_signal() {
    check_restore(
        "defreal",
        REAL_DISPLAY,
        false,
        false,
        false,
        "2500ps",
        &["5000 v=2.5", "PASS", FINISHED],
    );
}

#[test]
fn default_restore_displays_the_signal_value_from_an_artifact() {
    check_restore(
        "defart",
        INT_DISPLAY,
        false,
        false,
        true,
        "2500ps",
        &AFTER_FIRST,
    );
}

#[cfg(feature = "jit")]
#[test]
fn jit_restore_of_a_jit_checkpoint_displays_the_signal_value() {
    check_restore("jj", INT_DISPLAY, true, true, false, "2500ps", &AFTER_FIRST);
}

#[cfg(feature = "jit")]
#[test]
fn jit_restore_of_a_checkpoint_taken_before_the_first_change_displays_the_signal_value() {
    check_restore(
        "jearly",
        INT_DISPLAY,
        false,
        true,
        false,
        "1500ps",
        &FROM_START,
    );
}

#[cfg(feature = "jit")]
#[test]
fn jit_restore_displays_a_real_signal() {
    check_restore(
        "jreal",
        REAL_DISPLAY,
        true,
        true,
        false,
        "2500ps",
        &["5000 v=2.5", "PASS", FINISHED],
    );
}

#[cfg(feature = "jit")]
#[test]
fn default_restore_of_a_jit_checkpoint_displays_the_signal_value() {
    check_restore(
        "jdef",
        INT_DISPLAY,
        true,
        false,
        false,
        "2500ps",
        &AFTER_FIRST,
    );
}

#[cfg(feature = "jit")]
#[test]
fn jit_restore_of_a_checkpoint_saved_without_native_code_displays_the_signal_value() {
    check_restore(
        "njj",
        INT_DISPLAY,
        false,
        true,
        false,
        "2500ps",
        &AFTER_FIRST,
    );
}

#[cfg(feature = "jit")]
#[test]
fn jit_restore_displays_without_a_signal_argument() {
    check_restore(
        "jnoarg",
        NO_ARGUMENT,
        true,
        true,
        false,
        "2500ps",
        &["changed", "PASS", FINISHED],
    );
}
