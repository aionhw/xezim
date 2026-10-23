//! §15.5 named events — the four ways an event's IDENTITY could be lost.
//!
//! A module-scope `event e;` is backed by a 1-bit signal that `-> e` toggles
//! and `@e` arms on, and that worked. Every other spelling of "which event do
//! you mean" did not, and each failed SILENTLY — usually by waking a waiter
//! immediately at t=0, which reads like the wait succeeded:
//!
//! 1. An event ARRAY element (`ev[1]`) got neither an `events` entry nor a
//!    backing signal — only the scalar declarator path registered those. So
//!    `@ev[1]` armed on the array name, found no signal, and fell into the
//!    "not a real signal" delta-yield that exists for `uvm_wait_for_nba_region`
//!    — resuming instantly. `wait(ev[1].triggered)` never completed at all.
//! 2. A class-property event reached through a HANDLE (`h.ce`) had no identity
//!    outside the class: `->` parsed it to the placeholder name `"event"`, and
//!    `@(h.ce)` hit the same delta-yield. (`@ce` on `this` INSIDE a method
//!    already worked — that is the uvm_event path.)
//! 3. A `ref event` formal was bound BY VALUE, so `-> x` fired a name with no
//!    sync object; worse, the ref write-back on return rewrote the caller's
//!    toggle signal with the stale value captured at call time, cancelling the
//!    edge even once the trigger was routed correctly.
//! 4. §15.5.4 assignment `e1 = e2` recorded the alias, but only the TRIGGER
//!    side resolved it — a waiter armed on its own raw name and never saw
//!    `-> e2`. `e1 == e2` compared toggle bits and returned x.
//!
//! All expectations below are byte-identical to a reference simulator.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("top.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able (x/z?)", n))
}

/// An element of an event array is its own synchronization object: `@ev[1]`
/// blocks until `-> ev[1]`, and does not respond to a sibling element.
#[test]
fn event_array_element_is_its_own_sync_object() {
    let src = r#"
`timescale 1ns/1ns
module top;
  event ev [3];
  int t_one, t_two, woke_on_sibling;
  initial begin
    fork
      begin @ev[1]; t_one = $time; end
      begin @ev[2]; t_two = $time; end
    join_none
    #10 -> ev[0];              // neither waiter may move
    #5  woke_on_sibling = (t_one != 0) || (t_two != 0);
    #5  -> ev[1];
    #10 -> ev[2];
    #5  $finish;
  end
endmodule
"#;
    let sim = simulate(src, 200).expect("simulate failed");
    assert_eq!(
        u(&sim, "woke_on_sibling"),
        0,
        "a sibling element must not wake it"
    );
    assert_eq!(u(&sim, "t_one"), 20, "@ev[1] resumes when ev[1] fires");
    assert_eq!(u(&sim, "t_two"), 30, "@ev[2] likewise");
}

/// `.triggered` on an array element, and an `always` block sensitive to one.
#[test]
fn event_array_element_triggered_and_always_sensitivity() {
    let src = r#"
`timescale 1ns/1ns
module top;
  event ev [3];
  int t_trig, n_always;
  always @(ev[1]) n_always++;
  initial begin
    fork begin wait(ev[1].triggered); t_trig = $time; end join_none
    #10 -> ev[1];
    #10 -> ev[1];
    #5  $finish;
  end
endmodule
"#;
    let sim = simulate(src, 200).expect("simulate failed");
    assert_eq!(
        u(&sim, "t_trig"),
        10,
        "wait(ev[1].triggered) completes at the trigger"
    );
    assert_eq!(
        u(&sim, "n_always"),
        2,
        "always @(ev[1]) fires once per trigger"
    );
}

/// A class-property event driven and waited on from OUTSIDE the class.
#[test]
fn class_property_event_through_a_handle() {
    let src = r#"
`timescale 1ns/1ns
module top;
  class holder;
    event ce;
    task trig(); -> ce; endtask
  endclass
  holder h;
  int t_at, t_trig;
  initial begin
    h = new();
    fork
      begin @(h.ce); t_at = $time; end
      begin wait(h.ce.triggered); t_trig = $time; end
    join_none
    #20 -> h.ce;
    #10 $finish;
  end
endmodule
"#;
    let sim = simulate(src, 200).expect("simulate failed");
    assert_eq!(
        u(&sim, "t_at"),
        20,
        "@(h.ce) blocks until the handle's event fires"
    );
    assert_eq!(
        u(&sim, "t_trig"),
        20,
        "and .triggered reads through the handle"
    );
}

/// Two instances of the same class have INDEPENDENT events — the per-instance
/// identity has to be the handle, not the field name.
#[test]
fn class_events_are_per_instance() {
    let src = r#"
`timescale 1ns/1ns
module top;
  class holder;
    event ce;
  endclass
  holder h1, h2;
  int t1, t2;
  initial begin
    h1 = new(); h2 = new();
    fork
      begin @(h1.ce); t1 = $time; end
      begin @(h2.ce); t2 = $time; end
    join_none
    #10 -> h1.ce;
    #10 -> h2.ce;
    #5  $finish;
  end
endmodule
"#;
    let sim = simulate(src, 200).expect("simulate failed");
    assert_eq!(u(&sim, "t1"), 10, "h1's event fires alone");
    assert_eq!(u(&sim, "t2"), 20, "h2's event is a different object");
}

/// §13.5.2: an event passed by `ref` is bound by identity — `-> x` in the
/// callee wakes the CALLER's waiter, and the return must not clobber it.
#[test]
fn ref_event_formal_triggers_the_callers_event() {
    let src = r#"
`timescale 1ns/1ns
module top;
  event e;
  int t_woke;
  task automatic fire(ref event x); -> x; endtask
  initial begin
    fork begin @e; t_woke = $time; end join_none
    #30 fire(e);
    #5  $finish;
  end
endmodule
"#;
    let sim = simulate(src, 200).expect("simulate failed");
    assert_eq!(
        u(&sim, "t_woke"),
        30,
        "the caller's waiter resumes at the ref trigger"
    );
}

/// §15.5.4: after `e1 = e2` the two names denote ONE object — a waiter on
/// either resumes when either is triggered, and they compare equal.
#[test]
fn event_assignment_merges_identities() {
    let src = r#"
`timescale 1ns/1ns
module top;
  event e1, e2;
  int w1, w2, same;
  initial begin
    e1 = e2;
    fork
      begin @e1; w1 = $time; end
      begin @e2; w2 = $time; end
    join_none
    #10 -> e2;
    #5  same = (e1 == e2);
    #5  $finish;
  end
endmodule
"#;
    let sim = simulate(src, 200).expect("simulate failed");
    assert_eq!(u(&sim, "w1"), 10, "the aliased name's waiter resumes too");
    assert_eq!(u(&sim, "w2"), 10, "as does the target's");
    assert_eq!(u(&sim, "same"), 1, "merged events compare equal");
}

/// The guard: distinct events must NOT be merged, and `== null` still works.
/// Comparing event identities must not disturb ordinary integral `==`.
#[test]
fn unmerged_events_and_null_compare_correctly() {
    let src = r#"
module top;
  event a, b, n;
  int a_eq_b, a_eq_a, n_is_null, a_is_null, int_eq;
  int x, y;
  initial begin
    n = null;
    a_eq_b    = (a == b);
    a_eq_a    = (a == a);
    n_is_null = (n == null);
    a_is_null = (a == null);
    x = 5; y = 5;
    int_eq = (x == y);
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "a_eq_b"), 0, "distinct events are not equal");
    assert_eq!(u(&sim, "a_eq_a"), 1, "an event equals itself");
    assert_eq!(u(&sim, "n_is_null"), 1, "a nulled event equals null");
    assert_eq!(u(&sim, "a_is_null"), 0, "a live event does not");
    assert_eq!(u(&sim, "int_eq"), 1, "ordinary integral == is unaffected");
}

/// The behaviours that already worked and must keep working: a plain event,
/// `.triggered` scoping to one slot, `->>` deferring to the NBA region, and
/// N triggers in one slot waking a waiter once.
#[test]
fn scalar_event_behaviour_is_unchanged() {
    let src = r#"
`timescale 1ns/1ns
module top;
  event e;
  int woke, n_wakes, trig_same_slot, trig_next_slot, nb_seen_immediately;
  initial begin
    fork forever begin @e; n_wakes++; woke = $time; end join_none
    #10;
    -> e; -> e; -> e;                 // three triggers, ONE slot
    trig_same_slot = e.triggered;
    #1 trig_next_slot = e.triggered;  // must have cleared
    #9;
    ->> e;                            // nonblocking: not seen until the NBA region
    nb_seen_immediately = (n_wakes > 1);
    #5 $finish;
  end
endmodule
"#;
    let sim = simulate(src, 200).expect("simulate failed");
    assert_eq!(
        u(&sim, "n_wakes"),
        2,
        "one wake per slot, then the ->> wake"
    );
    assert_eq!(u(&sim, "woke"), 20, "the ->> trigger landed at t=20");
    assert_eq!(
        u(&sim, "trig_same_slot"),
        1,
        ".triggered holds for the slot"
    );
    assert_eq!(u(&sim, "trig_next_slot"), 0, "and clears in the next one");
    assert_eq!(
        u(&sim, "nb_seen_immediately"),
        0,
        "->> does not resume a waiter inline"
    );
}

/// §15.5.1 / §26.3: an event declared in a PACKAGE is one synchronization
/// object however it is named — `rt::ev` by scope resolution or `ev` through
/// `import rt::*`. `-> rt::ev` parsed to the member chain `rt.ev`, which
/// matched no declared event, so it toggled nothing and every waiter (scoped
/// or imported) slept forever. Covers each trigger/wait spelling pair, `->>`
/// (§15.5.2) and `.triggered` (§15.5.3) through the scoped name.
#[test]
fn package_event_scoped_and_imported_names_agree() {
    let src = r#"
`timescale 1ns/1ns
package rt;
  event e_ss, e_sb, e_bs, e_bb, e_nb;
endpackage
module top;
  import rt::*;
  int t_ss, t_sb, t_bs, t_bb, t_nb, trig_seen;
  initial begin
    fork
      begin @(rt::e_ss); t_ss = $time; end
      begin @(e_sb);     t_sb = $time; end
      begin @(rt::e_bs); t_bs = $time; end
      begin @(e_bb);     t_bb = $time; end
      begin @(rt::e_nb); t_nb = $time; end
    join_none
    #10 -> rt::e_ss;
    #10 -> rt::e_sb;
    #10 -> e_bs;
    #10 -> e_bb;
    #10 ->> rt::e_nb;
    #10 -> rt::e_ss; trig_seen = e_ss.triggered;
    #5 $finish;
  end
endmodule
"#;
    let sim = simulate(src, 200).expect("simulate failed");
    // Expected values from the reference simulator.
    assert_eq!(u(&sim, "t_ss"), 10, "-> rt::ev wakes @(rt::ev)");
    assert_eq!(u(&sim, "t_sb"), 20, "-> rt::ev wakes @(ev)");
    assert_eq!(u(&sim, "t_bs"), 30, "-> ev wakes @(rt::ev)");
    assert_eq!(u(&sim, "t_bb"), 40, "-> ev wakes @(ev)");
    assert_eq!(u(&sim, "t_nb"), 50, "->> rt::ev wakes @(rt::ev)");
    assert_eq!(u(&sim, "trig_seen"), 1, "-> rt::ev sets ev.triggered");
}
