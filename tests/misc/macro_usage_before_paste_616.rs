//! IEEE 1800-2017 §22.5.1: `` `` `` delimits tokens without white space,
//! and a macro usage ends at it. `` `JOIN(unit_`TAG, _n) `` with
//! `` `define JOIN(A, B) A``B `` is `unit_alpha_n`, not a use of a macro
//! `TAG_n` (#616, reported by git-hevangel); also one level down
//! (`` `JOIN(`OUTER, _n) ``). A name the paste itself forms (`` `ID_``n ``,
//! `ID_` no macro) is still pasted whole.
use xezim::simulate;

fn shown(src: &str, f: impl Fn(&str) -> bool) -> Vec<String> {
    simulate(src, 100)
        .expect("simulate")
        .output
        .iter()
        .map(|l| l.message.clone())
        .filter(|m| f(m))
        .collect()
}

#[test]
fn reported_testbench() {
    let out = shown(
        r#"// IEEE 1800-2017 22.5.1: a formal argument is replaced by the text of the actual
// argument, and `` delimits lexical tokens without introducing white space.
// In
//
//     `define JOIN(A, B) A``B
//     `JOIN(unit_`TAG, _n)
//
// the text after substitution is  unit_`TAG``_n . The `` ends the macro usage
// `TAG, which expands to alpha, and the result is glued to _n: unit_alpha_n.
// It is not the usage `TAG_n of a macro named TAG_n.
//
// Every line should print PASS. With the bug the file does not compile: the usage
// is left as text and the parser stops on it.
`define JOIN(A, B) A``B
`define TAG alpha
`define ROOT inst0

module t;
  int fails = 0;

  int unit_alpha_n      = 11;
  int vec_alpha_arr[3]  = '{1, 2, 3};
  int top_alpha_n       = 55;
  int top_alpha_m       = 66;
  int inst0_size        = 77;

  task automatic check(string label, int got, int expected);
    if (got == expected) $display("PASS %s", label);
    else begin $display("FAIL %s: got %0d, expected %0d", label, got, expected); fails++; end
  endtask

  initial begin
    check("usage at the end of an actual",   `JOIN(unit_`TAG, _n),       11);
    check("select after the pasted name",    `JOIN(vec_`TAG, _arr)[2],   3);
    check("two uses on one line",            `JOIN(top_`TAG, _n) + `JOIN(top_`TAG, _m), 121);
    check("usage as the whole first actual", `JOIN(`ROOT, _size),        77);
    if (fails == 0) $display("ALL PASS"); else $display("%0d FAILED", fails);
    $finish;
  end
endmodule
"#,
        |m| m.contains("PASS") || m.contains("FAIL"),
    );
    assert!(out.iter().any(|m| m == "ALL PASS"), "{out:?}");
    assert!(!out.iter().any(|m| m.contains("FAIL")), "{out:?}");
}

#[test]
fn nested_usage_and_controls() {
    assert_eq!(
        shown(
            r#"`define JOIN(A, B) A``B
`define TAG alpha
`define OUTER pre_`TAG
`define ID_n 77
`define MK(x) x``_q
module t;
  int pre_alpha_n = 5, foo_q = 6, ab = 8;
  initial begin
    $display("R %0d %0d %0d %0d", `JOIN(`OUTER, _n), `ID_``n, `MK(foo), `JOIN(a, b));
  end
endmodule
"#,
            |m| m.starts_with("R ")
        ),
        ["R 5 77 6 8"]
    );
}

/// A pasted name that is itself a macro is that macro (99 42).
#[test]
fn pasted_name_that_is_a_macro() {
    assert_eq!(
        shown(
            r#"`define A 1
`define B 2
`define AB 99
`define PASTE(x,y) `x``y
`define FOO_BAR 42
`define CONCAT(x) `FOO_``x
module t; initial $display("R %0d %0d", `PASTE(A,B), `CONCAT(BAR)); endmodule
"#,
            |m| m.starts_with("R ")
        ),
        ["R 99 42"]
    );
}
