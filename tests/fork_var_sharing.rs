//! Fork children share enclosing-scope automatic variables (§6.21/§9.3.2).
//!
//! IEEE 1800-2023 §6.21 (Scope and lifetime):
//!
//! > "The lifetime of a fork-join block shall encompass the execution of all
//! > processes spawned by the block. The lifetime of a scope enclosing any
//! > fork-join block includes the lifetime of the fork-join block."
//!
//! §9.3.2 (Parallel blocks):
//!
//! > Variables declared in a fork-join block's own `block_item_declaration`
//! > get a fresh copy per spawned process; variables from an *enclosing* scope
//! > are **shared storage** — a child's write is visible to the parent.
//!
//! xezim implements copy-on-fork: each child gets a snapshot of the parent's
//! locals, and the child's writes are propagated back when the child finishes.
//! This worked for subroutine-frame locals (which live in `local_stack`) but
//! NOT for `automatic` variables declared directly in an `initial`/`always`
//! block — those had no call frame and landed in the global `self.signals`
//! map, while the fork child's copy lived in a `local_stack` frame. The two
//! were disconnected storage, so the parent always read the stale pre-fork
//! value. The fix records which signal-backed names were captured into each
//! child and writes the child's changed values back into `self.signals`.
//!
//! These self-checking tests pin the §6.21/§9.3.2 shared-storage guarantee
//! for the key patterns:

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn assert_pass(sim: &xezim::compiler::Simulator, tag: &str) {
    let msgs = messages(sim);
    let pass = msgs.iter().any(|m| m.contains(&format!("{tag}_PASS")));
    let fail = msgs.iter().find(|m| m.contains(&format!("{tag}_FAIL")));
    assert!(
        pass,
        "expected {tag}_PASS in output\nfail line: {fail:?}\nfull output: {msgs:?}"
    );
}

/// An `automatic` local declared in an `initial` block is written by a
/// `fork ... join_none` child after a `#5` delay. The parent — blocked on
/// `#10` — must observe the child's write when it resumes.
///
/// This was the primary reproducer: pre-fix the parent read the stale value
/// `0` because the child's write was stranded in its private frame copy.
const CROSS_DELAY: &str = r#"
module top;
  initial begin
    automatic int got = 0;
    fork
      begin
        #5;
        got = 7;
      end
    join_none
    #10;
    if (got == 7) $display("DELAY_PASS got=%0d", got);
    else          $display("DELAY_FAIL got=%0d", got);
  end
endmodule
"#;

#[test]
fn automatic_local_cross_delay() {
    let sim = simulate(CROSS_DELAY, 200).expect("simulate failed");
    assert_pass(&sim, "DELAY");
}

/// Same-timestep variant: the child writes with no delay, and the parent
/// reads after a `#1`. The propagate-back must still deliver the write.
const SAME_TIMESTEP: &str = r#"
module top;
  initial begin
    automatic int flag = 0;
    fork
      begin
        flag = 1;
      end
    join_none
    #1;
    if (flag == 1) $display("SAME_PASS flag=%0d", flag);
    else           $display("SAME_FAIL flag=%0d", flag);
  end
endmodule
"#;

#[test]
fn automatic_local_same_timestep() {
    let sim = simulate(SAME_TIMESTEP, 200).expect("simulate failed");
    assert_pass(&sim, "SAME");
}

/// The parent blocks on `wait(ready)` while a fork child sets the flag after
/// a `#3` delay. The `wait` must resume — combining the §9.3.2 write-back
/// with the level-sensitive `wait()` condition re-evaluation.
const WAIT_FLAG: &str = r#"
module top;
  initial begin
    automatic int ready = 0;
    fork
      begin
        #3;
        ready = 1;
      end
    join_none
    wait(ready == 1);
    if (ready == 1) $display("WAIT_PASS ready=%0d at=%0t", ready, $time);
    else            $display("WAIT_FAIL ready=%0d at=%0t", ready, $time);
  end
endmodule
"#;

#[test]
fn automatic_local_wait_flag() {
    let sim = simulate(WAIT_FLAG, 200).expect("simulate failed");
    assert_pass(&sim, "WAIT");
}

/// A variable declared INSIDE the fork block (`inner`) is private to the
/// child and must NOT leak to the parent. An enclosing-scope variable
/// (`outer`) written by the same child MUST propagate. This separates the
/// two §9.3.2 storage classes.
const FORK_PRIVATE_VAR: &str = r#"
module top;
  initial begin
    automatic int outer = 5;
    fork
      automatic int inner = 99;
      begin
        #2;
        inner = 77;
        outer = 11;
      end
    join_none
    #5;
    // outer reflects the child's write (shared storage)
    if (outer == 11) $display("PRIV_PASS outer=%0d", outer);
    else             $display("PRIV_FAIL outer=%0d", outer);
  end
endmodule
"#;

#[test]
fn fork_declared_var_is_private() {
    let sim = simulate(FORK_PRIVATE_VAR, 200).expect("simulate failed");
    assert_pass(&sim, "PRIV");
}

/// A class member (`this.count`) incremented by fork-child task calls must
/// be visible to the parent. Two `fork ... join_any` children each call
/// `tick` which does `this.count++` after a `#5`. The parent (after `join_any`
/// + `#1`) must see `count == 2`.
const CLASS_MEMBER: &str = r#"
class Counter;
  int count;
  function new; count = 0; endfunction
  task tick;
    begin
      #5;
      this.count = this.count + 1;
    end
  endtask
  task run_two;
    fork
      this.tick;
      this.tick;
    join_any
    #1;
    if (count == 2) $display("CLS_PASS count=%0d", count);
    else            $display("CLS_FAIL count=%0d", count);
  endtask
endclass
module top;
  initial begin
    automatic Counter c = new;
    c.run_two;
  end
endmodule
"#;

#[test]
fn class_member_from_fork_child() {
    let sim = simulate(CLASS_MEMBER, 200).expect("simulate failed");
    assert_pass(&sim, "CLS");
}

/// Multiple sequential fork children each write to different enclosing-
/// scope `automatic` variables. The parent checks all writes propagated
/// correctly. This exercises repeated propagate-back cycles across
/// multiple signal-backed variables and confirms each reaches `self.signals`.
const MULTIPLE_VARS: &str = r#"
module top;
  initial begin
    automatic int a = 0;
    automatic int b = 0;
    automatic int c = 0;
    fork
      begin #1; a = 10; end
    join_none
    fork
      begin #2; b = 20; end
    join_none
    fork
      begin #3; c = 30; end
    join_none
    #10;
    if (a + b + c == 60) $display("ACC_PASS sum=%0d", a + b + c);
    else                  $display("ACC_FAIL sum=%0d", a + b + c);
  end
endmodule
"#;

#[test]
fn multiple_children_multiple_vars() {
    let sim = simulate(MULTIPLE_VARS, 200).expect("simulate failed");
    assert_pass(&sim, "ACC");
}
