//! Three silent wrong-value bugs from the clause-6 / clause-12 LRM audits.
//!
//! §6.12.2 — converting a real to an integral type ROUNDS to the nearest, ties
//! away from zero. Every conversion went through `f64 as u64`, which both
//! truncates AND saturates a negative value to 0: `int i = -5.0;` yielded 0.
//! (`$rtoi` is the one that truncates, and it saturated too.)
//!
//! §6.24.1 — `signed'(e)` / `unsigned'(e)` reinterpret the operand's
//! signedness. Every `type'(expr)` cast was parsed as a pass-through, so
//! `signed'(4'hF)` was 15 while `$signed(4'hF)` was correctly -1.
//!
//! §12.5.4 — `case (e) inside` matches items with the `inside` operator's
//! rules: inclusive ranges, and x/z in an item are wildcards. `CaseInside` fell
//! through to exact `case_eq`, so a range item compared against a garbage value
//! and a wildcard item never matched — both silently took `default`.

use xezim::simulate;

const REALS: &str = r#"
module tb;
  real rn, rp;
  int neg, pos, cast_up, half, rtoi_neg, rtoi_pos, neg_half;
  initial begin
    rn = -5.0;
    rp = 5.9;
    neg      = rn;          // must sign-extend, not saturate to 0
    pos      = rp;          // round to nearest
    cast_up  = int'(3.7);
    half     = int'(2.5);   // ties away from zero
    neg_half = int'(-2.5);
    rtoi_neg = $rtoi(-5.9); // $rtoi truncates toward zero
    rtoi_pos = $rtoi(5.9);
  end
endmodule
"#;

const CASTS: &str = r#"
module tb;
  int s_cast, s_sys, i_from_real;
  initial begin
    s_cast      = signed'(4'hF);
    s_sys       = $signed(4'hF);
    i_from_real = int'(3.7);
  end
endmodule
"#;

const CASE_INSIDE: &str = r#"
module tb;
  logic [3:0] s;
  int r_range, r_exact, r_wild, r_none;
  int op_range;
  initial begin
    s = 4'd5;
    r_range = 0;
    case (s) inside
      [4'd4:4'd7] : r_range = 2;
      4'd9        : r_range = 3;
      default     : r_range = 0;
    endcase

    s = 4'd9;
    r_exact = 0;
    case (s) inside
      [4'd4:4'd7] : r_exact = 2;
      4'd9        : r_exact = 3;
      default     : r_exact = 0;
    endcase

    s = 4'b1010;
    r_wild = 0;
    case (s) inside
      4'b10?? : r_wild = 4;
      default : r_wild = 0;
    endcase

    // Nothing matches -> default must still be taken.
    s = 4'd15;
    r_none = 0;
    case (s) inside
      [4'd4:4'd7] : r_none = 2;
      default     : r_none = 9;
    endcase

    s = 4'd5;
    op_range = (s inside {[4'd4:4'd7]});
  end
endmodule
"#;

fn i(sim: &xezim::compiler::Simulator, n: &str) -> i64 {
    let v = sim
        .get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able", n));
    v as u32 as i32 as i64
}

#[test]
fn real_to_integer_rounds_and_keeps_the_sign() {
    let sim = simulate(REALS, 100).expect("simulate failed");
    assert_eq!(i(&sim, "neg"), -5, "a negative real saturated to 0");
    assert_eq!(i(&sim, "pos"), 6, "a real assignment truncated instead of rounding");
    assert_eq!(i(&sim, "cast_up"), 4);
    // Ties round away from zero.
    assert_eq!(i(&sim, "half"), 3);
    assert_eq!(i(&sim, "neg_half"), -3);
    // $rtoi truncates toward zero, in both directions.
    assert_eq!(i(&sim, "rtoi_neg"), -5);
    assert_eq!(i(&sim, "rtoi_pos"), 5);
}

#[test]
fn signedness_casts_reinterpret_the_operand() {
    let sim = simulate(CASTS, 100).expect("simulate failed");
    assert_eq!(i(&sim, "s_cast"), -1, "signed'() was a pass-through");
    assert_eq!(i(&sim, "s_sys"), -1);
    // A plain type cast stays a width/type hint.
    assert_eq!(i(&sim, "i_from_real"), 4);
}

#[test]
fn case_inside_matches_ranges_and_wildcards() {
    let sim = simulate(CASE_INSIDE, 100).expect("simulate failed");
    assert_eq!(i(&sim, "r_range"), 2, "a range item never matched");
    assert_eq!(i(&sim, "r_exact"), 3);
    assert_eq!(i(&sim, "r_wild"), 4, "a wildcard item never matched");
    // default is still reachable.
    assert_eq!(i(&sim, "r_none"), 9);
    // The `inside` operator was always right; this guards it.
    assert_eq!(i(&sim, "op_range"), 1);
}
