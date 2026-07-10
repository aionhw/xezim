//! IEEE 1800-2017 Clause 11 conformance findings — operators and expressions.
//!
//! §11.4.10 — `>>>` fills with the sign bit ONLY when the left operand is
//! signed. On an unsigned operand it is a plain logical shift. xezim filled
//! with the MSB regardless of signedness, so `8'b1111_0000 >>> 1` silently
//! produced `1111_1000` instead of `0111_1000`.
//!
//! §11.4.13 — an `inside` set element is compared with the tested expression
//! using WILDCARD equality, so x/z bits in the element are don't-cares. The
//! `==?` operator did this correctly, but `inside` used plain `==`, so
//! `4'b1010 inside {4'b10xx}` was 0.
//!
//! Both are silent wrong-value bugs: no error, no X, just the wrong answer.

use xezim::simulate;

const SRC: &str = r#"
module tb;
  logic        [7:0] u;
  logic signed [7:0] s;
  logic        [3:0] v;
  int i;

  logic [7:0] u_lsr, u_asr, s_asr, s_lsr;
  int i_asr;
  int wild_hit, wild_miss, exact_hit, exact_miss, range_hit;
  int wildq;
  logic [3:0] patterns [2];

  initial begin
    u = 8'b1111_0000;
    s = 8'sb1111_0000;

    u_lsr = u >> 1;    // logical, unsigned
    u_asr = u >>> 1;   // unsigned operand -> ALSO logical
    s_asr = s >>> 1;   // signed operand   -> arithmetic
    s_lsr = s >> 1;    // logical even on a signed operand

    i = -16;
    i_asr = i >>> 1;   // signed int -> arithmetic

    v = 4'b1010;
    wild_hit   = (v inside {4'b10xx});   // x bits are don't-cares
    wild_miss  = (v inside {4'b11xx});
    exact_hit  = (v inside {4'b1010});
    exact_miss = (v inside {4'b1011});
    range_hit  = (v inside {[4'b1000:4'b1100]});

    // An array operand's elements are patterns too.
    patterns[0] = 4'b11xx;
    patterns[1] = 4'b10xx;
    wildq = (v inside {patterns});
  end
endmodule
"#;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able", n))
}

#[test]
fn arithmetic_shift_right_honours_operand_signedness() {
    let sim = simulate(SRC, 100).expect("simulate failed");

    // An UNSIGNED left operand makes `>>>` a logical shift (§11.4.10).
    assert_eq!(u(&sim, "u_lsr") & 0xFF, 0b0111_1000);
    assert_eq!(
        u(&sim, "u_asr") & 0xFF,
        0b0111_1000,
        ">>> sign-extended an unsigned operand"
    );

    // A SIGNED left operand makes it arithmetic; `>>` stays logical.
    assert_eq!(u(&sim, "s_asr") & 0xFF, 0b1111_1000);
    assert_eq!(u(&sim, "s_lsr") & 0xFF, 0b0111_1000);

    // -16 >>> 1 == -8
    assert_eq!(u(&sim, "i_asr") as i32 as i64, -8);
}

#[test]
fn inside_uses_wildcard_equality_for_set_elements() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "wild_hit"), 1, "4'b1010 inside {{4'b10xx}} must match");
    assert_eq!(u(&sim, "wild_miss"), 0);
    // An array operand's elements are patterns as well.
    assert_eq!(u(&sim, "wildq"), 1);
}

#[test]
fn inside_still_matches_exact_values_and_ranges() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "exact_hit"), 1);
    assert_eq!(u(&sim, "exact_miss"), 0);
    assert_eq!(u(&sim, "range_hit"), 1);
}
