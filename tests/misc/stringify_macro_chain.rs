//! IEEE 1800-2023 §22.5.1: a `` `" `` "indicates that the expansion shall
//! include the quotation mark, substitution of actual arguments, and
//! expansions of embedded macros", however deep the chain of macros in the
//! region. With `` `define INST `TOP.u0 `` and `` `define STR(p) `"p`" ``,
//! `` `STR(`INST.x) `` gave "`TOP.u0.x" instead of "top.u0.x": the pass that
//! expanded the first level made the delimiters plain quotes, and the next
//! pass read the rest as a string literal (#1338, reported by git-hevangel).
//! A plain string literal still expands nothing. The expected strings are
//! those of the reference simulator.
use xezim::simulate;

fn shown(src: &str) -> Vec<String> {
    simulate(src, 100)
        .expect("simulate")
        .output
        .iter()
        .map(|l| l.message.clone())
        .filter(|m| m.starts_with("PASS") || m.starts_with("FAIL"))
        .collect()
}

/// The reporter's testcase: ten shapes, four of them controls that were
/// right before.
#[test]
fn reported_testcase() {
    let out = shown(
        r#"
`define TOP top
`define INST `TOP.u0
`define LEAF `INST.sig
`define STR(p) `"p`"
`define BODY `"`INST.q`"
`define BRACKET(p) `"[p]`"
`define MSG(a, b) `"a: `\`"b`\`"`"
`define PASTE(p) `"p``_sfx`"

module top;
  int errors = 0;

  task automatic check(input string what, input string got, input string want);
    if (got == want) begin
      $display("PASS %s", what);
    end else begin
      errors++;
      $display("FAIL %s: got \"%s\", want \"%s\"", what, got, want);
    end
  endtask

  initial begin
    check("region without a macro", `STR(abc), "abc");
    check("one macro level", `STR(`TOP), "top");
    check("macro chain in an actual", `STR(`INST.x), "top.u0.x");
    check("three-level chain", `STR(`LEAF), "top.u0.sig");
    check("chain in the body of the macro", `BODY, "top.u0.q");
    check("chain with text around it", `BRACKET(`INST), "[top.u0]");
    check("escaped quotes, no macro", `MSG(left side,right side), "left side: \"right side\"");
    check("escaped quotes around chains", `MSG(`INST, `LEAF), "top.u0: \"top.u0.sig\"");
    check("paste inside the region", `PASTE(`INST), "top.u0_sfx");
    check("plain string literal", "`INST", {"`", "INST"});
    $finish;
  end
endmodule
"#,
    );
    assert_eq!(out.len(), 10, "{out:?}");
    assert!(out.iter().all(|l| l.starts_with("PASS")), "{out:?}");
}

/// Chains through other macro forms, and the white space after the formal
/// list of a macro with arguments, which is not part of the string.
#[test]
fn chains_through_other_macro_forms() {
    let out = shown(
        r#"
`define TOP top
`define INST `TOP.u0
`define LEAF `INST.sig
`define STR(p) `"p`"
`define WRAP(p) `STR(p)
`define F(a) a.u0
`define G `F(top)
`define PRE(x) x``_q
`define CH `PRE(top)
`define STRD(p=`INST) `"p`"
`define DQ `"`INST`"
`define TWO `DQ
`define SQ(p) `"p 'p' `"
`define E(a) a
`define G2(a)    `E(a)

module top;
  task automatic check(input string what, input string got, input string want);
    if (got == want) $display("PASS %s", what);
    else $display("FAIL %s: got \"%s\", want \"%s\"", what, got, want);
  endtask

  initial begin
    check("passed on by another macro", `WRAP(`INST), "top.u0");
    check("through a macro with arguments", `STR(`G), "top.u0");
    check("through a paste", `STR(`CH), "top_q");
    check("default actual", `STRD(), "top.u0");
    check("two regions on a line", {`STR(`TOP), "|", `STR(`INST)}, "top|top.u0");
    check("body reached through a macro", `TWO, "top.u0");
    check("quotes of another kind", `SQ(`INST), "top.u0 'top.u0' ");
    check("concatenated regions", {`STR(`INST), "/", `STR(`LEAF)}, "top.u0/top.u0.sig");
    check("macro with arguments first", `STR(`F(top)), "top.u0");
    check("macro with arguments after text", `STR(x`F(top)), "xtop.u0");
    check("nested macros with arguments", `STR(`G2(z)), "z");
    check("spaces of the actual", `STR(a `E(q) b), "a q b");
    $finish;
  end
endmodule
"#,
    );
    assert_eq!(out.len(), 12, "{out:?}");
    assert!(out.iter().all(|l| l.starts_with("PASS")), "{out:?}");
}
