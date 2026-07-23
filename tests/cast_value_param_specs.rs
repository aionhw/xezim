// Self-test: `$cast` across value-parameter specializations must fail.
// Without the fix, cast_type_ok checked only class NAME compatibility
// (class_is_a), so $cast(me[special_comp#(2)], a1[special_comp#(1)]) wrongly
// succeeded — leaking a special_comp#(2) typewide callback onto a
// special_comp#(1) instance (UVM 09callbacks/25params).

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
    function new(string name = "x");
      my_n = N;
    endfunction
  endclass

  int a1_to_2, a2_to_1, a1_to_1, a2_to_2;
  initial begin
    special_comp#(1) a1;
    special_comp#(2) a2;
    a1 = new("a1");
    a2 = new("a2");
    // Cross-specialization casts must FAIL (0); same-spec casts succeed (1).
    begin special_comp#(2) me2;  a1_to_2 = $cast(me2, a1); end
    begin special_comp#(1) me1;  a2_to_1 = $cast(me1, a2); end
    begin special_comp#(1) me1b; a1_to_1 = $cast(me1b, a1); end
    begin special_comp#(2) me2b; a2_to_2 = $cast(me2b, a2); end
  end
endmodule
"#;

#[test]
fn cast_value_param_specs() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    // Cross-specialization casts fail (0); same-spec succeed (1).
    assert_eq!(u(&sim, "a1_to_2"), 0, "#2 dest vs #1 src must fail");
    assert_eq!(u(&sim, "a2_to_1"), 0, "#1 dest vs #2 src must fail");
    assert_eq!(u(&sim, "a1_to_1"), 1, "#1 dest vs #1 src must succeed");
    assert_eq!(u(&sim, "a2_to_2"), 1, "#2 dest vs #2 src must succeed");
}
