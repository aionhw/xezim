//! §6.17 / §15.5: an `event` declared in a block or an automatic task is a
//! real event: `@(e)` waits for its next `-> e`. Such an event has no
//! backing signal, and the wait fell through to the NBA-region yield, so
//! the process resumed at once. A `ref event` formal bound to a local
//! event waits on it too.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("top.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able (x/z?)", n))
}

const SRC: &str = r#"
`timescale 1ns/1ns
module top;
  int t_task = -1, t_block = -1, t_ref = -1, t_ref2 = -1;
  task automatic w(ref event e, output int t); @(e); t = $time; endtask
  task automatic main();
    event l1, l2;
    fork #5 -> l1; join_none
    @(l1);
    t_task = $time;
    fork w(l2, t_ref); join_none
    #3 -> l2;
  endtask
  initial main();
  initial begin : blk
    event l3;
    fork #7 -> l3; join_none
    @(l3);
    t_block = $time;
  end
  initial #20 $finish;
endmodule
"#;

#[test]
fn local_events_wait_for_their_trigger() {
    let sim = simulate(SRC, 1000).expect("simulate failed");
    assert_eq!(u(&sim, "t_task"), 5, "task-local event");
    assert_eq!(u(&sim, "t_block"), 7, "block-local event");
    assert_eq!(u(&sim, "t_ref"), 8, "ref event formal bound to a local event");
}
