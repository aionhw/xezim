//! §5.7.1 / §11.6.1: unbased-unsized literals (`'0`/`'1`/`'x`/`'z`) replicate
//! to the width of the consuming context. Previously they materialized as
//! fixed 32-bit values: `4'hf === '1` was false, an 82-bit `= '1` zero-
//! extended, and the tristate lowering's internal `'z` leaked as 1-bit z into
//! wire resolution (breaking tran z-propagation). An implicit-type parameter
//! takes the literal's SELF-DETERMINED 1-bit size (§6.20.2).

use xezim::simulate;

fn output_of(sim: &xezim::compiler::Simulator) -> String {
    sim.output
        .iter()
        .map(|o| o.message.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn fill_takes_context_width() {
    const SRC: &str = r#"
module top;
  wire [81:0] one = '1;
  wire [15:0] expr_add = 16'h0 + '1;
  initial begin
    #1;
    $display("EQ %b", 4'hf === '1);
    $display("WIDE %b", one === {82{1'b1}});
    $display("ADD %h", expr_add);
  end
endmodule
"#;
    let out = output_of(&simulate(SRC, 100).expect("sim"));
    assert!(out.contains("EQ 1"), "'1 must widen in ===:\n{}", out);
    assert!(out.contains("WIDE 1"), "'1 must fill 82 bits:\n{}", out);
    assert!(out.contains("ADD ffff"), "'1 must fill the add context:\n{}", out);
}

#[test]
fn implicit_parameter_is_self_determined_one_bit() {
    const SRC: &str = r#"
module top;
  parameter P = '1;
  wire [3:0] w = P;
  initial begin
    #1;
    $display("W %b", w);
    $display("NEQ %b", w !== 4'h1);
  end
endmodule
"#;
    let out = output_of(&simulate(SRC, 100).expect("sim"));
    assert!(out.contains("W 0001"), "P must be 1-bit self-determined:\n{}", out);
    assert!(out.contains("NEQ 0"), "w !== 4'h1 must be false:\n{}", out);
}

#[test]
fn ternary_fill_branch_and_tran_z() {
    const SRC: &str = r#"
module top;
  logic en; logic [3:0] d;
  wire [3:0] a, b;
  assign a = en ? d : 4'bzzzz;
  tran t (a, b);
  logic ok_z, ok_v;
  initial begin
    en = 0; d = 4'h0;
    #1 ok_z = (b === 4'bzzzz);
    en = 1; d = 4'hA;
    #1 ok_v = (b === 4'hA);
    $display("Z %b V %b", ok_z, ok_v);
  end
endmodule
"#;
    let out = output_of(&simulate(SRC, 100).expect("sim"));
    assert!(out.contains("Z 1 V 1"), "tran z/value propagation:\n{}", out);
}
