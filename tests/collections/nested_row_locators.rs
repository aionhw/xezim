//! §7.12.1: locator methods operate on any unpacked array, including one
//! row of a nested collection (`m["a"].find_index with (..)`). Such a
//! receiver was not resolved and the call returned `'{0}` (#780).
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
fn locators_on_nested_rows() {
    let src = r#"class C;
  int m[string][int];
  int q[string][$];
  function int cnt(); return m["k"].find_index with (item > 1).size(); endfunction
endclass
function automatic int lf();
  int l[int][string];
  string r[$];
  l[1]["x"] = 5; l[1]["y"] = 0;
  r = l[1].find_index with (item > 1);
  return r.size();
endfunction
module tb;
  int m[string][int];
  int q[string][$];
  int n[int][string];
  int t[string][string][int];
  int r[$];
  string sr[$];
  int v[$];
  C c;
  initial begin
    m["a"][3] = 1; m["a"][8] = 4; m["a[b"][-2] = 9;
    q["x"] = '{4, 1, 7};
    n[5]["p"] = 3; n[5]["q"] = 9;
    t["u"]["v"][1] = 2; t["u"]["v"][6] = 8;
    r = m["a"].find_index with (item > 1);       $display("R %p", r);
    r = m["a[b"].find_index with (item > 1);     $display("R %p", r);
    r = m["zz"].find_index with (item > 1);      $display("R %p", r);
    v = q["x"].find with (item > 2);             $display("R %p", v);
    r = q["x"].find_index with (item > 2);       $display("R %p", r);
    sr = n[5].find_index with (item > 5);        $display("R %p", sr);
    r = t["u"]["v"].find_index with (item > 5);  $display("R %p", r);
    $display("R %0d", m["a"].find with (item > 1).size());
    c = new; c.m["k"][2] = 7; c.m["k"][4] = 0; c.q["z"] = '{3, 0, 5};
    r = c.m["k"].find_index with (item > 1);     $display("R %p", r);
    v = c.q["w"].find with (item > 0);           $display("R %p", v);
    v = c.q["z"].find with (item > 0);           $display("R %p", v);
    $display("R %0d %0d", c.cnt(), lf());
  end
endmodule
"#;
    assert_eq!(
        shown(src, &["R"]),
        [
            "R '{8}",
            "R '{-2}",
            "R '{}",
            "R '{4, 7}",
            "R '{0, 2}",
            r#"R '{"q"}"#,
            "R '{6}",
            "R 1",
            "R '{2}",
            "R '{}",
            "R '{3, 5}",
            "R 1 1",
        ]
    );
}
