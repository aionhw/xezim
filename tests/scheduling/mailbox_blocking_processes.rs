//! Zero-delay producer/consumer over a mailbox — found by the cross-tool
//! benchmark suite (`bench/crosstool/b4_oop_tb.sv`), reference-validated.
//!
//! Two independent defects, both of which made `fork ... join` return while a
//! child was still mid-loop:
//!
//! 1. §12.7.1: a variable declared in a `for` init has AUTOMATIC lifetime, but
//!    with no call frame (an initial block or a fork child) it was stored in
//!    the GLOBAL signal map. Two concurrent processes each running
//!    `for (int i = 0; ...)` therefore shared one counter and corrupted each
//!    other's index — the consumer's `i` jumped 2 -> 7 and its loop exited
//!    early.
//! 2. `is_pid_suspended` listed `mailbox_get_waiters` but not
//!    `mailbox_put_waiters`/`semaphore_get_waiters`, so a producer parked on a
//!    FULL bounded mailbox looked FINISHED: its process context (and with it
//!    its loop state) was discarded, and an enclosing `join` completed.
//!
//! Plus: a `get` parking on an empty box now admits a producer parked on
//! "full" first, which otherwise deadlocked both sides.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able (x/z?)", n))
}

/// The same `for (int i ...)` in two concurrent processes must not share `i`.
#[test]
fn concurrent_for_loop_indices_are_independent() {
    let src = r#"
module tb;
  localparam int N = 200;
  int a = 0, b = 0;
  logic tick = 0;
  initial begin
    fork
      begin for (int i = 0; i < N; i++) begin @(posedge tick); a++; end end
      begin for (int i = 0; i < N; i++) begin @(posedge tick); b++; end end
    join
  end
  always #1 tick = ~tick;
endmodule
"#;
    let sim = simulate(src, 2000).expect("simulate failed");
    assert_eq!(u(&sim, "a"), 200, "each process needs its own loop counter");
    assert_eq!(u(&sim, "b"), 200);
}

/// Zero-delay producer/consumer across every mailbox bound, including the
/// unbounded case. Checksum matches the reference simulator.
#[test]
fn zero_delay_mailbox_producer_consumer() {
    for bound in [0usize, 2, 8, 64] {
        let src = format!(
            r#"
module tb;
  localparam int N = 300;
  mailbox #(int) mbx = new({bound});
  int produced = 0, consumed = 0, sum = 0;
  initial begin
    fork
      begin
        for (int i = 0; i < N; i++) begin
          mbx.put(i);
          produced++;
        end
      end
      begin
        for (int i = 0; i < N; i++) begin
          automatic int v;
          mbx.get(v);
          sum += v;
          consumed++;
        end
      end
    join
  end
endmodule
"#
        );
        let sim = simulate(&src, 100).unwrap_or_else(|e| panic!("bound {bound}: {e}"));
        assert_eq!(u(&sim, "produced"), 300, "bound {bound}: producer short");
        assert_eq!(u(&sim, "consumed"), 300, "bound {bound}: consumer short");
        // 0 + 1 + ... + 299
        assert_eq!(u(&sim, "sum"), 44850, "bound {bound}: wrong data");
    }
}
