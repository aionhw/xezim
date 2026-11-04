//! IEEE 1800-2023 §7.4.5, §7.4.6, §7.5.1, §7.10.1, §8.12, §13.5.1: rows of
//! a declared collection of collections under the rules their parents set —
//! a write outside a fixed parent's bounds or past a queue or dynamic row's
//! size is ignored, a re-created row starts empty, and a copied object or a
//! by-value formal keeps every row of the collection it was given.
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
fn writes_outside_a_fixed_parent_are_ignored() {
    let src = r#"module tb;
  int a [2][string];
  int b [2][$];
  int r [3:1][$];
  initial begin
    a[5]["x"] = 1; a[1]["y"] = 2;
    b[5].push_back(3); b[7][0] = 4; b[0].push_back(6);
    r[0].push_back(1); r[3].push_back(2);
    $display("A %0d %0d %p", a[5].num(), a[1].num(), a);
    $display("B %0d %0d %p", b[5].size(), b[7].size(), b);
    $display("R %0d %p", r[3].size(), r);
  end
endmodule
"#;
    let want = r#"A 0 1 '{'{}, '{"y":2}}
B 0 0 '{'{6}, '{}}
R 1 '{'{2}, '{}, '{}}"#;
    let got = shown(src, &["A ", "B ", "R "]);
    assert_eq!(got.join("\n"), want, "{got:#?}");
}

#[test]
fn writes_past_a_row_size_are_ignored() {
    let src = r#"module tb;
  int m [int][$];
  int d [int][];
  initial begin
    m[2][0] = 7; m[2][1] = 8; m[2][5] = 9;
    d[1] = new[2]; d[1][1] = 3; d[1][2] = 4; d[1][5] = 5;
    $display("M %0d %p", m[2].size(), m);
    $display("D %0d %p", d[1].size(), d);
  end
endmodule
"#;
    let want = r#"M 2 '{2:'{7, 8}}
D 2 '{1:'{0, 3}}"#;
    let got = shown(src, &["M ", "D "]);
    assert_eq!(got.join("\n"), want, "{got:#?}");
}

#[test]
fn a_recreated_row_starts_empty() {
    let src = r#"module tb;
  int q [$][int];
  initial begin
    q[0][5] = 1;
    q.delete();
    q[0][7] = 2;
    $display("Q %0d %0d %p", q[0].num(), q[0].exists(5), q);
  end
endmodule
"#;
    let got = shown(src, &["Q "]);
    assert_eq!(got.join("\n"), "Q 1 0 '{'{7:2}}", "{got:#?}");
}

#[test]
fn a_shallow_copy_keeps_its_rows() {
    let src = r#"module tb;
  class C;
    int m [int][$];
  endclass
  C c1, c2;
  initial begin
    c1 = new;
    c1.m[1].push_back(1); c1.m[1].push_back(2);
    c2 = new c1;
    c2.m[1].push_back(9);
    $display("S %0d %0d", c1.m[1].size(), c2.m[1].size());
    foreach (c2.m[k, j]) $display("F %0d %0d %0d", k, j, c2.m[k][j]);
  end
endmodule
"#;
    let want = "S 2 3\nF 1 0 1\nF 1 1 2\nF 1 2 9";
    let got = shown(src, &["S ", "F "]);
    assert_eq!(got.join("\n"), want, "{got:#?}");
}

#[test]
fn by_value_formals_keep_their_rows() {
    let src = r#"module tb;
  int g [int][int];
  int h [string][int];
  int gq [int][$];
  function automatic void f(int x [int][int]);
    int k;
    $display("F %0d %0d", x[1].first(k), k);
    x[1].delete(5);
    $display("G %0d %0d", x[1].num(), g[1].num());
  endfunction
  class C;
    function void m(int x [string][int]);
      int k;
      $display("C %0d %0d %0d", x["a"].first(k), k, x["a"].num());
    endfunction
  endclass
  function automatic void fq(int x [int][$]);
    x[1].push_back(9);
    $display("Q %0d %0d", x[1].size(), gq[1].size());
  endfunction
  C c;
  initial begin
    g[1][5] = 1; g[1][7] = 2;
    f(g);
    h["a"][5] = 1; h["a"][7] = 2;
    c = new; c.m(h);
    gq[1].push_back(3);
    fq(gq);
  end
endmodule
"#;
    let want = "F 1 5\nG 1 2\nC 1 5 2\nQ 2 1";
    let got = shown(src, &["F ", "G ", "C ", "Q "]);
    assert_eq!(got.join("\n"), want, "{got:#?}");
}
