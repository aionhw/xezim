//! An expression with a side effect must be evaluated exactly once.
//!
//! Found while adding VPI system functions, but the defect is pure SystemVerilog
//! and predates them. Two separate double-evaluations:
//!
//! 1. `run_process_stmts` evaluated an `if` condition to decide whether the
//!    chosen branch contains blocking statements, then fell through to
//!    `exec_statement`, which evaluated it AGAIN. Any side effect in an `if`
//!    condition — `$random()`, `$urandom()`, a function call, `i++` — happened
//!    twice.
//!
//! 2. `infer_width` computes a context width for a binary operator's operands,
//!    and its fallback arm infers the width by EVALUATING the expression. So an
//!    operand with a side effect ran twice: once discarded for its width, once
//!    for its value.

use xezim::simulate;

/// A void function with an observable side effect, called from every position
/// an expression can occupy.
const SRC: &str = r#"
module tb;
  int calls;
  int r;
  int if_taken, else_taken;

  function automatic int bump(input int ret);
    calls = calls + 1;
    return ret;
  endfunction

  int c_bare, c_binop_const, c_binop_var, c_if_true, c_if_false;
  int c_ternary, c_unary, c_paren, c_two, c_while, c_nested;
  int v;

  initial begin
    v = 2;

    calls = 0; r = bump(1);                       c_bare = calls;
    calls = 0; r = bump(1) + 1;                   c_binop_const = calls;
    calls = 0; r = bump(1) + v;                   c_binop_var = calls;
    calls = 0; r = -bump(1);                      c_unary = calls;
    calls = 0; r = (bump(1));                     c_paren = calls;
    calls = 0; r = bump(1) + bump(2);             c_two = calls;
    calls = 0; r = bump(1) ? 10 : 20;             c_ternary = calls;

    // An `if` condition, both when taken and when not.
    calls = 0; if_taken = 0;
    if (bump(1)) if_taken = 1;
    c_if_true = calls;

    calls = 0; else_taken = 0;
    if (bump(0)) ; else else_taken = 1;
    c_if_false = calls;

    // A blocking branch takes a different path through run_process_stmts.
    calls = 0;
    if (bump(1)) begin #1; end
    c_nested = calls;

    // A `while` condition is evaluated once per iteration, plus the final
    // failing test: three calls for two iterations.
    calls = 0;
    while (bump(v) != 0) v = v - 1;
    c_while = calls;
  end
endmodule
"#;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able", n))
}

#[test]
fn a_side_effect_in_a_binary_operand_runs_once() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "c_bare"), 1, "bare call");
    assert_eq!(u(&sim, "c_binop_const"), 1, "`f() + 1` evaluated f() twice");
    assert_eq!(u(&sim, "c_binop_var"), 1, "`f() + v` evaluated f() twice");
    assert_eq!(u(&sim, "c_two"), 2, "`f() + f()` must call exactly twice");
}

#[test]
fn a_side_effect_in_other_operand_positions_runs_once() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "c_unary"), 1, "unary operand");
    assert_eq!(u(&sim, "c_paren"), 1, "parenthesized");
    assert_eq!(u(&sim, "c_ternary"), 1, "ternary condition");
}

#[test]
fn a_side_effect_in_an_if_condition_runs_once() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "c_if_true"), 1, "`if (f())` taken, evaluated f() twice");
    assert_eq!(u(&sim, "c_if_false"), 1, "`if (f())` not taken, evaluated f() twice");
    assert_eq!(u(&sim, "c_nested"), 1, "`if (f()) begin #1; end` (blocking branch)");
}

#[test]
fn the_if_still_selects_the_right_branch() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "if_taken"), 1, "a true condition must take the then-branch");
    assert_eq!(u(&sim, "else_taken"), 1, "a false condition must take the else-branch");
}

#[test]
fn a_while_condition_runs_once_per_test() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    // v = 2 -> bump(2) true, v=1; bump(1) true, v=0; bump(0) false. Three tests.
    assert_eq!(u(&sim, "c_while"), 3, "a while condition ran the wrong number of times");
}
