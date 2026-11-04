//! §7.8.4: an associative array whose index type is wider than 64 bits
//! names one element per index value, however wide the index expression:
//! `w[5]` and `w[100'h5]` are the same element. A narrow index was keyed
//! by its decimal text and a wide one by its hex digits, so `exists`,
//! `foreach` and the locators disagreed, and `next()` from such a key
//! restarted at the first key forever.
use xezim::simulate;

/// The `$display` lines of `src` that start with one of `prefixes`.
fn shown(src: &str, prefixes: &[&str]) -> Vec<String> {
    simulate(src, 1000)
        .expect("simulate")
        .output
        .iter()
        .map(|l| l.message.clone())
        .filter(|m| prefixes.iter().any(|p| m.starts_with(p)))
        .collect()
}

#[test]
fn wide_keys_name_one_element() {
    let src = r#"module tb;
  bit [99:0] w[bit [99:0]];
  bit [99:0] k;
  bit [99:0] wk[$];
  initial begin
    w[5] = 9; w[100'h1_0000_0000_0000_0000_0000] = 7; w[100'h7] = 3;
    $display("W %0d %0d %0d %0d", w.exists(5), w.exists(100'h5), w.exists(7), w.num());
    foreach (w[i]) $display("W fe %0h %0d", i, w[i]);
    if (w.first(k)) do $display("W it %0h", k); while (w.next(k));
    k = 6; if (w.next(k)) $display("W nx %0h", k);
    k = 6; if (w.prev(k)) $display("W pv %0h", k);
    wk = w.find_index with (item > 5); $display("W %0h %0h", wk[0], wk[1]);
    $display("W %0d", w.sum() with (int'(item.index[3:0])));
  end
endmodule
"#;
    // Values from the reference simulator.
    assert_eq!(
        shown(src, &["W"]),
        [
            "W 1 1 1 3",
            "W fe 5 9",
            "W fe 7 3",
            "W fe 100000000000000000000 7",
            "W it 5",
            "W it 7",
            "W it 100000000000000000000",
            "W nx 7",
            "W pv 5",
            "W 5 100000000000000000000",
            "W 12",
        ]
    );
}
