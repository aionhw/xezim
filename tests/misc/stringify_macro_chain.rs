//! IEEE 1800-2023 §22.5.1. Regression design from issue #1338.

#[test]
fn stringify_macro_chain() {
    let sim = xezim::simulate(r#"
// Macro chains inside a `" ... `" region (IEEE 1800-2023 section 22.5.1).
//
// A macro whose body holds another macro takes one expansion level per macro.
// The text of a `" ... `" region has to be expanded to the end: the quotation
// marks, the actual arguments and every embedded macro, however deep.
//
//   xezim --no-cache --simulate -s top stringify_chain.sv
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
    // Controls: at most one macro level inside the region.
    check("region without a macro", `STR(abc), "abc");
    check("one macro level", `STR(`TOP), "top");
    // The chains.
    check("macro chain in an actual", `STR(`INST.x), "top.u0.x");
    check("three-level chain", `STR(`LEAF), "top.u0.sig");
    check("chain in the body of the macro", `BODY, "top.u0.q");
    check("chain with text around it", `BRACKET(`INST), "[top.u0]");
    check("escaped quotes, no macro", `MSG(left side,right side), "left side: \"right side\"");
    check("escaped quotes around chains", `MSG(`INST, `LEAF), "top.u0: \"top.u0.sig\"");
    check("paste inside the region", `PASTE(`INST), "top.u0_sfx");
    // A plain string literal expands nothing (22.5.1).
    check("plain string literal", "`INST", {"`", "INST"});
    if (errors == 0) $display("PASS all checks");
    else $display("FAIL %0d check(s)", errors);
    $finish;
  end
endmodule
"#, 100).expect("simulate");
    let messages: Vec<_> = sim.output.iter().map(|o| o.message.as_str()).collect();
    assert!(messages.contains(&"PASS all checks"), "{messages:?}");
}
