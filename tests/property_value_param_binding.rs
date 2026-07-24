// Self-test: a class PROPERTY typed with a value-parameterized type
// (`special_comp#(1) a1;`) must bind the value parameter N at construction.
// Without the fix, class properties had no recorded `#(...)` type-args (only
// module-level decls and locals did), so `this.a1 = new(...)` constructed with
// NO type-args and N defaulted to 0 — collapsing `#(1)`/`#(2)` to `#(0)`
// (UVM 09callbacks/25params: callbacks printed special#(0) instead of #(1)).

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(&format!("top.{}", n))
        .or_else(|| sim.get_signal(n))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able", n))
        & 0xFFFF_FFFF
}

const SRC: &str = r#"
module top;
  class special_comp #(int N = 0);
    int my_n;
    function new(string name = "special_comp", int parent = 0);
      my_n = N;
    endfunction
  endclass

  class test;
    // PROPERTIES typed with value params (like 25params's a1/a2)
    special_comp#(1) a1;
    special_comp#(2) a2;
    function void build;
      a1 = new("a1", 0);
      a2 = new("a2", 0);
    endfunction
  endclass

  int r1, r2;
  initial begin
    test t;
    t = new;
    t.build;
    r1 = t.a1.my_n;
    r2 = t.a2.my_n;
  end
endmodule
"#;

#[test]
fn property_value_param_binding() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    // a1.my_n = 1, a2.my_n = 2 (value params bound from declared specialization)
    assert_eq!(u(&sim, "r1"), 1, "a1.my_n should be 1");
    assert_eq!(u(&sim, "r2"), 2, "a2.my_n should be 2");
}
