//! LRM §16.5 — a CONCURRENT assertion's pass/fail action block must actually
//! run at a clock fire. Previously the action / `else` action were tallied but
//! never executed, so a failing `assert property (...) else $error(...)` was
//! silent — a verification hole. Now the `else` runs on a fail, the pass action
//! on a non-vacuous pass, a cover action on a match, and a VACUOUS pass runs
//! nothing. Cross-checked against a reference simulator and a commercial simulator.

use xezim::simulate;

fn out(src: &str) -> String {
    let sim = simulate(src, 1000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect::<Vec<_>>().join("\n")
}

/// A failing property fires its `else` action every clock it fails.
#[test]
fn failing_property_fires_else_action() {
    let o = out(r#"
module t; logic clk=0, a=1, b=0; always #5 clk=~clk;
  ap: assert property (@(posedge clk) a |-> b) else $display("ELSE at %0t", $time);
  initial begin repeat(3) @(posedge clk); $finish; end
endmodule
"#);
    assert!(o.matches("ELSE at").count() >= 3,
        "failing a|->b must fire else each cycle; got: {}", o);
}

/// `req |=> ack` with ack arriving next cycle passes — the `else` stays silent.
#[test]
fn passing_implication_stays_silent() {
    let o = out(r#"
module t; logic clk=0, req=0, ack=0; always #5 clk=~clk;
  ap: assert property (@(posedge clk) req |=> ack) else $display("ELSE at %0t", $time);
  initial begin
    @(posedge clk) req<=1;
    @(posedge clk) begin req<=0; ack<=1; end
    @(posedge clk) ack<=0;
    repeat(2) @(posedge clk); $display("DONE"); $finish;
  end
endmodule
"#);
    assert!(o.contains("DONE") && !o.contains("ELSE at"),
        "passing implication must not fire else; got: {}", o);
}

/// A vacuous pass (false antecedent) must NOT run the pass action.
#[test]
fn vacuous_pass_runs_no_action() {
    let o = out(r#"
module t; logic clk=0, a=0, b=0; always #5 clk=~clk;
  ap: assert property (@(posedge clk) a |-> b) $display("PASSACT at %0t", $time);
  initial begin repeat(3) @(posedge clk); $display("DONE"); $finish; end
endmodule
"#);
    assert!(o.contains("DONE") && !o.contains("PASSACT"),
        "vacuous pass must not run the pass action; got: {}", o);
}

/// `cover property` runs its action on each match.
#[test]
fn cover_property_fires_on_match() {
    let o = out(r#"
module t; logic clk=0, a=1; always #5 clk=~clk;
  c: cover property (@(posedge clk) a) $display("COV at %0t", $time);
  initial begin repeat(3) @(posedge clk); $finish; end
endmodule
"#);
    assert!(o.matches("COV at").count() >= 3, "cover must fire on each match; got: {}", o);
}
