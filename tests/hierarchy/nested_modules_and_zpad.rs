//! J-family hierarchy closures — reference-validated (task #24).
//!
//! §23.4 nested module declarations parse and hoist to the definitions map
//! (self-contained nested modules; enclosing-scope name access unmodeled).
//! §23.3.3 a NARROWER actual on a wider input NET port drives only the low
//! bits — the unconnected high bits read z, REGARDLESS of signedness (the
//! zero/sign-extension model belongs to assignments, not net connections).

use xezim::simulate;

fn line(sim: &xezim::compiler::Simulator, tag: &str) -> String {
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .find(|m| m.starts_with(tag))
        .unwrap_or_else(|| panic!("no output line starting with {}", tag))
}

fn has(sim: &xezim::compiler::Simulator, want: &str) -> bool {
    sim.output.iter().any(|o| o.message == want)
}

/// Reference: both bodies run (inner at t=0, outer at t=1).
#[test]
fn nested_module_declares_and_instantiates() {
    let src = r#"
module tb;
  module inner;
    initial $display("T|inner");
  endmodule
  inner i1();
  initial begin #1 $display("T|outer"); end
endmodule
"#;
    let sim = simulate(src, 10).expect("nested module must elaborate");
    assert!(has(&sim, "T|inner"), "nested body runs");
    assert!(has(&sim, "T|outer"), "enclosing body runs");
}

/// Doubly nested with a parameter override — reference: leaf V=9, mid, top.
#[test]
fn doubly_nested_module_with_parameter() {
    let src = r#"
module tb;
  module mid;
    module leaf #(parameter V = 3);
      initial $display("T|leaf V=%0d", V);
    endmodule
    leaf #(.V(9)) l1();
    initial $display("T|mid");
  endmodule
  mid m1();
  initial begin #1 $display("T|top"); end
endmodule
"#;
    let sim = simulate(src, 10).expect("doubly nested must elaborate");
    assert!(has(&sim, "T|leaf V=9"), "nested-nested param override applies");
    assert!(has(&sim, "T|mid"));
    assert!(has(&sim, "T|top"));
}

/// §23.3.3.6: a port connection is an implicit continuous assignment, so a
/// narrower actual on an INPUT port zero-extends to the formal width.
///
/// DELIBERATE reference divergence: the reference simulator leaves the
/// unconnected high bits z (wide=zzzz1010 measured, and this test used to pin
/// that), but the other major simulator zero-extends, and the LRM reading
/// favors extension. The z bits walked through a production DUT's CDC and
/// stalled its write path as X (the cv8s DRAM testbench), which settled the
/// choice. INOUT ports keep the z-fill — they are bidirectional.
#[test]
fn narrow_actual_on_wider_input_port_zero_extends() {
    let src = r#"
module child(input [7:0] wide, output [3:0] narrow_o);
  assign narrow_o = wide[3:0];
  initial #1 $display("T|wide=%b", wide);
endmodule
module tb;
  logic [3:0] sm = 4'b1010;
  wire  [7:0] big_o;
  child c(.wide(sm), .narrow_o(big_o[3:0]));
  initial begin #2 $display("T|no=%b", big_o[3:0]); end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate failed");
    assert_eq!(line(&sim, "T|wide="), "T|wide=00001010");
    assert_eq!(line(&sim, "T|no="), "T|no=1010");
}

/// §10.7 via §23.3.3.6: a SIGNED narrower actual sign-extends to the input
/// port width, like the RHS of any continuous assignment. (Same deliberate
/// reference divergence as above — the reference z-fills, s=zzzz11111011.)
#[test]
fn signed_narrow_actual_sign_extends() {
    let src = r#"
module cp_s(output wire logic signed [11:0] dst, input wire logic signed [11:0] src);
  assign dst = src;
endmodule
module tb;
  logic signed [7:0] s_src;
  wire logic signed [11:0] s_dst;
  cp_s cs(.dst(s_dst), .src(s_src));
  initial begin
    s_src = -8'sd5;
    #1 $display("T|s=%b", s_dst);
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate failed");
    assert_eq!(line(&sim, "T|s="), "T|s=111111111011");
}

/// The production shape that settled the §23.3.3.6 choice: a testbench tying
/// unused DUT input ports to narrow sized constants (`.vld_p1(1'b0)` on a
/// 2-bit port). The z upper bit crossed a CDC as X and stalled the DUT's
/// write path. Nine-case matrix from the report, xrun-validated.
#[test]
fn narrow_constants_and_vars_zero_extend_on_input_ports() {
    let src = r#"
module sink2 (output logic [3:0] o, input logic [3:0] p); assign o = p; endmodule
module sink1 (output logic o, input logic p);              assign o = p; endmodule
module tb;
  logic [3:0] a, b, c, d, e, f, g, h4;
  logic i1;
  logic [1:0] var1 = 2'b00;
  sink2 uA (.o(a),  .p(1'b0));    // 1-bit sized const
  sink2 uB (.o(b),  .p(2'b0));    // 2-bit sized const
  sink2 uC (.o(c),  .p(0));       // unsized
  sink2 uD (.o(d),  .p(1'b1));
  sink2 uE (.o(e),  .p(1));
  sink2 uF (.o(f),  .p(2'b10));
  sink2 uG (.o(g),  .p(var1));    // narrow VARIABLE
  sink2 uH (.o(h4), .p(4'd0));    // width-matched
  sink1 uI (.o(i1), .p(1'b0));
  initial begin
    #1 $display("T|m=%b%b%b%b%b%b%b%b%b", a, b, c, d, e, f, g, h4, i1);
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate failed");
    assert_eq!(
        line(&sim, "T|m="),
        "T|m=000000000000000100010010000000000"
    );
}

