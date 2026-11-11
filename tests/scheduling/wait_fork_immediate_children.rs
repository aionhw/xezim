//! §9.6.1 — `wait fork` blocks on the calling process's IMMEDIATE children
//! only. Reference-validated.
//!
//! The implementation waited on the transitive descendant closure, and the
//! wake condition recomputed that closure against the live tree — doubly
//! wrong, because a completed child REPARENTS its grandchildren to the waiting
//! parent, so the wait chased the tree as it grew. A `join_none` grandchild
//! extended the wait past every child's completion; a persistent monitor
//! spawned by a child (the standard UVM driver shape) hung it forever. The
//! asymmetry is the LRM's own: `disable fork` kills all descendants, `wait
//! fork` waits on immediate children.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able (x/z?)", n))
}

/// A grandchild outliving its parent must not extend the wait.
#[test]
fn wait_fork_returns_when_children_finish() {
    let src = r#"
module tb;
  int t_after, t_deep;
  task automatic spawn_deep;
    fork begin #6; t_deep = $time; end join_none
  endtask
  task automatic t;
    fork begin #1 spawn_deep(); end join_none
    wait fork;
    t_after = $time;
  endtask
  initial begin
    t();
    #20;
  end
endmodule
"#;
    let sim = simulate(src, 200).expect("simulate failed");
    assert_eq!(u(&sim, "t_after"), 1, "returns when the CHILD finishes");
    assert_eq!(
        u(&sim, "t_deep"),
        7,
        "the grandchild still runs to completion"
    );
}

/// All immediate children are still awaited — the fix must not weaken that.
#[test]
fn wait_fork_still_waits_for_every_child() {
    let src = r#"
module tb;
  int t_after;
  initial begin
    fork
      #3;
      #7;
      #5;
    join_none
    wait fork;
    t_after = $time;
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulate failed");
    assert_eq!(
        u(&sim, "t_after"),
        7,
        "the slowest immediate child gates the wait"
    );
}

/// The `T|` lines of `module t` with `body`, in output order.
fn t_lines(body: &str) -> Vec<String> {
    let src = format!("module t;\n  int done, c;\n{body}\nendmodule\n");
    let sim = simulate(&src, 200).unwrap_or_else(|e| panic!("rejected: {e}"));
    sim.output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect()
}

// A child that has finished before `wait fork` runs hands its running
// children up to the waiting process so that `disable fork` still reaches
// them (§9.6.3). They are not that process's immediate children, so `wait
// fork` does not wait for them (§9.6.1). Expected values are the reference
// simulator's.

#[test]
fn a_grandchild_of_a_joined_child_is_not_waited_for() {
    let o = t_lines(
        r#"
  initial begin
    fork begin fork #10 done++; join_none end join
    wait fork;
    $display("T|w done=%0d t=%0t", done, $time);
    #20 $display("T|end done=%0d", done);
  end
"#,
    );
    assert_eq!(o, ["T|w done=0 t=0", "T|end done=1"]);
}

#[test]
fn a_grandchild_of_a_finished_join_none_child_is_not_waited_for() {
    let o = t_lines(
        r#"
  initial begin
    fork begin fork #10 done++; join_none end join_none
    #1;
    wait fork;
    $display("T|w done=%0d t=%0t", done, $time);
    #20 $display("T|end done=%0d", done);
  end
"#,
    );
    assert_eq!(o, ["T|w done=0 t=1", "T|end done=1"]);
}

#[test]
fn a_great_grandchild_is_not_waited_for() {
    let o = t_lines(
        r#"
  initial begin
    fork begin fork begin fork #10 done++; join_none end join_none end join
    wait fork;
    $display("T|w done=%0d t=%0t", done, $time);
    #20 $display("T|end done=%0d", done);
  end
"#,
    );
    assert_eq!(o, ["T|w done=0 t=0", "T|end done=1"]);
}

#[test]
fn wait_fork_waits_for_the_running_child_and_not_its_finished_siblings_child() {
    let o = t_lines(
        r#"
  initial begin
    fork #5 c++; begin fork #20 done++; join_none end join_any
    #1;
    wait fork;
    $display("T|w c=%0d done=%0d t=%0t", c, done, $time);
    #30 $display("T|end done=%0d", done);
  end
"#,
    );
    assert_eq!(o, ["T|w c=1 done=0 t=5", "T|end done=1"]);
}

#[test]
fn a_grandchild_forked_in_a_task_or_beside_a_new_child_is_not_waited_for() {
    let o = t_lines(
        r#"
  task automatic spawn; fork #10 done++; join_none endtask
  task automatic spawn_and_go; fork begin spawn(); end #3 c++; join endtask
  initial begin
    spawn_and_go();
    wait fork;
    $display("T|task done=%0d c=%0d t=%0t", done, c, $time);
    fork begin fork #10 done++; join_none end join
    fork #4 c++; join_none
    wait fork;
    $display("T|new c=%0d done=%0d t=%0t", c, done, $time);
    #30 $display("T|end done=%0d", done);
  end
"#,
    );
    assert_eq!(
        o,
        [
            "T|task done=0 c=1 t=3",
            "T|new c=2 done=0 t=7",
            "T|end done=2"
        ]
    );
}

#[test]
fn a_child_running_wait_fork_does_not_see_its_parents_grandchild() {
    let o = t_lines(
        r#"
  initial begin
    fork begin fork #10 done++; join_none end join
    fork begin wait fork; $display("T|child t=%0t", $time); end join
    wait fork;
    $display("T|w done=%0d t=%0t", done, $time);
    #20 $display("T|end done=%0d", done);
  end
"#,
    );
    assert_eq!(o, ["T|child t=0", "T|w done=0 t=0", "T|end done=1"]);
}

/// The handed-up grandchild is still a descendant: `disable fork` kills it,
/// and a child still running when `wait fork` starts is waited for.
#[test]
fn disable_fork_still_kills_a_handed_up_grandchild() {
    let o = t_lines(
        r#"
  initial begin
    fork begin fork #10 done++; join_none end join
    disable fork;
    #20 $display("T|disable done=%0d t=%0t", done, $time);
  end
  initial begin
    fork begin fork #10 c++; join_none #2; end join_none
    wait fork;
    $display("T|running c=%0d t=%0t", c, $time);
  end
"#,
    );
    assert_eq!(o, ["T|running c=0 t=2", "T|disable done=0 t=20"]);
}
