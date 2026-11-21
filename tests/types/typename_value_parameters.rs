//! Procedural `$typename` of value parameters (IEEE 1800-2023 §20.6.1).
//! Expected strings use slang's spelling where the LRM leaves it open.

use xezim::simulate;

fn check_types(source: &str, expected: &[&str]) {
    let sim = simulate(source, 100).expect("simulate failed");
    let mut actual: Vec<_> = sim
        .output
        .iter()
        .flat_map(|entry| entry.message.lines())
        .filter(|line| line.starts_with("T|"))
        .collect();
    actual.sort_unstable();
    assert_eq!(actual, expected);
}

/// §6.20.2: an explicit type applies even when the initializer is converted.
#[test]
fn declared_scalars_and_localparams() {
    const SRC: &str = r#"`timescale 1ns/1ps
module top;
  parameter real PR = 2;
  parameter int PI = 2.5;
  parameter bit PB = 1;
  localparam real LR = 2.5;
  localparam shortreal LS = 1.5;
  localparam byte LB = 3;
  localparam string LT = "abc";
  initial begin
    $display("T|parameters %s %s %s", $typename(PR), $typename(PI), $typename(PB));
    $display("T|localparams %s %s %s %s",
      $typename(LR), $typename(LS), $typename(LB), $typename(LT));
  end
endmodule
"#;
    check_types(
        SRC,
        &[
            "T|localparams real shortreal byte string",
            "T|parameters real int bit",
        ],
    );
}

/// §6.20.2, §23.10: signing alone takes the final width. A written range
/// retains its bounds and direction, including after an override.
#[test]
fn signing_only_and_declared_ranges() {
    const SRC: &str = r#"`timescale 1ns/1ps
module leaf #(parameter signed S = 5'd3, parameter [7:4] R = 3,
              parameter signed [0:2] SR = 1, parameter bit signed [5:2] B = 3) ();
  initial $display("T|%m %s %s %s %s",
    $typename(S), $typename(R), $typename(SR), $typename(B));
endmodule
module top;
  leaf a();
  leaf #(.S(9'd1), .R(32'hff), .SR(8'd4), .B(8'd5)) b();
endmodule
"#;
    check_types(
        SRC,
        &[
            "T|top.a logic signed[4:0] logic[7:4] logic signed[0:2] bit signed[5:2]",
            "T|top.b logic signed[8:0] logic[7:4] logic signed[0:2] bit signed[5:2]",
        ],
    );
}

/// §26.3, §7.4.2: qualification and both forms of import refer to the package
/// declaration. The unpacked parameter retains the element type and range.
#[test]
fn qualified_and_imported_package_parameters() {
    const SRC: &str = r#"`timescale 1ns/1ps
package cfg;
  parameter real PR = 2;
  parameter shortint PI = 3;
  localparam int A[2] = '{1, 2};
endpackage
package defaults;
  parameter byte PB = 4;
endpackage
module top;
  import cfg::PI;
  import defaults::*;
  initial begin
    $display("T|qualified %s %s %s", $typename(cfg::PR), $typename(cfg::A), $typename(cfg::A[0]));
    $display("T|imports %s %s", $typename(PI), $typename(PB));
  end
endmodule
"#;
    check_types(
        SRC,
        &["T|imports shortint byte", "T|qualified real int$[0:1] int"],
    );
}

/// §23.6, §23.10, §27.5: references inside a child and from its parent use
/// the child's declaration. Named and defparam overrides preserve its type.
#[test]
fn child_parameters_and_overrides() {
    const SRC: &str = r#"`timescale 1ns/1ps
module leaf #(parameter shortint P = 7, parameter real R = 1.5) ();
  if (1) begin : g
    initial $display("T|%m %s %s", $typename(P), $typename(R));
  end
endmodule
module top;
  leaf #(.P(5'sd4), .R(2)) a();
  leaf b();
  defparam b.P = 3.25;
  initial $display("T|hierarchical %s %s", $typename(a.P), $typename(b.R));
endmodule
"#;
    check_types(
        SRC,
        &[
            "T|hierarchical shortint real",
            "T|top.a.g shortint real",
            "T|top.b.g shortint real",
        ],
    );
}

/// §25.9, §26.3: a dotted reference through a virtual interface variable
/// denotes its member even when a package has the same two names.
#[test]
fn virtual_interface_member_with_package_namesake() {
    const SRC: &str = r#"`timescale 1ns/1ps
package bus;
  parameter real valid = 1.5;
endpackage
interface bus_if;
  logic valid;
endinterface
module top;
  bus_if i0();
  virtual bus_if bus;
  initial begin
    bus = i0;
    i0.valid = 1;
    $display("T|bus.valid %s %0d", $typename(bus.valid), bus.valid);
  end
endmodule
"#;
    check_types(SRC, &["T|bus.valid logic 1"]);
}

/// §6.18, §23.6, §26.3: the dotted name refers to the block variable.
/// Its typedef resolves to `int`.
#[test]
fn named_block_local_with_package_namesake() {
    const SRC: &str = r#"`timescale 1ns/1ps
package blk;
  parameter real Q = 2.5;
endpackage
module top;
  typedef int count_t;
  initial begin : blk
    count_t Q;
    $display("T|blk.Q %s", $typename(blk.Q));
  end
endmodule
"#;
    check_types(SRC, &["T|blk.Q int"]);
}

/// §23.9, §27.5: a block-local variable can hide a parameter, and a
/// parameter in a distinct generate scope does not change a variable's type.
#[test]
fn variables_and_parameters_with_the_same_name() {
    const SRC: &str = r#"`timescale 1ns/1ps
module top;
  parameter real P = 1.5;
  int V;
  if (1) begin : g
    localparam real V = 2.5;
  end
  initial begin
    $display("T|module V %s", $typename(V));
    begin
      int P;
      $display("T|local P %s", $typename(P));
    end
  end
endmodule
"#;
    check_types(SRC, &["T|local P int", "T|module V int"]);
}

/// §11.4, §11.6, §11.8.1: querying an expression names its own result type.
#[test]
fn expression_types() {
    const SRC: &str = r#"`timescale 1ns/1ps
module top;
  parameter real P = 1.5;
  int I;
  bit [3:0] V;
  initial $display("T|expressions %s %s %s %s",
    $typename(P+0), $typename(I+0), $typename({V,V}), $typename(1.5));
endmodule
"#;
    check_types(SRC, &["T|expressions real int bit[7:0] real"]);
}

/// §6.6, §6.18, §6.19, §7.2, §8.4: declarations of other kinds keep
/// their own resolved types. The packed enum select (§11.5.1) and the
/// struct member have built-in result types. Compare the class handle
/// with its declared type name so the check works with either spelling.
#[test]
fn typedef_enum_struct_net_and_class_operands() {
    const SRC: &str = r#"`timescale 1ns/1ps
class C;
endclass
module top;
  parameter real T = 1.5, E = 2.5, S = 3.5, N = 4.5, H = 5.5;
  if (1) begin : g
    typedef bit signed [3:0] T;
    typedef enum bit [1:0] {OFF, ON} state_t;
    state_t E;
    struct packed {bit [3:0] field;} S;
    wire N;
    C H;
    initial begin
      $display("T|typedef %s", $typename(T));
      $display("T|enum select %s", $typename(E[0]));
      $display("T|struct member %s", $typename(S.field));
      $display("T|net %s", $typename(N));
      $display("T|class matches %0d", $typename(H) == $typename(C));
    end
  end
endmodule
"#;
    check_types(
        SRC,
        &[
            "T|class matches 1",
            "T|enum select bit",
            "T|net logic",
            "T|struct member bit[3:0]",
            "T|typedef bit signed[3:0]",
        ],
    );
}

/// §6.20.2, §23.10, §27.5: names used to size a parameter are resolved
/// where it is declared. A generated localparam keeps its own atom type.
#[test]
fn instance_bound_ranges_and_generated_parameters() {
    check_types(
        r#"
module leaf #(parameter int W = 3, parameter bit [W:1] P = 0)();
  if (1) begin : g
    localparam shortint Q = 3;
    initial $display("T|%m %s %s", $typename(P), $typename(Q));
  end
endmodule
module top;
  leaf #(4) a(); leaf #(7) b();
  if (1) begin : g
    localparam byte Q = 4;
    initial $display("T|%m %s", $typename(Q));
  end
endmodule
"#,
        &[
            "T|top.a.g bit[4:1] shortint",
            "T|top.b.g bit[7:1] shortint",
            "T|top.g byte",
        ],
    );
}

/// §8.3, §23.9: inside a method its property's declaration precedes the
/// enclosing module parameter of the same name.
#[test]
fn class_property_shadows_module_parameter() {
    check_types(
        r#"
module top;
  parameter real RATE = 2.5;
  class sensor;
    shortint RATE;
    function void show();
      $display("T|property %s", $typename(RATE));
    endfunction
  endclass
  initial begin
    sensor s = new;
    s.show();
    $display("T|parameter %s", $typename(RATE));
  end
endmodule
"#,
        &["T|parameter real", "T|property shortint"],
    );
}
