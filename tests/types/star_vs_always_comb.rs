//! §9.2.2.2: `always @(*)` is sensitive only to a called function's
//! ARGUMENTS (not its contents) and does NOT auto-run at time 0 —
//! both are always_comb-only behaviors. Reference-validated.

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
  int g = 1, x = 1;
  int y_star, y_comb;
  int t0_star = 0, t0_comb = 0;
  function int f(input int v); return v + g; endfunction
  always @(*)  y_star = f(x);
  always_comb  y_comb = f(x);
  int a = 5, z1 = 0, z2 = 0;
  always_comb z1 = a + 1;
  always @(*) z2 = a + 1;
  initial begin
    #1 t0_comb = z1; t0_star = z2;   // pre-first-change samples
    x = 2;
    #1 g = 10;
    #1;
  end
endmodule
"#;

#[test]
fn star_is_not_always_comb() {
    let sim = simulate(SRC, 20).expect("simulate failed");
    // g changed after x's last change: @(*) must NOT re-fire on g.
    assert_eq!(u(&sim, "y_star"), 3, "@(*) not sensitive to fn contents");
    assert_eq!(u(&sim, "y_comb"), 12, "always_comb sensitive to fn contents");
    // At t=1 (no input change yet): always_comb ran at t0, @(*) did not.
    assert_eq!(u(&sim, "t0_comb"), 6, "always_comb runs at time 0");
    assert_eq!(u(&sim, "t0_star"), 0, "@(*) does not run at time 0");
}
