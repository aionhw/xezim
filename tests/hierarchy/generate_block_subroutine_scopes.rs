//! §23.9, §23.8.1 Example 2, §27.6, §9.6.2 (#897): a function or task
//! declared in a generate block belongs to the block's scope like its data.
//! A bare call from the block — a process, a continuous assign, another of
//! the block's subroutines, a nested block, a `disable` — binds to the
//! innermost enclosing block that declares the name, then to the module. Two
//! blocks declaring `look` used to share one flat entry, so the last block's
//! body answered every call; a block's `t` was a false duplicate of the
//! module's. Expected lines are the reference simulator's.
use xezim::simulate;

fn t_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 100).unwrap_or_else(|e| panic!("rejected: {e}"));
    sim.output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect()
}

/// The issue's testcase: a task that waits on the clock, in two blocks.
#[test]
fn two_labelled_blocks_each_run_their_own_task() {
    let src = r#"
module top;
    logic clk = 0;
    always #1 clk = ~clk;
    if (1) begin : g_a
        logic [3:0] sig;
        int seen;
        task automatic look();
            @(negedge clk);
            seen = sig;
        endtask
        initial begin
            sig = 4'h3;
            look();
        end
    end : g_a
    if (1) begin : g_b
        logic [3:0] sig;
        int seen;
        task automatic look();
            @(negedge clk);
            seen = sig;
        endtask
        initial begin
            sig = 4'h9;
            look();
        end
    end : g_b
    initial begin
        #10;
        $display("T|%s g_a.seen=%0d g_b.seen=%0d",
                 (g_a.seen == 3 && g_b.seen == 9) ? "PASS" : "FAIL", g_a.seen, g_b.seen);
        $finish;
    end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|PASS g_a.seen=3 g_b.seen=9"]);
}

#[test]
fn labelled_if_in_an_instance() {
    let src = r#"
module m;
  if (1) begin : g_a int seen; task automatic look(); seen = 3; endtask initial look(); end
  if (1) begin : g_b int seen; task automatic look(); seen = 9; endtask initial look(); end
endmodule
module top;
  m u();
  initial begin #1; $display("T|%0d %0d", u.g_a.seen, u.g_b.seen); $finish; end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|3 9"]);
}

#[test]
fn labelled_case_arms_with_functions() {
    let src = r#"
module top;
  case (1) 1: begin : g_a function automatic int f(); return 3; endfunction int v; initial v = f(); end endcase
  case (2) 2: begin : g_b function automatic int f(); return 9; endfunction int v; initial v = f(); end endcase
  initial begin #1; $display("T|%0d %0d", g_a.v, g_b.v); $finish; end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|3 9"]);
}

#[test]
fn nested_blocks_that_share_a_label() {
    let src = r#"
module top;
  if (1) begin : a if (1) begin : n int s; task automatic t(); s = 1; endtask initial t(); end end
  if (1) begin : b if (1) begin : n int s; task automatic t(); s = 2; endtask initial t(); end end
  initial begin #1; $display("T|%0d %0d", a.n.s, b.n.s); $finish; end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|1 2"]);
}

#[test]
fn unnamed_blocks_with_functions() {
    let src = r#"
module top;
  int a, b;
  if (1) begin function automatic int f(); return 1; endfunction initial a = f(); end
  if (1) begin function automatic int f(); return 2; endfunction initial b = f(); end
  initial begin #1; $display("T|%0d %0d", a, b); $finish; end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|1 2"]);
}

#[test]
fn function_call_in_a_continuous_assign() {
    let src = r#"
module top;
  if (1) begin : g_a function automatic int f(); return 1; endfunction wire [31:0] v; assign v = f(); end
  if (1) begin : g_b function automatic int f(); return 2; endfunction wire [31:0] v; assign v = f(); end
  initial begin #1; $display("T|%0d %0d", g_a.v, g_b.v); $finish; end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|1 2"]);
}

#[test]
fn hierarchical_call_whose_body_makes_a_bare_call() {
    let src = r#"
module top;
  if (1) begin : g_a
    function automatic int h(); return 1; endfunction
    function automatic int f(); return h(); endfunction
  end
  if (1) begin : g_b
    function automatic int h(); return 2; endfunction
    function automatic int f(); return h(); endfunction
  end
  initial begin #1; $display("T|%0d %0d", g_a.f(), g_b.f()); $finish; end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|1 2"]);
}

#[test]
fn module_level_call_does_not_see_a_same_named_block_task() {
    let src = r#"
module top;
  int a, b;
  task automatic t(); a = 1; endtask
  if (1) begin : g task automatic t(); b = 2; endtask initial t(); end
  initial t();
  initial begin #1; $display("T|a=%0d b=%0d", a, b); $finish; end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|a=1 b=2"]);
}

#[test]
fn disable_of_a_block_task_stops_only_that_blocks_task() {
    let src = r#"
module top;
  int a = 0, b = 0;
  if (1) begin : g_a
    task automatic t(); #5 a = 1; endtask
    initial begin fork t(); #1 disable t; join end
  end
  if (1) begin : g_b
    task automatic t(); #5 b = 1; endtask
    initial t();
  end
  initial begin #10; $display("T|a=%0d b=%0d", a, b); $finish; end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|a=0 b=1"]);
}

#[test]
fn generate_for_in_an_instance() {
    let src = r#"
module m;
  for (genvar i = 0; i < 2; i++) begin : g
    int seen;
    task automatic look(); seen = 10 + i; endtask
    initial look();
  end
endmodule
module top;
  m u();
  initial begin #1; $display("T|%0d %0d", u.g[0].seen, u.g[1].seen); $finish; end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|10 11"]);
}

#[test]
fn generate_for_at_the_top() {
    let src = r#"
module top;
  for (genvar i = 0; i < 2; i++) begin : g
    int seen;
    task automatic look(); seen = 10 + i; endtask
    initial look();
  end
  initial begin #1; $display("T|%0d %0d", g[0].seen, g[1].seen); $finish; end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|10 11"]);
}

#[test]
fn block_task_before_a_same_named_module_task() {
    let src = r#"
module top;
  int r;
  if (1) begin : g
    task automatic t(); r = 2; endtask
    if (1) begin : h initial t(); end
  end
  task automatic t(); r = 1; endtask
  initial begin #1; $display("T|r=%0d", r); $finish; end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|r=2"]);
}

// Neighbouring shapes, checked with the reference simulator.

#[test]
fn a_nested_loop_in_a_labelled_block_shadows_the_blocks_function() {
    let src = r#"
module top;
  if (1) begin : g
    function automatic int f(); return 100; endfunction
    int a;
    initial a = f();
    for (genvar i = 0; i < 2; i++) begin : l
      function automatic int f(); return 10 + i; endfunction
      int v;
      initial v = f();
      if (1) begin : h
        int w;
        initial w = f();
      end
    end
  end
  initial begin #1; $display("T|%0d %0d %0d %0d %0d", g.a, g.l[0].v, g.l[1].v, g.l[0].h.w, g.l[1].h.w); $finish; end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|100 10 11 10 11"]);
}

#[test]
fn a_block_function_called_by_hierarchical_name_from_another_module() {
    let src = r#"
module other;
  int r1, r2;
  initial begin #1; r1 = top.u.g.f(); r2 = top.g2.f(); end
endmodule
module m;
  if (1) begin : g function automatic int f(); return 7; endfunction end
endmodule
module top;
  m u();
  other o();
  if (1) begin : g2 function automatic int f(); return 8; endfunction end
  initial begin #2; $display("T|%0d %0d %0d", o.r1, o.r2, u.g.f()); $finish; end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|7 8 7"]);
}

#[test]
fn a_block_task_calls_a_module_task_and_a_block_function_recurses() {
    let src = r#"
module top;
  int a, b, c;
  task automatic mt(); a = 5; endtask
  if (1) begin : g
    task automatic t(); mt(); b = 6; endtask
    function automatic int fact(int n); return n <= 1 ? 1 : n * fact(n - 1); endfunction
    initial begin t(); c = fact(4); end
  end
  initial begin #1; $display("T|%0d %0d %0d", a, b, c); $finish; end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|5 6 24"]);
}

#[test]
fn a_class_method_of_the_same_name_is_untouched_and_a_formal_shadows_block_data() {
    let src = r#"
module top;
  class C; function int f(); return 42; endfunction endclass
  if (1) begin : g
    int x = 5;
    function automatic int f(int x); return x + 1; endfunction
    C obj;
    int r1, r2;
    initial begin obj = new; r1 = obj.f(); r2 = f(1); end
  end
  initial begin #1; $display("T|%0d %0d", g.r1, g.r2); $finish; end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|42 2"]);
}

#[test]
fn unnamed_blocks_nested_in_a_labelled_block_and_beside_a_module_task() {
    let src = r#"
module top;
  int r;
  if (1) begin : g
    function automatic int f(); return 1; endfunction
    int a, b;
    initial a = f();
    if (1) begin
      function automatic int f(); return 2; endfunction
      initial b = f();
    end
  end
  if (1) begin
    task automatic t(); r = 2; endtask
    initial t();
  end
  task automatic t(); r = 1; endtask
  initial begin #1; $display("T|%0d %0d %0d", g.a, g.b, r); $finish; end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|1 2 2"]);
}

#[test]
fn nested_loop_blocks_inside_an_instance_and_a_same_named_module_function() {
    let src = r#"
module m;
  for (genvar i = 0; i < 2; i++) begin : g
    function automatic int f(); return 20 + i; endfunction
    if (1) begin : h
      int w;
      initial w = f();
    end
  end
endmodule
module top;
  m u();
  function automatic int f(); return 99; endfunction
  int z;
  initial z = f();
  initial begin #10; $display("T|%0d %0d %0d", u.g[0].h.w, u.g[1].h.w, z); $finish; end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|20 21 99"]);
}

#[test]
fn continuous_assigns_in_an_instance_call_the_blocks_function_declared_before_or_after() {
    let src = r#"
module m;
  if (1) begin : g_a function automatic int f(); return 1; endfunction wire [31:0] v; assign v = f(); end
  if (1) begin : g_b wire [31:0] v; assign v = f(); function automatic int f(); return 2; endfunction end
endmodule
module top;
  m u();
  initial begin #1; $display("T|%0d %0d", u.g_a.v, u.g_b.v); $finish; end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|1 2"]);
}
