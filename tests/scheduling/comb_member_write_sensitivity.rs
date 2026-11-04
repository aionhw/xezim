//! IEEE 1800-2017 §9.2.2.2.1(b): an `always_comb` is not sensitive to what it
//! writes — at packed-struct MEMBER granularity, and at run time.
//!
//! Two gaps, found on CVA6's decoder (`decoder.sv`: one block writes
//! `instruction_o.{pc,op,…}` and reads back `instruction_o.op`; the exception
//! block writes `instruction_o.ex` in two steps and reads `instruction_o.pc`):
//!
//! 1. A member read records the base too (`decoded.op` AND `decoded`). With
//!    `decoded.op` written, the surviving `decoded` dependency re-armed the
//!    block on its own `decoded.op = 0; decoded.op = x;` glitch every settle
//!    pass. Now a base read only through self-written members is dropped
//!    (`self_written_read`) — the block evaluates ONCE, as the reference does.
//! 2. A block that reads another member of a struct it writes must keep the
//!    whole-signal dependency (that is the only signal carrying `s.b`), so the
//!    settle loop now refuses to re-arm an always_comb for a write it made
//!    itself (`trigger_deps!`). Another block's write still re-arms it.
//!
//! Before the fix every case here hit the settle limit (1000 passes) at time 0
//! and the CVA6 smoke stalled after its first instruction fetch.
//!
//! Not pinned here (pre-existing, follow-up): under the default eager
//! proc-write settle, a member-writing always_comb evaluates once MORE than the
//! reference after a procedural write to its input when another block writes a
//! sibling member (`runs` reads 3 where the reference prints 2;
//! `XEZIM_LAZY_PROC_SETTLE=1` gives 2). Values converge either way.

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.trim().to_string()).collect()
}

#[test]
fn self_written_member_does_not_rearm_the_block() {
    // Pins gap 1: exactly one evaluation at time 0, one per real input change.
    let sim = simulate(
        r#"
module top;
  typedef struct packed { logic op; logic ex; } decode_t;
  decode_t decoded;
  logic input_bit = 1;
  logic illegal, used;
  int runs = 0;
  always_comb begin
    runs = runs + 1;
    decoded.op = 0;
    decoded.op = input_bit;
    used = decoded.op;
    illegal = 0;
    illegal = input_bit;
    if (runs > 4) $fatal(1, "SELF_RETRIGGER");
  end
  always_comb begin
    decoded.ex = 0;
    decoded.ex = illegal;
  end
  initial begin
    #1 $display("FIRST %0d %b %b", runs, decoded, used);
    input_bit = 0;
    #1 $display("SECOND %b %b", decoded, used);
    $finish;
  end
endmodule
"#,
        20,
    )
    .expect("sim");
    assert_eq!(messages(&sim), ["FIRST 1 11 1", "SECOND 00 0"]);
}

#[test]
fn other_member_read_keeps_the_dependency() {
    // Pins gap 2: A writes `s.a`, reads `s.b` (written by B). A must follow
    // B's writes and must not loop on its own `s.a = 0; s.a = …` glitch.
    let sim = simulate(
        r#"
module top;
  typedef struct packed { logic [3:0] a; logic [3:0] b; } s_t;
  s_t s;
  logic [3:0] in1 = 4'd3, in2 = 4'd5;
  int runs_a = 0;
  always_comb begin
    runs_a = runs_a + 1;
    s.a = 4'd0;
    s.a = s.b + in1;
    if (runs_a > 8) $fatal(1, "SELF_RETRIGGER_A");
  end
  always_comb s.b = in2;
  initial begin
    #1 $display("A s.a=%0d s.b=%0d", s.a, s.b);
    in2 = 4'd7;
    #1 $display("B s.a=%0d s.b=%0d", s.a, s.b);
    $finish;
  end
endmodule
"#,
        20,
    )
    .expect("sim");
    assert_eq!(messages(&sim), ["A s.a=8 s.b=5", "B s.a=10 s.b=7"]);
}

#[test]
fn decoder_shaped_blocks_settle() {
    // The CVA6 shape: a decoder that reads back the member it writes, and an
    // exception block whose two-step member write sits beside a read of a
    // member the decoder owns.
    let sim = simulate(
        r#"
module top;
  typedef struct packed {
    logic [7:0] pc;
    logic [3:0] op;
    logic       ex_valid;
    logic [3:0] ex_cause;
  } sbe_t;
  sbe_t instr;
  logic [7:0] pc_i = 8'h10;
  logic [3:0] raw = 4'd3;
  logic illegal, ex_valid_i = 0;
  int runs1 = 0, runs2 = 0;
  always_comb begin : decoder
    runs1 = runs1 + 1;
    instr.pc = pc_i;
    instr.op = 4'd0;
    illegal = 1'b0;
    instr.op = raw;
    if (instr.op == 4'd3) illegal = 1'b1;
    if (runs1 > 6) $fatal(1, "DECODER_RETRIGGER");
  end
  always_comb begin : exception_handling
    runs2 = runs2 + 1;
    instr.ex_valid = ex_valid_i;
    instr.ex_cause = 4'd0;
    if (illegal) begin
      instr.ex_valid = 1'b1;
      instr.ex_cause = instr.pc[3:0];
    end
    if (runs2 > 6) $fatal(1, "EXC_RETRIGGER");
  end
  initial begin
    #1 $display("A pc=%h op=%0d v=%b c=%0d", instr.pc, instr.op, instr.ex_valid, instr.ex_cause);
    raw = 4'd5;
    pc_i = 8'h24;
    #1 $display("B pc=%h op=%0d v=%b c=%0d", instr.pc, instr.op, instr.ex_valid, instr.ex_cause);
    $finish;
  end
endmodule
"#,
        20,
    )
    .expect("sim");
    assert_eq!(
        messages(&sim),
        ["A pc=10 op=3 v=1 c=0", "B pc=24 op=5 v=0 c=0"]
    );
}
