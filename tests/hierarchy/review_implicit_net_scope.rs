//! §6.10: an implicit net of one module does not declare the name in another
//! module; the reference simulator rejects the read in `other`.
use xezim::simulate;

#[test]
fn an_implicit_net_of_one_module_does_not_declare_the_name_in_another() {
    let src = r#"
module sub(input a);
  buf g (gate_net, a);
  wire own = gate_net;
endmodule
module other(input a);
  wire y = gate_net;
endmodule
module top;
  sub u(.a(1'b1));
  other o(.a(1'b1));
  initial #1 $display("y=%b", o.y);
endmodule
"#;
    let err = simulate(src, 1000)
        .err()
        .expect("gate_net is undeclared in `other`");
    assert!(err.contains("'gate_net'"), "{err}");
}
