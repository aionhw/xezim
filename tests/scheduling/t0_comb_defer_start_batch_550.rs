//! #t0-comb-defer: cont-assign / port-connect propagation at time 0 stays
//! behind the WHOLE start batch.
//!
//! A commercial simulator queues the continuous-assign (and port-connect)
//! update events that a time-0 process write schedules AFTER every start
//! activation of the time-0 slot (active-region FIFO). A source-later
//! `always` with a blocking body therefore parks at its event control and
//! ARMS before the propagation fires, so it catches the t=0 settle of the
//! propagated signal. xezim drained the #35 deferred cont-assign entries at
//! the writing process's yield point — before the later starts had run — so
//! the edge fired while the receiver was not yet armed and was silently
//! lost (the port_handshake regression cluster: a blocking-body `always`
//! never entered its `wait` and the handshake stalled forever).
//!
//! Direct (unmediated) writes at t=0 are NOT caught by a source-later
//! receiver in the reference either (the receiver is not armed when the
//! write executes); that case is included as a guard against over-fixing.

use xezim::simulate;

fn lookup(sim: &xezim::compiler::Simulator, name: &str) -> u64 {
    sim.get_signal(name)
        .or_else(|| sim.get_signal(&format!("t.{}", name)))
        .or_else(|| sim.get_signal(&format!("tb.{}", name)))
        .unwrap_or_else(|| panic!("signal not found: {}", name))
        .to_u64()
        .unwrap_or_else(|| panic!("signal {} not u64-able", name))
}

/// p12b shape: `assign`-mediated t=0 write, source-later blocking-body
/// receiver. The t=0 X→0 settle of `p` must arm-then-fire the receiver
/// (reference: k=1; pre-fix xezim lost the t=0 edge and printed k=0).
const ASSIGN_BLOCKING: &str = r#"
`timescale 1ps/1ps
module t;
  logic a, p;
  int k = 0;
  initial begin a = 0; #100; a = 1; #100; a = 0; end
  assign p = a;
  always @(p iff (p === 0)) begin wait (p === 1); k++; end
  initial begin #1000; $finish; end
endmodule
"#;

/// minimize6 shape: the t=0 write crosses an output port connection into
/// the receiver's input port (the port connect keeps a propagation
/// cont-assign step). Same arm-then-fire requirement.
const PORT_CONNECT_BLOCKING: &str = r#"
`timescale 1ps/1ps
module drv0 (output logic p);
  initial begin p = 0; #100; p = 1; #100; p = 0; end
endmodule
module rcv (input logic p);
  int k = 0;
  always @(p iff (p === 0)) begin wait (p === 1); k++; end
endmodule
module tb;
  logic p0;
  drv0 d0 (.p(p0)); rcv x0 (.p(p0));
  initial begin #1000; $finish; end
endmodule
"#;

/// p12 shape (no-regression guard): `assign`-mediated t=0 write caught by a
/// source-later EDGE-BLOCK receiver (`always @(p) k++;`). This was
/// accidentally correct pre-fix (the eager arm caught the early drain); the
/// batch-end drain must keep catching it: t=0 X→0 and t=100 0→1 → k=2.
const ASSIGN_EDGE_BLOCK: &str = r#"
`timescale 1ps/1ps
module t;
  logic a, p;
  int k = 0;
  initial begin a = 0; #100; a = 1; end
  assign p = a;
  always @(p) k++;
  initial begin #1000; $finish; end
endmodule
"#;

/// p3 shape (over-fix guard): a DIRECT t=0 write (no assign, no port) is
/// NOT caught by a source-later blocking-body receiver — the receiver has
/// not armed when the write executes, and the reference behaves the same
/// (k=0: only the t=200 1→0 edge passes the iff guard, and the body then
/// parks in `wait (p === 1)` forever).
const DIRECT_BLOCKING: &str = r#"
`timescale 1ps/1ps
module t;
  logic p;
  int k = 0;
  initial begin p = 0; #100; p = 1; #100; p = 0; end
  always @(p iff (p === 0)) begin wait (p === 1); k++; end
  initial begin #1000; $finish; end
endmodule
"#;

#[test]
fn t0_assign_settle_caught_by_blocking_body_always() {
    let sim = simulate(ASSIGN_BLOCKING, 2000).expect("simulate failed");
    let k = lookup(&sim, "k");
    assert_eq!(
        k, 1,
        "the t=0 X->0 settle through `assign p = a` must arm-then-fire the \
         source-later blocking-body always (reference k=1; losing the t=0 \
         edge yields k=0 and stalls the handshake)"
    );
}

#[test]
fn t0_port_connect_settle_caught_by_blocking_body_always() {
    let sim = simulate(PORT_CONNECT_BLOCKING, 2000).expect("simulate failed");
    let k = lookup(&sim, "x0.k");
    assert_eq!(
        k, 1,
        "a t=0 write crossing an output port connect must propagate after \
         the whole start batch, arming the receiver's blocking-body always \
         first (reference k=1)"
    );
}

#[test]
fn t0_assign_settle_still_caught_by_edge_block() {
    let sim = simulate(ASSIGN_EDGE_BLOCK, 2000).expect("simulate failed");
    let k = lookup(&sim, "k");
    assert_eq!(
        k, 2,
        "the batch-end drain must not lose the t=0 edge for an edge-block \
         receiver (t=0 X->0 plus t=100 0->1 -> k=2)"
    );
}

#[test]
fn t0_direct_write_not_caught_by_source_later_blocking_body() {
    let sim = simulate(DIRECT_BLOCKING, 2000).expect("simulate failed");
    let k = lookup(&sim, "k");
    assert_eq!(
        k, 0,
        "a DIRECT t=0 write must stay uncaught by a source-later receiver \
         (reference k=0) — the fix must only defer cont-assign propagation"
    );
}
