//! §6.21, §15.5: a subroutine's local `event e;` is an event of its own.
//! It has no signal, so `@e` fell back to the wait for ordinary variables
//! and resumed in the same time step; two activations declaring `e` must
//! not wake each other. In a class task the local also shadows a class
//! event of the same name, which `@e`, `-> e` and `e.triggered` took
//! instead. Expected values are the reference simulator's.

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
fn local_event_waits_for_its_own_trigger() {
    const SRC: &str = r#"
class monitor;
  event e;
  task emit();
    #5 -> e;
  endtask
  task at_local();
    event e;
    fork
      begin #8 -> e; end
    join_none
    @e;
    $display("T|class @e t=%0t", $time);
  endtask
  task triggered_local();
    event e;
    fork
      begin #9 -> e; end
    join_none
    wait (e.triggered);
    $display("T|class e.triggered t=%0t", $time);
  endtask
endclass
module top;
  task automatic t1();
    event e;
    fork
      begin #8 -> e; end
    join_none
    @e;
    $display("T|module t1 t=%0t", $time);
  endtask
  task automatic t2();
    event e;
    fork
      begin #3 -> e; end
    join_none
    @(e);
    $display("T|module t2 t=%0t", $time);
  endtask
  initial begin
    monitor m = new();
    fork
      t1();
      t2();
      m.emit();
      m.at_local();
      m.triggered_local();
    join
    $finish;
  end
endmodule
"#;
    // Processes resumed in one time slot run in no defined order (§4.7).
    let mut got = t_lines(SRC);
    got.sort();
    let mut want = vec![
        "T|module t2 t=3",
        "T|module t1 t=8",
        "T|class @e t=8",
        "T|class e.triggered t=9",
    ];
    want.sort();
    assert_eq!(got, want);
}
