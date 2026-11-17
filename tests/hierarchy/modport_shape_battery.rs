//! Interface/modport shape battery — every expectation byte-verified
//! against the reference simulator (2026-08-18 audit), except the
//! method-prototype cases at the end: iverilog and Verilator do not
//! support explicit modport prototypes, so their expectations follow
//! IEEE 1800-2023 and the fixtures are checked with slang.

use xezim::simulate;

fn outs(src: &str, pfx: &str) -> Vec<String> {
    let sim = simulate(src, 1_000_000).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|l| l.starts_with(pfx))
        .collect()
}

/// Both modports of one interface bound to different modules: outputs drive,
/// inputs read, and `import` task/function calls go through the modport port.
const MODPORT_DIRECTIONS_AND_IMPORTS: &str = r#"
interface bus_if(input logic clk);
  logic [7:0] data;
  logic valid, ready;
  task automatic send(input logic [7:0] d);
    data = d; valid = 1;
  endtask
  function automatic logic [7:0] snoop();
    return data ^ {7'h0, valid};
  endfunction
  modport drv (output data, output valid, input ready, import send);
  modport mon (input data, input valid, output ready, import snoop);
endinterface

module producer(bus_if.drv b);
  initial begin
    #10 b.send(8'hA5);
    #10 b.data = 8'h3C; b.valid = 0;
  end
endmodule

module consumer(bus_if.mon b);
  initial begin
    b.ready = 1;
    #15 $display("P1A data=%h valid=%b snoop=%h", b.data, b.valid, b.snoop());
    #10 $display("P1B data=%h valid=%b", b.data, b.valid);
  end
endmodule

module top;
  logic clk = 0;
  bus_if bus(clk);
  producer u_p(bus);
  consumer u_c(bus);
  initial begin
    #40 $display("P1C ready=%b", bus.ready);
    $finish;
  end
endmodule

"#;

#[test]
fn modport_directions_and_imports() {
    assert_eq!(
        outs(MODPORT_DIRECTIONS_AND_IMPORTS, "P"),
        [
            "P1A data=a5 valid=1 snoop=a4",
            "P1B data=3c valid=0",
            "P1C ready=1",
        ]
    );
}

/// An array of interface instances wired through a generate-for, each
/// element bound to a parameterized module via a modport port.
const INTERFACE_ARRAY_GENERATE_MODPORTS: &str = r#"
interface ch_if;
  logic [15:0] cnt;
  modport src (output cnt);
  modport dst (input cnt);
endinterface

module pump #(parameter int K = 1) (ch_if.src c);
  initial begin
    c.cnt = 16'(K * 100);
    #5 c.cnt = c.cnt + 16'(K);
  end
endmodule

module top;
  ch_if chans[0:3]();
  for (genvar g = 0; g < 4; g++) begin : gen_p
    pump #(.K(g + 1)) u(chans[g]);
  end
  initial begin
    #20 $display("P2 %0d %0d %0d %0d", chans[0].cnt, chans[1].cnt, chans[2].cnt, chans[3].cnt);
    $finish;
  end
endmodule

"#;

#[test]
fn interface_array_generate_modports() {
    assert_eq!(
        outs(INTERFACE_ARRAY_GENERATE_MODPORTS, "P"),
        ["P2 101 202 303 404",]
    );
}

/// A class holds a virtual interface, drives through it from a task, and the
/// DUT reacts on the driven edge.
const VIRTUAL_INTERFACE_CLASS_DRIVER: &str = r#"
interface reg_if;
  logic [31:0] wdata, rdata;
  logic wen;
  modport ctl (output wdata, output wen, input rdata);
endinterface

class driver;
  virtual reg_if vif;
  function new(virtual reg_if v);
    vif = v;
  endfunction
  task run(input logic [31:0] x);
    vif.wdata = x;
    vif.wen = 1;
    #1 vif.wen = 0;
  endtask
endclass

module dut(reg_if r);
  always @(posedge r.wen) r.rdata <= r.wdata ^ 32'hffff_0000;
endmodule

module top;
  reg_if rif();
  dut u(rif);
  initial begin
    automatic driver d = new(rif);
    #5 d.run(32'h1234_5678);
    #5 $display("P3 rdata=%h", rif.rdata);
    $finish;
  end
endmodule

"#;

#[test]
fn virtual_interface_class_driver() {
    assert_eq!(
        outs(VIRTUAL_INTERFACE_CLASS_DRIVER, "P"),
        ["P3 rdata=edcb5678",]
    );
}

/// A MODPORT-typed virtual interface (`virtual pkt_if.wr`) assigned from a
/// full interface instance and driven from a class task.
const MODPORT_TYPED_VIRTUAL_INTERFACE: &str = r#"
interface pkt_if;
  logic [7:0] a, b;
  modport wr (output a, input b);
endinterface

class wrdrv;
  virtual pkt_if.wr vif;   // modport-typed virtual interface
  task push(input logic [7:0] x);
    vif.a = x;
  endtask
endclass

module top;
  pkt_if pif();
  assign pif.b = pif.a + 8'd1;
  initial begin
    automatic wrdrv d = new;
    d.vif = pif;
    #5 d.push(8'h41);
    #5 $display("P4 a=%h b=%h", pif.a, pif.b);
    $finish;
  end
endmodule

"#;

#[test]
fn modport_typed_virtual_interface() {
    assert_eq!(
        outs(MODPORT_TYPED_VIRTUAL_INTERFACE, "P"),
        ["P4 a=41 b=42",]
    );
}

/// An always block clocked and fed entirely through a modport port must
/// compile (this shape runs 20k edges; a fallback here is a perf cliff).
const CLOCKED_ALWAYS_THROUGH_MODPORT_COMPILES: &str = r#"
interface sync_if(input logic clk);
  logic [15:0] d, q;
  logic en;
  modport ff (input d, input en, output q, input clk);
endinterface

// compiled-path check: an always block clocked and fed entirely through a
// modport port
module ff_mod(sync_if.ff s);
  always @(posedge s.clk)
    if (s.en) s.q <= s.d + 16'd7;
endmodule

module top;
  logic clk = 0;
  always #5 clk = ~clk;
  sync_if sif(clk);
  ff_mod u(sif);
  logic [31:0] n = 0;
  always @(posedge clk) begin
    sif.d <= sif.d + 16'd3;
    sif.en <= ~sif.en;
    n <= n + 1;
  end
  initial begin
    sif.d = 0; sif.en = 0;
    wait (n >= 32'd10000);
    $display("P5 q=%h d=%h", sif.q, sif.d);
    $finish;
  end
endmodule

"#;

#[test]
fn clocked_always_through_modport_compiles() {
    assert_eq!(
        outs(CLOCKED_ALWAYS_THROUGH_MODPORT_COMPILES, "P"),
        ["P5 q=7534 d=7530",]
    );
}

/// The same interface function called through a modport `import` and
/// directly on the instance path.
const INTERFACE_FUNCTION_VIA_MODPORT_AND_DIRECT: &str = r#"
interface calc_if;
  logic [31:0] acc;
  function automatic logic [31:0] peek(input logic [31:0] k);
    return acc + k;
  endfunction
  modport user (import peek, output acc);
endinterface

module worker(calc_if.user c);
  initial begin
    c.acc = 32'd100;
    #5 $display("P6A peek=%0d", c.peek(32'd23));
  end
endmodule

module top;
  calc_if ci();
  worker w(ci);
  initial begin
    #10 $display("P6B direct=%0d", ci.peek(32'd1));
    $finish;
  end
endmodule

"#;

#[test]
fn interface_function_via_modport_and_direct() {
    assert_eq!(
        outs(INTERFACE_FUNCTION_VIA_MODPORT_AND_DIRECT, "P"),
        ["P6A peek=123", "P6B direct=101",]
    );
}

/// An interface instantiating another interface; members reached through
/// the nested path from a bound module.
const NESTED_INTERFACE_INSTANCE_MEMBERS: &str = r#"
interface inner_if;
  logic [7:0] x;
  modport m (output x);
endinterface

interface outer_if;
  inner_if leaf();
  logic [7:0] y;
  modport m (output y);
endinterface

module deep(outer_if o);
  initial begin
    o.leaf.x = 8'h5A;
    o.y = 8'hA5;
  end
endmodule

module top;
  outer_if oi();
  deep d(oi);
  initial begin
    #5 $display("P8 x=%h y=%h", oi.leaf.x, oi.y);
    $finish;
  end
endmodule

"#;

#[test]
fn nested_interface_instance_members() {
    assert_eq!(
        outs(NESTED_INTERFACE_INSTANCE_MEMBERS, "P"),
        ["P8 x=5a y=a5",]
    );
}

// IEEE 1800-2023 §25.3 Syntax 25-1 / §25.7: method prototypes and
// name-only imports share the existing modport task/function call path.
#[test]
fn modport_task_prototype_before_signal() {
    assert_eq!(
        outs(
            include_str!("modport_method_prototypes/task_before_signal.sv"),
            ""
        ),
        ["value=37", "PASS"]
    );
}

#[test]
fn modport_mixed_method_prototypes() {
    assert_eq!(
        outs(
            include_str!("modport_method_prototypes/mixed_prototypes.sv"),
            "T|"
        ),
        ["T|call=61", "T|observe=56 count=9", "T|last=59"]
    );
}

#[test]
fn modport_name_only_imports_and_views() {
    assert_eq!(
        outs(
            include_str!("modport_method_prototypes/name_only_and_views.sv"),
            "T|"
        ),
        ["T|observe=37", "T|expr=37", "T|state=37 9 11"]
    );
}

#[test]
fn modport_export_task_prototype() {
    assert_eq!(
        outs(include_str!("modport_method_prototypes/export_view.sv"), ""),
        ["value=37", "PASS"]
    );
}

// Syntax 25-1 / Annex A.2.6 footnote 25: require a callable name, a
// well-formed tf_port_list and the modport separator; no method specifier.
#[test]
fn modport_malformed_prototypes_are_rejected() {
    for (member, message) in [
        ("import task (input int n)", "expected identifier"),
        ("import task put(input int n int)", "expected RParen"),
        (
            "import task put(input int n) input value",
            "expected ',' or ')' after modport prototype",
        ),
        ("import function :final int get()", "modport prototype"),
        ("import task put(input int n;", "expected RParen"),
    ] {
        let src = format!(
            "interface bus_if; int value; task put(input int n); endtask
                           modport user({member}); endinterface module top; endmodule"
        );
        let err = simulate(&src, 1000)
            .err()
            .expect("malformed prototype accepted");
        assert!(err.contains(message), "{member}: {err}");
    }
}
