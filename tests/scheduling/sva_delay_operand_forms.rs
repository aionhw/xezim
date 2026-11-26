//! IEEE 1800-2023 A.2.10: the operand of a cycle delay with no left operand
//! (`##N sequence_expr`) is any sequence_expr. A concatenation, an unbased
//! literal or `first_match(...)` right after the delay left a bare delay and
//! a parse error at the closing parenthesis. §16.13.1, §9.4.2: a clocking
//! event inside a sequence may be `@ identifier` (`a ##1 @e b`).
//!
//! The failure times of the three singly clocked assertions match the
//! reference simulator (the order of two action blocks in one time step is
//! not defined, so the lines are compared sorted). The `@e` cover only has
//! to parse and run: clock changes inside a sequence are accepted but not
//! modelled (see the §16.13 known gaps in `tests/lrm/ch16_assertions.rs`).

use xezim::simulate;

#[test]
fn delay_operand_forms_simulate() {
    let src = r#"
module t;
  logic clk = 0, a = 0, b = 0;
  event e;
  logic [7:0] av = 8'b0110_1110, bv = 8'b0101_1011;
  always #5 clk = ~clk;
  always @(posedge clk) -> e;
  a1: assert property (@(posedge clk) a |-> (##1 {a, b} == 2'b11))
    else $display("a1 fail %0t", $time);
  a2: assert property (@(posedge clk) a |-> (##1 first_match(b ##[0:1] !b)))
    else $display("a2 fail %0t", $time);
  a3: assert property (@(posedge clk) a |-> (##1 '1 ##1 b))
    else $display("a3 fail %0t", $time);
  c4: cover property (@(posedge clk) a ##1 @e b);
  initial begin
    for (int i = 7; i >= 0; i--) begin
      @(negedge clk) a = av[i]; b = bv[i];
    end
    @(negedge clk) a = 0; b = 0;
    #40 $display("done");
    $finish;
  end
endmodule
"#;
    let sim = simulate(src, 1000).expect("cycle delay operands must parse");
    let mut got: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .collect();
    got.sort();
    let mut want = vec![
        "a1 fail 35",
        "a2 fail 35",
        "a1 fail 45",
        "a2 fail 55",
        "a1 fail 65",
        "a2 fail 65",
        "a1 fail 85",
        "a2 fail 85",
        "a3 fail 95",
        "done",
    ];
    want.sort();
    assert_eq!(got, want, "{:?}", sim.output);
}
