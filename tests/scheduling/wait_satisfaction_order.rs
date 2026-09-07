//! IEEE 1800-2023 §9.7.4 — level-sensitive `wait`ers whose conditions are
//! satisfied by BLOCKING writes resume, when the writer yields, in
//! WRITE-SATISFACTION order: a waiter made true by an earlier write runs
//! before one made true by a later write, regardless of when each parked.
//!
//! Root cause this guards: `drain_condition_waiters` resumed parked waiters in
//! FIFO parking order. If a later-parked waiter is satisfied by an EARLIER
//! write, it would run first and — via a re-arm on its entry (clearing a
//! shared sentinel that the earlier-parked waiter latches) — clobber the value
//! the earlier-parked waiter needs to observe, so that waiter re-parks forever
//! and never completes. This is the shape of the UVM driver/sequencer
//! `wait_for_item_done` / `item_done` race (a method-local `sequence_id` re-arm
//! vs a level-sensitive `wait(m_wait_for_item_sequence_id == sequence_id)`).
//!
//! Here `pa` parks at t=1 (after a `#1` prologue) while `pb` parks at t=0, so
//! parking order is [pb, pa], but write#1 satisfies `pa` and write#2 satisfies
//! `pb`. Correct behavior is pa-regains-empty, pb-clobbers-after: `pa` sees
//! m=10 and completes. A FIFO parking-order drain resumes pb first, whose
//! `wait_for_item_done` entry re-arms m := -1, so pa re-parks and `a_seen`
//! stays at its sentinel — the bug. Reference-validated
//! (tmp/svrun/wake_order_selftest.sv, min_task_class.sv — byte-for-byte).

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("signal {} not u64-able", n))
}

/// Two waiters, `pa` parking AFTER `pb`, where write#1 satisfies `pa` and
/// write#2 satisfies `pb`. `pa` must resume before `pb` (write order) and
/// observe m=10; otherwise `pb`'s re-arm clobbers it and `pa` never completes.
#[test]
fn earlier_satisfied_waiter_resumes_before_later_parked() {
    let src = r#"
`timescale 1ns/1ns
module top;
  int m_wait_for_item_sequence_id = -1;   // shared sentinel, re-armed by each entry
  bit  granted = 0;                        // second writer's release
  int  a_seen = -2;                        // what A observed (-2 = never resumed)

  task automatic wait_for_item_done(input int seq_id);
    m_wait_for_item_sequence_id = -1;      // re-arm on entry (the clobber)
    wait (m_wait_for_item_sequence_id == seq_id);
  endtask

  // WAITER A: parks at t=1 waiting for item 10.
  initial begin : pa
    #1;
    wait_for_item_done(10);
    a_seen = m_wait_for_item_sequence_id;
  end

  // WAITER B: parks at t=0 (earlier) waiting for the grant.
  initial begin : pb
    wait (granted);
    wait_for_item_done(1);                 // re-arms m := -1 on entry
  end

  // WRITER: satisfies A (m=10) then B (granted=1), no delay between writes.
  initial begin : pw
    #45;
    m_wait_for_item_sequence_id = 10;
    granted = 1;
  end

  initial #200 $finish;
endmodule
"#;
    let sim = simulate(src, 300).expect("simulate failed");
    // write#1 (m=10) satisfies A, so A resumes before B even though A parked
    // later. A must have observed m=10. A FIFO parking-order resume runs B
    // first, whose re-arm sets m := -1, so A re-parks and a_seen stays -2.
    assert_eq!(
        u(&sim, "a_seen"), 10,
        "A (satisfied by the earlier write) must observe m=10 before B's \
         re-arm clobbers it"
    );
}