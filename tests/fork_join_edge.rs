//! IEEE 1800-2017 §9.3 fork/join corner cases that were broken.
//!
//! 1. §9.3.2 — an EMPTY `fork ... join` (or one whose only items were
//!    declarations) dropped the entire continuation. A join/join_any suspends
//!    on a JoinWaiter that is re-checked only when a child finishes; with no
//!    children it was never re-checked, so the process never resumed.
//!
//! 2. §9.6.2 — `disable <named_fork_block>` where the block was `join_none`
//!    terminated the DISABLING process. The block had already returned, so the
//!    label-unwind ran off the end of the process and dropped its
//!    continuation. It must instead terminate the processes the fork spawned
//!    and carry on.

use xezim::simulate;

const EMPTY_FORK: &str = r#"
module tb;
  int stage;
  initial begin
    stage = 1;
    fork join
    stage = 2;
    fork join_any
    stage = 3;
    fork join_none
    stage = 4;
    // A fork whose only content is a declaration spawns no process either.
    fork
      automatic int unused = 7;
    join
    stage = 5;
  end
endmodule
"#;

/// An empty fork inside a loop must not stall the loop, and time must advance
/// normally around it.
const EMPTY_FORK_LOOP: &str = r#"
module tb;
  int count;
  int t_end;
  initial begin
    count = 0;
    for (int i = 0; i < 4; i++) begin
      fork join
      #5;
      count = count + 1;
    end
    t_end = $time;
  end
endmodule
"#;

const DISABLE_NAMED: &str = r#"
module tb;
  int reached_after, reached_end;
  logic straggler_ran;
  initial begin
    straggler_ran = 0;
    fork : blk
      begin #100; straggler_ran = 1; end   // must be killed
    join_none
    reached_after = 1;
    #10;
    disable blk;
    reached_after = 2;                       // continuation must survive
    #200;
    reached_end = $time;
  end
endmodule
"#;

/// Disabling a named fork block from a DIFFERENT process, and disabling a
/// block whose fork was `join` (not join_none).
const DISABLE_CROSS: &str = r#"
module tb;
  logic worker_finished;
  int killer_ran, end_time;
  initial begin
    worker_finished = 0;
    fork : worker
      begin #100; worker_finished = 1; end
    join_none
    fork
      begin #20; disable worker; killer_ran = 1; end
    join_none
    #200;
    end_time = $time;
  end
endmodule
"#;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able", n))
}

#[test]
fn an_empty_fork_completes_for_every_join_type() {
    let sim = simulate(EMPTY_FORK, 100).expect("simulate failed");
    // Reaching stage 5 means none of the four empty forks stalled.
    assert_eq!(u(&sim, "stage"), 5, "an empty fork dropped the continuation");
}

#[test]
fn an_empty_fork_in_a_loop_does_not_stall() {
    let sim = simulate(EMPTY_FORK_LOOP, 200).expect("simulate failed");
    assert_eq!(u(&sim, "count"), 4, "the loop stalled on an empty fork");
    assert_eq!(u(&sim, "t_end"), 20, "time did not advance around the empty forks");
}

#[test]
fn disabling_a_named_fork_block_keeps_the_disabling_process_alive() {
    let sim = simulate(DISABLE_NAMED, 500).expect("simulate failed");
    assert_eq!(u(&sim, "reached_after"), 2, "the disabling process was terminated");
    assert_eq!(u(&sim, "reached_end"), 210, "the continuation did not finish");
    assert_eq!(u(&sim, "straggler_ran") & 1, 0, "the fork block's process was not killed");
}

#[test]
fn a_named_fork_block_can_be_disabled_from_another_process() {
    let sim = simulate(DISABLE_CROSS, 500).expect("simulate failed");
    assert_eq!(u(&sim, "killer_ran"), 1, "the killer process did not run");
    assert_eq!(u(&sim, "worker_finished") & 1, 0, "the worker was not disabled");
    assert_eq!(u(&sim, "end_time"), 200);
}

/// §15.3.3 — `semaphore.get(n)` on an under-full semaphore must BLOCK until a
/// `put` supplies the keys. It used to return immediately having removed
/// nothing, so a fork/join semaphore handoff never synchronised.
const SEMAPHORE_BLOCK: &str = r#"
module tb;
  semaphore sem = new(0);
  int got_time;
  logic got;
  initial begin got = 0; #10; sem.put(1); end
  initial begin sem.get(1); got = 1; got_time = $time; end
endmodule
"#;

/// A get that CAN proceed must not block; FIFO ordering among waiters.
const SEMAPHORE_ORDER: &str = r#"
module tb;
  semaphore sem = new(2);
  int a_time, b_time;
  logic a_done, b_done;
  initial begin
    a_done = 0; b_done = 0;
    // Two immediate gets: 2 keys available, both proceed at t=0.
    sem.get(1); a_done = 1; a_time = $time;
    sem.get(1); b_done = 1; b_time = $time;
  end
endmodule
"#;

#[test]
fn semaphore_get_blocks_until_keys_are_available() {
    let sim = simulate(SEMAPHORE_BLOCK, 100).expect("simulate failed");
    assert_eq!(u(&sim, "got") & 1, 1, "the blocking get never completed");
    assert_eq!(u(&sim, "got_time"), 10, "get returned before the put supplied a key");
}

#[test]
fn semaphore_get_does_not_block_when_keys_are_available() {
    let sim = simulate(SEMAPHORE_ORDER, 100).expect("simulate failed");
    assert_eq!(u(&sim, "a_done") & 1, 1);
    assert_eq!(u(&sim, "b_done") & 1, 1, "the second available get blocked");
    assert_eq!(u(&sim, "a_time"), 0);
    assert_eq!(u(&sim, "b_time"), 0, "an available get must proceed at once");
}
