//! `wait` condition re-evaluation: the raise-then-drop synchronization idiom.
//!
//! IEEE 1800.1-2023 §10.4 `wait` suspends a process until its expression
//! becomes true, re-evaluating when any operand changes. A naive
//! implementation of "wait until all objections are dropped" as a bare
//! `wait(total == 0)` is wrong: at entry `total` is 0 (nothing has raised
//! yet), so the process returns immediately instead of waiting for a raise
//! followed by a drop. The correct idiom is the raise-then-drop pair:
//!
//! ```systemverilog
//! wait(total > 0);   // block until something is raised
//! wait(total == 0);  // then block until all are dropped
//! ```
//!
//! This is the plain-SV synchronization that an objection/phase waiter relies
//! on. Two facets are pinned here, both in pure SystemVerilog (no UVM
//! library, no special build mode):
//!
//!   1. The raise-then-drop idiom blocks until the drop (not released at t=0).
//!   2. The idiom works when the threshold/event is a VARIABLE (passed as a
//!      task argument), not only a literal condition.

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

/// Objection-style counter with a raise-then-drop waiter. The waiter must
/// release only after a raise followed by a drop, not at t=0.
const SRC: &str = r#"
class objection;
  int total;
  function new; total = 0; endfunction
  function void raise; total = total + 1; endfunction
  function void drop;  total = total - 1; endfunction
  task wait_done;
    wait(total > 0);   // block until first raise
    wait(total == 0);  // then block until all dropped
  endtask
endclass

module top;
  initial begin
    objection o;
    o = new;
    fork
      begin
        #50; o.raise();
        #10; o.drop();
        $display("RAISER_DONE at %0t", $time);
      end
    join_none
    o.wait_done();
    $display("WAITER_DONE at %0t", $time);
  end
endmodule
"#;

pub(super) fn raise_then_drop_idiom_blocks_until_drop_not_t0() {
    let sim = simulate(SRC, 200).expect("simulate failed");
    let msgs = messages(&sim);
    let waiter = msgs
        .iter()
        .find(|m| m.starts_with("WAITER_DONE"))
        .unwrap_or_else(|| panic!("waiter never released; output: {:?}", msgs));
    // raise at t=50, drop at t=60 -> release at t=60.
    // A bare `wait(total==0)` would release at t=0; a broken sensitivity
    // would never release.
    assert!(
        waiter.contains("at 60"),
        "expected waiter released at t=60 after raise+drop, got: {}",
        waiter
    );
}

/// The threshold/event may be a VARIABLE (a task argument), not only a
/// literal. A wait driven by a runtime value must re-evaluate correctly.
pub(super) fn wait_idiom_works_with_a_variable_threshold() {
    let src = r#"
class objection;
  int total;
  function new; total = 0; endfunction
  function void raise; total = total + 1; endfunction
  function void drop;  total = total - 1; endfunction
  // `level` is a VARIABLE passed in at call time, not a literal.
  task wait_until_le(int level);
    wait(total > level);
    wait(total <= level);
  endtask
endclass

module top;
  initial begin
    objection o;
    int lvl;
    o = new;
    lvl = 0;
    fork
      begin
        #30; o.raise();
        #40; o.drop();
      end
    join_none
    o.wait_until_le(lvl);   // variable threshold
    $display("VAR_WAITER_DONE at %0t", $time);
  end
endmodule
"#;
    let sim = simulate(src, 200).expect("simulate failed");
    let msgs = messages(&sim);
    let waiter = msgs
        .iter()
        .find(|m| m.starts_with("VAR_WAITER_DONE"))
        .unwrap_or_else(|| {
            panic!(
                "variable-threshold waiter never released; output: {:?}",
                msgs
            )
        });
    // raise at t=30 (total>0 true, then <=0 false), drop at t=70 (<=0 true).
    assert!(
        waiter.contains("at 70"),
        "expected variable-threshold waiter released at t=70, got: {}",
        waiter
    );
}
