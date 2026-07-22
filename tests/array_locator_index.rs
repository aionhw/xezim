//! IEEE 1800-2017 §7.12.2 — array locator/reduction methods return a QUEUE, so
//! indexing the result inline (`a.max()[0]`, `(a.find with (..))[0]`) must
//! index that queue, not bit-select the scalar the reduction path returns.
//! Previously `a.max()[0]` and `a.min()[0]` both returned 1 (bit 0 of the max
//! / min value), and a paren-wrapped `find` result indexed to 0.

use xezim::simulate;

fn line(src: &str) -> Vec<String> {
    simulate(src, 100)
        .expect("sim")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect()
}

#[test]
fn locator_result_indexed_inline() {
    let src = r#"
module t;
  int a[6] = '{3,1,4,1,5,9};
  int q[$];
  initial begin
    q = a.unique(); $display("U=%p", q);
    q = a.find with (item > 3); $display("F=%p", q);
    $display("M=%0d,%0d", a.max()[0], a.min()[0]);
    $display("UI=%0d,%0d", a.unique()[0], a.unique()[2]);
    $display("FI=%0d", a.find_index with (item == 4)[0]);
    $display("FF=%0d", a.find_first with (item > 3)[0]);
    $display("PF=%0d", (a.find with (item > 2))[1]);  // {3,4,5,9}[1]=4
    $finish;
  end
endmodule
"#;
    let out = line(src);
    let want = [
        "U='{3, 1, 4, 5, 9}", // unique preserves first-seen order
        "F='{4, 5, 9}",
        "M=9,1",   // max=9 not 1, min=1
        "UI=3,4",  // unique()[0]=3, unique()[2]=4
        "FI=2",    // index of the first 4
        "FF=4",    // first element > 3
        "PF=4",    // paren-wrapped find, index 1
    ];
    for w in want {
        assert!(out.iter().any(|m| m == w), "missing {:?}; got {:?}", w, out);
    }
}
