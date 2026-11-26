//! IEEE 1800-2023 §16.12.14: nested abort operators are evaluated in lexical
//! order. "If b becomes true before a and during the evaluation of p1, then
//! p fails" for `accept_on(a) reject_on(b) p1`: the inner abort ended the
//! evaluation of the outer operand, so a later `a` no longer accepts it.
//! When both hold in the same time step, the outer operator wins. An
//! asynchronous abort between ticks is reported at the next tick, and an
//! accepted attempt is a vacuous success (§16.14.8 ab). Both fixtures call
//! `$assertvacuousoff` to isolate abort ordering from vacuous pass actions.
use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    let result = simulate(src, 1_000).unwrap();
    result
        .output
        .iter()
        .map(|entry| entry.message.clone())
        .filter(|l| l.contains(" P ") || l.contains(" F "))
        .collect()
}

/// Ticks at 5, 15, 25 and 35; `start` holds only at the tick at 5.
#[test]
fn an_inner_abort_before_an_outer_one_decides_the_attempt() {
    let out = lines(
        r#"
module t;
  initial $assertvacuousoff(0);
  logic clk = 0, start = 1, a = 0, b = 0, a2 = 0, b2 = 0, a3 = 0, b3 = 0, a4 = 0, b4 = 0;
  always #5 clk = ~clk;
  initial begin #10 start = 0; end
  initial begin #7 b = 1; #1 b = 0; #3 a = 1; #1 a = 0; end
  initial begin #7 a2 = 1; #1 a2 = 0; #3 b2 = 1; #1 b2 = 0; end
  initial begin #7 a3 = 1; b3 = 1; #1 a3 = 0; b3 = 0; end
  initial begin #7 b4 = 1; #1 b4 = 0; #6 a4 = 1; #2 a4 = 0; end
  n1: assert property (@(posedge clk) start |-> accept_on (a) reject_on (b) ##3 1)
      $display("n1 P %0t", $time); else $display("n1 F %0t", $time);
  n2: assert property (@(posedge clk) start |-> accept_on (a2) reject_on (b2) ##3 1)
      $display("n2 P %0t", $time); else $display("n2 F %0t", $time);
  n3: assert property (@(posedge clk) start |-> accept_on (a3) reject_on (b3) ##3 1)
      $display("n3 P %0t", $time); else $display("n3 F %0t", $time);
  n4: assert property (@(posedge clk) start |-> accept_on (a4) reject_on (b4) ##3 1)
      $display("n4 P %0t", $time); else $display("n4 F %0t", $time);
  initial #40 $finish;
endmodule
"#,
    );
    // n1: `b` at 8, then `a` at 12. n2: `a` first. n3: both at 8. n4: `b`
    // at 8, then `a` sampled at the tick at 15.
    assert_eq!(out, ["n1 F 15", "n4 F 15"]);
}

/// A `not` of an accepted attempt fails, and an abort in one operand of an
/// `and` settles only that operand (§16.12.14 examples).
#[test]
fn not_and_and_of_aborts() {
    let out = lines(
        r#"
module t;
  initial $assertvacuousoff(0);
  logic clk = 0, start = 1, r = 0, s = 0;
  always #5 clk = ~clk;
  initial begin #10 start = 0; end
  initial begin #12 r = 1; #1 r = 0; end
  ng: assert property (@(posedge clk) start |-> not (accept_on (r) ##2 1))
      $display("ng P %0t", $time); else $display("ng F %0t", $time);
  an: assert property (@(posedge clk) start |-> (accept_on (r) ##2 0) and (reject_on (s) ##2 1))
      $display("an P %0t", $time); else $display("an F %0t", $time);
  ac: assert property (@(posedge clk) start |-> accept_on (r) ##2 0)
      $display("ac P %0t", $time); else $display("ac F %0t", $time);
  initial #40 $finish;
endmodule
"#,
    );
    assert_eq!(out, ["ng F 15", "an P 25"]);
}
