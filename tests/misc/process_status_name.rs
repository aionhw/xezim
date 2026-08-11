//! §9.7 `process::self().status().name()` — reference-validated (F10 audit).
//!
//! The process state enum (FINISHED, RUNNING, WAITING, SUSPENDED, KILLED) is
//! BUILT IN: it has no user enum table, so the generic `.name()` reflection
//! found no match and returned an empty string.

use xezim::simulate;

fn line(sim: &xezim::compiler::Simulator, tag: &str) -> String {
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .find(|m| m.starts_with(tag))
        .unwrap_or_else(|| panic!("no output line starting with {}", tag))
}

/// Reference: self=RUNNING.
#[test]
fn status_name_returns_the_state_string() {
    let src = r#"
module tb;
  initial begin
    process p;
    p = process::self();
    $display("S=[%s]", p.status().name());
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulate failed");
    assert_eq!(line(&sim, "S="), "S=[RUNNING]");
}
