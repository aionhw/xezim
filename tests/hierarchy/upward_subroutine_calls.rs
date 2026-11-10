//! §23.8.1: a bare call that names no subroutine of its own definition, its
//! compilation unit or its imports is looked up from the scope of its
//! instantiation, then from each enclosing instance's.
//! Verilator rejects upward calls; the expected values follow the clause.
use xezim::simulate;

/// The `T|` lines, sorted: the order of initial blocks of two instances is
/// not defined (§9.2.1).
fn t_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 100).unwrap_or_else(|e| panic!("rejected: {e}"));
    let mut lines: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect();
    lines.sort();
    lines
}

const CHILD: &str =
    "module c; int a; initial a = f(); initial #1 $display(\"T|%m %0d\", a); endmodule";

#[test]
fn a_child_declared_in_a_block_calls_the_blocks_function() {
    let src = format!(
        "{CHILD}
         module top; if (1) begin : g function automatic int f(); return 1; endfunction c x(); end
         endmodule"
    );
    assert_eq!(t_lines(&src), ["T|top.g.x 1"]);
}

#[test]
fn a_child_calls_the_blocks_function_before_the_modules() {
    let src = format!(
        "{CHILD}
         module top; function automatic int f(); return 1; endfunction
           if (1) begin : g function automatic int f(); return 2; endfunction c x(); end
           c y();
         endmodule"
    );
    assert_eq!(t_lines(&src), ["T|top.g.x 2", "T|top.y 1"]);
}

#[test]
fn a_child_calls_its_parent_instances_function_before_the_tops() {
    let src = format!(
        "{CHILD}
         module m #(parameter P = 0); function automatic int f(); return P; endfunction
           c x(); endmodule
         module top; function automatic int f(); return 100; endfunction
           m #(5) v(); m #(3) u(); endmodule"
    );
    assert_eq!(t_lines(&src), ["T|top.u.x 3", "T|top.v.x 5"]);
}

#[test]
fn a_child_in_a_generate_loop_of_an_instance_calls_its_iterations_function() {
    let src = format!(
        "{CHILD}
         module m; for (genvar i = 0; i < 2; i++) begin : g
           function automatic int f(); return 10 + i; endfunction c x(); end endmodule
         module top; m u(); endmodule"
    );
    assert_eq!(t_lines(&src), ["T|top.u.g[0].x 10", "T|top.u.g[1].x 11"]);
}

#[test]
fn a_child_in_a_nested_or_unlabelled_block_calls_the_nearest_function() {
    let src = "module c; int a; initial a = f(); initial #1 $display(\"T|%0d\", a); endmodule
         module top; function automatic int f(); return 100; endfunction
           if (1) begin : g function automatic int f(); return 4; endfunction
             if (1) begin : h c x(); end end
           if (1) begin function automatic int f(); return 8; endfunction c y(); end
         endmodule";
    assert_eq!(t_lines(src), ["T|4", "T|8"]);
}

#[test]
fn a_grandchild_and_a_bound_instance_call_an_enclosing_instances_function() {
    // §23.11: a bound instance sits in its target's scope.
    let src = format!(
        "{CHILD}
         module mid; c z(); endmodule
         module m #(parameter P = 0); function automatic int f(); return P; endfunction endmodule
         module top; if (1) begin : g function automatic int f(); return 6; endfunction mid x(); end
           m #(5) v(); m #(3) u(); bind m c k(); endmodule"
    );
    assert_eq!(
        t_lines(&src),
        ["T|top.g.x.z 6", "T|top.u.k 3", "T|top.v.k 5"]
    );
}

#[test]
fn a_childs_task_enable_function_body_and_continuous_assign_reach_the_block() {
    let src = "module c; wire [7:0] w; assign w = f();
           function automatic int h(); return f() + 1; endfunction
           initial begin t; t(); #1 $display(\"T|%0d %0d\", w, h()); end endmodule
         module top; int n = 0;
           if (1) begin : g function automatic int f(); return 9; endfunction
             task automatic t(); n++; endtask c x(); end
           initial #2 $display(\"T|n=%0d\", n); endmodule";
    assert_eq!(t_lines(src), ["T|9 10", "T|n=2"]);
}

#[test]
fn the_childs_unit_and_imports_come_before_the_enclosing_block() {
    // §23.8.1: the compilation unit is searched before the hierarchy, and
    // the child's own import is a name of its scope (§26.3).
    let src = "package p; function automatic int k(); return 7; endfunction endpackage
         function automatic int f(); return 42; endfunction
         module c; import p::*; int a, b; initial begin a = f(); b = k(); end
           initial #1 $display(\"T|%0d %0d\", a, b); endmodule
         module top; if (1) begin : g function automatic int f(); return 1; endfunction
           function automatic int k(); return 2; endfunction c x(); end endmodule";
    assert_eq!(t_lines(src), ["T|42 7"]);
}

#[test]
fn an_import_of_the_parent_is_not_visible_to_the_upward_call() {
    // §26.3: an imported name is not visible by hierarchical reference into
    // the importing scope, so the search goes on to the top's function.
    let src = format!(
        "{CHILD}
         package p; function automatic int f(); return 7; endfunction endpackage
         module m; import p::*; c x(); endmodule
         module top; function automatic int f(); return 100; endfunction m u(); endmodule"
    );
    assert_eq!(t_lines(&src), ["T|top.u.x 100"]);
}

#[test]
fn rooted_and_dotted_upward_calls_are_unchanged() {
    let src = "module c; int a, b; initial begin a = $root.top.g.f(); b = g.f(); end
           initial #1 $display(\"T|%0d %0d\", a, b); endmodule
         module top; if (1) begin : g function automatic int f(); return 1; endfunction end
           c x(); endmodule";
    assert_eq!(t_lines(src), ["T|1 1"]);
}
