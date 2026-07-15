//! Issue #35: mixed-sign relational operators in the constraint solver.
//!
//! IEEE 1800-2017 §11.8.1: when a signed and an unsigned operand meet, the
//! comparison is evaluated UNSIGNED. Inside a constraint that means an unsized
//! decimal bound such as `4294967290` (0xFFFFFFFA) is read by its unsigned
//! value against an unsigned target — not sign-extended to -6, which would
//! collapse `u >= 4294967290` to `u >= 0`.
//!
//! The failure surfaced as a SOLVER miss, not a wrong value: the bound never
//! narrowed the domain, so the solver drew a full-width random value and could
//! never land the six-value band [4294967290, 4294967295] near 2^32. The fix
//! turns a plain `prop REL const` into an allowed RANGE (evaluated with the
//! §11.8.1 sign of the comparison), so the picker samples the band directly.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("test.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} is X", n))
}

/// The reporter's five cases, verbatim in spirit: mixed-sign unsigned context,
/// a signed contradiction that must be UNSAT, concat/part-select sign stripping,
/// and signed extension of a small negative literal.
#[test]
fn mixed_sign_relational_operators_in_constraints() {
    const SRC: &str = r#"
class sign_test_class;
    rand int rand_signed;
    rand logic [31:0] rand_unsigned;
endclass
module test;
    int failures = 0;
    initial begin
        sign_test_class item = new();
        bit success;

        // §11.8.1: unsigned target -> unsigned comparison; the 6-value band
        // [4294967290, 4294967295] must be reachable.
        item.rand_unsigned = 0;
        success = item.randomize() with {
            rand_unsigned <= 32'hffff_ffff;
            rand_unsigned >= 4294967290;
        };
        if (!(success && (item.rand_unsigned >= 4294967290))) failures++;

        // §11.8.1: all operands signed -> signed comparison. `<= -1 && >= 1`
        // is impossible, so randomize() must FAIL.
        success = item.randomize() with {
            rand_signed <= 32'shffff_ffff;
            rand_signed >= 1;
        };
        if (success != 0) failures++;

        // §11.8.1: a concatenation result is unsigned; { 16'sh8000 } is +32768.
        success = item.randomize() with { rand_signed == { 16'sh8000 }; };
        if (!(success && (item.rand_signed == 32768))) failures++;

        // §11.8.1: a part-select result is unsigned.
        success = item.randomize() with { rand_signed[31:0] > 32'h7fff_ffff; };
        if (!(success && (item.rand_signed < 0))) failures++;

        // §11.8.2: 8'sh80 sign-extends to -128 in a 32-bit signed compare.
        success = item.randomize() with { rand_signed == 8'sh80; };
        if (!(success && (item.rand_signed == 32'shffff_ff80))) failures++;

        result = failures;
    end
    int result = -1;
endmodule
"#;
    let sim = simulate(SRC, 1000).expect("simulate failed");
    assert_eq!(
        u(&sim, "result"),
        0,
        "§11.8.1/§11.8.2: mixed-sign relational constraints must solve correctly"
    );
}

/// Sibling found while auditing #35: `std::randomize(v) with {…}` returned 1
/// UNCONDITIONALLY after its retry loop, so an unsatisfiable set — and a value
/// its i64 interval solver could not reach — reported a false SUCCESS (§18.11
/// requires 1 only when a consistent assignment was actually found). It must
/// report 0 instead, matching the class-`randomize()` path.
#[test]
fn std_randomize_reports_unsat_honestly() {
    let unsat = |decl: &str, body: &str| -> u64 {
        let src = format!(
            r#"
module test;
    {decl}
    int flag = 99;
    initial begin
        flag = std::randomize(v) with {{ {body} }};
    end
endmodule
"#
        );
        let sim = simulate(&src, 1000).expect("simulate failed");
        u(&sim, "flag")
    };
    // Plain contradictions across widths / signedness: all must be 0.
    assert_eq!(unsat("logic [31:0] v;", "v >= 100; v <= 5;"), 0, "32-bit UNSAT");
    assert_eq!(unsat("int v;", "v >= 100; v <= 5;"), 0, "signed UNSAT");
    assert_eq!(
        unsat("logic [63:0] v;", "v >= 64'h8000_0000_0000_0000; v <= 5;"),
        0,
        "64-bit UNSAT must not report false success"
    );
    // A satisfiable set must still report 1.
    assert_eq!(unsat("logic [31:0] v;", "v >= 100; v <= 200;"), 1, "SAT must be 1");
    assert_eq!(unsat("logic [31:0] v;", "v >= 4294967290;"), 1, "SAT high-band must be 1");
}

/// The narrow-band case in isolation, across several seeds: every solve must
/// succeed and land inside the six legal values — proving the bound narrowed
/// the domain rather than relying on a lucky full-width draw.
#[test]
fn narrow_high_unsigned_band_is_reachable() {
    for seed in [1u64, 7, 42, 99, 12345] {
        let src = format!(
            r#"
class C; rand logic [31:0] u; endclass
module test;
    int ok_flag = 0;
    logic [31:0] got;
    initial begin
        C c = new();
        bit ok;
        ok = c.randomize() with {{ u >= 4294967290; u <= 4294967295; }};
        got = c.u;
        ok_flag = ok;
    end
endmodule
"#
        );
        let plus = vec![format!("seed={}", seed)];
        let sim = xezim::simulate_multi(
            &[src],
            1000,
            None,
            &[],
            &[],
            None,
            false,
            None,
            None,
            &[],
            &plus,
            1,
            None,
            &[],
            0,
            u64::MAX,
            None,
            &[],
            None,
            None,
            None,
            None,
            false,
            None,
        )
        .expect("simulate failed");
        assert_eq!(u(&sim, "ok_flag"), 1, "seed {}: solve must succeed", seed);
        let got = u(&sim, "got");
        assert!(
            (4294967290..=4294967295).contains(&got),
            "seed {}: got {} outside the legal band [4294967290, 4294967295]",
            seed,
            got
        );
    }
}
