//! An unqualified call to an undefined subroutine inside a class method must
//! be reported as an error, not silently dropped.
//!
//! xezim's interpreter routes a bare call inside a class method through the
//! class-method dispatcher only when `class_has_method` (or a listed builtin)
//! matches; otherwise it fell through to package/module subroutine resolution
//! and, for a genuinely unknown name, returned a silent zero with no diagnostic
//! at all. So a stale method call whose name exists nowhere in scope — e.g. the
//! UVM-1.x `kill_sequence_activity()` / `kill_child_sequences()` tasks removed
//! from the 1800.2 library — executed as a no-op: the surrounding logic kept
//! running on garbage and, in a sequence-abort regression, only surfaced much
//! later as a misleading downstream fatal.
//!
//! Per IEEE 1800-2017 §13.3 a call to an undeclared task/function is an error;
//! reference simulators reject it at elaboration ("Failed to find 'X' in
//! hierarchical name"). This test pins xezim's corrected behavior: it must emit
//! a clear `undefined subroutine` error for such a call (only in a class-method
//! context), instead of silently succeeding.

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn xezim_bin() -> std::path::PathBuf {
    let mut p = std::env::current_exe().expect("test exe path");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("xezim")
}

fn run(src: &str) -> String {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("xezim_undefcall_{n}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let p = dir.join("t.sv");
    std::fs::write(&p, src).expect("write");
    let bin = xezim_bin();
    if !bin.exists() {
        return String::new(); // binary not built in this profile
    }
    let out = Command::new(bin)
        .arg("--simulate")
        .arg("-s")
        .arg("top")
        .arg(&p)
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// A class method calling an undefined subroutine as a bare statement. Before
/// the fix xezim silently no-op'd the call and printed the "body ran" tag; now
/// it must flag the undefined `kill_child_sequences`.
#[test]
fn undef_method_call_in_class_method_is_reported() {
    const SRC: &str = r#"
module top;
  class seq;
    task body();
      kill_child_sequences();
      $display("TAG: body ran without error");
    endtask
  endclass
  seq s = new;
  initial begin
    s.body();
    $display("TAG: done");
  end
endmodule
"#;
    let out = run(SRC);
    if out.is_empty() {
        return; // binary not built in this profile
    }
    println!("{out}");
    assert!(
        out.contains("undefined subroutine 'kill_child_sequences'"),
        "undefined method call must be reported; got:\n{out}"
    );
    // The call must NOT be silently no-op'd into a successful run.
    assert!(
        !out.contains("TAG: body ran without error"),
        "undefined call must not silently run; got:\n{out}"
    );
}

/// A bare call to a *defined* module-level task from inside a class method is
/// legal and must keep working (the error is only for genuinely unresolvable
/// names).
#[test]
fn defined_module_task_call_still_works() {
    const SRC: &str = r#"
module top;
  task helper();
    $display("TAG: helper ran");
  endtask
  class c;
    task body();
      helper(); // in-scope module task — must dispatch
      $display("TAG: body ran");
    endtask
  endclass
  c obj = new;
  initial begin
    obj.body();
  end
endmodule
"#;
    let out = run(SRC);
    if out.is_empty() {
        return; // binary not built in this profile
    }
    println!("{out}");
    assert!(
        out.contains("TAG: helper ran") && out.contains("TAG: body ran"),
        "in-scope module task call must dispatch; got:\n{out}"
    );
    assert!(
        !out.contains("undefined subroutine 'helper'"),
        "defined task must not be flagged; got:\n{out}"
    );
}