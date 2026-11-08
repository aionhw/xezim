//! §23.9, §6.21: a reference in a generate block names the block's variable,
//! parameter or net unless a nearer declaration hides it, and that holds at
//! every position of the block: final blocks (§9.2.3), gates (§28), classes
//! (§8), clocking blocks (§14.3), properties and sequences (§16) and the
//! dimensions of a process local. A bare call from a child searches upward
//! for a subroutine, past a variable of that name (§23.8.1). Verilator 5.055
//! gives the expected value on each program except the upward call, which it
//! rejects.
use xezim::simulate;

/// All output lines.
fn output(src: &str) -> Vec<String> {
    let sim = simulate(src, 1000).unwrap_or_else(|e| panic!("rejected: {e}"));
    sim.output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .collect()
}

/// The `T|` lines, sorted.
fn t_lines(src: &str) -> Vec<String> {
    let mut lines: Vec<String> = output(src)
        .into_iter()
        .filter(|l| l.starts_with("T|"))
        .collect();
    lines.sort();
    lines
}

#[test]
fn a_local_in_a_nested_block_of_a_block_task_hides_the_blocks_variable() {
    let src = r#"
module top;
  if (1) begin : g
    int v = 5;
    task t; begin int v = 7; $display("T|t=%0d", v); end endtask
    initial begin #1 t(); $display("T|g=%0d", v); end
  end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|g=5", "T|t=7"]);
}

const FINALS: &str = r#"
  if (1) begin : a
    int n = 1;
    final $display("T|a=%0d", n);
  end
  if (1) begin : b
    int n = 2;
    final $display("T|b=%0d", n);
  end
"#;

#[test]
fn two_final_blocks_read_their_own_blocks_variable() {
    let src = format!("module top; {FINALS} initial #1 $finish; endmodule");
    assert_eq!(t_lines(&src), ["T|a=1", "T|b=2"]);
}

#[test]
fn two_final_blocks_in_an_instance_read_their_own_blocks_variable() {
    let src =
        format!("module m; {FINALS} endmodule module top; m u(); initial #1 $finish; endmodule");
    assert_eq!(t_lines(&src), ["T|a=1", "T|b=2"]);
}

#[test]
fn final_blocks_of_two_loop_iterations_read_their_own_variable() {
    // §27.4: each iteration is a block of its own.
    let src = r#"
module top;
  for (genvar i = 0; i < 2; i++) begin : g
    int n = i + 1;
    final $display("T|g%0d=%0d", i, n);
  end
  initial #1 $finish;
endmodule
"#;
    assert_eq!(t_lines(src), ["T|g0=1", "T|g1=2"]);
}

#[test]
fn a_gate_in_a_block_connects_the_blocks_nets() {
    let src = r#"
module top;
  if (1) begin : a
    logic x = 1, y;
    not n1(y, x);
    initial #1 $display("T|a=%0d", y);
  end
  if (1) begin : b
    logic x = 0, y;
    not n1(y, x);
    initial #1 $display("T|b=%0d", y);
  end
endmodule
"#;
    let out = output(src);
    assert!(!out.iter().any(|l| l.contains("implicit")), "{out:?}");
    assert_eq!(t_lines(src), ["T|a=0", "T|b=1"]);
}

#[test]
fn a_class_in_a_block_of_an_instance_reads_its_blocks_variable() {
    let src = r#"
module m;
  if (1) begin : a
    int k = 1;
    class C; function int f(); return k; endfunction endclass
    initial begin C c = new; #1 $display("T|a=%0d", c.f()); end
  end
  if (1) begin : b
    int k = 2;
    class C; function int f(); return k; endfunction endclass
    initial begin C c = new; #1 $display("T|b=%0d", c.f()); end
  end
endmodule
module top; m u(); endmodule
"#;
    assert_eq!(t_lines(src), ["T|a=1", "T|b=2"]);
}

#[test]
fn a_class_in_a_block_of_an_instance_reads_its_blocks_parameter() {
    let src = r#"
module m;
  if (1) begin : a
    localparam int K = 1;
    class C; function int f(); return K; endfunction endclass
    initial begin C c = new; #1 $display("T|a=%0d", c.f()); end
  end
  if (1) begin : b
    localparam int K = 2;
    class C; function int f(); return K; endfunction endclass
    initial begin C c = new; #1 $display("T|b=%0d", c.f()); end
  end
endmodule
module top; m u(); endmodule
"#;
    assert_eq!(t_lines(src), ["T|a=1", "T|b=2"]);
}

#[test]
fn a_class_in_a_block_of_an_instance_reads_its_own_block_beside_a_top_block_of_that_name() {
    // §23.9: `a.k` of the instance, not of the top.
    let src = r#"
module m;
  if (1) begin : a
    int k = 1;
    class C; function int f(); return k; endfunction endclass
    initial begin C c = new; #1 $display("T|m=%0d", c.f()); end
  end
endmodule
module top;
  if (1) begin : a
    int k = 1000;
  end
  m u();
endmodule
"#;
    assert_eq!(t_lines(src), ["T|m=1"]);
}

#[test]
fn a_class_member_named_like_a_block_hides_the_blocks_data_in_an_instance() {
    // §23.9, §8.3: `cfg.depth` starts at the member `cfg`.
    let src = r#"
module m;
  if (1) begin : cfg
    int depth = 4;
  end
  class Cfg; int depth = 16; endclass
  class Env;
    Cfg cfg;
    function new(); cfg = new; endfunction
    function int depth(); return cfg.depth; endfunction
  endclass
  initial begin Env e = new; #1 $display("T|depth=%0d", e.depth()); end
endmodule
module top; m u(); endmodule
"#;
    assert_eq!(t_lines(src), ["T|depth=16"]);
}

#[test]
fn a_method_local_and_a_formal_named_like_a_block_hide_the_blocks_data_in_an_instance() {
    let src = r#"
module m;
  if (1) begin : a
    int k = 1;
  end
  class P; int k = 42; endclass
  class D;
    function int f(); P a = new; return a.k; endfunction
    function int g(P a); return a.k; endfunction
  endclass
  initial begin D d = new; P p = new; #1 $display("T|%0d %0d", d.f(), d.g(p)); end
endmodule
module top; m u(); endmodule
"#;
    assert_eq!(t_lines(src), ["T|42 42"]);
}

#[test]
fn a_class_member_in_the_block_named_like_the_block_hides_its_data_in_an_instance() {
    let src = r#"
module m;
  if (1) begin : a
    int k = 1;
    class P; int k = 42; endclass
    class D; P a; function new(); a = new; endfunction function int f(); return a.k; endfunction endclass
    initial begin D d = new; #1 $display("T|d=%0d", d.f()); end
  end
endmodule
module top; m u(); endmodule
"#;
    assert_eq!(t_lines(src), ["T|d=42"]);
}

#[test]
fn a_class_member_local_and_formal_hide_the_blocks_variable() {
    // §8.3, §13.3, §6.21.
    let src = r#"
module top;
  if (1) begin : a
    int k = 1;
    class C;
      int k = 5;
      function int f(); return k; endfunction
      function int g(); int k = 6; return k; endfunction
      function int h(int k); return k; endfunction
    endclass
    initial begin C c = new; #1 $display("T|a=%0d %0d %0d", c.f(), c.g(), c.h(7)); end
  end
  if (1) begin : b
    int k = 2;
    class C; function int f(); return k; endfunction endclass
    initial begin C c = new; #1 $display("T|b=%0d", c.f()); end
  end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|a=5 6 7", "T|b=2"]);
}

#[test]
fn a_block_parameter_sizes_a_process_local() {
    let src = r#"
module top;
  if (1) begin : g
    localparam W = 4;
    initial begin logic [W-1:0] v = '1; #1 $display("T|%0d", v); end
  end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|15"]);
}

#[test]
fn a_block_parameter_sizes_a_process_local_in_each_instance() {
    let src = r#"
module m #(parameter P = 4);
  if (1) begin : g
    localparam W = P;
    initial begin logic [W-1:0] v = '1; #1 $display("T|%0d", v); end
  end
endmodule
module top; m #(4) u(); m #(8) w(); endmodule
"#;
    assert_eq!(t_lines(src), ["T|15", "T|255"]);
}

#[test]
fn a_clocking_block_in_a_block_clocks_on_the_blocks_signal() {
    // The module's `c` never toggles.
    let src = r#"
module top;
  logic c = 0;
  if (1) begin : a
    logic c = 0;
    always #1 c = ~c;
    logic [3:0] d = 4'd5;
    int n = 0;
    clocking cb @(posedge c); input d; endclocking
    initial begin @(cb); n = n + 1; @(cb); n = n + 1; end
  end
  initial begin #6 $display("T|n=%0d", a.n); $finish; end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|n=2"]);
}

#[test]
fn two_blocks_properties_of_one_name_check_their_own_blocks_variable() {
    // §16.12: each block declares its own `p`.
    let src = r#"
module top;
  logic clk = 0; always #1 clk = ~clk;
  int fa = 0, fb = 0;
  if (1) begin : a
    logic v = 1;
    property p; @(posedge clk) v == 1; endproperty
    assert property (p) else fa = fa + 1;
  end
  if (1) begin : b
    logic v = 0;
    property p; @(posedge clk) v == 0; endproperty
    assert property (p) else fb = fb + 1;
  end
  initial begin #6 $display("T|a=%0d b=%0d", fa, fb); $finish; end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|a=0 b=0"]);
}

#[test]
fn two_blocks_sequences_of_one_name_check_their_own_blocks_variable() {
    // §16.8.
    let src = r#"
module top;
  logic clk = 0; always #1 clk = ~clk;
  int fails = 0;
  if (1) begin : a
    logic v = 1;
    sequence s; v; endsequence
    assert property (@(posedge clk) s) else fails = fails + 1;
  end
  if (1) begin : b
    logic v = 0;
    sequence s; !v; endsequence
    assert property (@(posedge clk) s) else fails = fails + 1;
  end
  initial begin #6 $display("T|fails=%0d", fails); $finish; end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|fails=0"]);
}

#[test]
fn a_constraint_in_a_block_class_reads_the_blocks_variable() {
    // §18.5. The constraint keeps the bare name; the solver finds the block's.
    let src = r#"
module m;
  if (1) begin : a
    int lim = 3;
    class C; rand int x; constraint c { x < lim; x >= 0; } endclass
    initial begin C c = new; void'(c.randomize()); #1 $display("T|a=%0d", c.x < 3); end
  end
endmodule
module top; m u(); endmodule
"#;
    assert_eq!(t_lines(src), ["T|a=1"]);
}

#[test]
fn a_bare_call_from_a_child_passes_a_variable_of_the_same_name_in_the_parents_block() {
    // §23.8.1: the search is for a scope named `f`, and `g.f` is a variable.
    let src = r#"
module child; initial #1 $display("T|%0d", f()); endmodule
module mid #(parameter R = 3);
  function int f(); return R; endfunction
  if (1) begin : g
    int f = 9;
    child c();
  end
endmodule
module top;
  function int f(); return 100; endfunction
  mid #(3) m(); mid #(4) n();
endmodule
"#;
    assert_eq!(t_lines(src), ["T|3", "T|4"]);
}
