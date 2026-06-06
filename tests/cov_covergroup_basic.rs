//! Covergroup sampling + coverpoint/cross hit tracking regression.
//!
//! Exercises the bare `cg c1 = new;` binding (`simulator.rs:18082`) and
//! the coverpoint/cross hit machinery so the end-of-sim summary +
//! `XEZIM_COV_DB` reflect what the design did. Covers the two fixes from
//! commit 7a6ea2d:
//!   * Bare `cg c1 = new;` allocates a real covergroup instance.
//!   * Cross items resolve to the named coverpoint's expression
//!     (previously aliased to `lookup_signal_value(name)` → 0 → 1
//!     tuple).

use xezim::simulate;

const SRC: &str = r#"
module tb;
  int a, b;
  covergroup cg;
    cp_a: coverpoint a;
    cp_b: coverpoint b;
    cross_ab: cross cp_a, cp_b;
  endgroup
  cg c1 = new;
  initial begin
    a = 1; b = 10; c1.sample();
    a = 2; b = 20; c1.sample();
    a = 3; b = 30; c1.sample();
    a = 1; b = 10; c1.sample();   // duplicate — exercises set semantics
    $finish;
  end
endmodule
"#;

#[test]
fn bare_new_covergroup_sample_and_cross() {
    let sim = simulate(SRC, 1000).expect("simulate failed");

    assert_eq!(
        sim.covergroup_instance_count(),
        1,
        "bare `cg c1 = new;` should allocate exactly 1 covergroup instance"
    );
    assert_eq!(
        sim.covergroup_sample_total(),
        4,
        "expected 4 sample() invocations, got {}",
        sim.covergroup_sample_total()
    );

    // 3 distinct values across the 4 samples (the 4th is a duplicate).
    assert_eq!(
        sim.coverpoint_hits("cg", "cp_a"),
        3,
        "cp_a should have 3 unique values, got {}",
        sim.coverpoint_hits("cg", "cp_a")
    );
    assert_eq!(
        sim.coverpoint_hits("cg", "cp_b"),
        3,
        "cp_b should have 3 unique values, got {}",
        sim.coverpoint_hits("cg", "cp_b")
    );

    // Cross tracks (cp_a, cp_b) pairs — 3 unique pairs.
    assert_eq!(
        sim.cross_hits("cg", "cross_ab"),
        3,
        "cross_ab should have 3 unique tuples, got {} \
         (this was 1 before the cross-resolves-coverpoint fix)",
        sim.cross_hits("cg", "cross_ab")
    );
}
