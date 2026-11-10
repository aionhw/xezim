//! §23.9: a bare name is searched in the enclosing scopes up to the module,
//! never into a generate block, a child instance or the parent. §23.8.1: only
//! a task or function name is searched upward. §27.6: an implicit `genblk<n>`
//! name is a name of the scope that holds the construct. The undeclared-name
//! checks read the binding, so these hold in an instance as at the top.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 1000).unwrap_or_else(|e| panic!("{e}\n{src}"));
    sim.output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|m| !m.is_empty() && !m.contains("$finish") && !m.contains("[xezim]"))
        .collect()
}

fn undeclared(src: &str, name: &str) {
    let err = simulate(src, 1000)
        .err()
        .unwrap_or_else(|| panic!("'{name}' is undeclared in\n{src}"));
    assert!(
        err.contains(&format!("Undeclared identifier '{name}'")),
        "{err}"
    );
}

#[test]
fn a_bare_read_of_an_unlabelled_blocks_variable_in_an_instance_is_undeclared() {
    undeclared(
        "module m; if (1) begin logic [7:0] x = 5; end initial $display(\"m x=%0d\", x); endmodule
module top; m u(); endmodule
",
        "x",
    );
}

#[test]
fn a_bare_read_of_a_labelled_blocks_variable_in_an_instance_is_undeclared() {
    undeclared(
        "module m;
  if (1) begin : g int x = 5; end
  initial #1 $display(\"m x=%0d\", x);
endmodule
module top; m u(); endmodule
",
        "x",
    );
}

#[test]
fn a_bare_write_of_a_blocks_variable_in_an_instance_is_undeclared() {
    undeclared(
        "module m;
  if (1) begin int x = 5; initial #2 $display(\"blk x=%0d\", x); end
  initial #1 x = 9;
endmodule
module top; m u(); endmodule
",
        "x",
    );
}

#[test]
fn a_net_initializer_reading_a_blocks_net_in_an_instance_is_undeclared() {
    // §6.10: a net initializer is not a position that declares an implicit net.
    // Verilator 5.055 creates one and prints y=1.
    undeclared(
        "module m;
  if (1) begin wire [7:0] w = 8'd6; end
  wire [7:0] y = w + 1;
  initial #1 $display(\"y=%0d\", y);
endmodule
module top; m u(); endmodule
",
        "w",
    );
}

#[test]
fn a_bare_read_of_a_child_instances_variable_is_undeclared() {
    undeclared(
        "module n; int x = 42; endmodule
module m;
  n s();
  initial #1 $display(\"m x=%0d\", x);
endmodule
module top; m u(); endmodule
",
        "x",
    );
}

#[test]
fn a_bare_read_of_the_parents_variable_from_an_instance_is_undeclared() {
    // §23.8.1: a variable name is not searched upward.
    undeclared(
        "module m; int r; initial begin #1 r = q; $display(\"r=%0d\", r); end endmodule
module top; int q = 77; m u(); endmodule
",
        "q",
    );
}

#[test]
fn a_bare_call_of_a_block_function_from_the_module_in_an_instance_is_undeclared() {
    undeclared(
        "module m; if (1) begin : a function automatic int f(); return 5; endfunction end
  initial $display(f()); endmodule
module top; m u(); endmodule
",
        "f",
    );
}

#[test]
fn an_upward_call_that_resolves_nowhere_is_undeclared() {
    // §23.8.1: the parent declares `f` in a block the child is not under.
    undeclared(
        "module child; int v; initial begin v = f(); #1 $display(\"%0d\", v); end endmodule
module top; if (1) begin : g function automatic int f(); return 5; endfunction end
  child c(); initial #5 $finish; endmodule
",
        "f",
    );
}

#[test]
fn an_upward_call_from_a_subroutine_that_resolves_nowhere_is_undeclared() {
    undeclared(
        "module child; function automatic int g(); return f(); endfunction
  initial #1 $display(\"%0d\", g()); endmodule
module top; if (1) begin : b function automatic int f(); return 5; endfunction end
  child c(); endmodule
",
        "f",
    );
}

#[test]
fn a_subroutine_reading_a_blocks_variable_bare_is_undeclared() {
    undeclared(
        "module m; if (1) begin int x = 5; end
  function automatic int g(); return x; endfunction
  initial #1 $display(\"%0d\", g()); endmodule
module top; m u(); endmodule
",
        "x",
    );
}

#[test]
fn an_implicit_name_that_names_no_block_is_undeclared() {
    // The first construct is labelled, so the unnamed block is genblk2.
    undeclared(
        "module top; if (1) begin : h int y = 1; end if (1) begin int x = 2; end
  initial $display(\"%0d\", genblk1.x); endmodule
",
        "genblk1",
    );
}

#[test]
fn an_implicit_name_that_names_no_block_in_an_instance_is_undeclared() {
    undeclared(
        "module m; if (1) begin : h int y = 1; end if (1) begin int x = 2; end
  initial #1 $display(\"%0d\", genblk7.x); endmodule
module top; m u(); endmodule
",
        "genblk7",
    );
}

#[test]
fn a_net_initializer_reading_an_undeclared_name_is_an_error() {
    // §6.10: no implicit net on the right-hand side. Verilator 5.055 creates
    // one and prints 1.
    undeclared(
        "module top; if (1) begin wire [7:0] w = 6; end wire [7:0] y = w + 1;
  initial #1 $display(y); endmodule
",
        "w",
    );
}

#[test]
fn a_continuous_assignment_reading_an_undeclared_name_is_an_error() {
    undeclared(
        "module top; wire [7:0] y; assign y = w; initial #1 $display(y); endmodule
",
        "w",
    );
}

#[test]
fn a_module_read_of_a_blocks_implicit_net_is_undeclared() {
    // §6.10: the implicit net belongs to the block that assigns it.
    // Verilator 5.055 accepts and prints 1.
    undeclared(
        "module top; wire a = 1; if (1) begin assign y = a; end
  initial #1 $display(y); endmodule
",
        "y",
    );
}

#[test]
fn an_upward_task_call_from_an_instance_runs() {
    // §23.8.1. Verilator 5.055 has no upward calls.
    let got = lines(
        "module m; initial #1 t(); endmodule
module top; task t(); $display(\"top.t\"); endtask m u(); endmodule
",
    );
    assert_eq!(got, ["top.t"]);
}

#[test]
fn an_upward_call_from_a_subroutine_of_an_instance_runs() {
    let got = lines(
        "module child; function automatic int g(); return f(); endfunction
  initial #1 $display(\"%0d\", g()); endmodule
module top; function automatic int f(); return 5; endfunction child c(); endmodule
",
    );
    assert_eq!(got, ["5"]);
}

#[test]
fn an_upward_path_from_an_instance_runs() {
    // §23.8: `inst_x` names an enclosing instance, `t` an enclosing task.
    let got = lines(
        "module upl; initial #1 $display(\"%0d %0d\", inst_x.tv, t.s); endmodule
module mtype; upl ul(); int tv = 42; task t(); static int s = 9; endtask endmodule
module top; mtype inst_x(); endmodule
",
    );
    assert_eq!(got, ["42 9"]);
}

#[test]
fn implicit_names_of_existing_blocks_resolve_in_an_instance() {
    // §27.6: genblk2 holds only a process and is still a block.
    let got = lines(
        "module m; if (1) begin int x = 2; function automatic int f(); return 3; endfunction end
  if (1) begin initial $display(\"p\"); end
  if (1) begin int y = 4; end
  initial #1 $display(\"%0d %0d %0d\", genblk1.x, genblk1.f(), genblk3.y); endmodule
module top; m u(); endmodule
",
    );
    assert_eq!(got, ["p", "2 3 4"]);
}

#[test]
fn legal_bare_names_in_an_instance_stay_accepted() {
    // §6.10 an implicit net of a port connection, §13.5.3 a formal default,
    // §16.10 a property local, §18.11 randomize arguments, §26.7 `process`,
    // A.2.2.3 `1step`.
    let got = lines(
        "module top;
  wire n;
  m u(.o(n));
  initial #10 $finish;
endmodule
module leaf(output o); assign o = 1'b1; endmodule
module m(output o);
  class C; rand int a; rand int b; endclass
  int got;
  leaf l(.o(w));
  function automatic int f(int a = 1, int b = a + 1); return a * 10 + b; endfunction
  logic clk = 0;
  always #1 clk = ~clk;
  property p_stable;
    int v;
    @(posedge clk) (1, v = got) |-> ##1 (v == v);
  endproperty
  assert property (p_stable);
  initial begin
    process p;
    C c;
    p = process::self();
    c = new;
    void'(c.randomize(a));
    #1step;
    got = f();
    $display(\"f=%0d w=%0d p=%0d\", got, w, p != null);
  end
  assign o = w;
endmodule
",
    );
    assert_eq!(got, ["f=12 w=1 p=1"]);
}

#[test]
fn a_randsequence_value_in_an_instance_is_accepted() {
    // §18.17.7: `r1` and `r2` are variables of the rule. The value they carry
    // is the run-time gap the chapter 18 tests pin.
    let src = "module m; int n;
  initial randsequence (vr)
    void vr : r1 r2 { n = r1 + r2; };
    int r1 : { return 5; };
    int r2 : { return 6; };
  endsequence
endmodule
module top; m u(); endmodule
";
    if let Err(e) = simulate(src, 1000) {
        panic!("{e}");
    }
}

#[test]
fn a_design_unit_name_in_an_instance_is_accepted() {
    // §23.9: a path may start with a module name.
    let got = lines(
        "module m; initial begin $dumpvars(0, top); $display(\"m\"); end endmodule
module top; m u(); endmodule
",
    );
    assert_eq!(got, ["m"]);
}

#[test]
fn a_nested_module_reads_the_outer_modules_variable_in_an_instance() {
    // §23.4. Verilator 5.055 does not support nested modules.
    let got = lines(
        "module m; int x = 7;
  module inner; initial #1 $display(\"%0d\", x); endmodule
  inner i();
endmodule
module top; m u(); endmodule
",
    );
    assert_eq!(got, ["7"]);
}

#[test]
fn the_names_of_a_concatenation_at_an_implicit_net_position_are_accepted() {
    // §6.10: a left-hand side and a port actual, here as concatenation
    // members.
    let got = lines(
        "module m(input logic [3:0] a, output logic [1:0] z); assign z = a[1:0]; endmodule
module top;
  logic [3:0] a = 4'd9, b = 4'd8;
  assign {cout, sum} = a + b;
  m u(.a(a), .z({z1, z0}));
  initial #1 $display(\"%b %b %b%b\", cout, sum, z1, z0);
endmodule
",
    );
    assert_eq!(got, ["0 1 01"]);
}

#[test]
fn a_blocks_implicit_net_read_in_its_own_block_is_accepted() {
    // §6.10: the net belongs to the block that connects it.
    let got = lines(
        "module m(input logic i, output logic o); assign o = i; endmodule
module top;
  if (1) begin : g
    m u(.i(1'b1), .o(w));
    initial #1 $display(\"w=%b\", w);
  end
endmodule
",
    );
    assert_eq!(got, ["w=1"]);
}

#[test]
fn bare_built_in_randomization_methods_in_a_class_are_accepted() {
    // §18.8, §18.9, §18.13: methods of every class.
    let got = lines(
        "module m;
  class C;
    rand int x; string st;
    function void go();
      rand_mode(0); constraint_mode(1); srandom(3);
      st = get_randstate(); set_randstate(st);
      $display(\"go\");
    endfunction
  endclass
  initial begin C c = new; c.go(); end
endmodule
module top; m u(); endmodule
",
    );
    assert_eq!(got, ["go"]);
}

#[test]
fn an_enum_range_with_a_parameter_bound_declares_no_names() {
    // A.2.4: the range of an enum_name_declaration is an integral_number.
    // xezim accepted a parameter there as an extension; Verilator 5.055
    // rejects it.
    undeclared(
        "module top; localparam int N = 3; typedef enum {A[N]} e_t;
  initial $display(\"%0d\", A2); endmodule
",
        "A2",
    );
}

#[test]
fn names_exported_by_export_star_star_are_visible_through_a_wildcard_import() {
    // §26.6: `export *::*` exports what the package imports, at the top and
    // in an instance.
    let got = lines(
        "package core; localparam int K = 3; function automatic int dbl(int x); return 2 * x; endfunction endpackage
package allx; import core::*; export *::*; localparam int KK = K + dbl(0); endpackage
module m; import allx::*; initial $display(\"m %0d %0d\", K, KK); endmodule
module top; import allx::*; m u(); initial #1 $display(\"top %0d %0d %0d\", K, dbl(1), KK); endmodule
",
    );
    assert_eq!(got, ["m 3 3", "top 3 2 3"]);
}

#[test]
fn export_star_star_re_exports_types_and_functions_of_each_package() {
    let got = lines(
        "package base_pkg; typedef enum {OK, ERR} status_t; typedef struct packed { logic [7:0] d; } tx_t;
  function automatic string st_name(status_t s); return s.name(); endfunction endpackage
package vip_pkg; import base_pkg::*; export *::*;
  class tx_item; tx_t tx; status_t st = OK; function string show(); return st_name(st); endfunction endclass endpackage
package p1; localparam int A = 1; endpackage
package p2; localparam int B = 2; endpackage
package both; import p1::*; import p2::*; export *::*; localparam int C = A + B; endpackage
module m; import vip_pkg::*; tx_item it = new; status_t s = ERR;
  initial $display(\"m %s %s %0d\", it.show(), st_name(s), $bits(tx_t)); endmodule
module top; import both::*; m u(); initial #1 $display(\"top %0d %0d %0d\", A, B, C); endmodule
",
    );
    assert_eq!(got, ["m OK ERR 8", "top 1 2 3"]);
}

#[test]
fn a_name_a_package_imports_without_exporting_is_undeclared_through_it() {
    // §26.6: without an export, an imported name is not visible through a
    // wildcard import of the importing package.
    undeclared(
        "package core; localparam int K = 3; endpackage
package allx; import core::*; localparam int KK = K; endpackage
module top; import allx::*; initial $display(\"%0d %0d\", KK, K); endmodule
",
        "K",
    );
}
