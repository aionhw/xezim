//! Issue #142: §10.6.2/§4 — when several nonblocking assignments to the same
//! variable execute in one evaluation, the LAST executed one determines the
//! final value. The main executor's `NbaAssignArray` arm ran its eval-time
//! elision against the REGISTER alone: a conditional override whose value
//! equaled the register's current value was dropped while the unconditional
//! default sat in the queue, so the default won. On the reporter's Intel
//! 4004, a taken branch-to-self (`ISZ` whose target is the instruction's own
//! address — `pc` frozen during the two-word fetch) committed `pc+2`.
//!
//! Every sibling arm (scalar NbaAssign, NbaAssignConst, the range forms, the
//! two-state stores) already checked the pending queue first; this arm and
//! the two isolated-executor twins did not. `nba_dup_targets` also never
//! counted array NBAs, so the twins' §10.4.2 scan could never arm.
//!
//! The shape needs all of: an ARRAY element target with a DYNAMIC index,
//! two writes in one evaluation, the second conditional and equal to the
//! current value — and enough surrounding block (a case, a second array) to
//! keep the block off the two-state path, whose stores were already correct.
//! Verilator 5.050 and a Yosys formal proof agree on the expectations.

use xezim::simulate;

fn msgs(src: &str) -> Vec<String> {
    simulate(src, 1000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect()
}

#[test]
fn conditional_override_equal_to_current_wins() {
    let out = msgs(
        r#"
module top;
  logic clk = 0;
  logic [11:0] stack [0:3];
  logic [3:0]  idx [0:15];
  logic [1:0] sp = 0;
  logic [3:0] opr = 4'h7, opa = 4'hd;
  logic cond = 0, instr_end = 0;
  logic [11:0] pc;
  logic [3:0] w2hi = 4'h8, w2lo = 4'h4;
  assign pc = stack[sp];
  wire [11:0] pc2 = pc + 12'd2;
  always #5 clk = ~clk;
  always @(posedge clk) begin
    if (instr_end) begin
      stack[sp] <= pc2;                                   // default: advance
      unique case (opr)
        4'h4: stack[sp] <= {opa, w2hi, w2lo};
        4'h7: begin
          idx[opa] <= idx[opa] + 4'd1;
          if (cond) stack[sp] <= {pc2[11:8], w2hi, w2lo}; // == CURRENT value
        end
        default: ;
      endcase
    end
  end
  initial begin
    for (int i = 0; i < 16; i++) idx[i] = 0;
    stack[0] = 12'h084; stack[1] = 0; stack[2] = 0; stack[3] = 0;
    cond = 1; instr_end = 1;
    #12 $display("A_%h_%h", stack[0], idx[13]);   // override wins: 084
    cond = 0;
    #10 $display("B_%h", stack[0]);               // default wins: 086
    $finish;
  end
endmodule
"#,
    );
    assert!(out.contains(&"A_084_1".to_string()), "{out:?}");
    assert!(out.contains(&"B_086".to_string()), "{out:?}");
}

#[test]
fn override_to_current_value_then_two_indices() {
    // The JMS shape: same element written twice, then a DIFFERENT dynamic
    // index — the pending-overwrite must key on the resolved ELEMENT, not
    // the array.
    let out = msgs(
        r#"
module top;
  logic clk = 0;
  logic [11:0] stack [0:3];
  logic [3:0] junk [0:3];
  logic [1:0] sp = 0;
  wire  [1:0] sp_next = sp + 2'd1;
  logic [11:0] pc;
  logic [1:0] mode = 0;
  assign pc = stack[sp];
  wire [11:0] pc2 = pc + 12'd2;
  always #5 clk = ~clk;
  always @(posedge clk) begin
    stack[sp] <= pc2;
    unique case (mode)
      2'd1: begin
        junk[sp] <= junk[sp] + 4'd1;
        stack[sp]      <= pc;              // == current: must still win
        stack[sp_next] <= 12'h123;         // different element
      end
      2'd2: begin
        junk[sp] <= junk[sp] + 4'd1;
        stack[sp] <= 12'h099;              // differing override (regression
      end                                  // guard for the plain case)
      default: ;
    endcase
  end
  initial begin
    for (int i = 0; i < 4; i++) junk[i] = 0;
    stack[0] = 12'h040; stack[1] = 0; stack[2] = 0; stack[3] = 0;
    mode = 2'd1;
    #12 $display("C_%h_%h", stack[0], stack[1]);  // 040 (kept), 123
    mode = 2'd2;
    #10 $display("D_%h", stack[0]);               // 099
    mode = 2'd0;
    #10 $display("E_%h", stack[0]);               // 09b (default pc+2)
    $finish;
  end
endmodule
"#,
    );
    assert!(out.contains(&"C_040_123".to_string()), "{out:?}");
    assert!(out.contains(&"D_099".to_string()), "{out:?}");
    assert!(out.contains(&"E_09b".to_string()), "{out:?}");
}
