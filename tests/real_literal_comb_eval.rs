//! Regression: a REAL literal in a combinational context (continuous assign
//! or `always @(*)`) was truncated to an integer.
//!
//! The bytecode compiler's `eval_number_static` lowered `NumberLiteral::Real(f)`
//! as `Value::from_u64(f as u64, 64)` — so `5.5` became `5`, `4.4` became `4`,
//! and `1.0 / 4.4` degraded to integer `1 / 4 = 0`. Real *signal* operands
//! were fine (they load as real Values and the VM does real arithmetic), so the
//! bug only bit expressions containing real *literals*. In a real PLL model
//! this drove clamp-mode `vcofbperiod = (1.0/4.4)*1000.0` to 0, and the
//! `always #(plloutperiod/2.0)` clock became a #0 spinner (zero-delay livelock).
//!
//! Fix: lower a real literal via `Value::from_f64`, preserving the IEEE-754
//! bits and `is_real` so the VM keeps it in the real domain. Procedural
//! (`initial`) evaluation was always correct — this only affected the compiled
//! comb/cont-assign path.

use xezim::simulate;

fn output_of(sim: &xezim::compiler::Simulator) -> String {
    sim.output
        .iter()
        .map(|o| o.message.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Continuous assigns of real-literal expressions must keep their fractional
/// value (was truncated to integer).
#[test]
fn cont_assign_real_literals_keep_fraction() {
    const SRC: &str = r#"
module top;
  real a, b, c, e, f;
  assign a = 1.0 / 4.4;
  assign b = 2.0 / 4.0;
  assign c = 9.0 / 2.0;
  assign e = 5.5;
  assign f = 3.0 * 2.5;
  initial begin
    #1 $display("R a=%.4f b=%.4f c=%.4f e=%.4f f=%.4f", a, b, c, e, f);
  end
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    let out = output_of(&sim);
    assert!(
        out.contains("R a=0.2273 b=0.5000 c=4.5000 e=5.5000 f=7.5000"),
        "real-literal cont-assigns must not truncate to integer:\n{}",
        out
    );
}

/// The `always @(*)` if/else shape from the PLL model: the else branch's
/// real-literal constant `(1.0/4.4)*1000.0` must evaluate to 227.27, not 0.
#[test]
fn always_comb_else_branch_real_constant() {
    const SRC: &str = r#"
module top;
  logic [1:0] clamp;
  real x;
  always @(*) begin
    if (clamp == 2'b00) x = 5.0;
    else x = (1.0 / 4.4) * 1000.0;
  end
  initial begin
    clamp = 0;  #1 $display("A x=%.3f", x);
    clamp = 1;  #1 $display("B x=%.3f", x);
  end
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    let out = output_of(&sim);
    assert!(
        out.contains("A x=5.000") && out.contains("B x=227.273"),
        "always @(*) real-literal arithmetic must match procedural (want B x=227.273):\n{}",
        out
    );
}
