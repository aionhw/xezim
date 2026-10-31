//! IEEE 1800-2017 §7.4.4, §7.8, §7.10: each row of a nested collection is
//! the collection its declaration names at that depth, created on a write
//! as its parent's kind allows and keyed by its own level's index type —
//! never guessed from the cells stored under it.
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
fn whole_rows_assign_and_read() {
    let src = r#"module tb;
  int m [int][$];
  int s [string][$];
  int a [int][];
  int q [$];
  int r [$];
  int d [];
  class C;
    int p [int][$];
    function void set(int k, int v[$]); p[k] = v; endfunction
  endclass
  C o;
  initial begin
    q = '{1, 2, 3}; d = new[2]; d = '{7, 8};
    m[1] = q; $display("A %0d %p", m[1].size(), m);
    m[2] = '{4, 5}; m[3] = {q, 9}; $display("B %p", m);
    r = m[1]; m[4] = m[2]; $display("C %p %p", r, m[4]);
    m[1] = '{6}; $display("D %0d %p %0d", m[1].size(), m[1], m[1].sum());
    void'(m[3].pop_front()); $display("E %0d %p", m[3].size(), m[3]);
    s["x"] = q; $display("F %p", s);
    a[1] = d; a[2] = new[3]; $display("G %p", a);
    o = new; o.p[1] = q; o.set(2, '{4, 5});
    $display("H %0d %p", o.p[1].size(), o.p);
  end
endmodule
"#;
    let want = r#"A 3 '{1:'{1, 2, 3}}
B '{1:'{1, 2, 3}, 2:'{4, 5}, 3:'{1, 2, 3, 9}}
C '{1, 2, 3} '{4, 5}
D 1 '{6} 6
E 3 '{2, 3, 9}
F '{"x":'{1, 2, 3}}
G '{1:'{7, 8}, 2:'{0, 0, 0}}
H 3 '{1:'{1, 2, 3}, 2:'{4, 5}}"#;
    let got = shown(src, &["A ", "B ", "C ", "D ", "E ", "F ", "G ", "H "]);
    assert_eq!(got.join("\n"), want, "{got:#?}");
}

#[test]
fn rows_created_by_push_key_by_their_level() {
    let src = r#"module tb;
  int x [string][int][$];
  int q [$];
  int v;
  class C;
    int p [string][int][$];
  endclass
  C o;
  initial begin
    q = '{1, 2, 3};
    x["a"][1].push_back(1); x["a"][2].push_back(5); x["a"][2].push_back(6);
    $display("A %p %0d %0d", x, x.num(), x["a"].num());
    foreach (x["a"][k]) $display("B %0d %0d", k, x["a"][k].size());
    v = x["a"][2].pop_back(); void'(x["a"][1].pop_front());
    $display("C %0d %0d %0d %p", v, x["a"][1].size(), x["a"][2].sum(), x);
    x["b"][7] = q; x["b"][8] = x["b"][7]; void'(x["b"][8].pop_front());
    $display("D %p", x["b"]);
    x["a"].delete(1); x["b"][7].delete();
    $display("E %0d %0d %0d %0d", x["a"].num(), x["a"].exists(2), x["b"].num(), x["b"][7].size());
    o = new; o.p["k"][3].push_back(4); o.p["k"][5] = q; void'(o.p["k"][5].pop_front());
    $display("F %p %0d", o.p, o.p["k"].num());
  end
endmodule
"#;
    let want = r#"A '{"a":'{1:'{1}, 2:'{5, 6}}} 1 2
B 1 1
B 2 2
C 6 0 5 '{"a":'{1:'{}, 2:'{5}}}
D '{7:'{1, 2, 3}, 8:'{2, 3}}
E 1 1 2 0
F '{"k":'{3:'{4}, 5:'{2, 3}}} 2"#;
    let got = shown(src, &["A ", "B ", "C ", "D ", "E ", "F "]);
    assert_eq!(got.join("\n"), want, "{got:#?}");
}

#[test]
fn copies_rows_as_collections() {
    let src = r#"module tb;
  int m [int][$];
  int n [int][$];
  int x [string][int][$];
  int y [string][int][$];
  class C;
    int p [int][$];
  endclass
  C o;
  initial begin
    m[0].push_back(2); m[1].push_back(3); m[1].push_back(4);
    n = m; $display("A %p %0d %0d %0d", n, n.num(), n[0].size(), n[1].size());
    void'(n[1].pop_front()); $display("B %p %p", n, m);
    n[5].push_back(9); n = m; $display("C %p %0d", n, n.num());
    x["a"][1].push_back(1); x["b"][2] = '{5, 6}; y = x;
    $display("D %p %0d", y, y["b"][2].sum());
    o = new; o.p = m; $display("E %p %0d", o.p, o.p[1].size());
  end
endmodule
"#;
    let want = r#"A '{0:'{2}, 1:'{3, 4}} 2 1 2
B '{0:'{2}, 1:'{4}} '{0:'{2}, 1:'{3, 4}}
C '{0:'{2}, 1:'{3, 4}} 2
D '{"a":'{1:'{1}}, "b":'{2:'{5, 6}}} 11
E '{0:'{2}, 1:'{3, 4}} 2"#;
    let got = shown(src, &["A ", "B ", "C ", "D ", "E "]);
    assert_eq!(got.join("\n"), want, "{got:#?}");
}

/// §7.8.6 Table 7-1: reading a missing row creates nothing. §7.4.5,
/// §7.5.1: a write past a dynamic array's end is ignored. §7.10.1: a queue
/// of queues grows only at `$+1`. §11.4.1: a receiver's key is evaluated
/// once. A function-local collection follows its declaration too.
#[test]
fn rows_are_created_as_their_parent_allows() {
    let src = r#"module tb;
  int m [int][$];
  int d [][$];
  int qq [$][$];
  int calls;
  int x;
  function int k(); calls++; return 1; endfunction
  function automatic int lf();
    int l [int][$];
    l[2].push_back(1); l[2].push_back(2); void'(l[2].pop_front());
    l[3] = '{7, 8, 9};
    return l[2].size() * 10 + l[3].size();
  endfunction
  initial begin
    x = m[9].size(); $display("A %0d %0d %0d", x, m.num(), m.exists(9));
    d = new[1]; d[3].push_back(1); $display("B %0d %0d", d.size(), d[3].size());
    qq[0].push_back(1); qq[1].push_back(2); qq[5].push_back(3);
    $display("C %0d %p", qq.size(), qq);
    calls = 0; m[k()].push_back(1); void'(m[k()].pop_front());
    $display("D %0d %0d", calls, m[1].size());
    $display("E %0d", lf());
  end
endmodule
"#;
    let want = r#"A 0 0 0
B 1 0
C 2 '{'{1}, '{2}}
D 2 0
E 13"#;
    let got = shown(src, &["A ", "B ", "C ", "D ", "E "]);
    assert_eq!(got.join("\n"), want, "{got:#?}");
}

/// §7.4.5, §7.10.1: locals of an unnamed block outside any subroutine keep
/// their bare names and follow their declarations too: a queue of queues
/// grows at `$+1`, and a popped queue row of an associative array shrinks.
#[test]
fn block_locals_follow_their_declaration() {
    let src = r#"module tb;
  initial begin
    int lq [$][$];
    int la [int][$];
    lq[0].push_back(1);
    la[3] = '{1, 2}; void'(la[3].pop_front());
    $display("R %0d %p %0d", lq.size(), lq, la[3].size());
  end
endmodule
"#;
    assert_eq!(shown(src, &["R "]), ["R 1 '{'{1}} 1"]);
}

/// §7.8.7: a write into an associative row of a queue of associative
/// arrays stores the entry — at module scope, in a function and in an
/// unnamed block alike. A local's row was never created, so the element
/// read back x and the row's `num()` stayed 0.
#[test]
fn local_queues_of_associative_rows_take_writes() {
    let src = r#"module tb;
  int g [$][string];
  function automatic void f();
    int l [$][string];
    l.push_back('{}); l[0]["q"] = 5;
    $display("L %0d %0d %0d", l[0]["q"], l[0].num(), l[0].exists("q"));
  endfunction
  initial begin
    g.push_back('{}); g[0]["q"] = 5;
    $display("G %0d %0d %0d", g[0]["q"], g[0].num(), g[0].exists("q"));
    f();
    begin
      int b [$][string];
      b.push_back('{}); b[0]["q"] = 5;
      $display("B %0d %0d %0d", b[0]["q"], b[0].num(), b[0].exists("q"));
    end
  end
endmodule
"#;
    assert_eq!(
        shown(src, &["G ", "L ", "B "]),
        ["G 5 1 1", "L 5 1 1", "B 5 1 1"]
    );
}
