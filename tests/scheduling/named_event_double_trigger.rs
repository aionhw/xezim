//! §15.5.1: triggering a named event wakes every process waiting on it at
//! that moment. A named event is backed by a 1-bit signal toggled per
//! trigger, and a waiter was judged later against the value it armed on:
//! two `-> e` from one process before the waiter ran toggled the signal
//! back and the wake was lost. A bounded buffer (a SystemC FIFO channel's
//! shape) deadlocked: the reader took two items, triggering `read_ev`
//! twice, and the blocked writer never resumed.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("top.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able (x/z?)", n))
}

const SRC: &str = r#"
module top;
  event write_ev, read_ev;
  byte q[$];
  int n, sum;
  task automatic t_write(byte c);
    if (q.size() == 2) @(read_ev);
    q.push_back(c);
    -> write_ev;
  endtask
  task automatic t_read(output byte c);
    if (q.size() == 0) @(write_ev);
    c = q.pop_front();
    -> read_ev;
  endtask
  initial begin t_write(1); t_write(2); t_write(3); t_write(4); t_write(5); t_write(6); end
  initial forever begin byte c; t_read(c); n++; sum += c; end
  initial #5 $finish;
endmodule
"#;

#[test]
fn double_trigger_wakes_waiter() {
    let sim = simulate(SRC, 1000).expect("simulate failed");
    assert_eq!(u(&sim, "n"), 6, "every item crosses the buffer");
    assert_eq!(u(&sim, "sum"), 21);
}
