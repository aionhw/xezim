//! Wide (>64-bit) rand-element draws and wide sum()/product() targets.
//!
//! Regression test for the full-width draw / 128-bit sum fixes on this
//! branch:
//!   C1  draw_elem() routed every element through Value::from_u64, which
//!       zero-fills bits above 63 -- the high half of a wide element was
//!       pinned to 0 in every draw (fresh baselines and the backstop).
//!   G2  the same zero-fill pinned the MSB field of a wide packed struct
//!       element, making `beats[i].vld == 1` structurally unsatisfiable
//!       for the generate-and-test backstop (the randomize grind).
//!   C10 the sum() path truncated at 64 bits three times over (target
//!       read via to_i64, domain capped at u64::MAX, shares written via
//!       from_u64, u64/i64 accumulators): a 96-bit target of 2^64 could
//!       not be honoured, an infeasible 2^97 target did not prove UNSAT,
//!       and sum()/product() over 96-bit elements read as the low word.
//!   Family guards (pass on the old solver too; they pin the
//!       neighbouring truncation/CSP-gate sites from the audit):
//!       C2/C2d != repair vs wide pins, C6 wide fixed-array foreach
//!       bit-select, C7 foreach relational bound, C8a nested [][2]
//!       member select, C8b implication contradiction.
//!
//! The comprehensive grind MWE's production-stack replicas and
//! wall-clock budgets stay in the eval area (cargo's simulate() cannot
//! inject plusargs; timing asserts are flaky in unit tests).
use xezim::simulate;

#[test]
fn rand_wide_elem_draw_sum() {
    let sim =
        simulate(include_str!("rand_wide_elem_draw_sum_588.sv"), 100).expect("simulate failed");
    let failures = sim
        .get_signal("failures")
        .or_else(|| sim.get_signal("tb_top.failures"))
        .unwrap_or_else(|| panic!("signal not found: failures"))
        .to_u64()
        .unwrap_or_else(|| panic!("failures not u64-able"));
    assert_eq!(
        failures, 0,
        "wide draw/sum sweep: every case must hold (see FAIL lines above)"
    );
    assert!(
        sim.output.iter().any(|o| o.message.trim() == "TEST_PASS"),
        "SVTEST_PASSFAIL must print TEST_PASS"
    );
}

/// §18.5.8.2: `a.sum() == K` over elements of 127 bits or more, whose domain
/// cap is i128::MAX, solves: the distributor's bounds saturate instead of
/// overflowing (a panic with overflow checks, an infeasible target without).
#[test]
fn sum_over_elements_of_128_bits_and_more() {
    let out: Vec<String> = simulate(
        r#"class c;
  rand bit [127:0] a[];
  rand bit [145:0] b[];
  constraint k { a.size() == 4; a.sum() == 5; b.size() == 3; b.sum() == 7; }
endclass
module t;
  initial begin
    c x = new;
    bit [127:0] sa; bit [145:0] sb;
    if (!x.randomize()) $display("R fail");
    else begin
      foreach (x.a[i]) sa += x.a[i];
      foreach (x.b[i]) sb += x.b[i];
      $display("R %0d %0d %0d %0d", x.a.size(), sa, x.b.size(), sb);
    end
  end
endmodule
"#,
        100,
    )
    .expect("simulate")
    .output
    .iter()
    .map(|l| l.message.clone())
    .filter(|m| m.starts_with("R "))
    .collect();
    assert_eq!(out, ["R 4 5 3 7"]);
}
