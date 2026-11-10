//! §23.9, §26.3, §6.21, §13.5, §8, §18.7, §3.12.1: a generate block's rename
//! follows the binding. A name that an inner scope hides keeps its spelling
//! and reaches the inner declaration. A name that binds to the block's
//! subroutine still reaches it, from every position.
use xezim::simulate;

fn t_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 100).unwrap_or_else(|e| panic!("rejected: {e}"));
    sim.output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect()
}

const BLOCK_F: &str = "function automatic int f(); return 5; endfunction";

#[test]
fn a_process_local_hides_a_block_function_of_the_same_name() {
    // §23.9, §6.21.
    let src = format!(
        "module top; if (1) begin : g {BLOCK_F}
           initial begin int f = 7; $display(\"T|%0d\", f); end
         end endmodule"
    );
    assert_eq!(t_lines(&src), ["T|7"]);
}

#[test]
fn a_local_in_a_nested_block_of_a_block_task_hides_a_block_function() {
    // §23.9, §6.21.
    let src = format!(
        "module top; if (1) begin : g {BLOCK_F}
           task automatic t(); begin int f = 7; $display(\"T|%0d\", f); end endtask
           initial t();
         end endmodule"
    );
    assert_eq!(t_lines(&src), ["T|7"]);
}

#[test]
fn an_inner_blocks_enum_label_hides_a_block_function() {
    // §6.19, §23.9.
    let src = format!(
        "module top; if (1) begin : g {BLOCK_F}
           if (1) begin : h
             typedef enum {{f = 9, x = 8}} e;
             initial #1 $display(\"T|%0d\", f);
           end
         end endmodule"
    );
    assert_eq!(t_lines(&src), ["T|9"]);
}

#[test]
fn an_inner_clocking_block_hides_a_block_function() {
    // §14.3, §23.9: `@(f)` waits on the clocking block.
    let src = format!(
        "module top; logic clk = 0; always #1 clk = ~clk; logic a;
         if (1) begin : g {BLOCK_F}
           if (1) begin : h
             clocking f @(posedge clk); input a; endclocking
             initial begin @(f); $display(\"T|cb ok\"); end
           end
         end
         initial #5 $finish; endmodule"
    );
    assert_eq!(t_lines(&src), ["T|cb ok"]);
}

#[test]
fn an_explicit_import_in_an_inner_block_hides_a_block_function() {
    // §26.3, and the enclosing block still sees its own function.
    let src = format!(
        "package p; function automatic int f(); return 100; endfunction endpackage
         module top; if (1) begin : g {BLOCK_F}
           if (1) begin : h
             import p::f;
             initial $display(\"T|h %0d\", f());
           end
           initial #1 $display(\"T|g %0d\", f());
         end endmodule"
    );
    assert_eq!(t_lines(&src), ["T|h 100", "T|g 5"]);
}

#[test]
fn an_import_after_the_call_in_an_inner_block_hides_a_block_function() {
    // §26.3: a call searches the locally visible identifiers to the end of
    // the current scope.
    let src = format!(
        "package p; function automatic int f(); return 100; endfunction endpackage
         module top; if (1) begin : g {BLOCK_F}
           if (1) begin : h
             initial $display(\"T|%0d\", f());
             import p::f;
           end
         end endmodule"
    );
    assert_eq!(t_lines(&src), ["T|100"]);
}

#[test]
fn a_class_member_in_a_randomize_with_hides_a_block_function() {
    // §18.7.
    let src = format!(
        "module top; if (1) begin : g {BLOCK_F}
           class C; rand int x; function int f(); return 9; endfunction endclass
           C c = new;
           initial begin
             int ok;
             ok = c.randomize() with {{ x == f(); }};
             $display(\"T|%0d\", c.x);
           end
         end endmodule"
    );
    assert_eq!(t_lines(&src), ["T|9"]);
}

#[test]
fn a_unit_scoped_call_in_a_block_that_declares_the_name_runs_the_unit_function() {
    // §3.12.1, §23.9.
    let src = format!(
        "function automatic int f(); return 42; endfunction
         module top; if (1) begin : g {BLOCK_F}
           initial $display(\"T|%0d %0d\", $unit::f(), f());
         end endmodule"
    );
    assert_eq!(t_lines(&src), ["T|42 5"]);
}

// The rename still reaches what binds to the block.

#[test]
fn a_hierarchical_call_from_a_block_reaches_the_module_function_past_the_blocks_own() {
    // §23.8.1 Example 2.
    let src = "module top;
        function automatic int f(); return 1; endfunction
        if (1) begin : b
          function automatic int f(); return 2; endfunction
          initial $display(\"T|%0d %0d\", f(), top.f());
        end
      endmodule";
    assert_eq!(t_lines(src), ["T|2 1"]);
}

#[test]
fn a_formal_default_reads_the_blocks_data_and_not_another_formal() {
    // §13.5.3.
    let src = "module top; if (1) begin : g
        int a = 1;
        int x = 10;
        function automatic int f(int x = 20, int y = a + x); return x + y; endfunction
        initial $display(\"T|%0d\", f());
      end endmodule";
    assert_eq!(t_lines(src), ["T|31"]);
}

#[test]
fn a_block_function_recurses_through_its_own_name() {
    // §13.4.1.
    let src = "module top; if (1) begin : g
        function automatic int f(int n); return n <= 1 ? 1 : n * f(n - 1); endfunction
        initial $display(\"T|%0d\", f(4));
      end endmodule";
    assert_eq!(t_lines(src), ["T|24"]);
}

#[test]
fn a_block_functions_result_variable_keeps_its_member_path() {
    // §13.4.1.
    let src = "module top; if (1) begin : g
        typedef struct { int a; } s_t;
        function automatic s_t f(); f.a = 4; endfunction
        initial begin s_t r; r = f(); $display(\"T|%0d\", r.a); end
      end endmodule";
    assert_eq!(t_lines(src), ["T|4"]);
}

#[test]
fn a_path_through_the_outer_block_reaches_its_function_from_a_nested_block() {
    // §23.8.
    let src = format!(
        "module top; if (1) begin : g {BLOCK_F}
           if (1) begin : h initial $display(\"T|%0d\", g.f()); end
         end endmodule"
    );
    assert_eq!(t_lines(&src), ["T|5"]);
}

#[test]
fn a_generate_for_in_an_instance_calls_each_iterations_function() {
    // §27.4.
    let src = "module m;
        for (genvar i = 0; i < 2; i++) begin : l
          function automatic int f(); return i + 1; endfunction
          initial #(i) $display(\"T|%0d\", f());
        end
      endmodule
      module top; m u(); endmodule";
    assert_eq!(t_lines(src), ["T|1", "T|2"]);
}

#[test]
fn nested_block_data_named_like_an_outer_block_function_is_still_reached() {
    // §23.9: `f` in `h` binds to `h`'s data.
    let src = format!(
        "module top; if (1) begin : g {BLOCK_F}
           if (1) begin : h
             int f = 3;
             initial $display(\"T|%0d\", f);
           end
           initial #1 $display(\"T|%0d\", f());
         end endmodule"
    );
    assert_eq!(t_lines(&src), ["T|3", "T|5"]);
}

#[test]
fn a_hidden_array_keeps_the_blocks_data_in_its_index() {
    // §23.9, §6.21: `f` is the local, `k` the block's.
    let src = format!(
        "module top; if (1) begin : g {BLOCK_F}
           int k = 1;
           initial begin
             int f[2] = '{{3, 4}};
             $display(\"T|%0d\", f[k]);
           end
         end endmodule"
    );
    assert_eq!(t_lines(&src), ["T|4"]);
}

#[test]
fn a_hidden_struct_array_keeps_the_blocks_data_in_a_member_path_index() {
    // §23.9, §6.21.
    let src = format!(
        "module top; if (1) begin : g {BLOCK_F}
           typedef struct {{ int v; }} s_t;
           int k = 1;
           initial begin
             s_t f[2];
             f[1].v = 6;
             $display(\"T|%0d\", f[k].v);
           end
         end endmodule"
    );
    assert_eq!(t_lines(&src), ["T|6"]);
}

#[test]
fn a_unit_scoped_call_inside_a_block_subroutine_runs_the_unit_function() {
    // §3.12.1.
    let src = format!(
        "function automatic int f(); return 42; endfunction
         module top; if (1) begin : g {BLOCK_F}
           function automatic int h(); return $unit::f(); endfunction
           task automatic t(); $display(\"T|task %0d\", $unit::f()); endtask
           initial begin $display(\"T|h %0d\", h()); t(); end
         end endmodule"
    );
    assert_eq!(t_lines(&src), ["T|h 42", "T|task 42"]);
}

#[test]
fn a_unit_scoped_call_in_a_blocks_own_function_does_not_recurse() {
    // §3.12.1.
    let src = "function automatic int f(); return 42; endfunction
      module top; if (1) begin : g
        function automatic int f(); return 5 + $unit::f(); endfunction
        initial $display(\"T|%0d\", f());
      end endmodule";
    assert_eq!(t_lines(src), ["T|47"]);
}

#[test]
fn a_unit_scoped_call_in_a_block_of_an_instance_runs_the_unit_function() {
    // §3.12.1.
    let src = format!(
        "function automatic int f(); return 42; endfunction
         module m; if (1) begin : g {BLOCK_F}
           initial $display(\"T|%0d %0d\", $unit::f(), f());
         end endmodule
         module top; m u(); endmodule"
    );
    assert_eq!(t_lines(&src), ["T|42 5"]);
}

#[test]
fn a_unit_scoped_task_enable_in_a_block_runs_the_unit_task() {
    // §3.12.1.
    let src = "task automatic t(); $display(\"T|unit\"); endtask
      module top; if (1) begin : g
        task automatic t(); $display(\"T|block\"); endtask
        initial begin $unit::t(); t(); end
      end endmodule";
    assert_eq!(t_lines(src), ["T|unit", "T|block"]);
}

#[test]
fn nested_data_named_like_a_subroutine_of_an_unlabelled_block_is_still_reached() {
    // §23.9, §27.6.
    let src = format!(
        "module top; if (1) begin {BLOCK_F}
           if (1) begin : h int f = 3; initial $display(\"T|%0d\", f); end
           initial #1 $display(\"T|%0d\", f());
         end endmodule"
    );
    assert_eq!(t_lines(&src), ["T|3", "T|5"]);
}

#[test]
fn a_nested_parameter_named_like_a_subroutine_of_an_unlabelled_block_is_still_reached() {
    // §23.9, §27.6.
    let src = format!(
        "module top; if (1) begin {BLOCK_F}
           if (1) begin : h localparam int f = 3; initial $display(\"T|%0d\", f); end
           initial #1 $display(\"T|%0d\", f());
         end endmodule"
    );
    assert_eq!(t_lines(&src), ["T|3", "T|5"]);
}

#[test]
fn a_name_only_an_unrelated_class_declares_reaches_the_block_from_a_randomize_with() {
    // §18.7.
    let src = format!(
        "class Other; function int f(); return 1; endfunction endclass
         class R; rand int x; endclass
         module top; if (1) begin : g {BLOCK_F}
           initial begin
             R r = new;
             void'(r.randomize() with {{ x == f(); }});
             $display(\"T|x=%0d\", r.x);
           end
         end endmodule"
    );
    assert_eq!(t_lines(&src), ["T|x=5"]);
}

#[test]
fn std_randomize_with_calls_the_blocks_function() {
    // §18.12.
    let src = format!(
        "class Other; function int f(); return 1; endfunction endclass
         module top; if (1) begin : g {BLOCK_F}
           int x;
           initial begin
             void'(std::randomize(x) with {{ x == f(); }});
             $display(\"T|x=%0d\", x);
           end
         end endmodule"
    );
    assert_eq!(t_lines(&src), ["T|x=5"]);
}
