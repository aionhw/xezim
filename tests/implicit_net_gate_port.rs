//! §6.10 implicit nets: an undeclared identifier used as a gate-instantiation
//! terminal or a submodule port connection is a net LOCAL to the module. When
//! such a module is inlined, every reference to that net must prefix to the
//! SAME net — otherwise the net splits (one copy driven, another read undriven)
//! and the value never propagates. Symptom: a gate-level cell whose clock is a
//! `buf` output read by inner instances never clocks (flops stay x). Fixed by
//! registering gate terminals / instance-port connections / specify delayed
//! nets in the module's local-name set before inlining.

use xezim::simulate;

#[test]
fn implicit_buf_output_read_by_inner_instances_propagates() {
    // `igx` is undeclared: driven by a buf, read by an inner instance's clk
    // port. `cvk` is undeclared: driven by an assign, read by the buf. All in a
    // submodule that gets inlined.
    let src = r#"
module inner(input clk, output reg q);
  always @(posedge clk) q <= 1;
endmodule
module mycell(output q, input CK);
  assign cvk = CK;
  buf ub(igx, cvk);
  inner ii(.clk(igx), .q(q));
endmodule
module tb;
  reg clk = 0; wire q;
  mycell c(.q(q), .CK(clk));
  initial begin
    #5 clk = 1;
    #5 if (q === 1'b1) $display("PROPAGATED");
    $finish;
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulate");
    let out: String = sim.output.iter().map(|o| o.message.clone()).collect();
    assert!(
        out.contains("PROPAGATED"),
        "implicit buf-output clock must reach the inner flop, got: {}",
        out
    );
}
