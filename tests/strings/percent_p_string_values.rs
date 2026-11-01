//! §21.2.1.7: `%p` of a string-valued CALL or concatenation printed the
//! string's bytes as one huge decimal number, and `%0p` of any string kept
//! the quotes the reference drops.
//!
//! UVM AVIPs log their agent configs with
//! `$sformatf("cfg = \n %0p", cfg.sprint())`; every such table came out as a
//! 1000-digit integer. The typed `%p` renderers only know variables, so a
//! call or concatenation fell through to the packed-integer default. The
//! compact form now also prints a lone string without quotes. Every
//! expectation below was cross-checked against the reference simulator.
//!
//! Expectations updated for #566: `%p` prints vector members as `'h` hex,
//! `%0p` as `%p`, and no space before an associative array's closing brace.

use xezim::simulate;

#[test]
fn string_calls_and_compact_strings() {
    let sim = simulate(
        r#"
class C;
  string nm = "cc";
  function string sprint();
    return {"line1\n", nm};
  endfunction
endclass
function string f();
  return "fx";
endfunction
module top;
  initial begin
    C c = new;
    string s = "ab";
    $display("A[%p][%0p]", s, s);
    $display("B[%p][%0p]", f(), f());
    $display("C[%p][%0p]", c.sprint(), c.sprint());
    $display("%s", $sformatf("D = \n %0p", c.sprint()));
    $display("E[%p]", {s, "z"});
  end
endmodule
"#,
        100,
    )
    .expect("simulate failed");
    let text = sim
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect::<Vec<_>>()
        .join("\n");
    for w in [
        "A[\"ab\"][\"ab\"]",
        "B[\"fx\"][\"fx\"]",
        "C[\"line1\ncc\"][\"line1\ncc\"]",
        "D = \n \"line1\ncc\"",
        "E[\"abz\"]",
    ] {
        assert!(text.contains(w), "expected {w:?} in:\n{text}");
    }
}

/// §7.12.1, §21.2.1.6: `%p` of a locator call prints an enum result (the
/// key of an enum-indexed array, or an enum element) by its labels (#782).
/// White space in `%p` output is implementation dependent; only the text
/// without it is checked.
#[test]
fn locator_calls_print_enum_labels() {
    let sim = simulate(
        r#"
package p; typedef enum {X0, X1} pe_t; endpackage
class C; typedef enum {LO, HI} lv_t; int m[lv_t]; endclass
module top;
  typedef enum {RED, GREEN, BLUE} col_t;
  typedef col_t alias_t;
  int e[col_t];
  int ea[alias_t];
  col_t cq[$];
  col_t ce[int];
  int pe[p::pe_t];
  C c;
  initial begin
    e[BLUE] = 1; e[RED] = 0; cq = '{GREEN, BLUE}; ce[4] = BLUE; pe[p::X1] = 3;
    c = new; c.m[C::HI] = 5;
    $display("A%p", e.find_index with (item == 1));
    $display("B%p", cq.find with (item != RED));
    $display("C%p", ce.find with (item == BLUE));
    $display("D%p", cq.find_index with (item == BLUE));
    $display("E%p", pe.find_index with (item == 3));
    $display("F%p", c.m.find_index with (item == 5));
    ea[GREEN] = 1;
    $display("G%p", ea.find_index with (item == 1));
    f();
  end
  function automatic void f();
    int le[col_t];
    le[BLUE] = 1;
    $display("H%p", le.find_index with (item == 1));
  endfunction
endmodule
"#,
        100,
    )
    .expect("simulate failed");
    let lines: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.split_whitespace().collect())
        .collect();
    for w in [
        "A'{BLUE}",
        "B'{GREEN,BLUE}",
        "C'{BLUE}",
        "D'{1}",
        "E'{X1}",
        "F'{HI}",
        "G'{GREEN}",
        "H'{BLUE}",
    ] {
        assert!(lines.iter().any(|l| l == w), "expected {w:?} in {lines:?}");
    }
}
