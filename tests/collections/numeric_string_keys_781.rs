//! §7.8.2, §7.9: a string-keyed associative array is ordered
//! lexicographically however its keys read. Keys that all looked like
//! integers were sorted numerically, and `first()` wrote the key as a
//! number into its string argument (#781).
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
fn numeric_looking_string_keys_order_as_strings() {
    let src = r#"module tb;
  int a[string];
  string s;
  initial begin
    a["9"] = 1; a["10"] = 2; a["-3"] = 3;
    void'(a.first(s)); $display("K first %s", s);
    void'(a.last(s));  $display("K last %s", s);
    s = "10"; void'(a.next(s)); $display("K next %s", s);
    s = "9";  void'(a.prev(s)); $display("K prev %s", s);
    $display("K %p", a);
    foreach (a[k]) $display("K fe %s", k);
  end
endmodule
"#;
    // Values from the reference simulator.
    assert_eq!(
        shown(src, &["K"]),
        [
            "K first -3",
            "K last 9",
            "K next 9",
            "K prev 10",
            r#"K '{"-3":3, "10":2, "9":1}"#,
            "K fe -3",
            "K fe 10",
            "K fe 9",
        ]
    );
}
