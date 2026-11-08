//! §27.4: a genvar is a localparam of each iteration of its loop. Every body
//! in the loop reads it as its iteration's value, unless a closer declaration
//! hides it. Outputs as Verilator 5.055 prints them.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 1000).unwrap_or_else(|e| panic!("{e}\n{src}"));
    sim.output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|m| !m.is_empty() && !m.contains("$finish") && !m.contains("[xezim]"))
        .collect()
}

#[test]
fn a_sequence_in_a_loop_reads_its_iterations_genvar() {
    // §16.8, §27.4: `s` of `g[0]` reads `i` as 0 and of `g[1]` as 1, so both hold.
    let src = r#"module top;
  logic clk = 0;
  always #1 clk = ~clk;
  int fails = 0;
  for (genvar i = 0; i < 2; i++) begin : g
    logic v;
    initial v = (i == 0);
    sequence s; v == (i == 0); endsequence
    assert property (@(posedge clk) s) else fails = fails + 1;
  end
  initial begin #6 $display("fails=%0d", fails); $finish; end
endmodule
"#;
    assert_eq!(lines(src), ["fails=0"]);
}

#[test]
fn a_property_in_nested_loops_reads_both_genvars() {
    // §16.12, §27.4: each `p` checks the bit of its own `(i, j)`.
    let src = r#"module top;
  logic clk = 0;
  always #1 clk = ~clk;
  int fails = 0;
  logic [3:0] v = 4'b1001;
  for (genvar i = 0; i < 2; i++) begin : g
    for (genvar j = 0; j < 2; j++) begin : h
      property p; @(posedge clk) v[2*i+j] == ((2*i+j) % 3 == 0); endproperty
      assert property (p) else fails = fails + 1;
    end
  end
  initial begin #6 $display("fails=%0d", fails); $finish; end
endmodule
"#;
    assert_eq!(lines(src), ["fails=0"]);
}

#[test]
fn a_formal_default_in_a_loop_reads_its_iterations_genvar() {
    // §13.5.3, §27.4.
    let src = r#"module top;
  for (genvar i = 0; i < 2; i++) begin : g
    function automatic int f(int a = 20 + i); return a; endfunction
    initial #(i + 1) $display("g%0d f=%0d", i, f());
  end
  initial #3 $finish;
endmodule
"#;
    assert_eq!(lines(src), ["g0 f=20", "g1 f=21"]);
}

#[test]
fn a_gate_in_a_loop_connects_its_iterations_bits() {
    // §28.3, §27.4: `u` of `g[i]` drives `y[i]`.
    let src = r#"module top;
  logic [1:0] a = 2'b01, b = 2'b11;
  wire [1:0] y;
  for (genvar i = 0; i < 2; i++) begin : g
    and u (y[i], a[i], b[i]);
  end
  initial begin #1 $display("y=%b", y); $finish; end
endmodule
"#;
    assert_eq!(lines(src), ["y=01"]);
}

#[test]
fn a_covergroup_in_a_loop_bins_its_iterations_value() {
    // §19.5, §27.4: each `cg` bins `5 + i` of its own iteration.
    let src = r#"module m;
  logic c = 0;
  always #1 c = ~c;
  for (genvar i = 0; i < 2; i++) begin : g
    function automatic int f(); return 5 + i; endfunction
    covergroup cg @(posedge c); coverpoint f() { bins b = {5 + i}; } endgroup
    cg k = new;
  end
endmodule
module top;
  m a();
  initial begin
    #2 $display("%0.0f %0.0f", a.g[0].k.get_coverage(), a.g[1].k.get_coverage());
    $finish;
  end
endmodule
"#;
    assert_eq!(lines(src), ["100 100"]);
}

#[test]
fn a_class_member_and_a_method_formal_hide_the_genvar() {
    // §8.3, §13.5, §27.4: `i` in the class is the member, in `h` the formal.
    let src = r#"module top;
  for (genvar i = 0; i < 2; i++) begin : g
    class C;
      int i = 5;
      function int f(); return i; endfunction
      function int h(int i); return i * 3; endfunction
    endclass
    initial begin C c = new; #(i + 1) $display("g%0d f=%0d h=%0d", i, c.f(), c.h(7)); end
  end
  initial #3 $finish;
endmodule
"#;
    assert_eq!(lines(src), ["g0 f=5 h=21", "g1 f=5 h=21"]);
}

#[test]
fn a_property_formal_hides_the_genvar() {
    // §16.8, §27.4: `p(0)` checks `v[0]` in both iterations.
    let src = r#"module top;
  logic clk = 0;
  always #1 clk = ~clk;
  int fails = 0;
  for (genvar i = 0; i < 2; i++) begin : g
    logic [1:0] v = 2'b01;
    property p(int i); @(posedge clk) v[i]; endproperty
    assert property (p(0)) else fails = fails + 1;
  end
  initial begin #6 $display("fails=%0d", fails); $finish; end
endmodule
"#;
    assert_eq!(lines(src), ["fails=0"]);
}

#[test]
fn a_module_function_formal_and_local_hide_the_genvar() {
    // §13.5, §6.21, §27.4: `i` in `f` is the formal, in `h` the local.
    let src = r#"module top;
  for (genvar i = 0; i < 2; i++) begin : g
    function automatic int f(int i); return i * 3; endfunction
    function automatic int h();
      int i = 9;
      return i + 1;
    endfunction
    initial #(i + 1) $display("g%0d f=%0d h=%0d", i, f(7), h());
  end
  initial #3 $finish;
endmodule
"#;
    assert_eq!(lines(src), ["g0 f=21 h=10", "g1 f=21 h=10"]);
}
