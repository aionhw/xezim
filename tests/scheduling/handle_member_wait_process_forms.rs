//! §9.4.2 / §9.2.2: an event control on a class property or named event
//! reached through a handle (`@(h.val)`, `@(h.ev)`) written DIRECTLY in a
//! module process — a one-shot `initial`, a `forever` loop, an `always` —
//! waits like the same control inside a task does. These process forms
//! used to compile the wait onto the static-sensitivity paths (the process
//! FSM, the `forever` sensitivity cache, `always` sensitivity extraction),
//! which have no signal for `h.val`: the waits never fired (and the
//! `always` fired once at time 0). An NBA to the property is a value change
//! like any other write.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able (x/z?)", n))
}

const SRC: &str = r#"
`timescale 1ns/1ns
module tb;
  class box_t;
    logic [3:0] val = 'x;
    event ch;
    task write(logic [3:0] v); val <= v; -> ch; endtask
  endclass

  box_t s = new;
  int once_val = -1, once_ev = -1;
  int n_forever = 0, n_forever_ev = 0, n_always = 0, last_always = -1;

  initial begin #1 s.write(4'b0101); #1 s.write(4'b0110); #1 s.write(4'b0110); end
  initial begin @(s.val); once_val = $time; end
  initial begin @(s.ch); once_ev = $time; end
  initial forever begin @(s.val); n_forever++; end
  initial forever begin @(s.ch); n_forever_ev++; end
  always @(s.val) begin n_always++; last_always = $time; end
  initial #10 $finish;
endmodule
"#;

#[test]
fn handle_member_waits_in_module_processes() {
    let sim = simulate(SRC, 1000).expect("simulate failed");
    assert_eq!(u(&sim, "once_val"), 1, "one-shot @(h.val)");
    assert_eq!(u(&sim, "once_ev"), 1, "one-shot @(h.ev)");
    // The third write repeats the value: no change, no wake.
    assert_eq!(u(&sim, "n_forever"), 2, "forever @(h.val)");
    assert_eq!(u(&sim, "n_forever_ev"), 3, "forever @(h.ev) wakes on every ->");
    assert_eq!(u(&sim, "n_always"), 2, "always @(h.val): no time-0 fire");
    assert_eq!(u(&sim, "last_always"), 2);
}
