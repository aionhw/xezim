//! IEEE 1800-2023 §16.5.1, §16.9.3: review follow-ups to the whole-array
//! sampling of concurrent assertions.
//!
//! - An array method in an assertion (`m.sum()`, `m.and()`, `m.sum() with
//!   (...)`) reads its receiver's cells at their sampled values, and so does
//!   a row of a two-dimensional array (`m2[1] == m`). Each edge writes a cell
//!   of `m`, the same cell of the row `m2[1]` and the vector (`c0` or `c1`)
//!   that mirrors it, so the sampled operands always agree. With only `m`
//!   sampled, the row was read after the NBA region and never matched.
//! - `$stable`, `$changed` and `$past` of a whole array track a two- and a
//!   descending two-dimensional array and an array with a negative bound
//!   (`[-1:0]`), and `==`/`!=` against `$past` of one compares cell by cell.
//!
//! The reference simulator reports no failure for any of these assertions.

use xezim::simulate;

const METHODS: &str = r#"
module top;
  logic clk = 0;
  always #1 clk = ~clk;
  bit [3:0] m [2];
  bit [3:0] m2 [2][2];
  bit [3:0] c0, c1;
  bit wa = 0;
  bit [3:0] din = 1;
  int f_sum = 0, f_and = 0, f_with = 0, f_row = 0, n = 0;
  initial begin
    foreach (m2[i, j]) m2[i][j] = 0;
    m[0] = 0; m[1] = 0; c0 = 0; c1 = 0;
  end
  always @(posedge clk) begin
    m[wa] <= din;
    m2[1][wa] <= din;
    if (wa) c1 <= din; else c0 <= din;
    wa <= !wa;
    din <= din + 1;
  end
  a_sum:  assert property (@(posedge clk) m.sum() == 4'(c0 + c1)) else f_sum = f_sum + 1;
  a_and:  assert property (@(posedge clk) m.and() == (c0 & c1)) else f_and = f_and + 1;
  a_with: assert property (@(posedge clk) m.sum() with (int'(item)) == int'(c0) + int'(c1)) else f_with = f_with + 1;
  a_row:  assert property (@(posedge clk) m2[1] == m) else f_row = f_row + 1;
  always @(posedge clk) n = n + 1;
  initial begin
    #20 $display("RESULT edges=%0d sum=%0d and=%0d with=%0d row=%0d", n, f_sum, f_and, f_with, f_row);
    $finish;
  end
endmodule
"#;

const SHAPES: &str = r#"
module top;
  logic clk = 0;
  always #1 clk = ~clk;
  logic [3:0] m2 [2][2];
  logic [3:0] md [1:0][0:1];
  logic [3:0] mn [-1:0];
  logic [3:0] din = 1;
  bit we = 0;
  int n = 0, f_2d = 0, f_2dp = 0, f_dd = 0, f_neg = 0, f_negp = 0;
  initial begin
    foreach (m2[i, j]) m2[i][j] = 0;
    foreach (md[i, j]) md[i][j] = 0;
    mn[-1] = 0; mn[0] = 0;
  end
  always @(negedge clk) begin
    if (we) begin
      m2[din[0]][din[1]] <= din;
      md[din[0]][din[1]] <= din;
      mn[-din[0]] <= din;
    end
    din <= din + 1;
    we <= !we;
  end
  a_2d:  assert property (@(posedge clk) $time < 4 || $stable(m2) == !$past(we)) else f_2d = f_2d + 1;
  a_2dp: assert property (@(posedge clk) $time < 4 || (m2 == $past(m2)) == !$past(we)) else f_2dp = f_2dp + 1;
  a_dd:  assert property (@(posedge clk) $time < 4 || ($past(md) != md) == $past(we)) else f_dd = f_dd + 1;
  a_neg: assert property (@(posedge clk) $time < 4 || $changed(mn) == $past(we)) else f_neg = f_neg + 1;
  a_negp: assert property (@(posedge clk) $time < 4 || (mn == $past(mn)) == !$past(we)) else f_negp = f_negp + 1;
  always @(posedge clk) n = n + 1;
  initial begin
    #20 $display("RESULT edges=%0d 2d=%0d 2dp=%0d dd=%0d neg=%0d negp=%0d", n, f_2d, f_2dp, f_dd, f_neg, f_negp);
    $finish;
  end
endmodule
"#;

#[test]
fn array_method_receivers_are_sampled() {
    let sim = simulate(METHODS, 1000).expect("simulate failed");
    let line = sim
        .output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .find(|l| l.starts_with("RESULT"))
        .unwrap_or_default();
    assert_eq!(line, "RESULT edges=10 sum=0 and=0 with=0 row=0");
}

#[test]
fn sampled_functions_track_multi_dimensional_and_negative_bound_arrays() {
    let sim = simulate(SHAPES, 1000).expect("simulate failed");
    let line = sim
        .output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .find(|l| l.starts_with("RESULT"))
        .unwrap_or_default();
    assert_eq!(line, "RESULT edges=10 2d=0 2dp=0 dd=0 neg=0 negp=0");
}
