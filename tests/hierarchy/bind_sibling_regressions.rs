//! IEEE 1800 §25.9–25.10 and §23.11: reference-checked sibling regressions.
use xezim::simulate_multi;
fn check(source: &str, expected: &[&str]) {
    let sim = simulate_multi(
        &[source.to_string()],
        10,
        Some("top"),
        &[],
        &[],
        None,
        false,
        None,
        None,
        &[],
        &[],
        None,
        &[],
        0,
        u64::MAX,
        None,
        &[],
        None,
        None,
        None,
        None,
        false,
    )
    .expect("simulate sibling regression");
    let mut lines: Vec<_> = sim
        .output
        .iter()
        .map(|o| o.message.trim_end())
        .filter(|s| s.starts_with("T|"))
        .collect();
    let mut expected = expected.to_vec();
    lines.sort();
    expected.sort();
    assert_eq!(lines, expected);
}

/// §23.11 and §25.5: a bound input follows a sibling's indexed modport.
#[test]
fn bound_port_reads_a_siblings_indexed_modport() {
    check(
        r#"
interface bus;
  logic valid;
  modport source(output valid);
endinterface
module monitor;
  bus b[1:0]();
  initial begin
    b[0].valid = 1'b0;
    b[1].valid = 1'b0;
    #1 b[0].valid = 1'b1;
    #2 b[0].valid = 1'b0;
  end
endmodule
module sink(input logic valid);
  initial begin
    #2 $display("T|first %b", valid);
    #2 $display("T|second %b", valid);
  end
endmodule
module host; endmodule
bind top.h monitor u_mon();
bind top.h sink u_sink(.valid(u_mon.b[0].source.valid));
module top;
  host h();
  initial #5 $finish;
endmodule
"#,
        &["T|first 1", "T|second 0"],
    );
}

#[test]
fn bind_defparam() {
    check(
        r####"module mem; logic [3:0] data=7; endmodule
module mon(input logic [3:0] s); initial #1 $display("T|%m=%0d",s); endmodule
module host #(parameter N=1); for(genvar g=0;g<N;g++) begin: bank mem m(); end endmodule
module top; host h(); defparam h.N=3; bind top.h.bank[2].m mon b(.s(data)); initial #2 $finish; endmodule
"####,
        &["T|top.h.bank[2].m.b=7"],
    );
}

#[test]
fn bind_type_parameter() {
    check(
        r####"module mem; logic [3:0] data=7; endmodule
module mon(input logic [3:0] s); initial #1 $display("T|%m=%0d",s); endmodule
module host #(parameter type T=bit); for(genvar g=0;g<$bits(T);g++) begin: bank mem m(); end endmodule
module top; host#(logic[2:0]) h(); bind top.h.bank[2].m mon b(.s(data)); initial #2 $finish; endmodule
"####,
        &["T|top.h.bank[2].m.b=7"],
    );
}

#[test]
fn bind_relative() {
    check(
        r####"module mem; logic [3:0] data=7; endmodule
module mon(input logic [3:0] s); initial #1 $display("T|%m=%0d",s); endmodule

module top; for(genvar g=0;g<2;g++) begin: bank mem m(); end bind bank[1].m mon b(.s(data)); initial #2 $finish; endmodule
"####,
        &["T|top.bank[1].m.b=7"],
    );
}

#[test]
fn bind_colon_relative() {
    check(
        r####"module mem; logic [3:0] data=7; endmodule
module mon(input logic [3:0] s); initial #1 $display("T|%m=%0d",s); endmodule

module top; mem m(), decoy(); bind mem : m mon b(.s(data)); initial #2 $finish; endmodule
"####,
        &["T|top.m.b=7"],
    );
}

#[test]
fn bind_colon_generated_relative() {
    check(
        r####"module mem; logic [3:0] data=7; endmodule
module mon(input logic [3:0] s); initial #1 $display("T|%m=%0d",s); endmodule

module top; for(genvar g=0;g<2;g++) begin: bank mem m(); end bind mem : bank[1].m mon b(.s(data)); initial #2 $finish; endmodule
"####,
        &["T|top.bank[1].m.b=7"],
    );
}

#[test]
fn bind_array_instance() {
    check(
        r####"module mem; logic [3:0] data=7; endmodule
module mon(input logic [3:0] s); initial #1 $display("T|%m=%0d",s); endmodule

module top; mem m[1:0](); bind top.m[1] mon b(.s(data)); initial #2 $finish; endmodule
"####,
        &["T|top.m[1].b=7"],
    );
}

#[test]
fn bind_root_prefix() {
    check(
        r####"module mem; logic [3:0] data=7; endmodule
module mon(input logic [3:0] s); initial #1 $display("T|%m=%0d",s); endmodule

module top; for(genvar g=0;g<2;g++) begin: bank mem m(); end bind $root.top.bank[1].m mon b(.s(data)); initial #2 $finish; endmodule
"####,
        &["T|top.bank[1].m.b=7"],
    );
}

#[test]
fn bind_interface_middle() {
    check(
        r####"interface child_if; logic [3:0] data=7; endinterface
interface watch_if(input logic [3:0] s); initial #1 $display("T|%m=%0d",s); endinterface
interface host_if; for(genvar g=0;g<2;g++) begin: bank child_if c(); end endinterface
module top; host_if h(); bind top.h.bank[1].c watch_if b(.s(data)); initial #2 $finish; endmodule
"####,
        &["T|top.h.bank[1].c.b=7"],
    );
}

#[test]
fn bind_unit_parameter() {
    check(
        r####"module mem; logic [3:0] data=7; endmodule
module mon(input logic [3:0] s); initial #1 $display("T|%m=%0d",s); endmodule
parameter int SEL=1;
module top; for(genvar g=0;g<2;g++) begin: bank mem m(); end bind top.bank[SEL].m mon b(.s(data)); initial #2 $finish; endmodule
"####,
        &["T|top.bank[1].m.b=7"],
    );
}

#[test]
fn bind_unit_function() {
    check(
        r####"module mem; logic [3:0] data=7; endmodule
module mon(input logic [3:0] s); initial #1 $display("T|%m=%0d",s); endmodule
function automatic int choose(); return 1; endfunction
module top; for(genvar g=0;g<2;g++) begin: bank mem m(); end bind top.bank[choose()].m mon b(.s(data)); initial #2 $finish; endmodule
"####,
        &["T|top.bank[1].m.b=7"],
    );
}

#[test]
fn bind_module_function() {
    check(
        r####"module mem; logic [3:0] data=7; endmodule
module mon(input logic [3:0] s); initial #1 $display("T|%m=%0d",s); endmodule

module top; function automatic int choose(); return 1; endfunction for(genvar g=0;g<2;g++) begin: bank mem m(); end bind top.bank[choose()].m mon b(.s(data)); initial #2 $finish; endmodule
"####,
        &["T|top.bank[1].m.b=7"],
    );
}

#[test]
fn bind_package_parameter() {
    check(
        r####"module mem; logic [3:0] data=7; endmodule
module mon(input logic [3:0] s); initial #1 $display("T|%m=%0d",s); endmodule
package cfg; parameter int SEL=1; endpackage
module top; for(genvar g=0;g<2;g++) begin: bank mem m(); end bind top.bank[cfg::SEL].m mon b(.s(data)); initial #2 $finish; endmodule
"####,
        &["T|top.bank[1].m.b=7"],
    );
}

#[test]
fn bind_lexical_parameter() {
    check(
        r####"module mem; logic [3:0] data=7; endmodule
module mon(input logic [3:0] s); initial #1 $display("T|%m=%0d",s); endmodule
module binder; localparam int SEL=1; bind top.bank[SEL].m mon b(.s(data)); endmodule
module top; localparam int SEL=0; for(genvar g=0;g<2;g++) begin: bank mem m(); end binder cfg(); initial #2 $finish; endmodule
"####,
        &["T|top.bank[1].m.b=7"],
    );
}

#[test]
fn bind_child_hier_parameter() {
    check(
        r####"module mem; logic [3:0] data=7; endmodule
module mon(input logic [3:0] s); initial #1 $display("T|%m=%0d",s); endmodule
module host; localparam int SEL=1; for(genvar g=0;g<2;g++) begin: bank mem m(); end endmodule
module top; host h(); bind top.h.bank[top.h.SEL].m mon b(.s(data)); initial #2 $finish; endmodule
"####,
        &["T|top.h.bank[1].m.b=7"],
    );
}

#[test]
fn bind_repeat_upward() {
    check(
        r####"module mem; logic [3:0] data=7; endmodule
module mon(input logic [3:0] s); initial #1 $display("T|%m=%0d",s); endmodule
module up; initial #1 $display("T|%m=%0d",mem.data); endmodule
module top; mem m(); bind top.m up a(); bind top.m up b(); initial #2 $finish; endmodule
"####,
        &["T|top.m.a=7", "T|top.m.b=7"],
    );
}

#[test]
fn bind_generate_localparam() {
    check(
        r####"module mem; logic [3:0] data=7; endmodule
module mon(input logic [3:0] s); initial #1 $display("T|%m=%0d",s); endmodule
module leaf #(parameter V=1); logic [3:0] data=V; endmodule
module top; for(genvar g=0;g<2;g++) begin: bank localparam int V=g+5; leaf#(V) m(); end bind top.bank[1].m mon b(.s(data)); initial #2 $finish; endmodule
"####,
        &["T|top.bank[1].m.b=6"],
    );
}

#[test]
fn bind_in_generate() {
    check(
        r####"module mem; logic [3:0] data=7; endmodule
module mon(input logic [3:0] s); initial #1 $display("T|%m=%0d",s); endmodule

module top; mem m(); if(1) begin: active bind mem mon b(.s(data)); end initial #2 $finish; endmodule
"####,
        &["T|top.m.b=7"],
    );
}

#[test]
fn bind_colon_absolute_control() {
    check(
        r####"module mem; logic [3:0] data=7; endmodule
module mon(input logic [3:0] s); initial #1 $display("T|%m=%0d",s); endmodule

module top; mem m(), decoy(); bind mem : top.m mon b(.s(data)); initial #2 $finish; endmodule
"####,
        &["T|top.m.b=7"],
    );
}

#[test]
fn bind_named_parameter_control() {
    check(
        r####"module mem; logic [3:0] data=7; endmodule
module mon(input logic [3:0] s); initial #1 $display("T|%m=%0d",s); endmodule
module host #(parameter N=1); for(genvar g=0;g<N;g++) begin: bank mem m(); end endmodule
module top; host#(.N(3)) h(); bind top.h.bank[2].m mon b(.s(data)); initial #2 $finish; endmodule
"####,
        &["T|top.h.bank[2].m.b=7"],
    );
}
