//! Pure-SystemVerilog regression for the indexed-event round-trip — the
//! `6e9a261` regression surface, distilled from the UVM objection idiom.
//!
//! The UVM objection stores per-object event records in an associative array
//! and BOTH waits on and triggers the event through an indexed element, from
//! inside the class:
//!
//!     `@(m_events[obj].all_dropped)`  ...  `-> m_events[obj].all_dropped`
//!
//! `6e9a261` made the WAIT side genuinely suspend (correct), but its FIRE side
//! could not resolve the matching key: the `->` parser's `flatten` helper did
//! not handle `Index`, so `-> arr[k].ev` collapsed to the placeholder name
//! `"event"` and the suspended waiter was never woken. Inside UVM this hung
//! the objection-drain process, spun the event loop, and broke ~130 tests.
//!
//! The fix teaches `flatten` to bake an index (variable ident OR integer
//! literal) into the dotted name as `base[idx]`, so the fire-side resolves the
//! receiver to the same heap handle the wait-side used.
//!
//! Like the sibling `class_field_named_event.rs` (the non-indexed `@ev`/`->ev`
//! case), this test drives the real CLI binary, which is the path UVM uses and
//! the path where wait/trigger identity must agree. Reference-validated: Questa
//! prints TAG_PASS.

use std::process::Command;

fn xezim() -> String {
    env!("CARGO_BIN_EXE_xezim").to_string()
}

fn run(src: &str, tag: &str) -> String {
    let path = format!("/tmp/ixdev_{tag}.sv");
    std::fs::write(&path, src).unwrap();
    let out = Command::new(xezim())
        .args(["--simulate", "-s", "top", &path])
        .output()
        .expect("xezim failed to start");
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// `@(arr[k].ev)` must suspend and `-> arr[k].ev` (through the SAME indexed
/// path, from inside the class) must wake it exactly once. Before the fix the
/// waiter hung forever (the trigger named a placeholder and never matched).
#[test]
fn indexed_event_wait_and_trigger_roundtrip() {
    let src = r#"module top;
  class evrec;
    event ev;
  endclass

  class holder;
    evrec arr [int];
    int   woke = 0;
    task wait_ev(int k);
      @(arr[k].ev);          // suspend on an event reached through an
      woke = woke + 1;       // associative-array element (UVM objection idiom)
    endtask
    task trig_ev(int k);
      -> arr[k].ev;          // fire the SAME event through the SAME indexed path
    endtask
  endclass

  initial begin
    holder h;
    evrec  e;
    h = new;
    e = new;
    h.arr[5] = e;

    fork
      h.wait_ev(5);
    join_none

    #1;
    if (h.woke != 0) begin
      $display("FAIL_A woke before trigger %0d", h.woke);
      $finish;
    end
    h.trig_ev(5);
    #1;
    if (h.woke !== 1)
      $display("FAIL_B woke=%0d (expected 1) -- trigger did not wake the waiter", h.woke);
    else
      $display("TAG_PASS");
    $finish;
  end
endmodule
"#;
    let out = run(src, "roundtrip");
    let tag = out
        .lines()
        .find(|l| l.contains("TAG_PASS") || l.contains("FAIL_"))
        .unwrap_or("(no output)");
    assert!(
        out.contains("TAG_PASS"),
        "@(arr[k].ev) must suspend and -> arr[k].ev must wake it exactly once\n{tag}"
    );
}

/// A variable index (`m_events[obj]`) must resolve identically on wait and
/// fire — the UVM objection indexes by the object handle, not a literal.
#[test]
fn indexed_event_variable_index_roundtrip() {
    let src = r#"module top;
  class evrec;
    event all_dropped;
  endclass

  class holder;
    evrec m_events [int];
    int   drained = 0;
    task wait_drop(int obj);
      @(m_events[obj].all_dropped);   // variable (handle-keyed) index
      drained = drained + 1;
    endtask
    task fire_drop(int obj);
      -> m_events[obj].all_dropped;   // SAME variable index
    endtask
  endclass

  initial begin
    holder h; evrec e; int key = 9;
    h = new; e = new;
    h.m_events[key] = e;
    fork h.wait_drop(key); join_none
    #1;
    if (h.drained != 0) begin $display("FAIL_A early %0d", h.drained); $finish; end
    h.fire_drop(key);
    #1;
    if (h.drained !== 1) $display("FAIL_B drained=%0d (expected 1)", h.drained);
    else                 $display("TAG_PASS");
    $finish;
  end
endmodule
"#;
    let out = run(src, "varidx");
    assert!(
        out.contains("TAG_PASS"),
        "variable-index event round-trip failed\n{}",
        out.lines()
            .find(|l| l.contains("TAG_PASS") || l.contains("FAIL_"))
            .unwrap_or("(no output)")
    );
}
