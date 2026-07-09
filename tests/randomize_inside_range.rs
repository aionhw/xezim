//! `std::randomize(x) with { x inside {[lo:hi]}; }` must be able to produce any
//! value in `[lo, hi]`.
//!
//! The range picker capped the span at 4096, so `[32'he000_0000:32'he000_2000]`
//! could never yield anything above `32'he000_0fff` — silently shrinking the
//! legal value set. The cap was guarding an enumeration that no longer exists;
//! picking is a single modulo.

use xezim::simulate;

const SRC: &str = r#"
module tb;
  bit [31:0] a;
  int out_of_range;
  int above_4k;      // values beyond the old 4096-wide cap
  int at_or_below_4k;
  initial begin
    for (int i = 0; i < 3000; i++) begin
      void'(std::randomize(a) with { a inside { [32'he000_0000:32'he000_2000] }; });
      if (a < 32'he000_0000 || a > 32'he000_2000) out_of_range++;
      if (a > 32'he000_0fff) above_4k++;
      else at_or_below_4k++;
    end
  end
endmodule
"#;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able", n))
        & 0xFFFF_FFFF
}

#[test]
fn inside_range_never_leaves_its_bounds() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "out_of_range"), 0, "randomize produced a value outside the range");
}

#[test]
fn inside_range_spans_the_whole_interval() {
    let sim = simulate(SRC, 100).expect("simulate failed");

    // Roughly half the 0x2001-wide range lies above the old 4096 cap. Over 3000
    // draws, seeing zero of them would mean the range is still truncated.
    // A loose bound keeps this deterministic-seed-independent.
    let above = u(&sim, "above_4k");
    let below = u(&sim, "at_or_below_4k");
    assert_eq!(above + below, 3000);
    assert!(above > 300, "only {} of 3000 draws exceeded the old 4096 cap", above);
    assert!(below > 300, "only {} of 3000 draws fell below the old cap", below);
}
