//! §7.12.1, §7.8.1: a locator method on one row of a nested associative
//! array (`m["a"].find_index with (..)`) searches that row, an associative
//! array of the row's own index type. The receiver was not resolved to its
//! row, so the call was not evaluated as a locator (#780).
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
fn locators_on_a_nested_row() {
    let src = r#"module tb;
  int m[string][int];
  int n[int][string];
  int r[$];
  string sr[$];
  initial begin
    m["a"][3] = 1; m["a"][8] = 4; m["a"][-2] = 6; m["b"][1] = 9;
    n[5]["p"] = 2; n[5]["q"] = 7;
    r = m["a"].find_index with (item > 1);        $display("R %p", r);
    r = m["a"].find with (item > 1);              $display("R %p", r);
    r = m["b"].find_first_index with (item == 9); $display("R %p", r);
    sr = n[5].find_index with (item > 5);         $display("R %p", sr);
    r = m["a"].max();                             $display("R %p", r);
    $display("R %p", m["a"].find_last_index with (item.index > 0));
  end
endmodule
"#;
    // Values from the reference simulator.
    assert_eq!(
        shown(src, &["R"]),
        [
            "R '{-2, 8}",
            "R '{6, 4}",
            "R '{1}",
            r#"R '{"q"}"#,
            "R '{6}",
            "R '{8}",
        ]
    );
}
