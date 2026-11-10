//! §6.21, §9.3.2: a fork child shares the automatic variables of the enclosing
//! scope (#935). Each child works on a copy, and a write is detected by
//! comparing the copy with a baseline. The baseline was the value at the fork,
//! so a write of that same value was taken for "unchanged" and dropped, and a
//! child that had only received a sibling's write sent it back as its own.
//! Expected values are the reference simulator's.

use xezim::simulate;

fn run(src: &str, tag: &str) {
    let sim = simulate(src, 1000).unwrap_or_else(|e| panic!("{e}"));
    let msgs: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert!(
        msgs.iter().any(|m| m.contains(&format!("{tag}_PASS"))),
        "{msgs:?}"
    );
}

/// A sibling writes the value the variable held at the fork, after the other
/// child changed it: the parent must see the sibling's write.
#[test]
fn a_sibling_write_of_the_fork_time_value_reaches_the_parent() {
    run(
        r#"
module top;
  task automatic t();
    int req = 5;
    fork
      begin req = 0; #10; end
      begin #5; req = 5; #20; end
    join_any
    if (req == 5) $display("SIB_TASK_PASS"); else $display("SIB_TASK_FAIL req=%0d", req);
    disable fork;
  endtask
  initial begin t(); $finish; end
endmodule
"#,
        "SIB_TASK",
    );
}

/// A child changes the variable, parks, then writes the fork-time value back.
#[test]
fn a_child_write_back_to_the_fork_time_value_reaches_the_parent() {
    run(
        r#"
module top;
  task automatic t();
    int req = 5;
    fork
      begin req = 0; #10; req = 5; end
      begin #100; end
    join_any
    if (req == 5) $display("SELF_TASK_PASS"); else $display("SELF_TASK_FAIL req=%0d", req);
    disable fork;
  endtask
  initial begin t(); $finish; end
endmodule
"#,
        "SELF_TASK",
    );
}

/// As the first case, for an automatic declared in the `initial` block.
#[test]
fn a_sibling_write_of_the_fork_time_value_reaches_a_block_local() {
    run(
        r#"
module top;
  initial begin
    automatic int req = 5;
    fork
      begin req = 0; #10; end
      begin #5; req = 5; #20; end
    join_any
    if (req == 5) $display("SIB_INIT_PASS"); else $display("SIB_INIT_FAIL req=%0d", req);
    disable fork;
    $finish;
  end
endmodule
"#,
        "SIB_INIT",
    );
}

/// The second child only received the first child's write. When it finishes
/// after the parent's own later write, it must not send that value back.
#[test]
fn a_child_that_only_received_a_sibling_write_does_not_resend_it() {
    run(
        r#"
module top;
  task automatic t();
    int req = 5;
    fork
      begin req = 0; #10; end
      begin #20; end
    join_none
    #15 req = 9;
    #10;
    if (req == 9) $display("PARENT_KEEPS_PASS"); else $display("PARENT_KEEPS_FAIL req=%0d", req);
  endtask
  initial begin t(); $finish; end
endmodule
"#,
        "PARENT_KEEPS",
    );
}
