//! §12.7.2 / §12.7.3 / §13.3.1 — repeat counts, partial-index foreach,
//! static-function implicit return persistence. Reference-validated.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} is x/z", n))
}

const SRC: &str = r#"
module tb;
  int m [2][3];
  int rows, reps_s4, reps_si;
  int c2, c4;
  function int halfset(int a);
    if (a > 0) halfset = a;
  endfunction
  initial begin
    rows = 0;
    foreach (m[i]) rows++;
    begin
      logic signed [3:0] s4 = -2;
      reps_s4 = 0; repeat (s4) reps_s4++;
    end
    begin
      int si = -3;
      reps_si = 0; repeat (si) reps_si++;
    end
    void'(halfset(5));  c2 = halfset(-5);
    void'(halfset(8));  c4 = halfset(0);
  end
endmodule
"#;

#[test]
fn loops_and_static_function_return() {
    let sim = simulate(SRC, 20).expect("simulate failed");
    assert_eq!(u(&sim, "rows"), 2, "foreach (m[i]) iterates dim 0 fully");
    assert_eq!(u(&sim, "reps_s4"), 0, "negative signed repeat runs 0 times");
    assert_eq!(u(&sim, "reps_si"), 0, "negative int repeat runs 0 times");
    assert_eq!(u(&sim, "c2"), 5, "static fn keeps prior return value");
    assert_eq!(u(&sim, "c4"), 8, "static fn keeps prior return value");
}
