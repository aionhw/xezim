//! §6.21, §15.5: an `event` declared in a labeled block of two processes is
//! two events, stored under the blocks' hierarchical names (`a.e`, `b.e`).
//! A wait on such a dotted name was not parked on the event: it resumed in
//! the same time step, so `a`'s waiter woke at time 0 and `c`'s, which
//! nothing triggers, woke too. Expected values are the reference simulator's.

use xezim::simulate;

const SRC: &str = r#"
module tb;
  int woke_a = 0, woke_c = 0;
  initial begin : a
    event e;
    fork
      begin @(e); woke_a++; $display("T|a woke at %0t", $time); end
    join_none
    #20 -> e;
  end
  initial begin : b
    event e;
    #5 -> e;
    $display("T|b fired its own e at %0t", $time);
  end
  initial begin : c
    event e;
    @(e); woke_c++;
  end
  initial #30 $display("T|done a=%0d c=%0d", woke_a, woke_c);
endmodule
"#;

#[test]
fn labeled_block_events_wake_only_their_own_waiters() {
    let lines: Vec<String> = simulate(SRC, 100)
        .expect("simulate")
        .output
        .iter()
        .map(|l| l.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect();
    assert_eq!(
        lines,
        [
            "T|b fired its own e at 5",
            "T|a woke at 20",
            "T|done a=1 c=0",
        ]
    );
}
