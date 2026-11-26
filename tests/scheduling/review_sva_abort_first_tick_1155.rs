//! IEEE 1800-2023 §16.12.14, F.5.3.1: an abort condition that holds in time
//! step i settles the evaluation from the steps before i, continued as if
//! every Boolean held (`accept_on`) or none did (`reject_on`). At an
//! attempt's first tick nothing precedes it, and an implication holds when
//! nothing can match its antecedent, so `reject_on (rst) req |=> ack` does
//! not fail the attempts that start while `rst` holds. Every one of them
//! failed, so the assertion failed at each tick of a reset; the reference
//! simulator fails none of them either. An attempt whose antecedent matched
//! is rejected at the next tick `rst` holds; the reference simulator
//! rejects it at the tick it starts in, so both fail it.
use xezim::simulate;

/// Ticks at 5, 15, 25, ...; `rst` and `rst2` sample 1 at 5 and 15, `req2`
/// only at 5, `req` only at 45 and `ack` only at 55.
#[test]
fn an_abort_at_the_first_tick_spares_an_unmatched_implication() {
    let result = simulate(
        r#"
module t;
  initial $assertvacuousoff(0);
  logic clk = 0, rst = 1, req = 0, ack = 0, rst2 = 1, req2 = 1;
  always #5 clk = ~clk;
  initial begin #22 rst = 0; rst2 = 0; end
  initial begin #10 req2 = 0; end
  initial begin #40 req = 1; #10 req = 0; end
  initial begin #50 ack = 1; #10 ack = 0; end
  rr: assert property (@(posedge clk) reject_on (rst) req |=> ack)
      $display("rr P %0t", $time); else $display("rr F %0t", $time);
  r2: assert property (@(posedge clk) reject_on (rst2) req2 |=> ack)
      $display("r2 P %0t", $time); else $display("r2 F %0t", $time);
  ra: assert property (@(posedge clk) accept_on (rst) req |=> ack)
      $display("ra P %0t", $time); else $display("ra F %0t", $time);
  sr: assert property (@(posedge clk) sync_reject_on (rst) req |=> ack)
      $display("sr P %0t", $time); else $display("sr F %0t", $time);
  initial #70 $finish;
endmodule
"#,
        1_000,
    )
    .unwrap();
    let lines: Vec<&str> = result
        .output
        .iter()
        .map(|o| o.message.as_str())
        .filter(|l| l.contains(" P ") || l.contains(" F "))
        .collect();
    // r2: the attempt from 5 matched `req2` and is rejected at 15.
    assert_eq!(lines, ["r2 F 15", "rr P 55", "ra P 55", "sr P 55"]);
}
