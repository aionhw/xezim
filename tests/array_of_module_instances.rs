//! §23.3.2 arrays of module instances: `m[N:0] (vec[N:0], scalar, ...)` must
//! expand into N instances with vector ports bit-distributed (element k gets
//! bit k) and scalar ports broadcast. Previously the `[N:0]` was ignored, a
//! whole vector landed on a scalar port, and nothing connected — arrayed flop
//! banks (a common config/PLL idiom) silently did nothing.

use xezim::simulate;

fn out_of(sim: &xezim::compiler::Simulator) -> String {
    sim.output.iter().map(|o| o.message.as_str()).collect::<Vec<_>>().join("\n")
}

#[test]
fn comb_array_bit_distributes() {
    const SRC: &str = r#"
module cmb(output logic q, input logic d); assign q = d; endmodule
module top;
  logic [3:0] out, din;
  cmb m[3:0] (out[3:0], din[3:0]);
  initial begin din = 4'hA; #1 $display("R %h", out); end
endmodule
"#;
    let out = out_of(&simulate(SRC, 100).expect("sim"));
    assert!(out.contains("R a"), "each element drives out[k]=din[k]:\n{}", out);
}

#[test]
fn ff_array_with_scalar_clock_broadcast() {
    const SRC: &str = r#"
module dff(output logic q, input logic clk, input logic d);
  always_ff @(posedge clk) q <= d;
endmodule
module top;
  logic clk; logic [3:0] out, din;
  dff m[3:0] (out[3:0], clk, din[3:0]);   // clk broadcast, out/din distribute
  initial begin
    clk = 0; din = 4'hC;
    #1 clk = 1; #1 clk = 0;
    #1 $display("F %h", out);
  end
endmodule
"#;
    let out = out_of(&simulate(SRC, 100).expect("sim"));
    assert!(out.contains("F c"), "flop bank latches din per bit (want F c):\n{}", out);
}

/// Non-zero-based / offset range: `m[3:1]` connected to `out[3:1]` must map
/// element to the ABSOLUTE bit (out[1..3]), not a nested select out[3:1][k].
#[test]
fn array_offset_range_absolute_bit() {
    const SRC: &str = r#"
module cmb(output logic q, input logic d); assign q = d; endmodule
module top;
  logic [3:0] out, din;
  cmb a0 (out[0], din[0]);
  cmb m[3:1] (out[3:1], din[3:1]);
  initial begin din = 4'hA; #1 $display("O %b", out); end
endmodule
"#;
    let out = out_of(&simulate(SRC, 100).expect("sim"));
    assert!(out.contains("O 1010"), "offset-range array drives absolute bits:\n{}", out);
}

/// §23.3.2 W-bit formal: an actual W*N wide is split into per-element W-bit
/// slices. Here W=2, N=4, actual 8 bits: element k gets out[2k+1:2k].
#[test]
fn wbit_formal_per_slice() {
    const SRC: &str = r#"
module inv2(output logic [1:0] o, input logic [1:0] i); assign o = ~i; endmodule
module top;
  logic [7:0] out, din;
  inv2 m[3:0] (.o(out), .i(din));
  initial begin din = 8'hA5; #1 $display("W %b", out); end
endmodule
"#;
    let out = out_of(&simulate(SRC, 100).expect("sim"));
    // ~8'hA5 = ~10100101 = 01011010, distributed per 2-bit slice.
    assert!(out.contains("W 01011010"), "2-bit formal splits 8-bit actual per slice:\n{}", out);
}

/// §23.3.2 replication: an actual whose width EQUALS the formal width is NOT
/// split — the same actual connects to every element. A 1-bit scalar input
/// broadcasts to all four inverters, so every output bit is ~in.
#[test]
fn scalar_actual_replicates() {
    const SRC: &str = r#"
module inv1(output logic o, input logic i); assign o = ~i; endmodule
module top;
  logic [3:0] out;
  logic in;
  inv1 m[3:0] (out[3:0], in);
  initial begin in = 1'b1; #1 $display("C %b", out); end
endmodule
"#;
    let out = out_of(&simulate(SRC, 100).expect("sim"));
    assert!(out.contains("C 0000"), "1-bit actual broadcasts to every element:\n{}", out);
}

/// §23.3.2 by-name connections may be written in ANY order and must still bind
/// each vector actual against its OWN formal port's width. With ports of
/// DIFFERENT widths (o: 2-bit, i: 1-bit) listed in reverse, a position-based
/// width lookup would slice each actual with the wrong stride.
#[test]
fn named_conn_reordered_binds_by_port() {
    const SRC: &str = r#"
module mix(output logic [1:0] o, input logic i); assign o = {i, ~i}; endmodule
module top;
  logic [7:0] o8;
  logic [3:0] i4;
  mix m[3:0] (.i(i4), .o(o8));   // reversed vs declaration order
  initial begin i4 = 4'b1010; #1 $display("N %b", o8); end
endmodule
"#;
    let out = out_of(&simulate(SRC, 100).expect("sim"));
    // element k: {i4[k], ~i4[k]}; i4=1010 -> k0=01,k1=10,k2=01,k3=10 -> 10011001
    assert!(out.contains("N 10011001"), "by-name conns slice by their own port width:\n{}", out);
}

/// §23.3.2 hierarchical read: an internal net of an array element is reachable
/// as `m[k].<net>`.
#[test]
fn element_internal_hier_read() {
    const SRC: &str = r#"
module mycell(output logic o, input logic i);
  logic internal; assign internal = ~i; assign o = internal;
endmodule
module top;
  logic [3:0] out, din;
  mycell m[3:0] (.o(out), .i(din));
  initial begin din = 4'b1100; #1 $display("H %b", m[2].internal); end
endmodule
"#;
    let out = out_of(&simulate(SRC, 100).expect("sim"));
    // din[2]=1 -> internal of element 2 = ~1 = 0
    assert!(out.contains("H 0"), "m[k].internal is readable:\n{}", out);
}
