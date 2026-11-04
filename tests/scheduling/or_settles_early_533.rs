//! Issue #533 (§16.12.6, §16.14.8): `p or q` holds as soon as either operand
//! holds, and it is nonvacuous if either operand's attempt is. An attempt
//! whose left operand held vacuously waited for the right operand to resolve,
//! so a strong obligation that never resolved (§16.12.2) left the attempt
//! open, and a sequence still matching at the end of the simulation dropped
//! it: neither ran its pass action. With `$assertvacuousoff` (§20.12) the
//! pass action runs once per nonvacuous success.
use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    simulate(src, 1000)
        .unwrap()
        .output
        .iter()
        .map(|o| o.message.clone())
        .filter(|s| s.starts_with("T_"))
        .collect()
}

#[test]
fn or_succeeds_beside_a_pending_strong_obligation() {
    let got = lines(
        r#"module top;
  logic clk = 0, a = 0, never = 0;
  int passes = 0, fails = 0, right_fails = 0;
  always #5 clk = ~clk;  // rising edges at 5, 15, ..., 65
  initial $assertvacuousoff;
  // `never |-> 1` holds vacuously at every edge. Where `a` is 1 the right
  // operand's antecedent matched, so the attempt is nonvacuous, although
  // its strong obligation never resolves.
  a_or: assert property (@(posedge clk) (never |-> 1) or (a |-> strong(##[1:$] never)))
    begin passes++; $display("T_or|P %0t", $time); end
    else fails++;
  // The right operand alone fails at the end once per match of `a`.
  a_right: assert property (@(posedge clk) a |-> strong(##[1:$] never))
    else right_fails++;
  // a at the edges: 0 1 0 1 1 0 0
  initial begin
    @(negedge clk) a = 1;
    @(negedge clk) a = 0;
    @(negedge clk) a = 1;
    @(negedge clk);
    @(negedge clk) a = 0;
    @(negedge clk);
    @(negedge clk) $finish;
  end
  final $display("T_tot|or %0d/%0d right fails %0d", passes, fails, right_fails);
endmodule"#,
    );
    assert_eq!(
        got,
        [
            "T_or|P 15",
            "T_or|P 35",
            "T_or|P 45",
            "T_tot|or 3/0 right fails 3",
        ]
    );
}

#[test]
fn or_with_a_sequence_operand_succeeds_when_the_other_holds() {
    let got = lines(
        r#"module top;
  logic clk = 0, a = 0, b = 0, never = 0;
  int passes = 0, fails = 0;
  always #5 clk = ~clk;  // rising edges at 5, 15, 25, 35
  initial $assertvacuousoff;
  // `never |-> b` holds vacuously at every edge, and an attempt of the
  // sequence `a ##2 b` is always nonvacuous, so every attempt is a
  // nonvacuous success at its first edge: the one from 15 does not wait for
  // `a ##2 b` to fail at 35, and the one from 35 is not dropped although
  // `a ##2 b` is still matching when the simulation ends.
  a_or: assert property (@(posedge clk) (never |-> b) or (a ##2 b))
    begin passes++; $display("T_or|P %0t", $time); end
    else fails++;
  // a at the edges: 0 1 0 1
  initial begin
    @(negedge clk) a = 1;
    @(negedge clk) a = 0;
    @(negedge clk) a = 1;
    @(negedge clk) $finish;
  end
  final $display("T_tot|or %0d/%0d", passes, fails);
endmodule"#,
    );
    assert_eq!(
        got,
        [
            "T_or|P 5",
            "T_or|P 15",
            "T_or|P 25",
            "T_or|P 35",
            "T_tot|or 4/0",
        ]
    );
}

#[test]
fn or_waits_for_an_operand_that_may_still_be_nonvacuous() {
    let got = lines(
        r#"module top;
  logic clk = 0, a = 1, c = 0, never = 0;
  int passes = 0, fails = 0;
  always #5 clk = ~clk;  // rising edges at 5, 15, 25, 35
  initial $assertvacuousoff;
  // At 5 the left operand holds vacuously while the right operand's
  // antecedent `a ##1 c` is still matching. It matches at 15, which makes
  // that attempt nonvacuous, so it succeeds there. The attempt from 25,
  // whose antecedent fails at 35, is vacuous.
  a_or: assert property (@(posedge clk) (never |-> 1) or ((a ##1 c) |-> 1))
    begin passes++; $display("T_or|P %0t", $time); end
    else fails++;
  // a at the edges: 1 0 1 0; c: 0 1 0 0
  initial begin
    @(negedge clk) begin a = 0; c = 1; end
    @(negedge clk) begin a = 1; c = 0; end
    @(negedge clk) a = 0;
    @(negedge clk) $finish;
  end
  final $display("T_tot|or %0d/%0d", passes, fails);
endmodule"#,
    );
    assert_eq!(got, ["T_or|P 15", "T_tot|or 1/0"]);
}
