//! §11.4.13 / §13.5.3 / §12.7.3 — $ range bounds, formal-referencing
//! defaults, packed foreach direction. Reference-validated.

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
  int x = -100;
  int r_ci, r_op, r_hi, r_f5, r_f2, first_b;
  logic [3:0] pv [1];
  function int f(int a, int b = a + 1);
    return a*10 + b;
  endfunction
  initial begin
    case (x) inside
      [$:-50] : r_ci = 1;
      default : r_ci = 0;
    endcase
    r_op = (x inside {[$:-50]});
    r_hi = (x inside {[-200:$]});
    r_f5 = f(5);
    r_f2 = f(2, 9);
    first_b = -1;
    foreach (pv[i, b]) if (first_b == -1) first_b = b;
  end
endmodule
"#;

#[test]
fn dollar_bounds_defaults_and_direction() {
    let sim = simulate(SRC, 20).expect("simulate failed");
    assert_eq!(u(&sim, "r_ci"), 1, "case-inside [$:-50] matches -100");
    assert_eq!(u(&sim, "r_op"), 1, "inside [$:-50] matches -100");
    assert_eq!(u(&sim, "r_hi"), 1, "inside [-200:$] matches -100");
    assert_eq!(u(&sim, "r_f5"), 56, "default reads earlier formal");
    assert_eq!(u(&sim, "r_f2"), 29, "explicit arg overrides default");
    assert_eq!(u(&sim, "first_b"), 3, "packed [3:0] foreach starts at 3");
}
