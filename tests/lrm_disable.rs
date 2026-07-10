//! IEEE 1800-2017 §9.6.2 `disable`.
//!
//! `disable_target` was recorded and never read. The only effect of a `disable`
//! was setting `break_flag` — a GLOBAL flag — so:
//!   - `disable <loop_body_block>` broke the loop instead of continuing it
//!     (the LRM's own canonical example);
//!   - `disable <other_process_block>` never terminated the target, silently
//!     truncated the DISABLING process, and dropped pending events belonging to
//!     unrelated processes;
//!   - `disable <task>` left a stale unwind signal behind that could stop later
//!     loops from clearing `break_flag`.
//!
//! A `disable` naming a block in the current process unwinds to the end of that
//! block; naming another process's top-level block terminates that process.

use xezim::simulate;

/// The LRM's canonical example: disabling the loop-body block is `continue`,
/// disabling a block that encloses the loop is `break`.
const LOOPS: &str = r#"
module tb;
  int cont_sum, break_sum;
  initial begin
    cont_sum = 0;
    for (int i = 0; i < 5; i++) begin : inner
      if (i == 2) disable inner;      // continue
      cont_sum = cont_sum + i;
    end

    break_sum = 0;
    begin : outer
      for (int i = 0; i < 5; i++) begin : inner2
        if (i == 2) disable outer;    // break
        break_sum = break_sum + i;
      end
    end
  end
endmodule
"#;

/// Disabling another process must kill it, leave the disabler running, and not
/// disturb any unrelated process's scheduled events.
const PROCESSES: &str = r#"
module tb;
  int worker_ran, disabler_time, unrelated_time, observed;
  initial begin : worker
    #10 worker_ran = 1;
    #90 worker_ran = 2;
  end
  initial begin
    #5 disable worker;
    disabler_time = $time;            // the disabler carries on
  end
  initial #20 unrelated_time = $time; // an unrelated event survives
  initial #30 observed = worker_ran;  // worker never ran
endmodule
"#;

/// `disable <task>` ends the invocation; a named block inside a task resumes
/// after the block. Neither may leave a stale unwind signal behind.
const TASKS: &str = r#"
module tb;
  int steps, nested_steps, later_sum;

  task automatic worker();
    steps = 1;
    disable worker;
    steps = 99;              // unreachable
  endtask

  task automatic nested();
    begin : blk
      nested_steps = 10;
      disable blk;
      nested_steps = 88;     // unreachable
    end
    nested_steps = nested_steps + 1;   // resumes after blk
  endtask

  initial begin
    steps = 0;        worker();
    nested_steps = 0; nested();

    // A stale `disable_target` would keep this loop from clearing break_flag.
    later_sum = 0;
    for (int i = 0; i < 4; i++) begin
      if (i == 2) break;
      later_sum = later_sum + i;
    end
  end
endmodule
"#;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able", n))
        & 0xFFFF_FFFF
}

#[test]
fn disabling_a_loop_body_block_continues_the_loop() {
    let sim = simulate(LOOPS, 200).expect("simulate failed");
    assert_eq!(u(&sim, "cont_sum"), 8, "0+1+3+4: disable inner must continue");
}

#[test]
fn disabling_a_block_enclosing_the_loop_breaks_out() {
    let sim = simulate(LOOPS, 200).expect("simulate failed");
    assert_eq!(u(&sim, "break_sum"), 1, "0+1: disable outer must break");
}

#[test]
fn disabling_another_process_kills_it_and_spares_everyone_else() {
    let sim = simulate(PROCESSES, 200).expect("simulate failed");
    assert_eq!(u(&sim, "observed"), 0, "the disabled process still ran");
    assert_eq!(u(&sim, "disabler_time"), 5, "the disabling process was truncated");
    assert_eq!(u(&sim, "unrelated_time"), 20, "an unrelated event was dropped");
}

#[test]
fn disabling_a_task_or_a_block_inside_one_leaves_no_stale_state() {
    let sim = simulate(TASKS, 200).expect("simulate failed");
    assert_eq!(u(&sim, "steps"), 1, "disable <task> must end the invocation");
    assert_eq!(u(&sim, "nested_steps"), 11, "execution resumes after the block");
    // A later, ordinary `break` still works.
    assert_eq!(u(&sim, "later_sum"), 1);
}
