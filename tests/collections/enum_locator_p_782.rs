//! §7.12.1, §21.2.1.6: `%p` of a locator call prints an enum by its label:
//! the `_index` result of an enum-keyed associative array is a queue of
//! the index type, and `find` on an enum array returns enum elements. Both
//! printed the encodings (#782).
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
fn locator_results_print_enum_labels() {
    let src = r#"typedef enum {RED, GREEN, BLUE} col_t;
class C;
  int m[col_t];
endclass
module tb;
  int e[col_t];
  col_t cq[$];
  C c;
  initial begin
    e[BLUE] = 1; e[RED] = 0; e[GREEN] = 1;
    $display("E %p", e.find_index with (item == 1));
    $display("E %p", e.find_first_index with (item == 0));
    cq = '{BLUE, RED, BLUE};
    $display("E %p", cq.find with (item != RED));
    $display("E %p", cq.min());
    $display("E %p", cq.find_index with (item == BLUE));
    c = new; c.m[GREEN] = 4; c.m[RED] = 2;
    $display("E %p", c.m.find_index with (item > 3));
  end
endmodule
"#;
    // Values from the reference simulator.
    assert_eq!(
        shown(src, &["E"]),
        [
            "E '{GREEN, BLUE}",
            "E '{RED}",
            "E '{BLUE, BLUE}",
            "E '{RED}",
            "E '{0, 2}",
            "E '{GREEN}",
        ]
    );
}
