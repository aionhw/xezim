//! §15.5, §7.4: an element of a class event-array property (`event
//! evs[3];`) is an event of its own. `@(h.evs[1])`, `@h.evs[1]`,
//! `@(evs[i])` and `@(this.evs[i])` had no identity for the element and
//! woke at once; `-> evs[i]`, `->> evs[i]` and `evs[i].triggered` now share
//! one per-object, per-element key with the waits. Expected values are the
//! reference simulator's.

use xezim::simulate;

fn t_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000_000).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect()
}

#[test]
fn class_event_array_element_waits_and_triggers() {
    const SRC: &str = r#"
class m;
  event evs[3];
  string tag;
  function new(string t);
    tag = t;
  endfunction
  task fire(int i);
    -> evs[i];
  endtask
  task fire_nb(int i);
    ->> evs[i];
  endtask
  task listen(int i);
    forever begin
      @(evs[i]);
      $display("T|%s forever evs[%0d] t=%0t", tag, i, $time);
    end
  endtask
  task watch_this(int i);
    @(this.evs[i]);
    $display("T|%s this.evs[%0d] t=%0t", tag, i, $time);
  endtask
  task trig(int i);
    wait (evs[i].triggered);
    $display("T|%s triggered evs[%0d] t=%0t", tag, i, $time);
  endtask
endclass
module top;
  m a, b;
  initial begin
    a = new("a");
    b = new("b");
    fork
      a.listen(2);
      b.listen(2);
      a.trig(0);
      a.watch_this(1);
      begin @(a.evs[1]); $display("T|handle paren t=%0t", $time); end
      begin @a.evs[1]; $display("T|handle bare t=%0t", $time); end
      begin @(b.evs[1]); $display("T|b.evs[1] t=%0t", $time); end
    join_none
    #5 a.fire(2);
    #5 b.fire_nb(2);
    #5 -> a.evs[0];
    #5 -> a.evs[1];
    #0 $display("T|t=%0t a1=%0b a2=%0b b1=%0b", $time, a.evs[1].triggered, a.evs[2].triggered,
                b.evs[1].triggered);
    #1 $finish;
  end
endmodule
"#;
    // Waiters woken in one time slot run in no defined order (§4.7).
    let mut got = t_lines(SRC);
    got.sort();
    let mut want = vec![
        "T|a forever evs[2] t=5",
        "T|b forever evs[2] t=10",
        "T|a triggered evs[0] t=15",
        "T|a this.evs[1] t=20",
        "T|handle paren t=20",
        "T|handle bare t=20",
        "T|t=20 a1=1 a2=0 b1=0",
    ];
    want.sort();
    assert_eq!(got, want);
}
