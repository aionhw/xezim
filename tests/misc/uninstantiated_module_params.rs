//! §23 elaboration starts at the top and walks instantiations, so a module
//! definition nothing reaches contributes no instance. Its parameter DEFAULTS
//! are placeholders an instantiation would have supplied, and const-folding
//! them invents errors no reference tool reports.
//!
//! Issue #238: `parameter W = 0` feeding `{{(W - 1){1'b0}}}` in a module that
//! is never instantiated read as a replication count of -1 and failed the
//! whole run, while Icarus just runs the design.
//!
//! The guard that matters is the second test: the TOP module's own parameters
//! ARE fixed at their defaults, so the same expression there must still be
//! diagnosed. The fix narrows which definitions are folded; it does not switch
//! the check off.

fn run(src: &str, top: &str) -> Result<xezim::compiler::Simulator, String> {
    xezim::simulate_multi(
        &[src.to_string()],
        1000,
        Some(top),
        &[],
        &[],
        None,
        false,
        None,
        None,
        &[],
        &[],
        None,
        &[],
        0,
        u64::MAX,
        None,
        &[],
        None,
        None,
        None,
        None,
        false,
    )
}

/// The reported case: an unreachable module's defaults are not evaluated.
#[test]
fn unreachable_module_default_params_are_not_folded() {
    let src = "\
module top;\n\
   initial $display(\"ok\");\n\
endmodule\n\
module unused #(parameter W = 0) (output [7:0] o);\n\
   assign o = {{(W - 1) {1'b0}}, 1'b0};\n\
endmodule";
    let sim = run(src, "top").expect("an uninstantiated module must not fail the run");
    let msgs: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert!(
        msgs.iter().any(|m| m.contains("ok")),
        "top should have run; output was {msgs:?}"
    );
}

/// The guard: the top's OWN parameters are fixed at their defaults, so a
/// negative replication count there is still an error.
#[test]
fn top_module_default_params_are_still_folded() {
    let src = "\
module top #(parameter W = 0) (output [7:0] o);\n\
   assign o = {{(W - 1) {1'b0}}, 1'b0};\n\
   initial $display(\"ok\");\n\
endmodule";
    let err = match run(src, "top") {
        Ok(_) => panic!("the top's own defaults must still be checked"),
        Err(e) => e,
    };
    assert!(
        err.contains("replication count cannot be negative"),
        "expected the §11.4.12.1 diagnostic, got: {err}"
    );
}

/// An unreachable module is still parsed and still gets the checks that do not
/// depend on a parameter value, so narrowing the fold does not blind the lint.
#[test]
fn unreachable_module_keeps_parameter_independent_checks() {
    let src = "\
module top;\n\
   initial $display(\"ok\");\n\
endmodule\n\
module unused (output [7:0] o);\n\
   assign o = {{(0 - 1) {1'b0}}, 1'b0};\n\
endmodule";
    let err = match run(src, "top") {
        Ok(_) => panic!("a literal negative replication count does not depend on a parameter"),
        Err(e) => e,
    };
    assert!(
        err.contains("replication count cannot be negative"),
        "expected the §11.4.12.1 diagnostic, got: {err}"
    );
}
