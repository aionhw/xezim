//! IEEE 1800-2023 §9.4.3 / §9.7.4 — a `repeat` whose body blocks via a CALL
//! to a task (the task itself contains a `wait`/`#delay`/`@event`) must
//! suspend the process on each iteration, not busy-spin on the synchronous
//! path.
//!
//! Root cause this guards: the suspend-aware statement runner only unrolled a
//! `repeat` when its body had a DIRECT `@event`/`#delay`
//! (`stmt_has_event_wait(body)`). A repeat that blocks only transitively —
//! `repeat(N) obj.do_step();` where `do_step()` contains a `wait(cond)` —
//! fell through to the synchronous `exec_statement`, so the body's calls were
//! never inlined and their nested `wait(cond)` (false) fell through instead
//! of parking the process. The loop then busy-spun to its iteration cap with
//! the waiter never actually blocking.
//!
//! The fix mirrors the `while`/`for` arms: also unroll when the body blocks
//! via a call (`stmt_is_blocking(body)`), so each iteration descends through
//! the suspend-aware path and the inner `wait` parks correctly.

use xezim::simulate;

fn lookup(sim: &xezim::compiler::Simulator, name: &str) -> u64 {
    sim.get_signal(name)
        .or_else(|| sim.get_signal(&format!("top.{}", name)))
        .unwrap_or_else(|| panic!("signal not found: {}", name))
        .to_u64()
        .unwrap_or_else(|| panic!("signal {} not u64-able", name))
}

/// `repeat(N)` calling a task whose body `wait`s on a class member. Each
/// iteration must suspend until the member is set by a peer process; the loop
/// must complete exactly N times, and every iteration must have observed the
/// gate (proving the `wait` actually blocked, rather than falling through).
#[test]
fn repeat_blocking_call_suspends_each_iteration() {
    let src = r#"
`timescale 1ns/1ns
class C;
  int cnt;
  int gate;
  int min_gate_seen;   // folded min: only drops if a 0 is ever observed

  function new();
    cnt = 0;
    gate = 0;
    min_gate_seen = 1;
  endfunction

  task automatic blocker(input int id);
    wait (gate == 1);
    if (gate < min_gate_seen) min_gate_seen = gate;
    cnt = cnt + 1;
  endtask
endclass

module top;
  C obj;
  int sum;
  int final_min_gate;

  initial begin
    obj = new;
    sum = 0;
    // Peer process: raise the gate one step at a time, then lower it again
    // so the next iteration has something fresh to wait for.
    fork
      begin
        repeat (5) begin
          #1;
          obj.gate = 1;
          #1;
          obj.gate = 0;
        end
      end
    join_none

    // repeat(N) whose body blocks ONLY via the call (no direct @event/#delay).
    repeat (5) begin
      obj.blocker(sum);
      sum = sum + 1;
    end

    final_min_gate = obj.min_gate_seen;
    #1 $finish;
  end
endmodule
"#;
    let sim = simulate(src, 30).expect("simulate failed");
    // Correct: the loop ran 5 iterations, each genuinely waiting for the gate.
    // Buggy (wait fell through): final_min_gate == 0, because the first
    // iteration ran at t=0 before the peer ever raised the gate.
    assert_eq!(lookup(&sim, "sum"), 5, "repeat body ran once per iteration");
    assert_eq!(
        lookup(&sim, "final_min_gate"),
        1,
        "each iteration observed gate==1, so the wait actually blocked"
    );
}
