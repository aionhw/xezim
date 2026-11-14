//! Checkpoints of processes that forked (IEEE 1800-2023 Annex D, §9.3.2).
//!
//! A checkpoint numbers each statement list it writes, and a list shared by
//! several suspended processes is written once and then named by its number.
//! The table of cached fork-child frames is written first, and an entry of it
//! that cannot be carried over (a fork child's frame owns itself, so the
//! statement its key names is not in the owner list) used to be skipped after
//! its owner list was numbered but before it was written. Every later list
//! was then numbered one too high for the restore: the shared body of a
//! process that had forked resolved to a wrong list and the process lost the
//! rest of its body, or no list at all and the restore failed with `unknown
//! statement list`.
//!
//! A restored run must print what the uninterrupted run prints after the
//! saved slot.

use std::path::{Path, PathBuf};
use std::process::Command;

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("xezim_ckpt_fork_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Run `design` with `args`; (the `R ` lines of the design, stderr).
fn run(design: &Path, args: &[&str]) -> (Vec<String>, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .arg("--no-cache")
        .args(args)
        .arg(design)
        .output()
        .expect("run xezim");
    let lines = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.starts_with("R "))
        .map(str::to_string)
        .collect();
    (lines, String::from_utf8_lossy(&out.stderr).into_owned())
}

/// Save `src` at `save_at` and restore it: (the lines of the uninterrupted
/// run, of the saving run, of the restored run, stderr of the restore).
fn save_and_restore(
    tag: &str,
    src: &str,
    save_at: &str,
) -> (Vec<String>, Vec<String>, Vec<String>, String) {
    let d = dir(tag);
    let f = d.join("tb.sv");
    std::fs::write(&f, src).unwrap();
    let (full, _) = run(&f, &[]);
    let ck = d.join("c.ckpt");
    let spec = format!("{save_at},{}", ck.display());
    let (saved, err) = run(&f, &["--save-at", &spec]);
    assert!(err.contains("saved"), "{err}");
    let (restored, err) = run(&f, &["--restore", ck.to_str().unwrap()]);
    let _ = std::fs::remove_dir_all(&d);
    (full, saved, restored, err)
}

/// A process that executed `fork ... join_none` before the save keeps the
/// statements it still owes: the rest of its body ran after the delay it was
/// parked in, and nothing after it (the simulation then ended without a
/// `$finish`).
#[test]
fn a_process_that_forked_join_none_keeps_its_body_after_a_restore() {
    let src = r#"`timescale 1ns/1ns
module tb;
  bit m;
  initial begin
    #2;
    fork begin #5 $display("R child %0t", $time); end join_none
    #50 m = 1;
    $display("R after m=1 %0t", $time);
    #10 $display("R done %0t", $time);
    $finish;
  end
endmodule
"#;
    // Saved at 7: the child has printed.
    let (full, saved, restored, err) = save_and_restore("body", src, "20ns");
    assert_eq!(full, ["R child 7", "R after m=1 52", "R done 62"]);
    assert_eq!(saved, full, "saving must not change the run");
    assert!(err.contains("restored"), "{err}");
    assert_eq!(restored, ["R after m=1 52", "R done 62"], "{err}");
}

/// Children forked before the save and still waiting for a condition or an
/// event wake after the restore, and the parent finishes its body.
#[test]
fn forked_waiters_wake_after_a_restore() {
    let src = r#"`timescale 1ns/1ns
class C;
  bit flag;
  task run();
    wait (flag == 1);
    $display("R C.run woke %0t", $time);
  endtask
endclass
module tb;
  C c;
  bit m, e;
  initial begin
    c = new;
    fork
      c.run();
      begin wait (m == 1); $display("R wait woke %0t", $time); end
      begin @(e); $display("R event woke %0t", $time); end
    join_none
    #50 c.flag = 1; m = 1; e = 1;
    #10 $display("R done %0t", $time);
    $finish;
  end
endmodule
"#;
    let (full, saved, restored, err) = save_and_restore("waiters", src, "20ns");
    assert_eq!(
        full,
        [
            "R C.run woke 50",
            "R wait woke 50",
            "R event woke 50",
            "R done 60"
        ]
    );
    assert_eq!(saved, full, "saving must not change the run");
    assert_eq!(restored, full, "{err}");
}

/// A testbench-style save: forks in class tasks (each child copies the loop's
/// handle in a fork-block declaration, §9.3.2), then `$save` from a task
/// followed by a delay. The restore used to fail before simulating, with
/// `unknown statement list` for the first list that referred back to one
/// the checkpoint had numbered but not written.
#[test]
fn a_save_from_a_task_after_forks_in_class_tasks_restores() {
    let d = dir("task_save");
    let f = d.join("tb.sv");
    let ck = d.join("c.ckpt");
    let src = r#"`timescale 1ns/1ns
class Waiter;
  bit go;
  int id;
  task run();
    wait (go == 1);
    $display("R W%0d woke at %0t", id, $time);
  endtask
endclass
class Env;
  Waiter w[3];
  task start();
    foreach (w[i]) begin
      w[i] = new;
      w[i].id = i;
      fork
        automatic Waiter x = w[i];
        x.run();
      join_none
    end
  endtask
  task save_here(string p);
    $save(p);
    #1;
  endtask
endclass
module tb;
  Env e;
  string p;
  initial begin
    e = new;
    e.start();
    #10;
    if ($value$plusargs("ck=%s", p)) e.save_here(p);
    #20;
    foreach (e.w[i]) e.w[i].go = 1;
    #10 $display("R done %0t", $time);
    $finish;
  end
endmodule
"#;
    std::fs::write(&f, src).unwrap();
    let arg = format!("+ck={}", ck.display());
    let (saved, err) = run(&f, &[&arg]);
    assert!(err.contains("saved"), "{err}");
    let want = [
        "R W0 woke at 31",
        "R W1 woke at 31",
        "R W2 woke at 31",
        "R done 41",
    ];
    assert_eq!(saved, want, "{err}");
    let (restored, err) = run(&f, &["--restore", ck.to_str().unwrap()]);
    assert!(err.contains("restored"), "{err}");
    assert_eq!(restored, want, "{err}");
    let _ = std::fs::remove_dir_all(&d);
}
