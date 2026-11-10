//! §23.9: a generate block's subroutine is reached by a bare call from every
//! position in the block: a class body (§8), a final block (§9.2.3), a gate
//! terminal (§28), a property or sequence (§16.12), a let (§11.13). A member of
//! the class hides it.
//! Expected values come from the clauses and match Verilator 5.055.
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
const CALLS_F: &str = "class C; function int m(); return f(); endfunction endclass";

#[test]
fn a_class_in_a_block_calls_the_blocks_function() {
    // §8, §23.9.
    let src = format!(
        "module top; if (1) begin : g {BLOCK_F} {CALLS_F}
           initial begin C c = new; $display(\"T|%0d\", c.m()); end
         end endmodule"
    );
    assert_eq!(t_lines(&src), ["T|5"]);
}

#[test]
fn a_class_in_a_block_of_an_instance_calls_the_blocks_function() {
    // §8, §23.9.
    let src = format!(
        "module m; if (1) begin : g {BLOCK_F} {CALLS_F}
           initial begin C c = new; $display(\"T|%0d\", c.m()); end
         end endmodule
         module top; m u(); endmodule"
    );
    assert_eq!(t_lines(&src), ["T|5"]);
}

#[test]
fn a_class_in_a_block_of_an_instance_keeps_the_functions_static_local() {
    // §13.3.1: one function, one static local, from the process and the class.
    let src = format!(
        "module dut; for (genvar i = 0; i < 1; i++) begin : g
           function int f(); static int n = 0; n++; return n; endfunction
           {CALLS_F}
           C c = new;
           initial begin
             $display(\"T|%0d\", f());
             $display(\"T|%0d\", c.m());
             $display(\"T|%0d\", f());
           end
         end endmodule
         module top; dut d(); endmodule"
    );
    assert_eq!(t_lines(&src), ["T|1", "T|2", "T|3"]);
}

#[test]
fn a_final_block_in_a_block_calls_the_blocks_function() {
    // §9.2.3, §23.9.
    let src = format!(
        "module top; if (1) begin : g {BLOCK_F}
           final $display(\"T|f=%0d\", f());
         end initial #1 $finish; endmodule"
    );
    assert_eq!(t_lines(&src), ["T|f=5"]);
}

#[test]
fn two_final_blocks_call_their_own_blocks_function() {
    // §9.2.3, §23.9. §9.2.3 does not order final blocks.
    let src = "module top;
         if (1) begin : g_a function automatic int f(); return 1; endfunction
           final $display(\"T|a=%0d\", f()); end
         if (1) begin : g_b function automatic int f(); return 2; endfunction
           final $display(\"T|b=%0d\", f()); end
         initial #1 $finish; endmodule";
    let mut got = t_lines(src);
    got.sort();
    assert_eq!(got, ["T|a=1", "T|b=2"]);
}

#[test]
fn a_gate_terminal_in_a_block_calls_the_blocks_function() {
    // §28.4, §23.9.
    let src = "module top; wire w;
         if (1) begin : g
           function automatic bit f(); return 1; endfunction
           buf b(w, f());
         end
         initial #1 $display(\"T|%b\", w); endmodule";
    assert_eq!(t_lines(src), ["T|1"]);
}

#[test]
fn a_named_property_in_a_block_calls_the_blocks_function() {
    // §16.12, §23.9.
    let src = "module top; logic c = 0; always #1 c = ~c;
         if (1) begin : g_a function automatic int f(); return 1; endfunction
           property p; @(posedge c) f() == 1; endproperty
           int n = 0, bad = 0; assert property (p) n++; else bad++; end
         if (1) begin : g_b function automatic int f(); return 2; endfunction
           property p; @(posedge c) f() == 2; endproperty
           int n = 0, bad = 0; assert property (p) n++; else bad++; end
         initial begin #2;
           $display(\"T|%0d %0d %0d %0d\", g_a.n, g_a.bad, g_b.n, g_b.bad); $finish; end
         endmodule";
    assert_eq!(t_lines(src), ["T|1 0 1 0"]);
}

#[test]
fn a_named_sequence_in_a_block_calls_the_blocks_function_and_not_the_modules() {
    // §16.7, §23.9.
    let src = "module top; logic clk = 0; always #1 clk = ~clk;
         function automatic bit f(); return 1; endfunction
         if (1) begin : g
           function automatic bit f(); return 0; endfunction
           sequence s; @(posedge clk) f(); endsequence
           assert property (s) $display(\"T|pass\"); else $display(\"T|fail\");
         end
         initial #2 $finish; endmodule";
    assert_eq!(t_lines(src), ["T|fail"]);
}

#[test]
fn a_let_in_a_block_calls_the_blocks_function() {
    // §11.13, §23.9.
    let src = "module top;
         if (1) begin : g
           function automatic int f(); return 3; endfunction
           let L = f() + 1;
           initial $display(\"T|L=%0d\", L);
         end endmodule";
    assert_eq!(t_lines(src), ["T|L=4"]);
}

#[test]
fn a_class_constraint_in_a_block_calls_the_blocks_function() {
    // §18.5.11, §23.9.
    let src = format!(
        "module top; if (1) begin : g {BLOCK_F}
           class C; rand int x; constraint k {{ x == f(); }} endclass
           initial begin C c = new; void'(c.randomize()); $display(\"T|%0d\", c.x); end
         end endmodule"
    );
    assert_eq!(t_lines(&src), ["T|5"]);
}

#[test]
fn a_class_in_a_nested_block_calls_the_nearest_function() {
    // §23.9.
    let src = format!(
        "module m; if (1) begin : g {BLOCK_F}
           if (1) begin : h
             function automatic int f(); return 6; endfunction
             {CALLS_F}
             initial begin C c = new; $display(\"T|%0d\", c.m()); end
           end
         end endmodule
         module top; m u(); endmodule"
    );
    assert_eq!(t_lines(&src), ["T|6"]);
}

#[test]
fn a_class_method_in_an_instance_calls_the_blocks_function_in_a_hot_loop() {
    // §23.9, with a module-level rival.
    let src = "module m;
         function automatic int f(int x); return x + 100; endfunction
         if (1) begin : g
           function automatic int f(int x); return x + 1; endfunction
           class C; function int m(); int s = 0;
             for (int i = 0; i < 5000; i++) s = f(s); return s; endfunction endclass
           initial begin C c = new; $display(\"T|%0d\", c.m()); end
           initial begin int s = 0;
             for (int i = 0; i < 5000; i++) s = f(s); $display(\"T|%0d\", s); end
         end endmodule
         module top; m u(); endmodule";
    assert_eq!(t_lines(src), ["T|5000", "T|5000"]);
}

#[test]
fn a_class_method_of_the_same_name_hides_the_blocks_function() {
    // §8.3, §23.9.
    let src = format!(
        "module top; if (1) begin : g {BLOCK_F}
           class C; function int f(); return 9; endfunction
             function int m(); return f(); endfunction endclass
           initial begin C c = new; $display(\"T|%0d\", c.m()); end
         end endmodule"
    );
    assert_eq!(t_lines(&src), ["T|9"]);
}

#[test]
fn a_base_class_method_of_the_same_name_hides_the_blocks_function() {
    // §8.13, §23.9.
    let src = format!(
        "module top; class B; function int f(); return 9; endfunction endclass
         if (1) begin : g {BLOCK_F}
           class C extends B; function int m(); return f(); endfunction endclass
           initial begin C c = new; $display(\"T|%0d\", c.m()); end
         end endmodule"
    );
    assert_eq!(t_lines(&src), ["T|9"]);
}

#[test]
fn a_class_property_initializer_and_a_method_default_call_the_blocks_function() {
    // §8.8, §13.5.3, §23.9.
    let src = format!(
        "module top; if (1) begin : g {BLOCK_F}
           class C; int v = f();
             task t(int a = f()); $display(\"T|%0d %0d\", v, a); endtask endclass
           initial begin C c = new; c.t(); end
         end endmodule"
    );
    assert_eq!(t_lines(&src), ["T|5 5"]);
}

#[test]
fn a_class_method_in_a_block_enables_the_blocks_task() {
    // §13.3, §23.9.
    let src = "module top; int a;
         if (1) begin : g
           task automatic t(); a = 7; endtask
           class C; task m(); t(); endtask endclass
           initial begin C c = new; c.m(); $display(\"T|%0d\", a); end
         end endmodule";
    assert_eq!(t_lines(src), ["T|7"]);
}

#[test]
fn a_class_and_a_final_block_in_an_unlabelled_block_call_its_function() {
    // §27.6: the block is `genblk1`.
    let src = format!(
        "module top; if (1) begin {BLOCK_F} {CALLS_F}
           initial begin C c = new; $display(\"T|%0d\", c.m()); end
           final $display(\"T|%0d\", f());
         end initial #1 $finish; endmodule"
    );
    assert_eq!(t_lines(&src), ["T|5", "T|5"]);
}

#[test]
fn a_class_nested_in_a_class_in_a_block_calls_the_blocks_function() {
    // §8.23, §23.9.
    let src = format!(
        "module top; if (1) begin : g {BLOCK_F}
           class O; {CALLS_F} endclass
           initial begin O::C c = new; $display(\"T|%0d\", c.m()); end
         end endmodule"
    );
    assert_eq!(t_lines(&src), ["T|5"]);
}

#[test]
fn final_blocks_in_a_generate_for_call_their_iterations_function() {
    // §27.4, §23.9. §9.2.3 does not order final blocks.
    let src = "module top; for (genvar i = 0; i < 2; i++) begin : g
           function automatic int f(); return 10 + i; endfunction
           final $display(\"T|%0d %0d\", i, f());
         end initial #1 $finish; endmodule";
    let mut got = t_lines(src);
    got.sort();
    assert_eq!(got, ["T|0 10", "T|1 11"]);
}

#[test]
fn a_final_block_in_two_instances_calls_each_instances_function() {
    // §23.9, §27.2. §9.2.3 does not order final blocks.
    let src = "module m #(parameter int P = 1); if (1) begin : g
           function automatic int f(); return P; endfunction
           final $display(\"T|%0d\", f());
         end endmodule
         module top; m #(1) a(); m #(2) b(); initial #1 $finish; endmodule";
    let mut got = t_lines(src);
    got.sort();
    assert_eq!(got, ["T|1", "T|2"]);
}
