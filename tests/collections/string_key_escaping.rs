//! §7.8.1: string keys may contain any characters, `[`, `]` and `\`
//! included. Stored names escape them; keys handed back to SV are decoded.
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

/// Bracket keys: flat, queue rows, two levels.
#[test]
fn bracket_keys_print_count_and_iterate() {
    let src = r#"module tb;
  int a[string];
  int q[string][$];
  int n[string][string];
  initial begin
    a["x[1]"] = 1; a["y]"] = 2; a["["] = 3; a["p.size"] = 4;
    $display("A %p %0d", a, a.num());
    foreach (a[k]) $display("AK %s=%0d", k, a[k]);
    q["k[0]"].push_back(7); q["k"].push_back(8); q["k"].push_back(9);
    $display("Q %p %0d %0d %0d", q, q.num(), q["k[0]"].size(), q["k"].size());
    foreach (q[k, j]) $display("QK %s %0d=%0d", k, j, q[k][j]);
    n["a[b"]["c]d"] = 5; n["a"]["b[c"] = 6;
    $display("N %p %0d %0d", n, n.num(), n["a[b"].num());
    foreach (n[i, j]) $display("NK %s %s=%0d", i, j, n[i][j]);
    n.delete("a[b");
    $display("N %p", n);
  end
endmodule
"#;
    assert_eq!(
        shown(src, &["A", "Q", "N"]),
        [
            r#"A '{"[":3, "p.size":4, "x[1]":1, "y]":2} 4"#,
            "AK [=3",
            "AK p.size=4",
            "AK x[1]=1",
            "AK y]=2",
            r#"Q '{"k":'{8, 9}, "k[0]":'{7}} 2 1 2"#,
            "QK k 0=8",
            "QK k 1=9",
            "QK k[0] 0=7",
            r#"N '{"a":'{"b[c":6}, "a[b":'{"c]d":5}} 2 1"#,
            "NK a b[c=6",
            "NK a[b c]d=5",
            r#"N '{"a":'{"b[c":6}}"#,
        ]
    );
}

/// Special-character keys: §7.8.4 order, `next`/`prev` from an absent key
/// (§7.8.5). `%p` prints strings raw in quotes, like the reference.
#[test]
fn special_keys_round_trip_in_order() {
    let src = r#"module tb;
  int a[string];
  string s;
  int r;
  initial begin
    a["\\"] = 1; a["\"q\""] = 2; a["\n"] = 3; a[""] = 4;
    a["a["] = 5; a["a]"] = 6; a["a\\"] = 7; a["\t\001"] = 8;
    $display("P %p", a);
    $display("E %0d %0d %0d", a.num(), a.exists("a["), a.exists("a("));
    if (a.first(s)) do $display("I %0d", a[s]); while (a.next(s));
    s = "a"; r = a.next(s); $display("X %0d %0d", r, a[s]);
    s = "a\\z"; r = a.prev(s); $display("X %0d %0d", r, a[s]);
    s = "zz"; r = a.prev(s); $display("X %0d %0d", r, a[s]);
    s = "zz"; r = a.next(s); $display("X %0d", r);
    a.delete("a[");
    $display("D %0d %0d", a.num(), a.exists("a["));
  end
endmodule
"#;
    assert_eq!(
        shown(src, &["P", "E", "I", "X", "D"]),
        [
            "P '{\"\":4, \"\t\u{1}\":8, \"\n\":3, \"\"q\"\":2, \"\\\":1, \"a[\":5, \"a\\\":7, \"a]\":6}",
            "E 8 1 0",
            "I 4",
            "I 8",
            "I 3",
            "I 2",
            "I 1",
            "I 5",
            "I 7",
            "I 6",
            "X 1 5",
            "X 1 7",
            "X 1 6",
            "X 0",
            "D 7 0",
        ]
    );
}

/// Initializer, class member, constraint `foreach` and `item.index` keys.
#[test]
fn initializer_class_constraint_and_index_keys() {
    let src = r#"class C;
  int m[string];
  int p[string][$];
  rand int r[string];
  constraint c { foreach (r[k]) r[k] inside {[1:3]}; }
endclass
module tb;
  int d[string] = '{"k[1]":1, "z":2};
  C c;
  int s;
  initial begin
    $display("D %p %0d", d, d["k[1]"]);
    foreach (d[k]) $display("DK %s", k);
    c = new;
    c.m["u[1]"] = 9; c.p["v]"].push_back(4); c.p["v]"].push_back(5);
    c.r["w[2]"] = 0; c.r["w"] = 0;
    void'(c.randomize());
    $display("C %p %p %0d %0d", c.m, c.p, c.p.num(), c.p["v]"].size());
    $display("R %0d", c.r["w[2]"] inside {[1:3]} && c.r["w"] inside {[1:3]});
    s = d.sum() with (item.index == "k[1]" ? item : 0);
    $display("S %0d", s);
  end
endmodule
"#;
    assert_eq!(
        shown(src, &["D", "C", "R", "S"]),
        [
            r#"D '{"k[1]":1, "z":2} 1"#,
            "DK k[1]",
            "DK z",
            r#"C '{"u[1]":9} '{"v]":'{4, 5}} 1 2"#,
            "R 1",
            "S 1",
        ]
    );
}
