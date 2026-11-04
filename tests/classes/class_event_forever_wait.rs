//! §9.2.2, §15.5: a `forever` loop waiting on a class event wakes on every
//! trigger. A `forever` whose body waits runs on a fast path that turns the
//! event control into signal sensitivities; a class event has no signal, so
//! nothing resolved and the `@` ran straight through without waiting. A
//! `forever begin @ev; ... end` listener never woke (forked or not), and
//! `forever @ev stmt` woke only once; `while (1)` was unaffected. Expected
//! values are the reference simulator's.

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
fn forever_loop_wakes_on_every_class_event() {
    const SRC: &str = r#"
class listener;
  event ev;
  static event sev;
  string tag;
  function new(string tag);
    this.tag = tag;
  endfunction
  task start();
    fork
      forever begin
        @ev;
        $display("T|%s field t=%0t", tag, $time);
      end
      forever @(this.ev) $display("T|%s this t=%0t", tag, $time);
    join_none
  endtask
  task watch_static();
    forever begin
      @sev;
      $display("T|%s static t=%0t", tag, $time);
    end
  endtask
endclass
module top;
  listener a = new("a");
  listener b = new("b");
  initial begin
    a.start();
    b.start();
    fork
      a.watch_static();
      forever begin @(b.ev); $display("T|handle b t=%0t", $time); end
    join_none
    #5 -> a.ev;
    #5 -> b.ev;
    #5 -> listener::sev;
    #5 -> a.ev;
    #1 $finish;
  end
endmodule
"#;
    // Listeners woken in one time slot run in no defined order (§4.7).
    let mut got = t_lines(SRC);
    got.sort();
    let mut want = vec![
        "T|a field t=5",
        "T|a this t=5",
        "T|b field t=10",
        "T|b this t=10",
        "T|handle b t=10",
        "T|a static t=15",
        "T|a field t=20",
        "T|a this t=20",
    ];
    want.sort();
    assert_eq!(got, want);
}
