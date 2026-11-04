//! §15.5.3, §8.9: `ce.triggered` inside a method of the class declaring
//! event `ce` is `this.ce.triggered`. The bare name was looked up as a
//! module event, which never fires, so `wait(ce.triggered)` hung and a read
//! returned 0 in the slot `-> ce` fired in; `@ce` and the `this.`/handle
//! spellings were unaffected. Expected values are the reference simulator's.

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
fn bare_class_event_triggered_is_this_field() {
    const SRC: &str = r#"
class monitor;
  event ce;
  string tag;
  function new(string tag);
    this.tag = tag;
  endfunction
  task emit(int d);
    #d -> ce;
  endtask
  task sample();
    wait(ce.triggered);
    $display("T|%s wait t=%0t", tag, $time);
  endtask
  function bit fired();
    return ce.triggered;
  endfunction
endclass
module top;
  monitor a = new("a");
  monitor b = new("b");
  initial begin
    fork
      a.emit(5);
      b.emit(9);
      a.sample();
      b.sample();
      begin #5 #0 $display("T|t=%0t a=%0d b=%0d", $time, a.fired(), b.fired()); end
    join
    $finish;
  end
endmodule
"#;
    assert_eq!(
        t_lines(SRC),
        ["T|a wait t=5", "T|t=5 a=1 b=0", "T|b wait t=9"]
    );
}
