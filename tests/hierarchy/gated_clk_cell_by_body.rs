//! A module named `gated_clk_cell` is a module like any other: its body
//! decides what its instances do. xezim-core replaced every instance of that
//! name with `assign clk_out = clk_in`, an optimization written for
//! OpenC910, whose cell is exactly that pass-through plus dead enable logic.
//! Wujian100 uses the name for a real latch-based clock gate, which then
//! never gated, and whose signals and sub-instances did not exist for
//! hierarchical references. The pass-through keeps the
//! direct assignment, but not when its clock net has a delay (§6.7.1,
//! §28.16): the direct assignment would drop it and every edge would arrive
//! early. Expected values follow the LRM; Verilator agrees
//! except that it does not apply the net delay.

fn lines(src: &str) -> Vec<String> {
    let sim = xezim::simulate(src, 1000).expect("simulate");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

const GATE: &str = r#"
// A real clock gate that happens to be named gated_clk_cell (Wujian100's
// shape: the enable is latched while the clock is low).
module gated_clk_cell(input clk_in, input en, output clk_out);
  reg en_l;
  always @(clk_in or en) if (!clk_in) en_l = en;
  assign clk_out = clk_in & en_l;
endmodule
module tb;
  reg clk = 0;
  always #5 clk = ~clk;
  reg en_hi = 1'b1;
  wire g0, g1;
  gated_clk_cell u0 (.clk_in(clk), .en(1'b0), .clk_out(g0));
  gated_clk_cell u1 (.clk_in(clk), .en(en_hi), .clk_out(g1));
  integer n0 = 0, n1 = 0;
  always @(posedge g0) n0 = n0 + 1;
  always @(posedge g1) n1 = n1 + 1;
  initial begin
    #100;
    $display("gate n0=%0d n1=%0d latch=%b", n0, n1, u1.en_l);
    $finish;
  end
endmodule
"#;

const PASSTHROUGH: &str = r#"
// The OpenC910 pass-through: clk_out follows clk_in whatever the enables.
module gated_clk_cell(clk_in, global_en, module_en, local_en, external_en,
                      pad_yy_icg_scan_en, clk_out);
  input  clk_in;
  input  global_en;
  input  module_en;
  input  local_en;
  input  external_en;
  input  pad_yy_icg_scan_en;
  output clk_out;
  wire   clk_en_bf_latch;
  wire   SE;
  assign clk_en_bf_latch = (global_en && (module_en || local_en)) || external_en;
  assign SE = pad_yy_icg_scan_en;
  assign clk_out = clk_in;
endmodule
module tb;
  reg clk = 0;
  always #5 clk = ~clk;
  wire g;
  gated_clk_cell u (.clk_in(clk), .global_en(1'b0), .module_en(1'b0), .local_en(1'b0),
                    .external_en(1'b0), .pad_yy_icg_scan_en(1'b0), .clk_out(g));
  integer n = 0;
  always @(posedge g) n = n + 1;
  initial begin
    #100;
    $display("passthrough n=%0d", n);
    $finish;
  end
endmodule
"#;

const DELAYED: &str = r#"
// The pass-through body, but its output net delays each edge by 3. The
// output drives a variable, so the port is an assignment from that net
// (§23.3.3) and the variable sees the delayed edge.
module gated_clk_cell(clk_in, en, clk_out);
  input  clk_in;
  input  en;
  output clk_out;
  wire #3 clk_out;
  assign clk_out = clk_in;
endmodule
module tb;
  reg clk = 0;
  always #5 clk = ~clk;
  logic g;
  gated_clk_cell u (.clk_in(clk), .en(1'b1), .clk_out(g));
  initial begin
    @(posedge g);
    $display("delayed first edge at %0t", $time);
    $finish;
  end
endmodule
"#;

#[test]
fn real_clock_gate_named_gated_clk_cell_gates() {
    let out = lines(GATE);
    assert!(
        out.iter().any(|l| l == "gate n0=0 n1=10 latch=1"),
        "the enable-low instance never ticks and its latch is visible: {out:?}"
    );
}

#[test]
fn passthrough_gated_clk_cell_follows_its_clock() {
    let out = lines(PASSTHROUGH);
    assert!(
        out.iter().any(|l| l == "passthrough n=10"),
        "the OpenC910 pass-through follows clk_in: {out:?}"
    );
}

#[test]
fn passthrough_with_a_net_delay_keeps_the_delay() {
    let out = lines(DELAYED);
    assert!(
        out.iter().any(|l| l == "delayed first edge at 8"),
        "the clock rises at 5 and its net delay adds 3: {out:?}"
    );
}

/// The pass-through body with a two-bit `clk_out` and a scalar `clk_in`: the
/// input port keeps one bit and the output zero-extends it (§23.3.3), where
/// a direct assignment would copy both bits of the connection.
const VECTOR_CLOCK: &str = r#"
module gated_clk_cell(clk_in, en, clk_out);
  input clk_in;
  input en;
  output [1:0] clk_out;
  wire en_q;
  assign en_q = en;
  assign clk_out = clk_in;
endmodule
module tb;
  reg [1:0] clk = 2'b11;
  wire [1:0] g;
  gated_clk_cell u (.clk_in(clk), .en(1'b1), .clk_out(g));
  initial begin
    #1 $display("vec 11->%b", g);
    clk = 2'b10;
    #1 $display("vec 10->%b", g);
    $finish;
  end
endmodule
"#;

#[test]
fn passthrough_with_a_vector_clock_output_extends() {
    let out = lines(VECTOR_CLOCK);
    let got: Vec<&str> = out
        .iter()
        .map(String::as_str)
        .filter(|l| l.starts_with("vec "))
        .collect();
    assert_eq!(got, ["vec 11->01", "vec 10->00"], "{out:?}");
}
