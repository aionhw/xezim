//! IEEE 1666-2023 §4.5.7: SystemC's unbounded `sc_start()` returns when no
//! events are left. The SystemC runtime package `sc_rt` lets sc_main wait
//! on `scrt_idle_ev`; when only blocked waiters remain, the scheduler
//! resumes that wait (at the current time) instead of ending the run, so
//! sc_main can go on and start the simulation again.

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
package sc_rt;
  event scrt_idle_ev;
endpackage
module tb;
  import sc_rt::*;
  event go;
  int runs = 0, t1 = -1, t2 = -1, done = 0;
  initial forever begin @(go); #3 runs++; end
  initial begin
    #2 -> go;
    @(scrt_idle_ev);
    t1 = $time;
    #4 -> go;
    @(scrt_idle_ev);
    t2 = $time;
    done = 1;
  end
endmodule
"#;

#[test]
fn idle_wait_resumes_when_only_waiters_remain() {
    let sim = simulate(SRC, 1000).expect("simulate failed");
    assert_eq!(u(&sim, "t1"), 5, "first return: after the last event");
    assert_eq!(u(&sim, "t2"), 12);
    assert_eq!(u(&sim, "runs"), 2);
    assert_eq!(u(&sim, "done"), 1);
}
