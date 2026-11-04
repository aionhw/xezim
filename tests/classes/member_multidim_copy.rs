//! IEEE 1800-2017 §7.6, §10.9.1: a fixed multi-dimensional class member is
//! assigned whole (`c.m = a`, `m = x` in a method, `back = c.m`, `d.m =
//! c.m`), by assignment pattern, and by row (`c.m[1] = '{..}`). Every one of
//! those writes was dropped, so the member stayed 0.
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

/// Copies to and from a member, through a method, and patterns.
#[test]
fn multidim_member_whole_copies() {
    let src = r#"class C;
  int m [2][3];
  int v [3];
  int m3 [2][2][2];
  function void set(int x [2][3]); m = x; endfunction
endclass
module tb;
  int a [2][3];
  int b [3];
  int a3 [2][2][2];
  int back [2][3];
  initial begin
    C c = new; C d = new;
    foreach (a[i, j]) a[i][j] = i * 10 + j;
    foreach (a3[i, j, k]) a3[i][j][k] = i*100 + j*10 + k;
    b = '{7, 8, 9};
    c.m = a;  $display("A %p", c.m);
    c.v = b;  $display("B %p", c.v);
    d.set(a); $display("C %p", d.m);
    c.m3 = a3; $display("D %p", c.m3);
    back = c.m; $display("E %p", back);
    c.m[1] = '{5, 5, 5}; $display("F %p", c.m);
    d.m = c.m; $display("G %p", d.m);
    c.m = '{'{1,1,1},'{2,2,2}}; $display("H %p", c.m);
  end
endmodule
"#;
    let want = r#"A '{'{0, 1, 2}, '{10, 11, 12}}
B '{7, 8, 9}
C '{'{0, 1, 2}, '{10, 11, 12}}
D '{'{'{0, 1}, '{10, 11}}, '{'{100, 101}, '{110, 111}}}
E '{'{0, 1, 2}, '{10, 11, 12}}
F '{'{0, 1, 2}, '{5, 5, 5}}
G '{'{0, 1, 2}, '{5, 5, 5}}
H '{'{1, 1, 1}, '{2, 2, 2}}"#;
    let got = shown(src, &["A ", "B ", "C ", "D ", "E ", "F ", "G ", "H "]);
    assert_eq!(got.join("\n"), want, "{got:#?}");
}

/// Row writes in a method and outside, descending dimensions, 3-D members.
#[test]
fn multidim_member_row_writes() {
    let src = r#"class C;
  int m [2][3];
  int d [3:1][1:0];
  int n3 [2][2][2];
  function void rowset(); m[0] = '{7, 8, 9}; this.m[1] = '{4, 4, 4}; endfunction
endclass
module tb;
  int a [2][3];
  initial begin
    C c = new; C e = new;
    c.rowset(); $display("A %p", c.m);
    c.d = '{'{1,2},'{3,4},'{5,6}}; $display("B %p %0d %0d", c.d, c.d[3][1], c.d[1][0]);
    c.d[2] = '{9, 8}; $display("C %p", c.d);
    c.n3[1][0] = '{6, 7}; $display("D %p", c.n3);
    c.n3[0] = '{'{1,2},'{3,4}}; $display("E %p", c.n3);
    a = c.m; $display("F %p", a);
    e.n3 = c.n3; $display("G %p", e.n3);
    a[1] = c.m[0]; $display("H %p", a);
  end
endmodule
"#;
    let want = r#"A '{'{7, 8, 9}, '{4, 4, 4}}
B '{'{1, 2}, '{3, 4}, '{5, 6}} 1 6
C '{'{1, 2}, '{9, 8}, '{5, 6}}
D '{'{'{0, 0}, '{0, 0}}, '{'{6, 7}, '{0, 0}}}
E '{'{'{1, 2}, '{3, 4}}, '{'{6, 7}, '{0, 0}}}
F '{'{7, 8, 9}, '{4, 4, 4}}
G '{'{'{1, 2}, '{3, 4}}, '{'{6, 7}, '{0, 0}}}
H '{'{7, 8, 9}, '{7, 8, 9}}"#;
    let got = shown(src, &["A ", "B ", "C ", "D ", "E ", "F ", "G ", "H "]);
    assert_eq!(got.join("\n"), want, "{got:#?}");
}
