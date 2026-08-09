//! §23.10 / §13.4.3 / §5.7.1 / §6.24.1 — ordered overrides bind flattened
//! slots; const function calls and casts work in parameter contexts.
//! Reference-validated (agentJ audit; the silent-zero family behind the
//! customer's [N-1:0] underflow).

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} is x/z", n))
}

const SRC: &str = r#"
package fp;
  function automatic int dbl(int x); return 2*x; endfunction
endpackage
module m #(parameter A=1, B=2, N=1);
  parameter  BP = fp::dbl(3);
  localparam BL = fp::dbl(4);
  localparam [7:0] ONES = '1;
  int r_a, r_b, r_n, r_bp, r_bl, r_ones;
  initial begin
    r_a = A; r_b = B; r_n = N; r_bp = BP; r_bl = BL; r_ones = ONES;
  end
endmodule
module tb;
  m #(10, 20) u1();
  m #(.N(fp::dbl(5))) u2();
  localparam AW = 3;
  localparam [AW-1:0] TOP = AW'(6);
  int r_top, r_dyn;
  initial begin
    r_top = TOP;
    r_dyn = AW'(6);
  end
endmodule
"#;

#[test]
fn param_const_eval_contexts() {
    let sim = simulate(SRC, 20).expect("simulate failed");
    assert_eq!(u(&sim, "u1.r_a"), 10, "ordered override slot 0");
    assert_eq!(u(&sim, "u1.r_b"), 20, "ordered override slot 1");
    assert_eq!(u(&sim, "u2.r_n"), 10, "pkg fn call in override expr");
    assert_eq!(u(&sim, "u1.r_bp"), 6, "pkg fn in body parameter");
    assert_eq!(u(&sim, "u1.r_bl"), 8, "pkg fn in body localparam");
    assert_eq!(u(&sim, "u1.r_ones"), 0xff, "'1 fills declared [7:0]");
    assert_eq!(u(&sim, "r_top"), 6, "AW'(6) in localparam init");
    // Runtime display of a size-cast SIGNED operand keeps its sign
    // (§6.24.1 / ivtest size_cast3-5); the reference's %0d of `AW'(6)`
    // prints 6 — a display-context divergence noted in debug_notes,
    // deliberately not matched here. r_dyn is an int assignment, where
    // 3-bit signed 110 sign-extends to -2.
    assert_eq!(u(&sim, "r_dyn") as u32 as i32, -2, "AW'(6) signed narrowing");
}
