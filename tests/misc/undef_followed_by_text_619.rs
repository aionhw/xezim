//! IEEE 1800-2017 §22.5.2: `` `undef `` names one macro; the text after it
//! on the line is ordinary source. It was dropped, which also lost the text
//! after a macro whose body ends in `` `undef `` (#619, reported by
//! git-hevangel).
use xezim::simulate;

#[test]
fn text_after_undef_is_kept() {
    let out: Vec<String> = simulate(
        r#"// IEEE 1800-2017 22.5.2: undefine_compiler_directive ::= `undef text_macro_identifier
// The directive names ONE macro. Whatever follows the identifier on the same line
// is ordinary source text: it is not part of the name, and it is not dropped.
//
// Two shapes:
//   1. `undef LIMIT int tail_one = 5;      the declaration after the directive
//   2. a macro whose body defines a helper, uses it and undefines it, used in
//      the middle of a line:  `ITEM(RED) `ITEM(GREEN) BLUE   - the text that
//      follows each expansion lands on the line of its last `undef.
//
// Every line should print PASS. With the bug the text after `undef is dropped and
// the macro stays defined, so the names declared there do not exist.
`define LIMIT 8

`define ITEM(n) \
  `define CUR n \
  `CUR, \
  `undef CUR

module t;
  int fails = 0;

`undef LIMIT int tail_one = 5;

  typedef enum {`ITEM(RED) `ITEM(GREEN) BLUE} color_e;

  task automatic check(string label, int got, int expected);
    if (got == expected) $display("PASS %s", label);
    else begin $display("FAIL %s: got %0d, expected %0d", label, got, expected); fails++; end
  endtask

  initial begin
    color_e c;
    check("text after `undef is kept",           tail_one,        5);
`ifdef LIMIT
    check("macro named by `undef is undefined",  1,               0);
`else
    check("macro named by `undef is undefined",  0,               0);
`endif
    check("enum member before the generated ones", RED,           0);
    check("enum member from the second expansion", GREEN,         1);
    check("enum member after the last expansion",  BLUE,          2);
    c = BLUE;
    check("enum value is usable",                c,               2);
`ifdef CUR
    check("helper macro is undefined again",     1,               0);
`else
    check("helper macro is undefined again",     0,               0);
`endif
    if (fails == 0) $display("ALL PASS"); else $display("%0d FAILED", fails);
    $finish;
  end
endmodule
"#,
        100,
    )
    .expect("simulate")
    .output
    .iter()
    .map(|l| l.message.clone())
    .filter(|m| m.contains("PASS") || m.contains("FAIL"))
    .collect();
    assert!(out.iter().any(|m| m == "ALL PASS"), "{out:?}");
    assert!(!out.iter().any(|m| m.contains("FAIL")), "{out:?}");
}
