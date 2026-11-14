//! A blocking `mailbox.get()` on a NULL handle must fault where the fault is.
//!
//! `get`/`peek` suspend by parking the caller in `mailbox_get_waiters`, keyed
//! by a LIVE mailbox handle. A handle that was never `new`ed has no entry, so
//! the call fell through to generic dispatch and returned immediately without
//! blocking. Wrapped in the `forever` that virtually every consumer loop uses,
//! that spins until the stall detector trips at 100_000 iterations — and the
//! stall report then blames a missing `#delay`/`@event`, sending the reader
//! hunting for a timing control in a loop whose actual fault is an
//! unconstructed mailbox. This cost a user a real debugging session.
//!
//! Also pinned here: the stall report's line number. Spans index the
//! PREPROCESSED text, into which every `\`include` is spliced whole, so the
//! reported line can land far past the end of the file it names — a 10-line
//! file that includes 3000 lines reported line 3005. The report now says the
//! number is post-expansion and quotes the offending source instead.

use std::process::Command;

fn xezim_bin() -> std::path::PathBuf {
    // Cargo names the binary it built for the integration tests.
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_xezim"))
}

/// Run the binary on `files` (name, source) written to a temp dir, returning
/// combined stdout+stderr.
fn run(tag: &str, files: &[(&str, &str)], extra: &[&str]) -> String {
    let dir = std::env::temp_dir().join(format!("xezim_nullmbx_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let mut paths = Vec::new();
    for (n, src) in files {
        let p = dir.join(n);
        std::fs::write(&p, src).expect("write");
        if n.ends_with(".sv") {
            paths.push(p);
        }
    }
    let bin = xezim_bin();
    if !bin.exists() {
        // The binary is not built in this profile — nothing to assert against.
        let _ = std::fs::remove_dir_all(&dir);
        return String::new();
    }
    let mut cmd = Command::new(bin);
    cmd.arg("--simulate");
    for e in extra {
        cmd.arg(e);
    }
    cmd.arg("-I").arg(&dir);
    for p in &paths {
        cmd.arg(p);
    }
    let out = cmd.output().expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

const NULL_LOCAL: &str = r#"
module top;
  mailbox #(int) mbx_null;
  initial begin
    fork
      forever begin
        int v;
        mbx_null.get(v);
      end
    join_none
    #10 $finish;
  end
endmodule
"#;

const NULL_PROPERTY: &str = r#"
package bfm_pkg;
  class driver;
    mailbox #(int) mbx;
    task automatic run();
      forever begin
        int v;
        mbx.get(v);
      end
    endtask
  endclass
endpackage
module top;
  import bfm_pkg::*;
  driver d;
  initial begin
    d = new();
    fork d.run(); join_none
    #100 $finish;
  end
endmodule
"#;

#[test]
fn null_mailbox_get_reports_the_null_handle_not_a_stall() {
    let out = run("local", &[("t.sv", NULL_LOCAL)], &[]);
    if out.is_empty() {
        return; // binary not built in this profile
    }
    assert!(
        out.contains("null mailbox handle"),
        "must name the null handle; got:\n{out}"
    );
    assert!(
        out.contains("mbx_null"),
        "must name the offending mailbox variable; got:\n{out}"
    );
    assert!(
        !out.contains("no `#delay`"),
        "must NOT blame a missing timing control; got:\n{out}"
    );
}

/// A mailbox declared as a class PROPERTY reaches the check by a different
/// route (the declared type lives on the class, not on any signal).
#[test]
fn null_class_property_mailbox_is_also_reported() {
    let out = run("prop", &[("t.sv", NULL_PROPERTY)], &[]);
    if out.is_empty() {
        return;
    }
    assert!(
        out.contains("null mailbox handle"),
        "must name the null handle; got:\n{out}"
    );
    assert!(
        !out.contains("no `#delay`"),
        "must NOT blame a missing timing control; got:\n{out}"
    );
}

/// A genuine no-timing-control loop must STILL report as a stall — the null
/// check must not swallow the case it was carved out of.
#[test]
fn a_real_untimed_loop_still_reports_a_stall() {
    let src = r#"
module top;
  reg a;
  initial begin
    fork forever a = ~a; join_none
    #10 $finish;
  end
endmodule
"#;
    let out = run("real", &[("t.sv", src)], &[]);
    if out.is_empty() {
        return;
    }
    assert!(out.contains("STALLED"), "got:\n{out}");
    assert!(out.contains("no `#delay`"), "got:\n{out}");
    assert!(
        out.contains("its source:") && out.contains("forever"),
        "the report must quote the offending source; got:\n{out}"
    );
}

/// An `\`include` shifts every following line in the preprocessed text. The
/// report must still name the line (and column) in the file itself, not a
/// post-expansion number the file does not have.
#[test]
fn stall_line_after_an_include_names_the_real_line() {
    let filler = "// filler\n".repeat(3000);
    let top = r#"`include "big.svh"
module top;
  reg a;
  initial begin
    fork forever a = ~a; join_none
    #10 $finish;
  end
endmodule
"#;
    let out = run("inc", &[("big.svh", &filler), ("t.sv", top)], &[]);
    if out.is_empty() {
        return;
    }
    assert!(out.contains("STALLED"), "got:\n{out}");
    assert!(
        out.contains("t.sv:5:10 ") && !out.contains("preprocessed line"),
        "the stall must be located in t.sv itself; got:\n{out}"
    );
    assert!(
        out.contains("its source:") && out.contains("forever"),
        "and the source must be quoted so the block is still identifiable; got:\n{out}"
    );
}
