//! Immediate-assertion tracking regression.
//!
//! Verifies that the simulator records per-site pass/fail counts for
//! `assert` / `assume` / `cover` (IEEE 1800-2023 §16.3) so the
//! end-of-sim `[COV]` summary and `XEZIM_COV_DB` JSON file reflect what
//! the design did.

use xezim::simulate;

const SRC: &str = r#"
module tb;
  int a;
  initial begin
    a = 5;
    // 4 asserts: 3 pass + 1 fail
    assert (a == 5);
    assert (a < 100);
    assert (a > 0);
    assert (a == 999);   // fail
    // 2 covers: 1 hit + 1 miss
    cover  (a >= 0);
    cover  (a > 500);
    // 1 assume (always trivially true here, counted as pass)
    assume (a == 5);
    $finish;
  end
endmodule
"#;

#[test]
fn per_site_pass_fail_counts() {
    let sim = simulate(SRC, 1000).expect("simulate failed");

    // 4 assert + 2 cover + 1 assume = 7 distinct sites.
    assert_eq!(
        sim.assertion_site_count(),
        7,
        "expected 7 distinct assertion sites, got {}",
        sim.assertion_site_count()
    );

    // Pass total: 3 asserts true + 1 cover true + 1 assume true = 5.
    assert_eq!(
        sim.assertion_pass_total(),
        5,
        "expected 5 passes, got {}",
        sim.assertion_pass_total()
    );

    // Fail total: 1 assert false + 1 cover false = 2.
    assert_eq!(
        sim.assertion_fail_total(),
        2,
        "expected 2 fails, got {}",
        sim.assertion_fail_total()
    );
}
