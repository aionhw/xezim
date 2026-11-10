//! IEEE 1800-2023 §7.5.1: `new[n](src)` copies `min(n, src.size())`
//! elements of `src`, also when `src` is a ROW of an array of dynamic arrays.
//!
//! `d[1] = new[2](d[1])` (and `h.rows[1] = new[2](h.rows[1])` on a class
//! member `int rows[][]`) resized the row but lost its elements: the source
//! operand was an index expression, which the sized-`new` path took for a
//! scalar and copied nothing from (`R1 2 0 0`). A row source into another row
//! or a plain dynamic array lost its elements the same way.
//!
//! Expected values are the reference simulator's.

use xezim::simulate;

const SRC: &str = r#"
class H;
  int rows[][];
endclass
module top;
  int mrows[][];
  initial begin
    H h = new;
    int t[];
    h.rows = new[3]; h.rows[1] = '{1, 2, 3}; h.rows[0] = '{7};
    h.rows[1] = new[2](h.rows[1]);
    $display("R1 %0d %0d %0d", h.rows[1].size(), h.rows[1][0], h.rows[1][1]);
    h.rows[2] = new[4](h.rows[0]);
    $display("R2 %0d %0d %0d", h.rows[2].size(), h.rows[2][0], h.rows[2][1]);
    t = '{5, 6};
    h.rows[0] = new[3](t);
    $display("R3 %0d %0d %0d %0d", h.rows[0].size(), h.rows[0][0], h.rows[0][1], h.rows[0][2]);
    t = new[1](h.rows[1]);
    $display("R4 %0d %0d", t.size(), t[0]);
    mrows = new[2]; mrows[1] = '{1, 2, 3};
    mrows[1] = new[2](mrows[1]);
    $display("R5 %0d %0d %0d", mrows[1].size(), mrows[1][0], mrows[1][1]);
    mrows[0] = new[4](mrows[1]);
    $display("R6 %0d %0d %0d %0d", mrows[0].size(), mrows[0][0], mrows[0][1], mrows[0][2]);
    $finish;
  end
endmodule
"#;

#[test]
fn dyn_row_source_of_sized_new_is_copied() {
    let sim = simulate(SRC, 10).expect("simulate failed");
    let out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    for want in [
        "R1 2 1 2",
        "R2 4 7 0",
        "R3 3 5 6 0",
        "R4 1 1",
        "R5 2 1 2",
        "R6 4 1 2 0",
    ] {
        assert!(
            out.iter().any(|l| l.trim() == want),
            "missing {want:?} in {out:?}"
        );
    }
}
