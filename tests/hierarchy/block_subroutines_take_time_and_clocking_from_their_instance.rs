//! §3.14, §14.3, §23.9: a generate block is not a design element. A subroutine
//! or process inside one runs on the time scope of the nearest enclosing
//! instance, and finds a clocking block in the block itself or in an enclosing
//! scope of that instance.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 10_000_000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn a_block_task_in_an_instance_runs_on_the_instance_timescale_bare_and_by_path() {
    let src = r#"
`timescale 1us/1ns
module top; m u(); initial #1 $finish; endmodule
`timescale 1ns/1ps
module m;
  if (1) begin : g
    task automatic t(); #3; $display("%0d %f", $time, $realtime); endtask
    initial begin t(); g.t(); end
  end
endmodule
"#;
    assert_eq!(lines(src), ["3 3.000000", "6 6.000000"]);
}

#[test]
fn a_task_in_a_top_level_block_runs_on_the_top_timescale() {
    let src = r#"
`timescale 1ns/1ps
module top;
  if (1) begin : g
    task automatic t(); #3; $display("%0d %f", $time, $realtime); endtask
    initial begin t(); g.t(); #1 $finish; end
  end
endmodule
"#;
    assert_eq!(lines(src), ["3 3.000000", "6 6.000000"]);
}

#[test]
fn generate_for_and_nested_block_tasks_run_on_the_instance_timescale() {
    let src = r#"
`timescale 1us/1ns
module top; m u(); initial #1 $finish; endmodule
`timescale 1ns/1ps
module m;
  for (genvar i = 0; i < 2; i++) begin : g
    task automatic t(); #3; $display("for%0d %0d", i, $time); endtask
    initial if (i == 1) begin t(); g[1].t(); end
    if (1) begin : h
      task automatic n(); #5; $display("nest%0d %0d", i, $time); endtask
      initial if (i == 0) begin n(); g[0].h.n(); h.n(); end
    end
  end
endmodule
"#;
    assert_eq!(
        lines(src),
        ["for1 3", "nest0 5", "for1 6", "nest0 10", "nest0 15"]
    );
}

#[test]
fn the_nearest_instance_wins_when_instances_and_blocks_alternate() {
    let src = r#"
`timescale 1us/1ns
module top; for (genvar i = 0; i < 2; i++) begin : a m u(); end initial #1 $finish; endmodule
`timescale 1ns/1ps
module m;
  for (genvar j = 0; j < 2; j++) begin : b
    if (1) begin : h
      task automatic t(); #3; $display("%m %0d", $time); endtask
      initial if (j == 1) begin t(); b[1].h.t(); end
    end
  end
endmodule
"#;
    let mut got = lines(src);
    got.sort();
    assert_eq!(
        got,
        [
            "top.a[0].u.b[1].h.t 3",
            "top.a[0].u.b[1].h.t 6",
            "top.a[1].u.b[1].h.t 3",
            "top.a[1].u.b[1].h.t 6",
        ]
    );
}

#[test]
fn a_block_task_in_an_instance_array_element_runs_on_the_instance_timescale() {
    let src = r#"
`timescale 1us/1ns
module top; m u[1:0](); initial #1 $finish; endmodule
`timescale 1ns/1ps
module m;
  if (1) begin : g
    task automatic t(); #3; $display("%m %0d", $time); endtask
    initial begin t(); g.t(); end
  end
endmodule
"#;
    let mut got = lines(src);
    got.sort();
    assert_eq!(
        got,
        [
            "top.u[0].g.t 3",
            "top.u[0].g.t 6",
            "top.u[1].g.t 3",
            "top.u[1].g.t 6",
        ]
    );
}

#[test]
fn a_block_task_called_from_another_module_runs_on_its_own_instance_timescale() {
    let src = r#"
`timescale 1us/1ns
module top;
  m u();
  initial begin u.g.t(); $display("top %0d", $time); $finish; end
endmodule
`timescale 1ns/1ps
module m;
  if (1) begin : g
    task automatic t(); #3000; $display("t %0d %f", $time, $realtime); endtask
  end
endmodule
"#;
    assert_eq!(lines(src), ["t 3000 3000.000000", "top 3"]);
}

#[test]
fn a_process_and_a_function_in_a_block_read_time_on_the_instance_timescale() {
    let src = r#"
`timescale 1us/1ns
module top; m u(); initial #1 $finish; endmodule
`timescale 1ns/1ps
module m;
  if (1) begin : g
    function automatic time now(); return $time; endfunction
    initial begin #3; $display("proc %0d %f", $time, $realtime); end
    initial begin #4; $display("fn %0d", now()); end
  end
endmodule
"#;
    assert_eq!(lines(src), ["proc 3 3.000000", "fn 4"]);
}

#[test]
fn an_instance_named_like_the_top_module_keeps_its_children_on_their_timescale() {
    let src = r#"
`timescale 1us/1ns
module top; m top(); initial #1 $finish; endmodule
module m; s x(); endmodule
`timescale 1ns/1ps
module s;
  initial begin #3; $display("s %0d %f", $time, $realtime); end
  if (1) begin : g
    initial begin #4; $display("g %0d %f", $time, $realtime); end
  end
endmodule
"#;
    assert_eq!(lines(src), ["s 3 3.000000", "g 4 4.000000"]);
}

#[test]
fn block_tasks_of_two_top_modules_each_run_on_their_own_timescale() {
    let src = r#"
`timescale 1us/1ns
module t1;
  if (1) begin : g
    task automatic t(); #1; $display("t1 %0d %f", $time, $realtime); endtask
    initial t();
  end
endmodule
`timescale 1ns/1ps
module t2;
  if (1) begin : g
    task automatic t(); #3; $display("t2 %0d %f", $time, $realtime); endtask
    initial t();
  end
endmodule
"#;
    assert_eq!(lines(src), ["t2 3 3.000000", "t1 1 1.000000"]);
}

#[test]
fn a_block_task_in_an_interface_instance_uses_the_interface_timescale_and_clocking() {
    let src = r#"
`timescale 1us/1ns
module top; ifc i(); initial #1 $finish; endmodule
`timescale 1ns/1ps
interface ifc;
  logic c = 0; always #1 c = ~c;
  int d = 5;
  clocking cb @(posedge c); input d; endclocking
  if (1) begin : g
    task automatic t(); @(cb); $display("%0d %0d", $time, cb.d); endtask
    initial begin t(); g.t(); end
  end
endinterface
"#;
    assert_eq!(lines(src), ["1 5", "3 5"]);
}

#[test]
fn a_block_task_in_an_instance_finds_the_modules_clocking_block_bare_and_by_path() {
    let src = r#"
module m;
  logic c = 0; always #1 c = ~c;
  int d = 5;
  clocking cb @(posedge c); input d; endclocking
  if (1) begin : g
    task automatic t(); @(cb); $display("%0t %0d", $time, cb.d); endtask
    initial begin t(); g.t(); end
  end
endmodule
module top; m u(); initial #9 $finish; endmodule
"#;
    assert_eq!(lines(src), ["1 5", "3 5"]);
}

#[test]
fn a_clocking_block_in_the_block_itself_is_found_in_an_instance_and_at_top() {
    let body = r#"
  logic c = 0; always #1 c = ~c;
  if (1) begin : g
    int e = 7;
    clocking inner @(posedge c); input e; endclocking
    task automatic t(); @(inner); $display("%0t %0d", $time, inner.e); endtask
    initial begin t(); g.t(); end
  end
"#;
    let in_instance =
        format!("module m;{body}endmodule\nmodule top; m u(); initial #9 $finish; endmodule\n");
    let at_top = format!("module top;{body}initial #9 $finish; endmodule\n");
    assert_eq!(lines(&in_instance), ["1 7", "3 7"]);
    assert_eq!(lines(&at_top), ["1 7", "3 7"]);
}

#[test]
fn a_clocking_block_in_the_block_hides_the_modules_block_of_the_same_name() {
    let src = r#"
module m;
  logic c = 0; always #1 c = ~c;
  int d = 5;
  clocking cb @(posedge c); input d; endclocking
  if (1) begin : g
    int e = 7;
    clocking cb @(negedge c); input e; endclocking
    task automatic t(); @(cb); $display("%0t %0d", $time, cb.e); endtask
    initial begin #3; t(); g.t(); end
  end
endmodule
module top; m u(); initial #9 $finish; endmodule
"#;
    assert_eq!(lines(src), ["4 7", "6 7"]);
}

#[test]
fn generate_for_and_nested_block_tasks_find_the_modules_clocking_block() {
    let src = r#"
module m;
  logic c = 0; always #1 c = ~c;
  int d = 5;
  clocking cb @(posedge c); input d; endclocking
  for (genvar i = 0; i < 2; i++) begin : h
    task automatic w(); @(cb); $display("for %0t %0d", $time, cb.d); endtask
    if (i == 1) begin : n
      task automatic x(); @(cb); $display("nest %0t %0d", $time, cb.d); endtask
      initial begin w(); x(); h[1].n.x(); end
    end
  end
endmodule
module top; m u(); initial #9 $finish; endmodule
"#;
    assert_eq!(lines(src), ["for 1 5", "nest 3 5", "nest 5 5"]);
}

#[test]
fn a_task_in_a_nested_block_waits_on_a_clocking_block_of_the_outer_block() {
    let src = r#"
module m;
  logic c = 0; always #1 c = ~c;
  if (1) begin : g
    clocking cb @(posedge c); endclocking
    if (1) begin : h
      task automatic t(); @(cb); $display("%0t", $time); endtask
      initial begin t(); h.t(); g.h.t(); end
    end
  end
endmodule
module top; m u(); initial #9 $finish; endmodule
"#;
    assert_eq!(lines(src), ["1", "3", "5"]);
}

#[test]
fn a_process_in_a_block_waits_on_and_reads_the_modules_clocking_block() {
    let src = r#"
module m;
  logic c = 0; always #1 c = ~c;
  int d = 5;
  clocking cb @(posedge c); input d; endclocking
  if (1) begin : g
    function automatic int rd(); return cb.d; endfunction
    initial begin @(cb); $display("proc %0t %0d", $time, cb.d); end
    always @(cb) if ($time == 3) $display("alw %0t %0d", $time, cb.d);
    initial begin #6 $display("fn %0d", rd()); end
  end
endmodule
module top; m u(); initial #9 $finish; endmodule
"#;
    assert_eq!(lines(src), ["proc 1 5", "alw 3 5", "fn 5"]);
}

#[test]
fn a_default_clocking_block_is_read_from_a_block_task_in_an_instance() {
    let src = r#"
module m;
  logic c = 0; always #1 c = ~c;
  int d = 5;
  default clocking cb @(posedge c); input d; endclocking
  if (1) begin : g
    task automatic t(); ##2; $display("%0t %0d", $time, cb.d); endtask
    initial begin t(); g.t(); end
  end
endmodule
module top; m u(); initial #9 $finish; endmodule
"#;
    assert_eq!(lines(src), ["3 5", "7 5"]);
}

#[test]
fn a_block_task_with_a_wait_called_from_a_compiled_loop_keeps_time_and_clocking() {
    let src = r#"
`timescale 1us/1ns
module top; m u(); initial #1 $finish; endmodule
`timescale 1ns/1ps
module m;
  logic c = 0; always #1 c = ~c;
  int d = 5;
  clocking cb @(posedge c); input d; endclocking
  int n = 0;
  if (1) begin : g
    task automatic t(); @(cb); n += cb.d; #0.4; n++; endtask
    always begin t(); if ($time == 21) $display("%0d %0d %f", $time, n, $realtime); end
  end
endmodule
"#;
    assert_eq!(lines(src), ["21 66 21.400000"]);
}
