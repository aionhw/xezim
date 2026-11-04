//! §7.12.1: a locator method returns a queue, so queue methods (§7.10.2)
//! and reductions (§7.12.3) apply to its result. A method called directly
//! on a locator call (`q.find with (..).size()`) returned 0 (#779).
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
fn methods_on_locator_results() {
    let src = r#"class C;
  int q[$];
  string s[$];
  function int cnt(int t); return q.find(x) with (x > t).size(); endfunction
  function int scnt(); return s.find with (item == "b").size(); endfunction
endclass
function automatic int f(int t);
  int l[$] = '{3, 4, 9};
  return l.find_index with (item > t).size();
endfunction
module tb;
  int q[$];
  int r[$];
  int a[int];
  C c;
  int k;
  initial begin
    q = '{1, 5, 7}; r = '{5, 6}; a[2] = 5; a[9] = 7;
    $display("M %0d %0d", q.find_index with (item > 2).size(), (q.find_index with (item > 2)).size());
    $display("M %0d %0d %0d", q.unique().size(), q.unique.size(), q.min().size());
    $display("M %0d %0d", a.find with (item > 2).size(), q.find with (item > 2).sum());
    c = new; c.q = '{2, 8, 9}; c.s = '{"a", "b", "b"};
    $display("M %0d %0d %0d", f(3), c.cnt(5), c.scnt());
    $display("M %0d", q.find_first_index with (item > 100).size());
    $display("M %0d", q.find with (item > r.find with (item > 5).size()).size());
    for (int i = 0; i < 2; i++) k += q.find with (item > i * 5).size();
    $display("M %0d", k);
    $display("M %0d %0d", q.find with (item > 2).sum() with (item * 2),
             q.find with (item > 2).sum with (item * 2));
    $display("M %0d", a.find_index with (item > 2).sum());
  end
endmodule
"#;
    assert_eq!(
        shown(src, &["M"]),
        [
            "M 2 2", "M 3 3 1", "M 2 12", "M 2 2 2", "M 0", "M 2", "M 4", "M 24 24", "M 11",
        ]
    );
}
