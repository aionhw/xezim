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

/// Reference: wide=zzzz1010 — the 4 unconnected high bits of the 8-bit input
/// float z; the connected low bits carry the actual.
#[test]
fn narrow_actual_on_wider_input_port_z_pads() {
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
    assert_eq!(line(&sim, "T|wide="), "T|wide=zzzz1010");
    assert_eq!(line(&sim, "T|no="), "T|no=1010");
}

/// Reference: z-fill applies to SIGNED actuals too (s=zzzz11111011 for -5,
/// u=zzzz10100101) — net connections never sign-extend.
#[test]
fn signed_narrow_actual_still_z_pads() {
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
    assert_eq!(line(&sim, "T|s="), "T|s=zzzz11111011");
}
