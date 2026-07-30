//! IEEE 1800-2017 §23.3.3: every uninstantiated module is a top-level
//! instance, and ALL of their initial/always/continuous-assign blocks execute
//! concurrently. There is no single "root" — multiple top-level modules
//! coexist.
//!
//! Before this fix, xezim's auto-detection picked exactly ONE uninstantiated
//! module as the elaboration root and silently ignored the rest, so only that
//! one module's `initial` blocks ran. This broke multi-top UVM testbenches
//! that spawn two top modules (one driving UVM objections, the other calling
//! `run_test()`).

use xezim::simulate;

fn out(src: &str) -> String {
    let sim = simulate(src, 1_000_000).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Three uninstantiated modules: each one's `initial` block must run.
#[test]
fn multiple_top_level_modules_all_run() {
    let o = out(r#"
module mod_a; initial $display("RAN_A"); endmodule
module mod_b; initial $display("RAN_B"); endmodule
module mod_c; initial $display("RAN_C"); endmodule
"#);
    assert!(o.contains("RAN_A"), "mod_a initial must run; got: {}", o);
    assert!(o.contains("RAN_B"), "mod_b initial must run; got: {}", o);
    assert!(o.contains("RAN_C"), "mod_c initial must run; got: {}", o);
}

/// Two top modules whose `initial` blocks advance simulation time independently;
/// both delays must be observed (the second module's block is not dropped).
#[test]
fn multiple_top_level_modules_time_both_advance() {
    let o = out(r#"
module a; initial begin #10; $display("A_DONE %0t", $time); end endmodule
module b; initial begin #20; $display("B_DONE %0t", $time); end endmodule
"#);
    assert!(o.contains("A_DONE 10"), "got: {}", o);
    assert!(o.contains("B_DONE 20"), "got: {}", o);
}

/// Signal name collisions across top modules must not clash: each module has
/// its own `clk`, and each block sees its own copy (instance-prefixed by the
/// flattener under the synthetic `__xezim_multi_top` wrapper).
#[test]
fn multiple_top_level_modules_namespace_signals() {
    let o = out(r#"
module a; logic clk; initial begin clk = 1; $display("A_CLK %0d", clk); end endmodule
module b; logic clk; initial begin clk = 0; $display("B_CLK %0d", clk); end endmodule
"#);
    assert!(o.contains("A_CLK 1"), "got: {}", o);
    assert!(o.contains("B_CLK 0"), "got: {}", o);
}

/// A single top module (the common case) is unaffected: no wrapper is built,
/// and its `%m` hierarchy name is the bare module name.
#[test]
fn single_top_module_unchanged() {
    let o = out(r#"
module top; initial $display("M=%m"); endmodule
"#);
    assert!(o.contains("M=top"), "single-top %m must be the bare name; got: {}", o);
    assert!(
        !o.contains("__xezim_multi_top"),
        "single-top must not synthesize a wrapper; got: {}",
        o
    );
}
