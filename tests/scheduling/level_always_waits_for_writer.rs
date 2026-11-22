//! §4.5, §9.2.2.2, §9.4.2.2: a level-sensitive `always` block (`always_comb`,
//! `always @*`) is a process of its own. A blocking write in another process
//! only schedules it: it runs once that process suspends, after the
//! continuous assignments it reads have been updated, and once for all the
//! writes the process made in a row.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 1000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.trim().to_string()).collect()
}

const ADDER: &str = r#"
module add(input logic [3:0] a, b, output logic [4:0] s);
  assign s = a + b;
  always_comb begin
    if ($time > 0) $display("comb ran at %0t", $time);
    assert (s == a + b) else $display("comb failed at %0t", $time);
  end
  always @(*) begin
    if ($time > 0) $display("star ran at %0t", $time);
    assert (s == a + b) else $display("star failed at %0t", $time);
  end
endmodule

module tb;
  logic [3:0] a, b;
  logic [4:0] s;
  add u (.a, .b, .s);
  initial begin
    for (int i = 0; i < 20; i++) begin
      a = i;
      b = 3 * i;
      #1;
    end
    $finish;
  end
endmodule
"#;

/// The testbench writes `a`, then `b`, then waits. Run inline after each
/// write, the blocks saw `a` new with `b` and `s` old, then `a` and `b` new
/// with `s` old, and their assertions failed twice per step. Only
/// `always_comb`'s time-0 run (§9.2.2.2.2) can see `s == a + b` unknown.
#[test]
fn level_always_runs_after_the_writer_and_the_assignments() {
    let out = lines(ADDER);
    let failed: Vec<&String> = out.iter().filter(|l| l.contains("failed")).collect();
    // §9.2.2.2.2 puts the time-0 run after every procedure has started, but
    // does not order it against their first writes.
    assert!(failed.is_empty() || failed == ["comb failed at 0"], "{out:#?}");
    for kind in ["comb", "star"] {
        for t in 1..20 {
            let ran = out.iter().filter(|l| **l == format!("{kind} ran at {t}")).count();
            assert_eq!(ran, 1, "{kind} at {t}: {out:#?}");
        }
    }
}

const GATE_WOKEN: &str = r#"
module tb;
  logic a = 0, x = 0, g, y;
  buf (g, a);
  always_comb y = g ^ x;
  always @(x) $display("observer ran at %0t", $time);
  initial begin
    #1 a = 1;
    #0 $display("y=%b at %0t", y, $time);
    #1 a = 0;
    #0 $display("y=%b at %0t", y, $time);
    #1 $finish;
  end
endmodule
"#;

/// §9.4.2: an `always @(x)` runs only when `x` changes. The `always_comb`
/// that the gate's output `g` wakes (the writer changed only `a`) waits for
/// the writer, and at the yield only the signals that changed wake blocks
/// again: listing every read of the `always_comb` re-dirtied `x` too and ran
/// the observer.
#[test]
fn deferred_block_wakes_nothing_on_its_unchanged_reads() {
    let out = lines(GATE_WOKEN);
    assert!(!out.iter().any(|l| l.contains("observer ran")), "{out:#?}");
    assert!(out.iter().any(|l| l == "y=1 at 1"), "{out:#?}");
    assert!(out.iter().any(|l| l == "y=0 at 2"), "{out:#?}");
}
