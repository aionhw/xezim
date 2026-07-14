//! IEEE 1800-2017 §4.4.2.3 / §4.5 — `#0` and the NBA region.
//!
//! A `#0` suspends the process and reschedules its continuation into the
//! Inactive region of the SAME time slot. The commercial consensus
//! (VCS / Questa / Riviera all agree) is that a nonblocking assignment
//! posted before the `#0` IS visible after it: the NBA region for the
//! active pass that posted the update commits before the suspended
//! process's continuation resumes.
//!
//! Before the fix, xezim scheduled the `#0` continuation into the plain
//! event_queue at the current time; run_one_tick's batch drain re-fetched
//! it into the SAME active pass, before apply_nba — so the continuation
//! read the stale pre-NBA value. The fix parks `#0` continuations in a
//! dedicated Inactive-region queue (`inactive_queue`) promoted back into
//! the event queue only after the tick's NBA region has been applied.

use xezim::simulate;

fn lookup(sim: &xezim::compiler::Simulator, name: &str) -> u64 {
    sim.get_signal(name)
        .or_else(|| sim.get_signal(&format!("tb.{}", name)))
        .unwrap_or_else(|| panic!("signal not found: {}", name))
        .to_u64()
        .unwrap_or_else(|| panic!("signal {} not u64-able", name))
}

/// An NBA posted in the active region must NOT be visible before the `#0`
/// (still the same active pass) but MUST be visible after it (the NBA
/// region drains before the `#0` continuation resumes).
#[test]
fn nba_visible_after_zero_delay() {
    const SRC: &str = r#"
module tb;
  logic [7:0] nb;
  logic [7:0] before_z = 8'hFF;
  logic [7:0] after_z  = 8'hFF;
  initial begin
    nb = 8'h00;
    nb <= 8'hAA;
    before_z = nb; // pre-#0: NBA not yet applied -> 00
    #0;
    after_z = nb;  // post-#0: NBA region committed -> aa
  end
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    let before_z = lookup(&sim, "before_z") & 0xFF;
    let after_z = lookup(&sim, "after_z") & 0xFF;
    assert_eq!(
        before_z, 0x00,
        "before #0 the NBA must not have been applied yet (§4.4.2.3), got {:02x}",
        before_z
    );
    assert_eq!(
        after_z, 0xAA,
        "after #0 the NBA region must have committed (commercial consensus), got {:02x}",
        after_z
    );
}

/// The classic NBA swap, observed through a `#0`: both right-hand sides
/// were sampled in the active pass, both updates commit in the NBA
/// region, and the `#0` continuation sees the swapped values.
#[test]
fn nba_swap_visible_after_zero_delay() {
    const SRC: &str = r#"
module tb;
  int a, b;
  int ra = -1, rb = -1;
  initial begin
    a = 1; b = 2;
    a <= b; b <= a; // RHS sampled pre-commit: classic swap
    #0;
    ra = a; // want 2
    rb = b; // want 1
  end
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    let ra = lookup(&sim, "ra") & 0xFFFFFFFF;
    let rb = lookup(&sim, "rb") & 0xFFFFFFFF;
    assert_eq!(ra, 2, "after #0 a must hold the swapped value 2, got {}", ra);
    assert_eq!(rb, 1, "after #0 b must hold the swapped value 1, got {}", rb);
}
