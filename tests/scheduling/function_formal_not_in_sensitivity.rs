//! IEEE 1800-2017 §13.4, §10.3.2: a continuous assign that calls a function
//! is sensitive to the function's module-level reads, not to its formals or
//! block locals. A formal named like an unrelated design net
//! (`uj_imm(logic [31:0] instruction_i)` beside a decoder port `instruction_i`,
//! as in CVA6's instr_scan) leaked into the read set, so the assign re-ran on
//! every change of that net — and tripped the unobserved-port report when the
//! net was an elided input.

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.trim().to_string()).collect()
}

#[test]
fn formal_named_like_a_net_does_not_retrigger_the_caller() {
    let sim = simulate(
        r#"
module top;
  logic [31:0] instruction_i = 0;
  logic [31:0] instr = 32'h00000013;
  logic [63:0] imm;
  function automatic logic [63:0] uj_imm(logic [31:0] instruction_i);
    logic [63:0] tmp;
    $display("EVAL %h", instruction_i);
    tmp = {{44{instruction_i[31]}}, instruction_i[19:12], instruction_i[20], instruction_i[30:21], 1'b0};
    return tmp;
  endfunction
  assign imm = uj_imm(instr);
  initial begin
    #1 $display("A imm=%h", imm);
    instruction_i = 32'hffffffff; #1;
    instruction_i = 32'h12345678; #1;
    $display("B imm=%h", imm);
    instr = 32'h0040006f; #1;
    $display("C imm=%h", imm);
  end
endmodule
"#,
        10,
    )
    .expect("sim");
    let msgs = messages(&sim);
    let a = msgs.iter().position(|m| m.starts_with("A ")).expect("A");
    let b = msgs.iter().position(|m| m.starts_with("B ")).expect("B");
    let between: Vec<&String> = msgs[a..b].iter().filter(|m| m.starts_with("EVAL")).collect();
    assert!(between.is_empty(), "re-evaluated on an unrelated net: {msgs:?}");
    assert!(msgs.iter().any(|m| m == "C imm=0000000000000004"), "{msgs:?}");
}
