//! IEEE 1800-2017 §18.5.8.1, §12.7.3: a `foreach` constraint over an empty
//! dynamic dimension adds no constraint. The solver walked the empty
//! dimension `(0, -1)` as the descending range -1..0, so an item on the
//! indices failed, and a size constraint did not reach the loop (#599,
//! reported by akashlevy, with the patch applied here).
use xezim::simulate;

#[test]
fn foreach_over_an_empty_dynamic_dimension() {
    let out: Vec<String> = simulate(
        r#"// §18.5.8.1: a foreach constraint over an empty array adds no constraint.
module t;
  int b[][2];
  int r1, r2, r3, r4;
  initial begin
    r1 = std::randomize(b) with { b.size() == 0; foreach (b[i, j]) i >= 0; };
    r2 = std::randomize(b) with { b.size() == 0; foreach (b[i, j]) 0; };
    r3 = std::randomize(b) with { b.size() == 0; foreach (b[i, j]) b[i][j] > 5; };
    r4 = std::randomize(b) with { b.size() == 2; foreach (b[i, j]) i >= 0; };
    $display("r=%0d %0d %0d %0d size=%0d expect r=1 1 1 1 size=2", r1, r2, r3, r4, b.size());
  end
endmodule

"#,
        100,
    )
    .expect("simulate")
    .output
    .iter()
    .map(|l| l.message.clone())
    .filter(|m| m.contains("expect"))
    .collect();
    assert_eq!(out, ["r=1 1 1 1 size=2 expect r=1 1 1 1 size=2"]);
}
