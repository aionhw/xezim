//! Pure-SystemVerilog regression for a parenthesless ("bare") method call
//! appearing as an OPERAND in a larger expression.
//!
//! Distilled from UVM's `uvm_class_pair` (the `assert(t1.randomize &
//! t2.randomize)` in its synchronous sequence): a flattened 2-segment
//! `Ident([obj, method])` (`obj.method` with NO parens) used as a value was
//! read as an object property and returned 0 WITHOUT running the function —
//! so the fields it was supposed to fill stayed at their defaults.
//!
//! LRM 1800-2023 §13.4.1: `obj.f` with no parens invokes the no-argument
//! function `f` and yields its return value.
use xezim::simulate;

fn line(sim: &xezim::compiler::Simulator, tag: &str) -> String {
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .find(|m| m.starts_with(tag))
        .unwrap_or_else(|| panic!("no {} line", tag))
}

/// A bare (no-parens) method call used as a bitwise-AND operand must invoke
/// the function and contribute its return value and side effect.
#[test]
fn bare_parameterless_call_in_binary_expr_invokes_and_returns() {
    const SRC: &str = r#"
module top;
  class C;
    int v;
    function bit tick();
      v = 42;
      return 1;
    endfunction
  endclass
  C a, b;
  int x;
  initial begin
    a = new; b = new;
    x = a.tick & b.tick & a.tick;   // bare method calls, no parens
    $display("X=%0d a.v=%0d", x, a.v);
  end
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(
        line(&sim, "X="),
        "X=1 a.v=42",
        "bare calls must each run (v=42) and AND to 1"
    );
}