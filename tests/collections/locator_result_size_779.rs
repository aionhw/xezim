//! §7.12.1, §7.10.2: a locator method returns a queue, so `size()` applies
//! to its result. `q.find_index with (item > 2).size()` read 0: the call
//! reached the scalar locator fallback instead of the locator (#779).
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
fn size_of_a_locator_result() {
    let src = r#"module tb;
  int q[$];
  int a[int];
  string s[string];
  initial begin
    q = '{1, 5, 7}; a[2] = 5; a[9] = 7; a[-4] = 1;
    s["x"] = "p"; s["y"] = "q";
    $display("S %0d", q.find_index with (item > 2).size());
    $display("S %0d", q.find with (item > 9).size());
    $display("S %0d", a.find with (item > 2).size());
    $display("S %0d", a.find_index with (item < 6).size());
    $display("S %0d", s.find_index with (item == "q").size());
    if (q.find_first with (item > 4).size() == 1) $display("S first");
  end
endmodule
"#;
    // Values checked against the same queues assigned first (the
    // reference simulator rejects `with (..).size()`).
    assert_eq!(
        shown(src, &["S"]),
        ["S 2", "S 0", "S 2", "S 2", "S 1", "S first"]
    );
}
