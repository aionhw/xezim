//! `always @(sig)` stayed dead after firing once from a write made inside an
//! edge continuation (a process resumed by `@(negedge clk)` that then wrote
//! `sig`): the dirty-driven edge scan took its pending-position list before
//! the scan, the position pushed DURING the scan landed on the fresh list,
//! and the wrapper overwrote that list with the emptied one while the
//! position's "seen" flag stayed set. Every later write to `sig`, from any
//! process, was then dropped.
use xezim::simulate;

fn messages(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn always_at_signal_survives_a_write_inside_an_edge_continuation() {
    let msgs = messages(
        "module tb;
  logic clk = 0; logic sig = 0; int hit = 0;
  always #5 clk = ~clk;
  always @(sig) hit++;
  initial begin
    #21; @(negedge clk);
    sig = 1; #2; sig = 0; #2; sig = 1; #2; sig = 0;
    #1; $display(\"P1 hit=%0d\", hit);
  end
  initial begin
    #101; sig = 1; #2; sig = 0; #2; sig = 1; #2; sig = 0;
    #1; $display(\"P2 hit=%0d\", hit);
    $finish;
  end
endmodule",
    );
    assert!(msgs.iter().any(|m| m == "P1 hit=4"), "{msgs:?}");
    assert!(msgs.iter().any(|m| m == "P2 hit=8"), "{msgs:?}");
}
