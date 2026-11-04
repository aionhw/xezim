//! §7.8.2: string indices are ordered lexicographically, also when every key
//! looks like a number. Such an array was ordered numerically and `first()`
//! wrote the key's integer value into its string argument (#781).
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
fn numeric_looking_string_keys_order_lexicographically() {
    let src = r#"class C; int m[string]; endclass
module tb;
  int a[string];
  int t[string][int];
  int n[int];
  string s;
  string sq[$];
  int i;
  C c;
  initial begin
    a["9"] = 1; a["10"] = 2; a["-3"] = 3;
    void'(a.first(s)); $display("S %s", s);
    void'(a.next(s));  $display("S %s", s);
    void'(a.last(s));  $display("S %s", s);
    void'(a.prev(s));  $display("S %s", s);
    $display("S %p", a);
    foreach (a[k]) $display("S %s", k);
    sq = a.find_index with (item > 0); $display("S %p", sq);
    t["9"][1] = 1; t["10"][2] = 2;
    foreach (t[k, j]) $display("S %s %0d", k, j);
    $display("S %p", t);
    c = new; c.m["9"] = 1; c.m["10"] = 2;
    void'(c.m.first(s)); $display("S %s %p", s, c.m);
    n[9] = 1; n[10] = 2; n[-3] = 3;
    void'(n.first(i)); $display("I %0d %p", i, n);
  end
endmodule
"#;
    assert_eq!(
        shown(src, &["S", "I"]),
        [
            "S -3",
            "S 10",
            "S 9",
            "S 10",
            r#"S '{"-3":3, "10":2, "9":1}"#,
            "S -3",
            "S 10",
            "S 9",
            r#"S '{"-3", "10", "9"}"#,
            "S 10 2",
            "S 9 1",
            r#"S '{"10":'{2:2}, "9":'{1:1}}"#,
            r#"S 10 '{"10":2, "9":1}"#,
            "I -3 '{-3:3, 9:1, 10:2}",
        ]
    );
}
