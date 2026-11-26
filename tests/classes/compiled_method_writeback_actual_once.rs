//! §13.5.2: a compiled method that passes a class member (a scalar or a
//! queue element) as an `output` or `inout` actual hands that call to the
//! interpreter, which copies the formal back into the member. The callee
//! runs once per call.
use xezim::simulate;

fn gate_on() -> bool {
    super::compiled_method_test_env::eager()
}

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("top.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} is x/z", n))
}

#[test]
fn member_output_and_inout_actuals_call_once() {
    if !gate_on() {
        return;
    }
    let src = r#"
module top;
  class counter;
    int m;
    int q[$];
    int calls;
    function void set(output int o);
      calls++;
      o = calls;
    endfunction
    function void add(inout int o);
      calls++;
      o = o + 10;
    endfunction
    function void run();
      q = '{0, 0};
      for (int i = 0; i < 3; i++) set(m);
      set(q[1]);
      add(m);
      add(q[0]);
    endfunction
  endclass
  counter c;
  int calls, m, q0, q1;
  initial begin
    c = new;
    c.run();
    calls = c.calls;
    m = c.m;
    q0 = c.q[0];
    q1 = c.q[1];
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulation should run");
    assert_eq!(u(&sim, "calls"), 6, "each call runs the callee once");
    assert_eq!(u(&sim, "m"), 13, "output then inout member scalar");
    assert_eq!(u(&sim, "q1"), 4, "output member queue element");
    assert_eq!(u(&sim, "q0"), 10, "inout member queue element");
}
