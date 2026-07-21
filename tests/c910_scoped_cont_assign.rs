use xezim::simulate;

const SRC: &str = r#"
`timescale 1ns/1ps

module data_leaf #(parameter [63:0] INIT = 0) (output reg [63:0] data);
  initial begin
    data = INIT;
    #5 data = INIT + 64'd1;
  end
endmodule

module core #(parameter [63:0] INIT = 0) ();
  data_leaf #(.INIT(INIT)) x_sub();
endmodule

module wrapper #(parameter [63:0] INIT = 0) (output wire [63:0] observed);
  core #(.INIT(INIT)) x_core();
  // C910 retains this RHS as a relative multi-segment path after inlining,
  // while the LHS is an absolute dotted name stored in one Ident segment.
  assign observed = x_core.x_sub.data[63:0];
endmodule

module tb;
  wire [63:0] observed0;
  wire [63:0] observed1;
  reg pass = 0;

  wrapper #(.INIT(64'd10)) u0(.observed(observed0));
  wrapper #(.INIT(64'd20)) u1(.observed(observed1));

  initial begin
    #10;
    pass = observed0 == 64'd11 && observed1 == 64'd21;
    $finish;
  end
endmodule
"#;

#[test]
fn relative_hierarchical_cont_assign_is_scoped_and_retriggered() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    let pass = sim
        .get_signal("pass")
        .or_else(|| sim.get_signal("tb.pass"))
        .expect("pass signal missing")
        .to_u64()
        .expect("pass is X/Z");
    assert_eq!(pass, 1);
}
