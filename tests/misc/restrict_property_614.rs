//! IEEE 1800-2017 §16.14.4: `restrict property ( ... );` constrains a
//! formal tool's state space; a simulator reads it and checks nothing. It
//! was a parse error that lost the rest of the module (#614, reported by
//! git-hevangel). Labelled, split across lines and in a generate block, and
//! the items after the restrictions stay in the design.
use xezim::simulate;

#[test]
fn restrictions_are_read_and_not_checked() {
    let out: Vec<String> = simulate(r#"// IEEE 1800-2017 16.14.4: restrict_property_statement ::= restrict property ( property_spec ) ;
// A restriction constrains the state space of a formal verification tool. It is
// "not verified in simulation" and has no action block. It is a concurrent
// assertion statement (A.2.10), so it may carry a label and may stand at module
// level and in a generate block.
//
// The stimulus below violates every restriction on purpose. A simulator must
// accept the statements, ignore them, and keep everything around them: the
// continuous assignment, the checked assertion and the cover that follow.
//
// Every line should print PASS. With the bug the file does not compile
// (error: unexpected: "restrict"), and the module after the first restriction is lost.
module t;
  logic       clk = 0;
  logic [3:0] v   = 0;
  logic [3:0] twice;
  logic [3:0] twice_at_five;
  int         ctl_fails = 0;     // failures of the one real assertion below
  int         cov_hits  = 0;
  int         fails     = 0;
  bit         ran_to_end = 0;

  always #5 clk = ~clk;          // posedges at 5, 15, 25, ...

  // Constraints for a formal tool, violated by the stimulus.
  r_range:  restrict property (@(posedge clk) v inside {[1:3]});
  restrict property (@(posedge clk) disable iff (v == 0) v != 5);
  r_stable: restrict property (@(posedge clk) ##1 $stable(v));
  r_split:
    restrict
      property
      ( @(posedge clk)
        v != 6 )
      ;

  generate
    if (1) begin : g
      r_gen: restrict property (@(posedge clk) v == 0);
    end
  endgenerate

  // Items after the restrictions: they must still be in the design.
  assign twice = v << 1;
  a_ctl: assert property (@(posedge clk) v != 7) else ctl_fails++;
  c_three: cover property (@(posedge clk) v == 3) cov_hits++;

  task automatic check(string label, bit ok);
    if (ok) $display("PASS %s", label);
    else begin $display("FAIL %s", label); fails++; end
  endtask

  initial begin
    // v takes 0, 1, ..., 7, 0, 1 at successive posedges; set on negedges
    for (int k = 0; k < 10; k++) begin
      @(negedge clk);
      v = k % 8;
      if (v == 5) begin
        #1 twice_at_five = twice;
      end
    end
    @(negedge clk);
    ran_to_end = 1;

    check("simulation ran to the end",                ran_to_end);
    check("the assign after the restrictions is kept", twice_at_five == 4'd10);
    check("the assertion after them fails once, at v == 7", ctl_fails == 1);
    check("the cover after them matches once, at v == 3",   cov_hits == 1);
    if (fails == 0) $display("ALL PASS"); else $display("%0d FAILED", fails);
    $finish;
  end
endmodule
"#, 1000)
        .expect("simulate")
        .output
        .iter()
        .map(|l| l.message.clone())
        .filter(|m| m.contains("PASS") || m.contains("FAIL"))
        .collect();
    assert!(out.iter().any(|m| m == "ALL PASS"), "{out:?}");
    assert!(!out.iter().any(|m| m.contains("FAIL")), "{out:?}");
}
