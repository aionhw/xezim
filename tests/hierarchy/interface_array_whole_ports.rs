//! IEEE 1800-2023 §23.3.3.5 and §7.6: equal-size formal and actual arrays pair
//! elements in declared order.

use xezim::simulate;

fn check_output(src: &str, expected: &str) {
    let sim = simulate(src, 100).expect("simulate");
    let out: Vec<_> = sim
        .output
        .iter()
        .map(|line| line.message.as_str())
        .filter(|line| line.starts_with("T|"))
        .collect();
    assert_eq!(out, [expected]);
}

fn connect(src: &str, left: &str, right: &str, actual: &str) -> String {
    src.replace("FORMAL_LEFT", left)
        .replace("FORMAL_RIGHT", right)
        .replace("ACTUAL", actual)
}

macro_rules! connection_tests {
    ($module:ident, $source:ident, $expected:literal) => {
        mod $module {
            use super::{check_output, connect, $source};

            #[test]
            fn whole_offset_bounds() {
                check_output(&connect($source, "3", "2", "a"), $expected);
            }

            #[test]
            fn explicit_slice() {
                check_output(&connect($source, "3", "2", "a[0:1]"), $expected);
            }

            #[test]
            fn whole_reversed_bounds() {
                check_output(&connect($source, "1", "0", "a"), $expected);
            }
        }
    };
}

const MEMBER_WRITES: &str = r#"
interface bus_if;
  int data;
endinterface
module client(bus_if p[FORMAL_LEFT:FORMAL_RIGHT]);
  initial begin
    #1;
    p[FORMAL_LEFT].data = 33;
    p[FORMAL_RIGHT].data = 22;
  end
endmodule
module top;
  bus_if a[0:1]();
  client u(.p(ACTUAL));
  initial begin
    #2;
    $display("T|actual=%0d,%0d", a[0].data, a[1].data);
    $finish;
  end
endmodule
"#;
connection_tests!(member_writes, MEMBER_WRITES, "T|actual=33,22");

// §9.2.2.4 and §9.4.2: both processes sample 0x10 on rising edges.
const CLOCKED_LOGIC: &str = r#"
interface bus_if(input logic clk);
  logic [7:0] d, q;
endinterface
module client(bus_if p[FORMAL_LEFT:FORMAL_RIGHT]);
  always_ff @(posedge p[FORMAL_LEFT].clk) p[FORMAL_LEFT].q <= p[FORMAL_LEFT].d + 8'd1;
  always_ff @(posedge p[FORMAL_RIGHT].clk) p[FORMAL_RIGHT].q <= p[FORMAL_RIGHT].d + 8'd1;
endmodule
module top;
  logic clk = 0;
  always #1 clk = ~clk;
  bus_if a[0:1](clk);
  client u(.p(ACTUAL));
  initial begin
    a[0].d = 8'h10;
    a[1].d = 8'h10;
    #4;
    $display("T|q=%h,%h", a[0].q, a[1].q);
    $finish;
  end
endmodule
"#;
connection_tests!(clocked_logic, CLOCKED_LOGIC, "T|q=11,11");

// §9.4.2: each signal changes four times (including x to 0), with two rising edges.
const EDGE_EVENTS: &str = r#"
interface bus_if;
  logic s;
  int changes = 0, rises = 0;
endinterface
module client(bus_if p[FORMAL_LEFT:FORMAL_RIGHT]);
  always @(p[FORMAL_LEFT].s) p[FORMAL_LEFT].changes++;
  always @(p[FORMAL_RIGHT].s) p[FORMAL_RIGHT].changes++;
  always @(posedge p[FORMAL_LEFT].s) p[FORMAL_LEFT].rises++;
  always @(posedge p[FORMAL_RIGHT].s) p[FORMAL_RIGHT].rises++;
endmodule
module top;
  bus_if a[0:1]();
  client u(.p(ACTUAL));
  initial begin
    #1; a[0].s = 0; a[1].s = 0;
    #1; a[0].s = 1; a[1].s = 1;
    #1; a[0].s = 0; a[1].s = 0;
    #1; a[0].s = 1; a[1].s = 1;
    #1;
    $display("T|changes=%0d,%0d rises=%0d,%0d",
             a[0].changes, a[1].changes, a[0].rises, a[1].rises);
    $finish;
  end
endmodule
"#;
connection_tests!(edge_events, EDGE_EVENTS, "T|changes=4,4 rises=2,2");

// §9.4.2 and §10.4.2: each of five rising edges schedules one increment.
const EDGE_WITH_NBA: &str = r#"
interface bus_if(input logic clk);
  logic [7:0] data;
endinterface
module client(bus_if p[FORMAL_LEFT:FORMAL_RIGHT]);
  always @(posedge p[FORMAL_LEFT].clk) p[FORMAL_LEFT].data <= p[FORMAL_LEFT].data + 1;
  always @(posedge p[FORMAL_RIGHT].clk) p[FORMAL_RIGHT].data <= p[FORMAL_RIGHT].data + 1;
endmodule
module top;
  logic clk = 0;
  always #1 clk = ~clk;
  bus_if a[0:1](clk);
  client u(.p(ACTUAL));
  initial begin
    a[0].data = 0;
    a[1].data = 0;
    #10;
    $display("T|count=%0d,%0d", a[0].data, a[1].data);
    $finish;
  end
endmodule
"#;
connection_tests!(edge_with_nba, EDGE_WITH_NBA, "T|count=5,5");

// §7.6: whole assignment copies the fixed array and sizes the queue and dynamic array.
const AGGREGATE_ASSIGNMENT: &str = r#"
interface bus_if;
  int arr[3], q[$], d[];
endinterface
module client(bus_if p[FORMAL_LEFT:FORMAL_RIGHT]);
  int values[3] = '{1, 2, 3};
  initial begin
    p[FORMAL_LEFT].arr = values;
    p[FORMAL_RIGHT].arr = values;
    p[FORMAL_LEFT].q = {4, 5};
    p[FORMAL_RIGHT].q = {4, 5};
    p[FORMAL_LEFT].d = '{6, 7, 8};
    p[FORMAL_RIGHT].d = '{6, 7, 8};
  end
endmodule
module top;
  bus_if a[0:1]();
  client u(.p(ACTUAL));
  initial begin
    #1;
    $display("T|arr=%0d,%0d q=%0d,%0d d=%0d,%0d",
             a[0].arr[2], a[1].arr[2], a[0].q.size(), a[1].q.size(), a[0].d.size(), a[1].d.size());
    $finish;
  end
endmodule
"#;
connection_tests!(
    aggregate_assignment,
    AGGREGATE_ASSIGNMENT,
    "T|arr=3,3 q=2,2 d=3,3"
);

// §12.7.3: foreach visits index 3 of each fixed array member.
const FOREACH_MEMBER: &str = r#"
interface bus_if;
  int arr[4];
endinterface
module client(bus_if p[FORMAL_LEFT:FORMAL_RIGHT]);
  initial begin
    foreach (p[FORMAL_LEFT].arr[i]) p[FORMAL_LEFT].arr[i] = 10 + i;
    foreach (p[FORMAL_RIGHT].arr[i]) p[FORMAL_RIGHT].arr[i] = 10 + i;
  end
endmodule
module top;
  bus_if a[0:1]();
  client u(.p(ACTUAL));
  initial begin
    #1;
    $display("T|arr=%0d,%0d", a[0].arr[3], a[1].arr[3]);
    $finish;
  end
endmodule
"#;
connection_tests!(foreach_member, FOREACH_MEMBER, "T|arr=13,13");

// §18.7: each inline constraint has the unique solution v == 77.
const RANDOMIZE_MEMBER: &str = r#"
package pk;
  class C;
    rand int v;
  endclass
endpackage
interface bus_if;
  pk::C c;
endinterface
module client(bus_if p[FORMAL_LEFT:FORMAL_RIGHT]);
  initial begin
    p[FORMAL_LEFT].c = new;
    p[FORMAL_RIGHT].c = new;
    void'(p[FORMAL_LEFT].c.randomize() with { v == 77; });
    void'(p[FORMAL_RIGHT].c.randomize() with { v == 77; });
  end
endmodule
module top;
  bus_if a[0:1]();
  client u(.p(ACTUAL));
  initial begin
    #1;
    $display("T|v=%0d,%0d", a[0].c.v, a[1].c.v);
    $finish;
  end
endmodule
"#;
connection_tests!(randomize_member, RANDOMIZE_MEMBER, "T|v=77,77");

// §25.9 and §7.6: each virtual interface refers to an instance in the port array.
const VIRTUAL_INTERFACE_ARRAY: &str = r#"
interface bus_if;
  int data;
endinterface
module client(bus_if p[FORMAL_LEFT:FORMAL_RIGHT]);
  virtual bus_if v[FORMAL_LEFT:FORMAL_RIGHT] = p;
  initial begin
    #1;
    v[FORMAL_LEFT].data = 5;
    v[FORMAL_RIGHT].data = 5;
  end
endmodule
module top;
  bus_if a[0:1]();
  client u(.p(ACTUAL));
  initial begin
    #2;
    $display("T|actual=%0d,%0d", a[0].data, a[1].data);
    $finish;
  end
endmodule
"#;
connection_tests!(
    virtual_interface_array,
    VIRTUAL_INTERFACE_ARRAY,
    "T|actual=5,5"
);

// §23.9: p inside the named procedural block refers to the local declaration.
const LOCAL_SHADOW: &str = r#"
interface bus_if;
endinterface
module client(bus_if p[1:0]);
  initial begin : local_scope
    int p[1];
    p[0] = 1;
    $display("T|local=%0d", p[0]);
    #1 $finish;
  end
endmodule
module top;
  bus_if a[0:1]();
  client u(a);
endmodule
"#;

#[test]
fn block_local_read_shadows_interface_formal() {
    check_output(LOCAL_SHADOW, "T|local=1");
}

// §5.6.1 and §23.3.2.3–4: each escaped name is a single identifier.
// The implicit port connections use the d declared in top.
const ESCAPED_INSTANCES: &str = r#"
module leaf(input logic [7:0] d, output logic [7:0] q);
  assign q = d + 1;
endmodule
module top;
  logic [7:0] d = 10;
  wire [7:0] q, q2;
  if (1) begin : g
    logic [7:0] d = 20;
  end
  leaf \g.u (.d, .q);
  leaf \g.v (.q(q2), .*);
  initial begin
    #1;
    $display("T|q=%0d q2=%0d", q, q2);
    $finish;
  end
endmodule
"#;

#[test]
fn escaped_instance_names_keep_implicit_connection_scope() {
    check_output(ESCAPED_INSTANCES, "T|q=11 q2=11");
}

// §23.9 and §27.5: the inner a is a separate declaration in both block forms.
const GENERATE_SHADOW: &str = r#"
interface bus_if;
  int data;
endinterface
module client(bus_if p[3:2]);
  initial begin
    #1;
    p[3].data = 33;
    p[2].data = 22;
  end
endmodule
module top;
  bus_if a[3:2]();
  client outer_client(a);
  if (1) begin BLOCK_NAME
    bus_if a[0:1]();
    client inner_client(a);
  end
  initial begin
    #2;
    $display("T|outer=%0d,%0d", a[3].data, a[2].data);
    $finish;
  end
endmodule
"#;

#[test]
fn named_generate_array_does_not_shadow_outer_connection() {
    check_output(
        &GENERATE_SHADOW.replace("BLOCK_NAME", ": g"),
        "T|outer=33,22",
    );
}

#[test]
fn unnamed_generate_array_does_not_shadow_outer_connection() {
    check_output(&GENERATE_SHADOW.replace(" BLOCK_NAME", ""), "T|outer=33,22");
}
