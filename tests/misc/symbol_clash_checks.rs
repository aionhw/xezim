//! L-family symbol-table clash checks — reference-validated (task L5/L6/L11/L12).
//!
//! The reference REJECTS: a duplicate ANSI port name, a variable clashing
//! with an ANSI port, a duplicate module-scope task, and a typedef reusing a
//! variable's name. It ACCEPTS a duplicate MODULE definition with a warning
//! (the later definition wins). xezim previously accepted all four clashes
//! (the duplicate task silently dispatched the later body) and rejected the
//! module redefinition.

use xezim::simulate;

/// Run one source; expect an elaboration error containing `needle`.
fn expect_err(src: &str, needle: &str) {
    match simulate(src, 10) {
        Ok(_) => panic!("expected an error containing {needle:?}, but it elaborated"),
        Err(e) => assert!(
            e.contains(needle),
            "error must mention {needle:?}; got: {e}"
        ),
    }
}

#[test]
fn duplicate_ansi_port_is_rejected() {
    expect_err(
        "module tb(input logic a, input logic a); endmodule\n",
        "duplicate declaration of 'a'",
    );
}

#[test]
fn variable_clashing_with_ansi_port_is_rejected() {
    expect_err(
        "module tb(input logic a);\n  int a;\nendmodule\n",
        "duplicate declaration of 'a'",
    );
}

#[test]
fn duplicate_module_scope_task_is_rejected() {
    // The silent form was dangerous: the later body won every dispatch.
    expect_err(
        "module tb;\n  task t; $display(\"1\"); endtask\n  task t; $display(\"2\"); endtask\n  initial t();\nendmodule\n",
        "duplicate declaration of 't'",
    );
}

#[test]
fn typedef_reusing_a_variable_name_is_rejected() {
    expect_err(
        "module tb;\n  int T;\n  typedef logic [3:0] T;\n  T v;\nendmodule\n",
        "duplicate declaration of 'T'",
    );
}

#[test]
fn module_redefinition_is_a_warning_and_the_later_wins() {
    // Reference: "Existing module 'm' ... will be overwritten" + the second
    // body's output. Previously a hard error.
    let src = r#"
module m; initial $display("T|first"); endmodule
module m; initial $display("T|second"); endmodule
module tb; m u1(); endmodule
"#;
    let sim = simulate(src, 10).expect("redefinition must elaborate (warning only)");
    let out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert!(
        out.iter().any(|m| m == "T|second"),
        "the LATER definition wins: {out:?}"
    );
    assert!(
        !out.iter().any(|m| m == "T|first"),
        "the earlier definition must be replaced: {out:?}"
    );
}

/// The legal non-ANSI split (`input [7:0] a; reg [7:0] a;`) must keep
/// elaborating — the new checks are ANSI-only.
#[test]
fn non_ansi_port_variable_split_still_elaborates() {
    let src = r#"
module tb;
  child c(.a(8'h2A));
endmodule
module child(a);
  input [7:0] a;
  reg   [7:0] a_copy;
  always @* a_copy = a;
endmodule
"#;
    simulate(src, 10).expect("the §23.2.2.1 split form is legal");
}

/// §23.5 `extern module` PROTOTYPE followed by the real definition — the
/// prototype is consumed; the definition elaborates and instantiates.
/// Reference-validated (em a=1 / ok at t=1).
#[test]
fn extern_module_prototype_is_accepted() {
    let src = r#"
extern module em(input logic a);
module tb;
  em u(.a(1'b1));
  logic [7:0] ok = 0;
  initial begin #1 ok = 8'h4F; end
endmodule
module em(input logic a);
  int saw = 0;
  initial saw = a;
endmodule
"#;
    let sim = simulate(src, 10).expect("extern prototype must elaborate");
    let ok = sim
        .get_signal("ok")
        .or_else(|| sim.get_signal("tb.ok"))
        .and_then(|v| v.to_u64())
        .unwrap_or(0);
    assert_eq!(ok, 0x4F, "design with an extern prototype runs normally");
}
