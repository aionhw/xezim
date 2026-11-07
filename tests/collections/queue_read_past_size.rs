//! §7.9 / §7.10.4: an element at or past a queue's or dynamic array's
//! current SIZE does not exist, so reading it yields the element type's
//! default — x for a 4-state element, 0 for a 2-state one — at the element's
//! width. It must NOT be the cell that used to sit at that index, and the
//! read must not create an element.
//!
//! The store keeps an element cell after the queue shrinks: `delete()`,
//! `delete(i)`, `pop_front` and `pop_back` only move `.size`, they never
//! remove the cells the shrink orphaned. Three element-read funnels looked the
//! cell up without asking whether the index was still inside `size()`:
//!
//! * the AST funnel for a module-scope or block-local queue/dynamic array
//!   (which also had a "invalid index reads the element default" arm for FIXED
//!   arrays that deliberately skipped dynamic ones, because a queue is
//!   registered over a placeholder 0..63 range rather than its live length);
//! * the AST funnel for a class-member collection (`obj.q[i]` / `q[i]` in a
//!   method);
//! * the compiled class-method `LoadCollElem` exec arm.
//!
//! So a queue that had held 8 elements and now held 4 read `q[4]` and `q[5]`
//! as the OLD bytes 4 and 5 instead of x. A queue that only ever grew read
//! correctly, which is why the gap survived.
//!
//! Found through code that classified a packet by indexing an optional field
//! — `pkt.data[5]` after `pkt.data.delete()` had shrunk the queue from 8 to 4
//! — where the stale byte made the field look present and a write was treated
//! as a read.
//!
//! The module-scope block below was cross-checked with Icarus (`iverilog
//! -g2012`); Icarus rejects queues in classes, so the class-member block
//! relies on the expected output here rather than a cross-simulator diff.

use xezim::simulate;

const SRC: &str = r#"
class holder_c;
  logic [7:0] q[$];
  int n;
  integer g3, g4, g5;

  task automatic refill8_then4();
    q.delete();
    for (int i = 0; i < 8; i++) q.push_back(8'hA0 + i[7:0]);
    q.delete();
    for (int i = 0; i < 4; i++) q.push_back(8'hB0 + i[7:0]);
  endtask

  task automatic probe();
    refill8_then4();
    n  = q.size();
    g3 = q[3];                       // in range: 0xB3
    g4 = q[4];                       // x
    g5 = q[5];                       // x
  endtask
endclass

module top;
  logic [7:0] q[$];
  bit   [7:0] bq[$];
  integer g1, g2, g3;
  int n, nb, b1;

  task automatic probe_module();
    // (a) delete() then refill shorter: the old cells 4..7 remain orphaned.
    q.delete();
    for (int i = 0; i < 8; i++) q.push_back(8'hA0 + i[7:0]);
    q.delete();
    for (int i = 0; i < 4; i++) q.push_back(8'hB0 + i[7:0]);
    n  = q.size();
    g1 = q[3];                       // in range: 0xB3
    g2 = q[4];                       // x
    g3 = q[5];                       // x
  endtask

  task automatic probe_shrinks();
    // (b) every shrink shape leaves orphans; each read must still be x.
    q.delete();
    for (int i = 0; i < 8; i++) q.push_back(8'hA0 + i[7:0]);
    repeat (4) void'(q.pop_back());
    g1 = q[4];

    q.delete();
    for (int i = 0; i < 8; i++) q.push_back(8'hA0 + i[7:0]);
    repeat (4) void'(q.pop_front());
    g2 = q[4];

    q.delete();
    for (int i = 0; i < 8; i++) q.push_back(8'hA0 + i[7:0]);
    q.delete(0);
    for (int i = 0; i < 4; i++) void'(q.pop_back());
    g3 = q[4];
  endtask

  task automatic probe_two_state();
    // (c) a 2-state element defaults to 0, not x.
    bq.delete();
    for (int i = 0; i < 8; i++) bq.push_back(8'hA0 + i[7:0]);
    bq.delete();
    for (int i = 0; i < 4; i++) bq.push_back(8'hB0 + i[7:0]);
    nb = bq.size();
    b1 = bq[5];
  endtask

  // (d) an empty queue, and no element created by the read.
  task automatic probe_empty();
    q.delete();
    for (int i = 0; i < 8; i++) q.push_back(8'hA0 + i[7:0]);
    q.delete();
    n  = q.size();
    g1 = q[0];
  endtask
  // (e) a block-local queue takes the interpreter's per-call store.
  task automatic probe_local(output integer o);
    logic [7:0] lq[$];
    for (int i = 0; i < 8; i++) lq.push_back(8'hA0 + i[7:0]);
    lq.delete();
    for (int i = 0; i < 4; i++) lq.push_back(8'hB0 + i[7:0]);
    o = lq[5];
  endtask

  initial begin
    integer li;
    holder_c h;
    h = new();

    probe_module();
    $display("A %0d %0h %0b %0b", n, g1, (^g2) === 1'bx, (^g3) === 1'bx);

    probe_shrinks();
    $display("B %0b %0b %0b", (^g1) === 1'bx, (^g2) === 1'bx, (^g3) === 1'bx);

    probe_two_state();
    $display("C %0d %0h", nb, b1);

    probe_empty();
    $display("D %0d %0b %0d", n, (^g1) === 1'bx, q.size());

    probe_local(li);
    $display("E %0b", (^li) === 1'bx);

    h.probe();
    $display("F %0d %0h %0b %0b", h.n, h.g3, (^h.g4) === 1'bx, (^h.g5) === 1'bx);
    $finish;
  end
endmodule
"#;

#[test]
fn queue_read_past_size_is_the_element_default() {
    let sim = simulate(SRC, 10).expect("simulate failed");
    let out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    for want in [
        "A 4 b3 1 1",
        "B 1 1 1",
        "C 4 0",
        "D 0 1 0",
        "E 1",
        "F 4 b3 1 1",
    ] {
        assert!(
            out.iter().any(|l| l.trim() == want),
            "missing {want:?} in {out:?}"
        );
    }
}
