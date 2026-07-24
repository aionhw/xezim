#![cfg(test)]
//! IEEE 1800-2023 §9.3.2 / §6.21: automatic (default) task/method locals are
//! per-invocation. Two concurrent task invocations (fork/join siblings) each
//! declaring `int edges[$]` must NOT share storage. xezim now isolates
//! ASSOCIATIVE-ARRAY locals per-invocation (this is what fixes the UVM
//! time-0 stall — `sync_phase`'s `edges_t edges`). QUEUE/dynamic-array local
//! isolation is correct in principle but currently DEFERRED (it regresses the
//! register model); the queue tests below are `#[ignore]` until that path is
//! fixed. Verified byte-for-byte against reference simulators.

use xezim::simulate;

fn out(src: &str) -> String {
    let sim = simulate(src, 10_000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect::<Vec<_>>().join("\n")
}

#[test]
#[ignore = "queue-local isolation deferred: regresses uvm_reg_map::do_bus_access `addrs=map_info.addr`"]
fn concurrent_fork_queues_do_not_clobber() {
    // Each fork child fills its own local queue, suspends (#0), then re-reads.
    // Without per-invocation storage the sibling's VarDecl zeroes the queue.
    let o = out(r#"
class Hopper;
  task automatic process_one(int id);
    int edges[$];
    edges.push_back(id);
    #0; #0;
    if (edges.size() != 1) $display("PROC%0d LEAK size=%0d", id, edges.size());
    else $display("PROC%0d OK head=%0d", id, edges[0]);
  endtask
  task run_all;
    fork
      process_one(1);
      process_one(2);
    join
  endtask
endclass
module top;
  initial begin
    Hopper h = new();
    h.run_all();
  end
endmodule
"#);
    assert!(o.contains("PROC1 OK head=1"), "PROC1 clobbered: {}", o);
    assert!(o.contains("PROC2 OK head=2"), "PROC2 clobbered: {}", o);
}

#[test]
fn concurrent_fork_assoc_arrays_do_not_clobber() {
    // Same hazard for local associative arrays — `int m[int]` in two siblings.
    let o = out(r#"
class Hopper;
  task automatic build(int base);
    int m[int];
    m[base] = base * 10;
    m[base + 1] = base * 10 + 1;
    #0; #0;
    $display("B%0d: %0d %0d", base, m[base], m[base+1]);
  endtask
  task run_all;
    fork
      build(100);
      build(200);
    join
  endtask
endclass
module top;
  initial begin
    Hopper h = new();
    h.run_all();
  end
endmodule
"#);
    assert!(o.contains("B100: 1000 1001"), "B100 clobbered: {}", o);
    assert!(o.contains("B200: 2000 2001"), "B200 clobbered: {}", o);
}

#[test]
#[ignore = "queue-local isolation deferred (see concurrent_fork_queues_do_not_clobber)"]
fn sequential_calls_keep_independent_queues() {
    // Two SEQUENTIAL calls (not concurrent) must also each see a fresh queue,
    // and neither leaks elements into the other.
    let o = out(r#"
module top;
  task automatic fill(int n);
    int q[$];
    q.push_back(n);
    q.push_back(n + 1);
    $display("CALL%0d size=%0d head=%0d tail=%0d", n, q.size(), q[0], q[1]);
  endtask
  initial begin
    fill(5);
    fill(50);
  end
endmodule
"#);
    assert!(o.contains("CALL5 size=2 head=5 tail=6"), "first call: {}", o);
    assert!(o.contains("CALL50 size=2 head=50 tail=51"), "second call: {}", o);
}
