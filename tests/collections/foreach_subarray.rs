//! IEEE 1800-2017 §12.7.3: `foreach (a[i][j])` inside `foreach (a[i])`
//! iterates the selected sub-array (its loop variables map onto the
//! dimensions after the selected ones), and every loop variable runs from
//! its dimension's left bound to its right bound (`[3:1]` visits 3, 2, 1).
//!
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

/// 3-D, descending, constant-row, packed-bit, block-local and parameter-sized arrays.
#[test]
fn nested_foreach_walks_the_selected_subarray() {
    let src = r#"module tb #(parameter int R = 2);
  int a3 [2][3][2];
  int d [1:2][3:1];
  logic [3:0] b [2];
  logic [1:0][2:0] pk [2];
  int pr [R][R+1];
  initial begin
    automatic int l [2][2] = '{'{1,2},'{3,4}};
    foreach (a3[i, j, k]) a3[i][j][k] = i*100 + j*10 + k;
    foreach (a3[i]) foreach (a3[i][j, k]) $display("A %0d%0d%0d=%0d", i, j, k, a3[i][j][k]);
    foreach (a3[i, j]) foreach (a3[i][j][k]) $display("B %0d%0d%0d", i, j, k);
    foreach (a3[1][j]) $display("C %0d", j);
    foreach (d[i]) foreach (d[i][j]) $display("D %0d %0d", i, j);
    b[0] = 4'b1010; b[1] = 4'b0110;
    foreach (b[i]) foreach (b[i][j]) $display("E %0d %0d %b", i, j, b[i][j]);
    foreach (pk[i]) foreach (pk[i][j, k]) $display("F %0d %0d %0d", i, j, k);
    foreach (l[i]) foreach (l[i][j]) $display("G %0d %0d %0d", i, j, l[i][j]);
    foreach (pr[i]) foreach (pr[i][j]) $display("H %0d %0d", i, j);
  end
endmodule
"#;
    let want = r#"A 000=0
A 001=1
A 010=10
A 011=11
A 020=20
A 021=21
A 100=100
A 101=101
A 110=110
A 111=111
A 120=120
A 121=121
B 000
B 001
B 010
B 011
B 020
B 021
B 100
B 101
B 110
B 111
B 120
B 121
C 0
C 1
C 2
D 1 3
D 1 2
D 1 1
D 2 3
D 2 2
D 2 1
E 0 3 1
E 0 2 0
E 0 1 1
E 0 0 0
E 1 3 0
E 1 2 1
E 1 1 1
E 1 0 0
F 0 1 2
F 0 1 1
F 0 1 0
F 0 0 2
F 0 0 1
F 0 0 0
F 1 1 2
F 1 1 1
F 1 1 0
F 1 0 2
F 1 0 1
F 1 0 0
G 0 0 1
G 0 1 2
G 1 0 3
G 1 1 4
H 0 0
H 0 1
H 0 2
H 1 0
H 1 1
H 1 2"#;
    let got = shown(src, &["A ", "B ", "C ", "D ", "E ", "F ", "G ", "H "]);
    assert_eq!(got.join("\n"), want, "{got:#?}");
}

/// Descending outer and inner dimensions, one and several loop variables.
#[test]
fn foreach_follows_declared_direction() {
    let src = r#"module tb;
  int e [3:1][2];
  int f [3:0];
  int g [2:0][1:0][0:1];
  initial begin
    foreach (e[i, j]) $display("A %0d %0d", i, j);
    foreach (e[i]) $display("B %0d", i);
    foreach (f[i]) $display("C %0d", i);
    foreach (g[i, j, k]) $display("D %0d %0d %0d", i, j, k);
    foreach (g[i]) foreach (g[i][j]) $display("E %0d %0d", i, j);
    foreach (e[, j]) $display("F %0d", j);
  end
endmodule
"#;
    let want = r#"A 3 0
A 3 1
A 2 0
A 2 1
A 1 0
A 1 1
B 3
B 2
B 1
C 3
C 2
C 1
C 0
D 2 1 0
D 2 1 1
D 2 0 0
D 2 0 1
D 1 1 0
D 1 1 1
D 1 0 0
D 1 0 1
D 0 1 0
D 0 1 1
D 0 0 0
D 0 0 1
E 2 1
E 2 0
E 1 1
E 1 0
E 0 1
E 0 0
F 0
F 1"#;
    let got = shown(src, &["A ", "B ", "C ", "D ", "E ", "F "]);
    assert_eq!(got.join("\n"), want, "{got:#?}");
}
