//! IEEE 1800-2017 §19.8/§19.11 — get_inst_coverage()/get_coverage()
//! regression.
//!
//! Covers three fixes:
//!   * A covergroup-typed procedural LOCAL (`cg c = new();` inside an
//!     `initial` block) now allocates a real covergroup instance —
//!     previously the initializer fell through to generic eval, so
//!     sample()/get_inst_coverage() and the @(event) clocking
//!     registration were silent no-ops (coverage always read 0).
//!   * get_inst_coverage()/get_coverage() return a REAL Value
//!     (§19.8) — they used to return integer bits, which `%f`
//!     reinterpreted as an f64 and printed 0.0.
//!   * Coverpoint coverage is the fraction of its `bins` that were hit
//!     (§19.8), not the old any-value-sampled → 100% shortcut, so a
//!     one-of-two-bins case reads exactly 50.0.

use xezim::simulate;

/// Event-driven sampling (§19.5 `covergroup cg @(posedge clk)`): both
/// bins hit across successive clock edges → 100.0 from both queries.
const SRC_EVENT_FULL: &str = r#"
module tb;
  logic clk = 0;
  logic [3:0] sig;
  always #5 clk = ~clk;
  covergroup cg @(posedge clk);
    cp: coverpoint sig {
      bins low = {[0:7]};
      bins high = {[8:15]};
    }
  endgroup
  initial begin
    cg c = new();
    sig = 3;
    @(posedge clk);
    sig = 12;
    @(posedge clk);
    @(posedge clk);
    $display("COV=%0.1f", c.get_inst_coverage());
    $display("GC=%0.1f", c.get_coverage());
    $finish;
  end
endmodule
"#;

/// Event-driven sampling where only ONE of the two bins is ever hit:
/// §19.8 says coverpoint coverage = bins hit / bin count → exactly 50.0.
const SRC_EVENT_PARTIAL: &str = r#"
module tb;
  logic clk = 0;
  logic [3:0] sig;
  always #5 clk = ~clk;
  covergroup cg @(posedge clk);
    cp: coverpoint sig {
      bins low = {[0:7]};
      bins high = {[8:15]};
    }
  endgroup
  initial begin
    cg c = new();
    sig = 3;
    @(posedge clk);
    @(posedge clk);
    $display("COV=%0.1f", c.get_inst_coverage());
    $display("GC=%0.1f", c.get_coverage());
    $finish;
  end
endmodule
"#;

/// Explicit `c.sample()` (§19.8) on a covergroup with no clocking
/// event, constructed as a procedural local — both bins hit → 100.0.
const SRC_EXPLICIT_SAMPLE: &str = r#"
module tb;
  logic [3:0] sig;
  covergroup cg;
    cp: coverpoint sig {
      bins low = {[0:7]};
      bins high = {[8:15]};
    }
  endgroup
  initial begin
    cg c = new();
    sig = 3;
    c.sample();
    sig = 12;
    c.sample();
    $display("COV=%0.1f", c.get_inst_coverage());
    $display("GC=%0.1f", c.get_coverage());
    $finish;
  end
endmodule
"#;

fn displayed_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 100000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn assert_line(lines: &[String], want: &str) {
    assert!(
        lines.iter().any(|l| l.trim() == want),
        "expected a `{}` line, got: {:?}",
        want,
        lines
    );
}

#[test]
fn event_sampled_full_coverage_is_100() {
    let lines = displayed_lines(SRC_EVENT_FULL);
    assert_line(&lines, "COV=100.0");
    assert_line(&lines, "GC=100.0");
}

#[test]
fn event_sampled_partial_coverage_is_exactly_50() {
    let lines = displayed_lines(SRC_EVENT_PARTIAL);
    assert_line(&lines, "COV=50.0");
    assert_line(&lines, "GC=50.0");
}

#[test]
fn explicit_sample_full_coverage_is_100() {
    let lines = displayed_lines(SRC_EXPLICIT_SAMPLE);
    assert_line(&lines, "COV=100.0");
    assert_line(&lines, "GC=100.0");
}
