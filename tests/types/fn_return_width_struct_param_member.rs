//! IEEE 1800-2017 §13.4: a function's return type is sized by its declared
//! packed range. `logic [Cfg.VLEN-1:0]` with `Cfg` a packed-struct parameter
//! resolved to 2 bits inside a function (the same type on a port was 64):
//! the elaboration-side width resolver keys parameters by name and cannot
//! read a member out of the struct's packed value, so the branch-offset
//! immediates in a CVA6 instruction scanner were truncated to 2 bits.

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.trim().to_string()).collect()
}

#[test]
fn function_return_and_formal_sized_by_struct_param_member() {
    let sim = simulate(
        r#"
package cfgp;
  typedef struct packed { int unsigned VLEN; int unsigned XLEN; } cfg_t;
  localparam cfg_t C = '{VLEN: 64, XLEN: 64};
endpackage
module scan #(parameter cfgp::cfg_t Cfg = cfgp::C)
  (input logic [31:0] instr_i, output logic [Cfg.VLEN-1:0] imm_o, output logic [Cfg.VLEN-1:0] imm2_o);
  function automatic logic [Cfg.VLEN-1:0] sb_imm(logic [31:0] i);
    return {{51 + Cfg.VLEN - 64{i[31]}}, i[31], i[7], i[30:25], i[11:8], 1'b0};
  endfunction
  function automatic logic [Cfg.VLEN-1:0] uj_imm(logic [31:0] i);
    return {{44 + Cfg.VLEN - 64{i[31]}}, i[19:12], i[20], i[30:21], 1'b0};
  endfunction
  function automatic logic [Cfg.XLEN-1:0] widen(logic [Cfg.VLEN-1:0] v);
    return v;
  endfunction
  assign imm_o = instr_i[3] ? uj_imm(instr_i) : sb_imm(instr_i);
  always_comb imm2_o = widen(sb_imm(instr_i));
endmodule
module top;
  logic [31:0] instr = 32'hfe734ce3; logic [63:0] imm, imm2;
  scan u (.instr_i(instr), .imm_o(imm), .imm2_o(imm2));
  initial begin
    #1 $display("sb imm=%h imm2=%h", imm, imm2);
    instr = 32'h00130313;
    #1 $display("uj imm=%h imm2=%h", imm, imm2);
  end
endmodule
"#,
        10,
    )
    .expect("sim");
    let msgs = messages(&sim);
    for want in ["sb imm=fffffffffffffff8 imm2=fffffffffffffff8", "uj imm=0000000000000006 imm2=0000000000000006"] {
        assert!(msgs.iter().any(|m| m == want), "missing {want:?}: {msgs:?}");
    }
}
