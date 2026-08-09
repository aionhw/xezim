//! Pure-SystemVerilog regression for two xezim fixes that surfaced in the UVM
//! phasing suites (40phasing / 35objections), distilled to the raw SV idiom
//! with NO Rust-side shims and NO UVM library. Each test is deterministic and
//! self-checking.

use xezim::simulate;

/// [A] An `event` member of a class reached through an associative-array
/// index, `@(arr[key].ev)`, must genuinely SUSPEND. Previously xezim computed
/// an empty sensitivity list for this shape and executed the body immediately;
/// inside uvm_objection's `while (need_to_check_all)` loop that made the loop
/// re-run forever -> stack overflow / UVM TIMEOUT.
///
/// Deterministic arming handshake (LRM 9.7.4 level wait) — no fork race:
/// the forked waiter sets `armed` immediately before the `@`; the parent
/// level-waits on `armed`, and if the `@` did not suspend, `e.resumed` is
/// already 1 at that instant.
#[test]
fn obj_indexed_event_wait_suspends() {
    const SRC: &str = r#"
module top;
  class evrec;
    event all_dropped;
    bit   resumed = 0;
  endclass

  class ph;
    evrec m_events [int];          // handle-valued associative array
    bit   armed = 0;
    task wait_for(int key);
      if (m_events[key] == null) begin
        $display("FAIL_A null key");
        return;
      end
      armed = 1;
      @(m_events[key].all_dropped);     // the shape under test
      m_events[key].resumed = 1;
    endtask
  endclass

  initial begin
    ph hp;
    evrec e;
    hp = new;
    e = new;
    hp.m_events[7] = e;

    fork
      hp.wait_for(7);
    join_none

    wait (hp.armed);
    if (e.resumed == 1) begin
      $display("FAIL_A event wait did not suspend");
      $finish;
    end
    -> e.all_dropped;
    #1;
    if (e.resumed != 1) begin
      $display("FAIL_A not resumed after trigger");
      $finish;
    end
    $display("TAG_A_PASS");
    $finish;
  end
endmodule
"#;

    let sim = simulate(SRC, 100).expect("simulation failed");
    let msgs: Vec<String> = sim
        .output
        .iter()
        .map(|l| l.message.clone())
        .filter(|m| m.contains("TAG_") || m.contains("FAIL_A"))
        .collect();
    assert!(sim.finished, "simulation must terminate: {:?}", msgs);
    assert_eq!(msgs, vec!["TAG_A_PASS"], "event wait/trigger must collaborate");
}

/// [B] `disable fork` must drop the killed subprocesses' FUTURE-time `#delay`
/// from the timing wheel. Otherwise the event queue's `next_time()` stays
/// non-empty forever and a quiet simulation never terminates (idle detection
/// is gated on an empty queue) — the residual 9200 s run-phase watchdog timer
/// made 40jump/00nojump spin to --max-time instead of finishing.
///
/// Assertion: no `$finish` is used, so the sim must terminate purely by
/// idleness. We assert on the final `sim.time`: if the killed watchdog's
/// far-future `#delay` were left in the queue the loop would advance time
/// toward it (well past a tiny bound) instead of idling at ~0.
#[test]
fn disable_fork_drops_killed_timer() {
    const SRC: &str = r#"
module B;
  bit ran = 0;
  initial begin
    fork
      begin
        #100000000000;                 // far future; beyond max_time
        ran = 1;
        $display("FAIL_B watchdog survived disable fork");
      end
    join_none
    #0;
    disable fork;                      // kill watchdog + drop its timer
    $display("B_KILLED ran=%0d", ran);
    // no $finish: rely on idleness detection to terminate
  end
endmodule
"#;

    // Use a max_time far beyond the watchdog timer so the ONLY thing that can
    // terminate the simulation is idleness. With the fix the killed watchdog's
    // future `#delay` is dropped and the sim idles at time 0. If the timer were
    // left behind (the bug), the queue stays non-empty and `self.time` would
    // advance all the way toward the leaked far event, so `sim.time` blowing
    // past a tiny bound proves the leak — independent of where the event loop
    // max-time stops.
    let sim = simulate(SRC, 100_000_000_000).expect("simulation failed");
    let msgs: Vec<String> = sim
        .output
        .iter()
        .map(|l| l.message.clone())
        .filter(|m| m.contains("B_KILLED") || m.contains("FAIL_B"))
        .collect();
    assert!(
        sim.time < 1_000,
        "simulation time advanced to {} — a killed process's #delay was left in the timing wheel\n{msgs:?}",
        sim.time
    );
    assert!(
        msgs.iter().all(|m| !m.contains("FAIL_B")),
        "the disable-fork-killed watchdog must never run: {msgs:?}"
    );
    assert!(
        msgs.iter().any(|m| m.starts_with("B_KILLED ran=0")),
        "watchdog must have been killed (ran must be 0): {msgs:?}"
    );
}