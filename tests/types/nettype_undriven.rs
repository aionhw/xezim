//! IEEE 1800-2023 §6.6.7 and §6.7.3 require a time-zero resolver call.
//! A net with no drivers receives an empty driver array.

fn check_output(source: &str, expected: &[&str]) {
    let sim = xezim::simulate(source, 10_000).expect("simulate resolved nettype");
    let output: Vec<_> = sim.output.iter().map(|line| line.message.trim()).collect();
    assert_eq!(output, expected);
}

#[test]
fn undriven_real_net() {
    let source = r#"`timescale 1ns/1ps
package p;
  function automatic real fold(input real d[]);
    fold = 10.0;
    foreach (d[i]) fold += d[i];
  endfunction
  nettype real N with fold;
endpackage

module top;
  import p::*;
  N n;
  initial begin
    #1;
    $display("n=%g", n);
    $finish;
  end
endmodule
"#;
    check_output(source, &["n=10"]);
}

/// §6.6.7, §10.3: the driver array size counts drivers even when their values are zero.
#[test]
fn zero_one_and_two_drivers() {
    let source = r#"`timescale 1ns/1ps
module top;
  function automatic real fold(input real d[]);
    real result = 10.0 + d.size();
    foreach (d[i]) result += d[i];
    return result;
  endfunction
  nettype real N with fold;
  N empty, one, two;
  assign one = 0.0;
  assign two = 0.0;
  assign two = 0.0;
  initial begin
    #1 $display("counts=%g,%g,%g", empty, one, two);
    $finish;
  end
endmodule
"#;
    check_output(source, &["counts=10,11,12"]);
}

/// §6.6.7: a nettype synonym retains the resolver of the original nettype.
#[test]
fn nettype_synonym() {
    let source = r#"`timescale 1ns/1ps
package p;
  function automatic real constant_value(input real d[]);
    return 42.0;
  endfunction
  nettype real N with constant_value;
  nettype N Synonym;
endpackage

module top;
  import p::*;
  N original;
  Synonym renamed;
  initial begin
    #1 $display("synonym=%g,%g", original, renamed);
    $finish;
  end
endmodule
"#;
    check_output(source, &["synonym=42,42"]);
}

/// §6.6.7(a), §6.7.3: resolve the whole packed value at time zero.
#[test]
fn packed_logic_net() {
    let source = r#"`timescale 1ns/1ps
function automatic logic [3:0] fold(input logic [3:0] d[]);
  logic [3:0] result = 4'ha + d.size();
  foreach (d[i]) result += d[i];
  return result;
endfunction
nettype logic [3:0] N with fold;

module top;
  N empty, driven;
  assign driven = 4'h2;
  initial begin
    #1 $display("packed=%h,%h", empty, driven);
    $finish;
  end
endmodule
"#;
    check_output(source, &["packed=a,d"]);
}

/// §6.6.7(d), §7.2.2: one resolution returns all members of an unpacked struct.
#[test]
fn unpacked_struct_net_in_child() {
    let source = r#"`timescale 1ns/1ps
typedef struct { real r; int i; } Payload;
function automatic Payload fold(input Payload d[]);
  Payload result = '{10.0, 100};
  foreach (d[j]) begin
    result.r += d[j].r;
    result.i += d[j].i;
  end
  return result;
endfunction
nettype Payload N with fold;

module child;
  N empty, driven;
  assign driven = '{2.0, 1};
endmodule

module top;
  child u();
  initial begin
    #1 $display("struct=%g,%0d/%g,%0d", u.empty.r, u.empty.i, u.driven.r, u.driven.i);
    $finish;
  end
endmodule
"#;
    check_output(source, &["struct=10,100/12,101"]);
}

/// §6.6.7, §7.4.2: each element of an unpacked array of nets is a separate net.
#[test]
fn unpacked_array_of_nets() {
    let source = r#"`timescale 1ns/1ps
function automatic real fold(input real d[]);
  return 10.0 + d.size();
endfunction
nettype real N with fold;

module top;
  N descending[1:0], negative[-2:-1];
  initial begin
    #1 $display("array=%g,%g/%g,%g", descending[1], descending[0], negative[-2], negative[-1]);
    $finish;
  end
endmodule
"#;
    check_output(source, &["array=10,10/10,10"]);
}

/// §6.6.7, §23.3.3: same-nettype port connections share one net and its drivers.
#[test]
fn port_component_without_drivers() {
    let source = r#"`timescale 1ns/1ps
function automatic real fold(input real d[]);
  return 10.0 + d.size();
endfunction
nettype real N with fold;

module source(output N n);
endmodule

module sink(input N n);
endmodule

module top;
  N n;
  source s(n);
  sink a(n), b(n);
  initial begin
    #1 $display("ports=%g,%g,%g,%g", n, s.n, a.n, b.n);
    $finish;
  end
endmodule
"#;
    check_output(source, &["ports=10,10,10,10"]);
}

/// §6.6.8, §7.4.2, §23.3.3.7.1: each interconnect element joins its own ports.
#[test]
fn interconnect_array_with_one_driven_element() {
    let source = r#"`timescale 1ns/1ps
function automatic real fold(input real d[]);
  real result = 10.0;
  foreach (d[i]) result += d[i];
  return result;
endfunction
nettype real N with fold;

module source(output N n);
  assign n = 2.0;
endmodule

module sink(input N n);
endmodule

module top;
  interconnect link[-2:0];
  source s(link[-1]);
  sink a(link[-1]), b(link[-2]), c(link[0]);
  initial begin
    #1 $display("interconnect=%g,%g,%g", a.n, b.n, c.n);
    $finish;
  end
endmodule
"#;
    check_output(source, &["interconnect=12,10,10"]);
}

/// §10.11: the aliases name the same physical net.
#[test]
fn undriven_net_alias() {
    let source = r#"`timescale 1ns/1ps
function automatic real fold(input real d[]);
  return 10.0 + d.size();
endfunction
nettype real N with fold;

module top;
  N n, m;
  alias n = m;
  initial begin
    #1 $display("alias=%g,%g", n, m);
    $finish;
  end
endmodule
"#;
    check_output(source, &["alias=10,10"]);
}

/// §27.5: a branch discarded at elaboration contributes no driver.
#[test]
fn conditional_generate() {
    let source = r#"`timescale 1ns/1ps
function automatic real fold(input real d[]);
  real result = 10.0 + d.size();
  foreach (d[i]) result += d[i];
  return result;
endfunction
nettype real N with fold;

module top;
  N pruned, driven;
  if (0) begin : disabled
    assign pruned = 2.0;
  end
  if (1) begin : enabled
    N empty;
    assign driven = 2.0;
  end
  initial begin
    #1 $display("generate=%g,%g,%g", pruned, driven, enabled.empty);
    $finish;
  end
endmodule
"#;
    check_output(source, &["generate=10,13,10"]);
}

/// §6.7.3, §25.2: the net inside an interface also requires the resolver call.
#[test]
fn undriven_net_in_interface() {
    let source = r#"`timescale 1ns/1ps
function automatic real fold(input real d[]);
  return 10.0 + d.size();
endfunction
nettype real N with fold;

interface bus;
  N n;
endinterface

module top;
  bus b();
  initial begin
    #1 $display("interface=%g", b.n);
    $finish;
  end
endmodule
"#;
    check_output(source, &["interface=10"]);
}

/// §6.6.7, §10.3, §23.6, §27.4: the sum of one driver is that driver's value.
#[test]
fn generate_loop_driven_from_testbench() {
    let source = r#"`timescale 1ns/1ps
function automatic real sum(input real d[]);
  real result = 0.0;
  foreach (d[i]) result += d[i];
  return result;
endfunction
nettype real N with sum;

module dut;
  for (genvar i = 0; i < 2; i++) begin : g
    N n;
  end
endmodule

module top;
  dut d();
  real stimulus = 1.25;
  assign d.g[0].n = stimulus;
  assign d.g[1].n = 2.0 * stimulus;
  initial begin
    #1 $display("testbench=%g,%g", d.g[0].n, d.g[1].n);
    #1 stimulus = 3.0;
    #1 $display("testbench=%g,%g", d.g[0].n, d.g[1].n);
    $finish;
  end
endmodule
"#;
    check_output(source, &["testbench=1.25,2.5", "testbench=3,6"]);
}

/// §6.6.7, §10.3, §23.6, §27.4: both indexed paths supply drivers.
#[test]
fn generate_loop_driven_from_enclosing_module_and_other_iteration() {
    let source = r#"`timescale 1ns/1ps
function automatic real sum(input real d[]);
  real result = 0.0;
  foreach (d[i]) result += d[i];
  return result;
endfunction
nettype real N with sum;

module top;
  for (genvar i = 0; i < 2; i++) begin : g
    N n;
    if (i == 1) assign g[0].n = 4.0;
  end
  assign g[1].n = 5.0;
  initial begin
    #1 $display("indexed=%g,%g", g[1].n, g[0].n);
    $finish;
  end
endmodule
"#;
    check_output(source, &["indexed=5,4"]);
}

/// §6.6.7, §10.3, §23.6, §27.4: indexed paths in a child use its own instance.
#[test]
fn generate_loop_driven_inside_child() {
    let source = r#"`timescale 1ns/1ps
function automatic real sum(input real d[]);
  real result = 0.0;
  foreach (d[i]) result += d[i];
  return result;
endfunction
nettype real N with sum;

module child #(parameter real Drive = 5.0);
  for (genvar i = 0; i < 2; i++) begin : g
    N n;
  end
  assign g[1].n = Drive;
endmodule

module top;
  child a();
  child #(8.0) b();
  initial begin
    #1 $display("child=%g,%g", a.g[1].n, b.g[1].n);
    $finish;
  end
endmodule
"#;
    check_output(source, &["child=5,8"]);
}

/// §6.6.7, §10.3, §23.6, §23.8: both hierarchical names refer to driven top-level nets.
#[test]
fn upward_and_rooted_continuous_drivers() {
    let source = r#"`timescale 1ns/1ps
function automatic real sum(input real d[]);
  real result = 0.0;
  foreach (d[i]) result += d[i];
  return result;
endfunction
nettype real N with sum;

module child;
  assign top.n = 5.0;
  assign $root.top.m = 6.0;
endmodule

module top;
  N n, m;
  child u();
  initial begin
    #1 $display("upward=%g,%g", n, m);
    $finish;
  end
endmodule
"#;
    check_output(source, &["upward=5,6"]);
}

/// §6.6.7, §14.16: OR with the initial Z driver yields X. A drive of 7 yields 7.
#[test]
fn module_clocking_output() {
    let source = r#"`timescale 1ns/1ps
function automatic logic [3:0] fold(input logic [3:0] d[]);
  logic [3:0] result = '0;
  foreach (d[i]) result |= d[i];
  return result;
endfunction
nettype logic [3:0] N with fold;

module top;
  logic clk = 0;
  always #1 clk = ~clk;
  N n;
  clocking cb @(posedge clk);
    output n;
  endclocking
  initial begin
    @(cb);
    cb.n <= 4'h7;
  end
  initial begin
    #0.5 $display("before=%h", n);
    #4 $display("after=%h", n);
    $finish;
  end
endmodule
"#;
    check_output(source, &["before=x", "after=7"]);
}

/// §6.6.7, §14.16, §27.5: the generated clocking output has the same driver semantics.
#[test]
fn generated_clocking_output() {
    let source = r#"`timescale 1ns/1ps
function automatic logic [3:0] fold(input logic [3:0] d[]);
  logic [3:0] result = '0;
  foreach (d[i]) result |= d[i];
  return result;
endfunction
nettype logic [3:0] N with fold;

module top;
  logic clk = 0;
  always #1 clk = ~clk;
  if (1) begin : g
    N n;
    clocking cb @(posedge clk);
      output n;
    endclocking
    initial begin
      @(cb);
      cb.n <= 4'h7;
    end
  end
  initial begin
    #0.5 $display("before=%h", g.n);
    #4 $display("after=%h", g.n);
    $finish;
  end
endmodule
"#;
    check_output(source, &["before=x", "after=7"]);
}

/// §5.6.1, §6.7.3: brackets inside an escaped scalar name belong to its name.
#[test]
fn undriven_escaped_scalar() {
    let source = r#"
function automatic real fold(input real d[]);
  return 10.0 + d.size();
endfunction
nettype real N with fold;
module top;
  N \escaped[0] ;
  initial begin
    #1 $display("escaped=%g", \escaped[0] );
    $finish;
  end
endmodule
"#;
    check_output(source, &["escaped=10"]);
}
