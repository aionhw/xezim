//! §9.6.2: `disable` names a task or a named block, resolved like any other
//! name from where it is written (§23.9), through generate scopes by path
//! (§27.5, §27.4), and upward from a child instance (§23.8.1). Verilator
//! 5.055 reports a `disable` by path into a generate block as unsupported;
//! where it runs a program, its value is the one expected here.
use xezim::simulate;

/// The `T|` lines, sorted.
fn t_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 1000).unwrap_or_else(|e| panic!("rejected: {e}"));
    let mut lines: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect();
    lines.sort();
    lines
}

#[test]
fn a_disable_by_path_stops_the_block_task_it_names() {
    let src = r#"
module top;
  int hit = 0;
  if (1) begin : g
    task automatic t(); #5; hit = 1; endtask
    initial t();
  end
  initial begin #1; disable g.t; #9; $display("T|%0d", hit); end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|0"]);
}

#[test]
fn a_disable_by_path_from_another_block_stops_that_blocks_task() {
    let src = r#"
module top; int a = 0, b = 0;
  if (1) begin : ga task automatic t(); #5 a = 1; endtask initial t(); end
  if (1) begin : gb task automatic t(); #5 b = 1; endtask initial t(); initial #1 disable ga.t; end
  initial #9 $display("T|a=%0d b=%0d", a, b);
endmodule
"#;
    assert_eq!(t_lines(src), ["T|a=0 b=1"]);
}

#[test]
fn a_disable_by_path_of_one_of_two_running_namesakes_leaves_the_other() {
    let src = r#"
module top; int x = 0, y = 0;
  if (1) begin : a task automatic t(); #5 x = 1; endtask end
  if (1) begin : b task automatic t(); #5 y = 1; endtask end
  initial a.t();
  initial b.t();
  initial #1 disable a.t;
  initial #9 $display("T|x=%0d y=%0d", x, y);
endmodule
"#;
    assert_eq!(t_lines(src), ["T|x=0 y=1"]);
}

/// Neither `disable` names the running `b.t`.
#[test]
fn a_disable_of_an_idle_block_task_leaves_a_running_namesake() {
    let src = r#"
module top; int y = 0, z = 0;
  if (1) begin : a task automatic t(); #5 y = 2; endtask initial #1 disable t; end
  if (1) begin : b task automatic t(); #5 y = 1; endtask end
  if (1) begin : c task automatic t(); #5 z = 2; endtask end
  if (1) begin : d task automatic t(); #5 z = 1; endtask end
  initial b.t();
  initial d.t();
  initial #1 disable c.t;
  initial #9 $display("T|y=%0d z=%0d", y, z);
endmodule
"#;
    assert_eq!(t_lines(src), ["T|y=1 z=1"]);
}

#[test]
fn a_disable_by_path_names_a_task_of_a_nested_block() {
    let src = r#"
module top; int hit = 0;
  if (1) begin : g if (1) begin : h task automatic t(); #5 hit = 1; endtask initial t(); end end
  initial begin #1 disable g.h.t; #9 $display("T|%0d", hit); end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|0"]);
}

#[test]
fn a_disable_by_path_with_a_loop_index_stops_that_iterations_task() {
    // §27.4.
    let src = r#"
module top; int hit[2];
  for (genvar i = 0; i < 2; i++) begin : g task automatic t(); #5 hit[i] = 1; endtask initial t(); end
  initial begin #1 disable g[1].t; #9 $display("T|%0d %0d", hit[0], hit[1]); end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|1 0"]);
}

#[test]
fn a_disable_by_path_inside_an_instance_stops_that_instances_task() {
    let src = r#"
module m; int hit = 0;
  if (1) begin : g task automatic t(); #5 hit = 1; endtask initial t(); end
  initial #1 disable g.t;
endmodule
module top; m u1(); m u2();
  initial #10 $display("T|%0d %0d", u1.hit, u2.hit);
endmodule
"#;
    assert_eq!(t_lines(src), ["T|0 0"]);
}

/// Verilator: `0 0`.
#[test]
fn a_disable_inside_a_block_of_an_instance_stops_the_blocks_task() {
    let src = r#"
module m; int hit = 0;
  if (1) begin : g task automatic t(); #5 hit = 1; endtask initial t(); initial #1 disable t; end
endmodule
module top; m u1(); m u2();
  initial #10 $display("T|%0d %0d", u1.hit, u2.hit);
endmodule
"#;
    assert_eq!(t_lines(src), ["T|0 0"]);
}

#[test]
fn a_child_disables_the_task_of_its_parents_block_upward() {
    // §23.8.1.
    let src = r#"
module child; initial #1 disable t; endmodule
module top #(parameter int P = 1);
  int a = 0;
  if (P == 1) begin : g task automatic t(); #3 a = 1; endtask initial t(); child c(); end
  if (P == 2) begin : h task automatic t(); endtask end
  initial #5 $display("T|a=%0d", a);
endmodule
"#;
    assert_eq!(t_lines(src), ["T|a=0"]);
}

/// Disabling `g_a.t` stops the `g_b.t` it is executing.
#[test]
fn a_disable_of_the_calling_task_stops_the_task_it_calls() {
    let src = r#"
module top; int a = 0, b = 0;
  if (1) begin : g_b task automatic t(); #5 b = 1; endtask end
  if (1) begin : g_a task automatic t(); g_b.t(); a = 1; endtask initial t(); end
  initial #1 disable g_a.t;
  initial #9 $display("T|a=%0d b=%0d", a, b);
endmodule
"#;
    assert_eq!(t_lines(src), ["T|a=0 b=0"]);
}

/// §9.3.4, §23.9: under the unnamed fork, label `t` is a name of `o`, nearer
/// than the block's task. Verilator: `0 1`.
#[test]
fn a_disable_of_a_label_that_hides_a_block_task_stops_the_labelled_block() {
    let src = r#"
module top; int a = 0, b = 0;
  if (1) begin : g task automatic t(); #5 b = 1; endtask initial t();
    initial begin : o fork begin : t #5 a = 1; end #1 disable t; join end
  end
  initial #9 $display("T|%0d %0d", a, b);
endmodule
"#;
    assert_eq!(t_lines(src), ["T|0 1"]);
}

/// §9.3.5: the process label `t` of `b` is nearer than `a`'s task.
/// Verilator: `0`.
#[test]
fn a_disable_binds_to_a_process_label_of_the_inner_block() {
    let src = r#"
module top; int x = 0;
  if (1) begin : a task automatic t(); endtask
    if (1) begin : b initial begin : t #5 x = 1; end initial #1 disable t; end
  end
  initial #10 $display("T|%0d", x);
endmodule
"#;
    assert_eq!(t_lines(src), ["T|0"]);
}

#[test]
fn a_task_called_by_path_disables_itself_by_its_name() {
    let src = r#"
module top;
  if (1) begin : a task t; $display("T|a in"); disable t; $display("T|a BAD"); endtask end
  if (1) begin : b task t; $display("T|b in"); #1 disable t; $display("T|b BAD"); endtask end
  initial begin a.t(); b.t(); $display("T|end %0t", $time); end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|a in", "T|b in", "T|end 1"]);
}

/// Verilator: `after 1`, `x=0`.
#[test]
fn a_disable_by_path_of_a_named_block_stops_another_process() {
    let src = r#"
module top; int x = 0;
  initial begin : blk #5 x = 1; end
  initial begin #1 disable top.blk; $display("T|after %0t", $time); #9 $display("T|x=%0d", x); end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|after 1", "T|x=0"]);
}
