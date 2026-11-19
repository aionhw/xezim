//! §13.5.2: a call that a compiled method hands to the interpreter, because
//! an `output`, `inout` or `ref` actual is a class member, runs once in
//! every statement form: under `void'()`, as a task enable, through a
//! static method, another handle or `super`, and with an array element.
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
fn member_actual_calls_run_once_in_every_statement_form() {
    if !gate_on() {
        return;
    }
    let src = r#"
module top;
  class base_c;
    int calls;
    function void bump(output int o);
      calls++;
      o = calls;
    endfunction
  endclass
  class counter extends base_c;
    int m;
    int arr[4];
    static int scalls;
    counter other;
    function int fo(output int o);
      calls++;
      o = calls * 10;
      return calls;
    endfunction
    task tb(output int o);
      calls++;
      o = calls + 100;
    endtask
    static function void sb(output int o);
      scalls++;
      o = scalls;
    endfunction
    function void fr(ref int o);
      calls++;
      o = o + 1;
    endfunction
    function void case_void();   void'(fo(m)); endfunction
    task          case_task();   tb(m); endtask
    function void case_static(); sb(m); endfunction
    function void case_other();  other.bump(m); endfunction
    function void case_super();  super.bump(m); endfunction
    function void case_arr();    for (int i = 0; i < 2; i++) bump(arr[i]); endfunction
    function void case_ref();    fr(m); fr(m); endfunction
  endclass
  counter c;
  int void_calls, void_m, task_calls, task_m, static_calls, static_m;
  int other_calls, other_m, super_calls, super_m, arr_calls, arr0, arr1;
  int ref_calls, ref_m;
  initial begin
    c = new; c.case_void();   void_calls = c.calls; void_m = c.m;
    c = new; c.case_task();   task_calls = c.calls; task_m = c.m;
    c = new; c.case_static(); static_calls = counter::scalls; static_m = c.m;
    c = new; c.other = new; c.case_other(); other_calls = c.other.calls; other_m = c.m;
    c = new; c.case_super();  super_calls = c.calls; super_m = c.m;
    c = new; c.case_arr();    arr_calls = c.calls; arr0 = c.arr[0]; arr1 = c.arr[1];
    c = new; c.case_ref();    ref_calls = c.calls; ref_m = c.m;
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulation should run");
    for (calls, want) in [
        ("void_calls", 1),
        ("task_calls", 1),
        ("static_calls", 1),
        ("other_calls", 1),
        ("super_calls", 1),
        ("arr_calls", 2),
        ("ref_calls", 2),
    ] {
        assert_eq!(u(&sim, calls), want, "{calls}");
    }
    assert_eq!(u(&sim, "void_m"), 10);
    assert_eq!(u(&sim, "task_m"), 101);
    assert_eq!(u(&sim, "static_m"), 1);
    assert_eq!(u(&sim, "other_m"), 1);
    assert_eq!(u(&sim, "super_m"), 1);
    assert_eq!((u(&sim, "arr0"), u(&sim, "arr1")), (1, 2));
    assert_eq!(u(&sim, "ref_m"), 2);
}
