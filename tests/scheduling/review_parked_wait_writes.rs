//! Review regressions of #802: a parked `wait` re-checks its condition when
//! an operand changes. A mailbox's `num()` changes on put and get, which
//! write no variable (§15.4.3), and a `static` local of a class method is
//! one cell shared by every call, so a write from another process changes
//! it (§6.21). Both waiters woke only when something unrelated was written.
//! Expected values are the reference simulator's.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    simulate(src, 1000)
        .expect("simulate")
        .output
        .iter()
        .map(|l| l.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect()
}

#[test]
fn a_wait_on_mailbox_occupancy_wakes_on_put_and_get() {
    let src = r#"
class item; int id; function new(int i); id = i; endfunction endclass
class consumer;
  mailbox #(item) mbx = new();
  task run();
    item it;
    wait (mbx.num() > 0);
    void'(mbx.try_get(it));
    $display("T|%0t got %0d", $time, it.id);
    wait (mbx.num() == 0);
    $display("T|%0t empty", $time);
  endtask
endclass
module top;
  consumer c;
  initial begin
    c = new;
    fork c.run(); join_none
    #5 begin item t = new(7); c.mbx.put(t); end
    #20 $display("T|%0t end", $time);
  end
endmodule
"#;
    assert_eq!(lines(src), ["T|5 got 7", "T|5 empty", "T|25 end"]);
}

#[test]
fn a_wait_on_a_static_method_local_wakes_on_another_process_write() {
    let src = r#"
module top;
  int nb = 0;
  class C;
    task t(input int id);
      static int cnt = 0;
      if (id == 0) begin
        wait (cnt == 1);
        $display("T|%0t woke nb=%0d", $time, nb);
      end else begin
        #5 cnt = 1;
        nb <= 5;
        forever #10;
      end
    endtask
  endclass
  C c = new;
  initial c.t(0);
  initial c.t(1);
  initial #60 $finish;
endmodule
"#;
    assert_eq!(lines(src), ["T|5 woke nb=0"]);
}
