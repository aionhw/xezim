//! §18.5.5: `unique {a, b}` over rand dynamic arrays makes every element of
//! every listed array pairwise distinct. Both classes are satisfiable, so
//! randomize() succeeds every time (§18.3).
#[test]
fn unique_over_several_dynamic_arrays() {
    let result = xezim::simulate(
        r#"
typedef enum bit [4:0] {C0, C1, C2, C3, C4, C5, C6, C7, C8, C9, C10, C11, C12, C13, C14, C15,
  C16, C17, C18, C19, C20, C21, C22, C23, C24, C25, C26, C27, C28, C29, C30, C31} code_t;
class two_c;
  rand code_t cnt[];
  rand code_t lim[];
  rand bit [2:0] n;
  constraint k {
    n inside {[1:2]};
    cnt.size() == n;
    lim.size() == n;
    foreach (cnt[i]) cnt[i] != C0;
    unique {cnt, lim};
  }
endclass
class three_c;
  rand bit [3:0] a[], b[], d[];
  constraint k { a.size() == 3; b.size() == 2; d.size() == 4; unique {a, b, d}; }
endclass
module t;
  initial begin
    two_c o = new();
    three_c p = new();
    int ok = 0, bad = 0;
    repeat (4) if (o.randomize()) begin
      ok++;
      if (o.cnt.size() != o.n || o.lim.size() != o.n) bad++;
      foreach (o.cnt[i]) if (o.cnt[i] == C0) bad++;
      foreach (o.cnt[i]) foreach (o.lim[j]) if (o.cnt[i] == o.lim[j]) bad++;
      foreach (o.cnt[i]) foreach (o.cnt[j]) if (i < j && o.cnt[i] == o.cnt[j]) bad++;
      foreach (o.lim[i]) foreach (o.lim[j]) if (i < j && o.lim[i] == o.lim[j]) bad++;
    end
    $display("two %0d %0d", ok, bad);
    ok = 0;
    bad = 0;
    repeat (4) if (p.randomize()) begin
      bit [3:0] all[$];
      all = {p.a, p.b, p.d};
      ok++;
      if (all.size() != 9) bad++;
      foreach (all[i]) foreach (all[j]) if (i < j && all[i] == all[j]) bad++;
    end
    $display("three %0d %0d", ok, bad);
    $finish;
  end
endmodule
"#,
        1_000_000,
    )
    .unwrap();
    let lines: Vec<_> = result
        .output
        .iter()
        .map(|entry| entry.message.as_str())
        .collect();
    assert_eq!(lines, ["two 4 0", "three 4 0"]);
}
