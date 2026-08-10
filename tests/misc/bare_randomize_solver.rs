//! Regression for a parenthesless ('bare') call to the built-in `randomize()`
//! method appearing as an operand of a larger expression.
//!
//! UVM 05components/10uvm_pair/02class does `assert(t1.randomize & t2.randomize)`.
//! `t1.randomize` (no parens) parses as a flattened 2-segment
//! `Ident([t1, randomize])`. Because the built-in `randomize` is NOT a declared
//! class method, `class_parameterless_function` returned false and the operand
//! fell through to a property read — returning 0 without ever calling the
//! solver, so both transactions randomized to zeros. LRM 1800-2023 §13.4.1 +
//! §18.11: `obj.randomize` with no parens invokes the built-in randomize() of
//! every class object.
use xezim::simulate;

fn line(sim: &xezim::compiler::Simulator, tag: &str) -> String {
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .find(|m| m.starts_with(tag))
        .unwrap_or_else(|| panic!("no {} line", tag))
}

/// A bare (no-paren) `a.randomize` used as a bitwise-AND operand must run the
/// built-in solver (whose `post_randomize` side effect proves it ran) and
/// contribute its success bit.
#[test]
fn bare_builtin_randomize_dispatchs_solver() {
    const SRC: &str = r#"
module top;
  class C;
    rand int v;
    int post_calls;
    function new(); v = 0; endfunction
    function void post_randomize(); post_calls++; endfunction
  endclass
  C a, b;
  bit ok;
  initial begin
    a = new; b = new;
    ok = a.randomize & b.randomize;      // bare randomize, no parens
    $display("OK=%0d post=%0d", ok, a.post_calls + b.post_calls);
  end
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    // `post_randomize` must have run exactly twice (once per randomize call),
    // proving both bare randomizes dispatched the solver, and ok must be 1.
    assert_eq!(
        line(&sim, "OK="),
        "OK=1 post=2",
        "both bare randoms must run the solver (post_randomize twice) and AND to 1"
    );
}