/// §3.12.1, §26.3: an unimported enum cannot set a module bound.
#[test]
fn unit_parameter_over_hidden_enum() {
    let source = r#"
parameter RED = 0;
package p;
  typedef enum int { RED = 64 } color_t;
endpackage
module t;
  localparam int N = RED;
  logic [N:0] v;
  if (N == 0) initial $display("gen=0");
  else initial $display("gen=64");
  initial $display("N=%0d bits=%0d", N, $bits(v));
endmodule
"#;
    let sim = xezim::simulate(source, 100).expect("simulation failed");
    let messages: Vec<_> = sim.output.iter().map(|line| line.message.as_str()).collect();
    assert!(messages.contains(&"gen=0"), "{messages:?}");
    assert!(messages.contains(&"N=0 bits=1"), "{messages:?}");
}
