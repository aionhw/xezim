//! §7.10.2.3 / §7.4: `q[i] = v` on a QUEUE with `i >= size` APPENDS — the
//! size becomes `i+1`. A queue filled element-wise (`for i in 0..n q[i] = …`,
//! exactly how UVM's `uvm_unpack_queueN` restores an empty queue) must report
//! the right `.size()` afterward; without this the unpacked object's size read
//! 0 and every later field desynced (PCKSZ errors / X compare).

use xezim::simulate;

fn sim_src(src: &str) -> Vec<String> {
    let sim = simulate(src, 200).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

const QUEUE_INDEXED_GROW: &str = r#"
module top;
  class C;
    byte q[$];
    function void grow();
      for (int i = 0; i < 3; i++) q[i] = i + 10;
    endfunction
    function void show();
      $display("SIZE %0d q2 %0d", q.size(), q[2]);
    endfunction
  endclass
  C c;
  initial begin
    c = new;
    c.grow();
    c.show();
    if (c.q.size() == 3 && c.q[0] == 10 && c.q[2] == 12)
      $display("TAG_PASS"); else $display("TAG_FAIL size=%0d", c.q.size());
  end
endmodule
"#;

#[test]
fn queue_indexed_assign_auto_grows_size() {
    let msgs = sim_src(QUEUE_INDEXED_GROW);
    assert!(
        msgs.iter().any(|m| m == "SIZE 3 q2 12"),
        "expected q indexed writes to grow size to 3, got: {:?}",
        msgs
    );
    assert!(
        msgs.iter().any(|m| m == "TAG_PASS"),
        "expected queue index auto-grow, got: {:?}",
        msgs
    );
}