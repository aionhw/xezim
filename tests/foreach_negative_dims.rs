//! §12.7.3 — `foreach` over arrays with negative / descending declared bounds
//! and over the element's PACKED dimensions.
//!
//! Two bugs fixed:
//!  1. The index variable (implicitly `int`, signed) was stored via `from_u64`,
//!     so a bound like `[-2:-4]` read back as 4294967294 — breaking `%0d`,
//!     arithmetic, and negative array indexing in the body.
//!  2. A loop var beyond the unpacked dims pushed the FLAT element width (e.g.
//!     160 bits for `[-1:-5][31:0]`) as its range, iterating every bit instead
//!     of the packed DIMENSION's index set. Now driven by `packed_full_dims`.

use xezim::simulate;

fn line(src: &str) -> Vec<String> {
    simulate(src, 1000)
        .expect("sim")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect()
}

#[test]
fn foreach_negative_unpacked_index_is_signed() {
    let src = r#"
module tb;
  logic [31:0] a [-2:-4];
  initial begin
    string s = "";
    foreach (a[i]) s = {s, $sformatf("%0d,", i)};
    $display("IDX=%s", s);
  end
endmodule
"#;
    let out = line(src);
    assert!(
        out.iter().any(|m| m == "IDX=-2,-3,-4,"),
        "foreach negative unpacked index not signed/ordered; got {:?}",
        out
    );
}

#[test]
fn foreach_packed_dimension_index_set() {
    // d[i,j]: i over unpacked [-2:-4], j over the packed dim [-1:-5].
    let src = r#"
module tb;
  logic [-1:-5][31:0] d [-2:-4];
  int fails = 0;
  initial begin
    foreach (d[i,j]) d[i][j] = (i*10) + j;
    foreach (d[i,j]) begin
      int e; e = (i*10) + j;
      if (d[i][j] !== e) fails++;
    end
    $display("FAILS=%0d", fails);
  end
endmodule
"#;
    let out = line(src);
    assert!(
        out.iter().any(|m| m == "FAILS=0"),
        "foreach over packed dimension wrong; got {:?}",
        out
    );
}

#[test]
fn foreach_positive_still_works() {
    // Regression guard for the common 0-based case.
    let src = r#"
module tb;
  logic [3:0] a [0:1];
  string s = "";
  initial begin
    foreach (a[i,j]) s = {s, $sformatf("%0d.%0d ", i, j)};
    $display("R=%s", s);
  end
endmodule
"#;
    let out = line(src);
    // i over [0:1], j over packed [3:0] => 8 tuples, j from 0..3.
    assert!(
        out.iter().any(|m| m.starts_with("R=0.0 0.1 0.2 0.3 1.0 1.1 1.2 1.3")),
        "positive foreach regressed; got {:?}",
        out
    );
}
