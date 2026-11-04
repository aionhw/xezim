//! Objection-gated run-phase reaching a target time, with a component-tree
//! build — a pure-SystemVerilog distillation of a full UVM test run.
//!
//! The genuine-UVM integration run (real Accellera library + DPI `.so`) is
//! heavy and host-dependent. The SV *mechanisms* it ultimately exercises —
//! building a component hierarchy, an objection-gated `run_phase` that raises,
//! advances to a target time, then drops, and a phase waiter that completes
//! when the objection drops — are expressible in plain SystemVerilog. This
//! test reproduces that core loop hermetically (no UVM library, no DPI, runs
//! in-process via `simulate`):
//!
//!   - build: construct a small `component` hierarchy and count its children;
//!   - run_phase body: `raise` (§ raise-then-drop), advance `#99`, `drop`;
//!   - phase waiter: `wait(total > 0); wait(total == 0)` completes on the drop.
//!
//! Assertions mirror the genuine run: build succeeds, the Starting/Finishing
//! banners print, and the phase completes at t=100 with no fatal/error.

use xezim::simulate;

const SRC: &str = r#"
class component;
  string name;
  component children[$];
  function new(string n); name = n; endfunction
  function void add(component c); children.push_back(c); endfunction
  function int count; return children.size(); endfunction
endclass

class phase;
  // Shared objection total for the run-phase / waiter handshake.
  static int total = 0;
  static function void raise; total = total + 1; endfunction
  static function void drop;  total = total - 1; endfunction
  // raise-then-drop idiom: wait for a raise, then for all drops.
  static task wait_done;
    wait(total > 0);
    wait(total == 0);
  endtask
endclass

module top;
  int built;
  int done_time;
  initial begin
    // build_phase: construct a small hierarchy test -> env -> agent.
    automatic component root  = new("test");
    automatic component env   = new("env");
    automatic component agent = new("agent");
    root.add(env);
    env.add(agent);
    built = root.count() + env.count();
    $display("BUILT children=%0d at %0t", built, $time);

    // run_phase body: raise, advance to the target time, drop.
    fork
      begin
        #1; phase::raise();
        $display("STARTING at %0t", $time);
        #99;
        $display("FINISHING at %0t", $time);
        phase::drop();
      end
    join_none

    // Phase waiter: completes once the objection is raised then dropped.
    phase::wait_done();
    done_time = $time;
    $display("PHASE_DONE done_time=%0d at %0t", done_time, $time);
  end
endmodule
"#;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

pub(super) fn objection_gated_run_phase_reaches_target_time() {
    let sim = simulate(SRC, 1000).expect("simulate failed");
    let msgs = messages(&sim);

    // build_phase constructed the hierarchy (root has 1 child, env has 1).
    assert!(
        msgs.iter().any(|m| m == "BUILT children=2 at 0"),
        "build_phase should report 2 children; output: {:?}",
        msgs
    );
    // run_phase raised and printed Starting.
    assert!(
        msgs.iter().any(|m| m == "STARTING at 1"),
        "run_phase should start at t=1; output: {:?}",
        msgs
    );
    // run_phase advanced #99 and printed Finishing.
    assert!(
        msgs.iter().any(|m| m == "FINISHING at 100"),
        "run_phase should finish at t=100; output: {:?}",
        msgs
    );
    // The objection-gated phase waiter completed at the drop (t=100).
    assert!(
        msgs.iter().any(|m| m == "PHASE_DONE done_time=100 at 100"),
        "phase should complete at t=100; output: {:?}",
        msgs
    );
    // No fatal/error surfaced during the run.
    assert!(
        !msgs
            .iter()
            .any(|m| m.to_lowercase().contains("fatal") || m.to_lowercase().contains("error")),
        "expected a clean run (no fatal/error); output: {:?}",
        msgs
    );
}
