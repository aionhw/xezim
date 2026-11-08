//! IEEE 1800-2023 §20.7 / §20.6.2: the array queries and `$bits` of a ROW of
//! a dynamic array or queue, and of a class member dynamic array or queue of
//! fixed rows.
//!
//! * `$size(h.raw[0])` of a class member `bit [7:0] raw[][2]` returned the
//!   element's 8 bits instead of the row's 2 elements: the queries looked the
//!   handle-qualified member up under no instance store, so neither
//!   `$unpacked_dimensions(h.raw)` nor `$size(h.raw, 2)` found its shape.
//! * `$bits` of a row of a module-scope dynamic array (`$bits(d[i])` on
//!   `int d[][]`, `$bits(r[i])` on `bit [7:0] r[][2]`) read one element's
//!   width instead of the row's.
//!
//! Expected values are the reference simulator's.

use xezim::simulate;

const SRC: &str = r#"
class H;
  int rows[][];
  bit [7:0] raw[][2];
  int q2[$][3];
  function void probe();
    $display("M %0d %0d %0d %0d", $size(raw[0]), $size(raw), $size(raw, 2),
             $unpacked_dimensions(raw));
  endfunction
endclass
module top;
  int mrows[][];
  bit [7:0] mraw[][2];
  initial begin
    H h = new;
    h.rows = new[3]; h.rows[0] = '{5, 6, 0}; h.rows[1] = '{1, 2};
    mrows = new[2]; mrows[1] = '{1, 2};
    h.raw = new[2];
    mraw = new[2];
    h.probe();
    $display("S1 %0d %0d %0d %0d", $size(h.raw[0]), $size(h.raw), $size(h.raw, 2),
             $unpacked_dimensions(h.raw));
    $display("S2 %0d %0d %0d %0d", $left(h.raw[1]), $right(h.raw[1]), $high(h.raw[0]),
             $dimensions(h.raw));
    $display("S3 %0d %0d %0d %0d", $size(mraw[0]), $size(mraw, 2), $size(h.rows[1]),
             $size(h.rows));
    h.q2.push_back('{1, 2, 3});
    $display("S4 %0d %0d", $size(h.q2[0]), $size(h.q2));
    $display("B %0d %0d", $bits(mraw[0]), $bits(mrows[1]));
    $finish;
  end
endmodule
"#;

#[test]
fn dyn_row_queries_and_bits() {
    let sim = simulate(SRC, 10).expect("simulate failed");
    let out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    for want in [
        "M 2 2 2 2",
        "S1 2 2 2 2",
        "S2 0 1 1 3",
        "S3 2 2 2 3",
        "S4 3 1",
        "B 16 64",
    ] {
        assert!(
            out.iter().any(|l| l.trim() == want),
            "missing {want:?} in {out:?}"
        );
    }
}
