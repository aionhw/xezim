//! IEEE 1800-2023 §9.6.2 and §9.6.3: named disable terminates the
//! subprocesses of the resolved block in every active invocation. The
//! disable fork statement follows the calling process's descendants.

use xezim::simulate;

fn check(source: &str, expected: &[&str]) {
    let sim = simulate(source, 10_000).expect("simulation failed");
    let output: Vec<_> = sim
        .output
        .iter()
        .map(|line| line.message.as_str())
        .collect();
    assert_eq!(output, expected);
}

#[test]
fn each_instance_disables_its_own_named_fork() {
    let source = r#"`timescale 1ns/1ps
module child(input bit go);
  int seen;
  initial begin
    fork : dl
      #1;
      begin wait (go); seen = 1; end
    join_any
    disable dl;
  end
endmodule

module top;
  bit go;
  child a(go), b(go);
  initial begin
    #2 go = 1;
    #1;
    $display("top.a seen=%0d", a.seen);
    $display("top.b seen=%0d", b.seen);
    if (a.seen !== 0 || b.seen !== 0) $fatal(1, "FAIL");
    $display("PASS");
    $finish;
  end
endmodule
"#;
    check(source, &["top.a seen=0", "top.b seen=0", "PASS"]);
}

#[test]
fn local_disable_spares_the_other_instances_fork() {
    let source = r#"`timescale 1ns/1ps
module child #(parameter bit CANCEL = 0)(input bit go);
  int seen;
  initial begin
    fork : dl
      #1;
      begin wait (go); seen = 1; end
    join_any
    if (CANCEL) disable dl;
  end
endmodule

module top;
  bit go;
  child #(1) a(go);
  child b(go);
  initial begin
    #2 go = 1;
    #1;
    $display("top.a seen=%0d", a.seen);
    $display("top.b seen=%0d", b.seen);
    if (a.seen !== 0 || b.seen !== 1) $fatal(1, "FAIL");
    $display("PASS");
    $finish;
  end
endmodule
"#;
    check(source, &["top.a seen=0", "top.b seen=1", "PASS"]);
}

#[test]
fn hierarchical_disable_selects_the_named_instance() {
    let source = r#"`timescale 1ns/1ps
module child(input bit go);
  int seen;
  initial begin
    fork : dl
      #1;
      begin wait (go); seen = 1; end
    join_any
  end
endmodule

module top;
  bit go;
  child a(go), b(go);
  initial begin
    #2 disable a.dl;
    #1 go = 1;
    #1;
    $display("top.a seen=%0d", a.seen);
    $display("top.b seen=%0d", b.seen);
    if (a.seen !== 0 || b.seen !== 1) $fatal(1, "FAIL");
    $display("PASS");
    $finish;
  end
endmodule
"#;
    check(source, &["top.a seen=0", "top.b seen=1", "PASS"]);
}

#[test]
fn one_disable_reaches_concurrent_calls_of_the_same_task() {
    let source = r#"`timescale 1ns/1ps
module top;
  bit go;
  int a_seen, b_seen;

  task automatic work(input bit second);
    fork : dl
      #2;
      begin
        wait (go);
        if (second) b_seen = 1;
        else a_seen = 1;
      end
    join_any
    if (!second) disable dl;
  endtask

  initial work(0);
  initial #1 work(1);
  initial begin
    #4 go = 1;
    #1;
    $display("a seen=%0d", a_seen);
    $display("b seen=%0d", b_seen);
    if (a_seen !== 0 || b_seen !== 0) $fatal(1, "FAIL");
    $display("PASS");
    $finish;
  end
endmodule
"#;
    check(source, &["a seen=0", "b seen=0", "PASS"]);
}

#[test]
fn disabling_the_only_named_fork_spares_the_unnamed_fork() {
    let source = r#"`timescale 1ns/1ps
module child(input bit go);
  int seen;
  initial begin
    fork : dl
      #1;
      begin wait (go); seen = 1; end
    join_any
    disable dl;
  end
endmodule

module unnamed_child(input bit go);
  int seen;
  initial begin
    fork
      #1;
      begin wait (go); seen = 1; end
    join_any
  end
endmodule

module top;
  bit go;
  child a(go);
  unnamed_child b(go);
  initial begin
    #2 go = 1;
    #1;
    $display("top.a seen=%0d", a.seen);
    $display("top.b seen=%0d", b.seen);
    if (a.seen !== 0 || b.seen !== 1) $fatal(1, "FAIL");
    $display("PASS");
    $finish;
  end
endmodule
"#;
    check(source, &["top.a seen=0", "top.b seen=1", "PASS"]);
}

#[test]
fn disable_fork_follows_each_callers_descendants() {
    let source = r#"`timescale 1ns/1ps
module child(input bit go);
  int seen;
  initial begin
    fork : dl
      #1;
      begin wait (go); seen = 1; end
    join_any
    disable fork;
  end
endmodule

module top;
  bit go;
  child a(go), b(go);
  initial begin
    #2 go = 1;
    #1;
    $display("top.a seen=%0d", a.seen);
    $display("top.b seen=%0d", b.seen);
    if (a.seen !== 0 || b.seen !== 0) $fatal(1, "FAIL");
    $display("PASS");
    $finish;
  end
endmodule
"#;
    check(source, &["top.a seen=0", "top.b seen=0", "PASS"]);
}

#[test]
fn disable_reaches_a_named_fork_in_an_automatic_task() {
    let source = r#"`timescale 1ns/1ps
module child(input bit go);
  int seen;
  task automatic work();
    fork : dl
      #1;
      begin wait (go); seen = 1; end
    join_any
    disable dl;
  endtask

  initial work();
endmodule

module top;
  bit go;
  child a(go);
  initial begin
    #2 go = 1;
    #1;
    $display("top.a seen=%0d", a.seen);
    if (a.seen !== 0) $fatal(1, "FAIL");
    $display("PASS");
    $finish;
  end
endmodule
"#;
    check(source, &["top.a seen=0", "PASS"]);
}

#[test]
fn named_disable_selects_one_of_two_distinct_labels() {
    let source = r#"`timescale 1ns/1ps
module top;
  bit go;
  int a_seen, b_seen;

  initial begin
    fork : first
      begin wait (go); a_seen = 1; end
    join_none
    fork : second
      begin wait (go); b_seen = 1; end
    join_none
    #1 disable first;
    #1 go = 1;
    #1;
    $display("a seen=%0d", a_seen);
    $display("b seen=%0d", b_seen);
    if (a_seen !== 0 || b_seen !== 1) $fatal(1, "FAIL");
    $display("PASS");
    $finish;
  end
endmodule
"#;
    check(source, &["a seen=0", "b seen=1", "PASS"]);
}

#[test]
fn disable_reaches_the_named_fork_in_a_single_instance() {
    let source = r#"`timescale 1ns/1ps
module child(input bit go);
  int seen;
  initial begin
    fork : dl
      #1;
      begin wait (go); seen = 1; end
    join_any
    disable dl;
  end
endmodule

module top;
  bit go;
  child a(go);
  initial begin
    #2 go = 1;
    #1;
    $display("top.a seen=%0d", a.seen);
    if (a.seen !== 0) $fatal(1, "FAIL");
    $display("PASS");
    $finish;
  end
endmodule
"#;
    check(source, &["top.a seen=0", "PASS"]);
}
