//! IEEE 1800-2023 §16.5.1: a concurrent assertion reads sampled values, and
//! a WHOLE unpacked array it names (`v inside {m}`, `m == '{c0, c1}`) is
//! read cell by cell at the cells' sampled values, as are the operands of an
//! assignment pattern. Neither was in the site's sampled set (only selects
//! of an array were), so the assertion read them after the edge's NBA
//! region.
//!
//! Each edge copies into `v` the cell of `m` that the next edge overwrites,
//! so at every edge the sampled `v` is a sampled cell of `m`, while the cell
//! it came from already holds this edge's write. Verilator 5.036
//! (`--binary --timing --assert`) runs the `inside` check, with the arrays
//! 2-state, with no failure; it cannot compile the pattern comparison.

use xezim::simulate;

const SRC: &str = r#"
module top;
  logic clk = 0;
  always #1 clk = ~clk;
  bit [3:0] m [2];
  bit [3:0] v;
  bit wa = 0;
  bit [3:0] din = 1;
  // Two vectors written with the cells.
  bit [3:0] c0, c1;
  int f_in = 0, f_eq = 0, n = 0;
  always @(posedge clk) begin
    v <= m[!wa];
    m[wa] <= din;
    if (wa) c1 <= din; else c0 <= din;
    wa <= !wa;
    din <= din + 1;
  end
  a_in: assert property (@(posedge clk) v inside {m}) else f_in = f_in + 1;
  a_eq: assert property (@(posedge clk) m == '{c0, c1}) else f_eq = f_eq + 1;
  always @(posedge clk) n = n + 1;
  initial begin
    #20 $display("RESULT edges=%0d inside_fails=%0d eq_fails=%0d", n, f_in, f_eq);
    $finish;
  end
endmodule
"#;

#[test]
fn whole_unpacked_array_reads_are_sampled() {
    let sim = simulate(SRC, 1000).expect("simulate failed");
    let line = sim
        .output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .find(|l| l.starts_with("RESULT"))
        .unwrap_or_default();
    assert_eq!(line, "RESULT edges=10 inside_fails=0 eq_fails=0");
}
