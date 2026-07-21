// Regression test for nested-fork shared-object write/dispatch visibility
// (IEEE 1800-2023 §9.3.2 / §6.21).
//
// A fork child that inherits a captured local frame (because the fork body
// declares an `automatic` variable) must STILL be able to write to a SHARED
// object's instance properties and dispatch methods on it. Before the fix,
// the 2-segment-Ident write path and the method-dispatch path both used
//
//     if let Some(locals) = local_stack.last() { locals.get(obj) } else { signals }
//
// When the child's captured frame existed but did NOT contain `obj`,
// `locals.get(obj)` returned None and the `else` (signals/heap) branch was
// never taken — so the property write was silently dropped and the method
// call was never dispatched. This broke any nested-fork producer/consumer
// that touches a shared object (the UVM phase hopper's objection/queue model).
//
// Verified byte-for-byte against reference simulators.

use std::process::Command;

fn run(src: &str, tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!("xezim_forkfix_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{tag}.sv"));
    std::fs::write(&path, src).unwrap();
    let bin = env!("CARGO_BIN_EXE_xezim");
    let out = Command::new(bin)
        .arg("--simulate")
        .arg("-s")
        .arg("top")
        .arg(path.to_str().unwrap())
        .output()
        .expect("failed to run xezim");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

// The minimal trigger: a fork child with an `automatic` declaration (which
// captures a frame) then a property write + method call on a shared object.
// Both must persist to the object visible to the parent.
#[test]
fn nested_fork_child_writes_and_calls_on_shared_object() {
    let src = r#"class Box;
  int n;
  function void setn(int x);
    n = x;
  endfunction
endclass

module top;
  Box b;
  initial begin
    b = new();
    fork
      begin
        fork
          automatic int dummy = 0;   // captures a frame in the child
          begin
            #10;
            b.setn(7);                // method call on shared object
            b.n = b.n + 1;            // property write on shared object
            $display("child: b.n=%0d", b.n);
          end
        join_none
      end
    join_none
    #30;
    if (b.n == 8) $display("PASS shared-write-dispatch");
    else $display("FAIL b.n=%0d (expected 8)", b.n);
  end
endmodule
"#;
    let out = run(src, "shared_write");
    assert!(
        out.contains("PASS shared-write-dispatch") && !out.contains("FAIL"),
        "nested-fork shared-object write/dispatch failed.\n{out}"
    );
}

// The UVM-phase-hopper pattern distilled: a forked forever-loop consumer that
// blocks on `wait(q.size()!=0)`, forks a worker per item, and the worker pushes
// the successor back onto a shared queue. The successor chain (1→2→3) must
// complete — it requires nested-fork writes to a shared object to persist.
#[test]
fn nested_fork_hopper_successor_chain() {
    let src = r#"class Hopper;
  int q[$];
  int done_count;
  task automatic get(output int x);
    wait (q.size() != 0);
    x = q.pop_front();
  endtask
  function void push(int x);
    q.push_back(x);
  endfunction
endclass

module top;
  Hopper h;
  int seq_order[$];
  initial begin
    h = new();
    fork
      forever begin
        int ph;
        h.get(ph);
        fork
          automatic int p = ph;
          begin
            #10;
            seq_order.push_back(p);
            h.done_count++;
            if (p < 3) h.push(p + 1);   // schedule successor
          end
        join_none
      end
    join_none
    h.push(1);
    wait (h.done_count == 3);
    #1;
    $display("order: %p", seq_order);
    if (seq_order.size() == 3 && seq_order[0]==1 && seq_order[1]==2 && seq_order[2]==3)
      $display("PASS hopper-chain");
    else
      $display("FAIL hopper-chain order=%p", seq_order);
  end
endmodule
"#;
    let out = run(src, "hopper_chain");
    assert!(
        out.contains("PASS hopper-chain") && !out.contains("FAIL"),
        "nested-fork hopper successor chain failed.\n{out}"
    );
}
