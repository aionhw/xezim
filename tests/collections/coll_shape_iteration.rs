//! IEEE 1800-2017 §12.7.3, §21.2.1.7, §7.9: iteration and printing of a
//! nested collection follow its declared shape — every level, whatever the
//! outer dimension's kind. `foreach (d[i, k, j])` walks a queue or dynamic
//! level by index and an associative level by its keys in §7.8.4 order, each
//! loop variable taking that level's index type; `%p` prints each row as the
//! collection it is. A dynamic or queue outer dimension over associative rows
//! printed every row as `0`, and foreach read a queue row's indices as keys.
//! The `%p` layout of an associative array is implementation-defined; the
//! expected values use the house style (`'{key:value}`, see
//! assoc_coll_rows.rs).
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

/// Every shape, module scope: `%p`, `num`/`size`, `exists`, `first`/`next`,
/// and a full-depth and partial `foreach`.
#[test]
fn module_shapes_iterate_and_print_by_declaration() {
    let src = r#"module tb;
  int a [int][$];
  int a2 [string][int][$];
  int d [][string][$];
  int f [2][string][$];
  int q [$][int];
  string s;
  int i, k;
  initial begin
    a[3].push_back(1); a[1].push_back(2); a[1].push_back(3);
    a2["y"][2].push_back(1); a2["x"][5] = '{7};
    d = new[2]; d[0]["a"].push_back(1); d[1]["b"] = '{4, 5}; d[1]["a"].push_back(6);
    f[0]["k"].push_back(1); f[1]["m"] = '{2, 3};
    q.push_back('{}); q.push_back('{}); q[0][7] = 1; q[1][2] = 2; q[1][9] = 3;
    $display("P %p %p", a, a2);
    $display("P %p %p %p", d, f, q);
    $display("N %0d %0d %0d %0d %0d %0d", a.num(), a2["x"].num(), d[1].num(), d[1].size(), f[1].num(), q[1].num());
    $display("E %0d %0d %0d %0d %0d", d[1].exists("a"), d[1].exists("z"), f[0].exists("k"), q[1].exists(9), a2.exists("y"));
    void'(d[1].first(s)); $display("F %s", s); void'(d[1].next(s)); $display("F %s", s);
    void'(q[1].first(k)); $display("F %0d", k); void'(q[1].next(k)); $display("F %0d", k);
    foreach (d[i, s, k]) $display("D %0d %s %0d %0d", i, s, k, d[i][s][k]);
    foreach (d[i, s]) $display("R %0d %s", i, s);
    foreach (a2[s, i, k]) $display("X %s %0d %0d", s, i, k);
    foreach (q[i, k]) $display("Q %0d %0d", i, k);
    foreach (f[i, s]) $display("G %0d %s", i, s);
  end
endmodule
"#;
    let want = r#"P '{1:'{2, 3}, 3:'{1}} '{"x":'{5:'{7}}, "y":'{2:'{1}}}
P '{'{"a":'{1}}, '{"a":'{6}, "b":'{4, 5}}} '{'{"k":'{1}}, '{"m":'{2, 3}}} '{'{7:1}, '{2:2, 9:3}}
N 2 1 2 2 1 2
E 1 0 1 1 1
F a
F b
F 2
F 9
D 0 a 0 1
D 1 a 0 6
D 1 b 0 4
D 1 b 1 5
R 0 a
R 1 a
R 1 b
X x 5 0
X y 2 0
Q 0 7
Q 1 2
Q 1 9
G 0 k
G 1 m"#;
    let got = shown(src, &["P ", "N ", "E ", "F ", "D ", "R ", "X ", "Q ", "G "]);
    assert_eq!(got.join("\n"), want, "{got:#?}");
}

/// §7.8.4: signed keys in numerical order and the loop variable at the
/// key's width; §12.7.3: an omitted loop variable skips its binding, not its
/// level; a dynamic level under an associative one.
#[test]
fn key_types_omitted_variables_and_mixed_levels() {
    let src = r#"module tb;
  int a [int][$];
  byte sg [shortint][$];
  int ad [int][][$];
  int i, k;
  shortint h;
  initial begin
    a[3].push_back(1); a[-2].push_back(2); a[-2].push_back(3);
    foreach (a[i, k]) $display("A %0d %0d %0d", i, k, a[i][k]);
    foreach (a[, k]) $display("O %0d", k);
    sg[-5].push_back(1); sg[7].push_back(2); sg[-30000].push_back(3);
    foreach (sg[h, k]) $display("S %0d %0d", h, k);
    ad[1] = new[2]; ad[1][1].push_back(4); ad[1][0].push_back(5);
    $display("M %p", ad);
    foreach (ad[i, k, h]) $display("M %0d %0d %0d", i, k, h);
  end
endmodule
"#;
    let want = r#"A -2 0 2
A -2 1 3
A 3 0 1
O 0
O 1
O 0
S -30000 0
S -5 0
S 7 0
M '{1:'{'{5}, '{4}}}
M 1 0 0
M 1 1 0"#;
    let got = shown(src, &["A ", "O ", "S ", "M "]);
    assert_eq!(got.join("\n"), want, "{got:#?}");
}

/// A class member walked from a method and printed through a handle, and a
/// function-local collection of collections.
#[test]
fn class_members_and_locals() {
    let src = r#"class C;
  int p [][string][$];
  function void show(); foreach (p[i, s, k]) $display("C %0d %s %0d", i, s, p[i][s][k]); endfunction
endclass
module tb;
  C o;
  function automatic void lf();
    int l [][string][$];
    l = new[1]; l[0]["z"].push_back(9); l[0]["b"].push_back(8);
    $display("L %p %0d", l, l[0].num());
    foreach (l[i, s, k]) $display("L %0d %s %0d", i, s, l[i][s][k]);
  endfunction
  initial begin
    o = new; o.p = new[2]; o.p[1]["x"] = '{3, 4};
    o.show(); $display("C %p", o.p);
    lf();
  end
endmodule
"#;
    let want = r#"C 1 x 3
C 1 x 4
C '{'{}, '{"x":'{3, 4}}}
L '{'{"b":'{8}, "z":'{9}}} 2
L 0 b 8
L 0 z 9"#;
    let got = shown(src, &["C ", "L "]);
    assert_eq!(got.join("\n"), want, "{got:#?}");
}
