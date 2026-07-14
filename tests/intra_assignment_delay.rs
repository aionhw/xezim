//! IEEE 1800-2017 §9.4.5 intra-assignment delay: `lhs = #d rhs;`.
//!
//! The RHS is evaluated IMMEDIATELY, the process (for a blocking assignment)
//! suspends d time units, then the pre-computed value is assigned. This was
//! broken: the parser discarded the delay, so the assignment landed at once.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able", n))
}

/// Blocking form: the statement itself blocks for the delay, and the value
/// lands only when it expires.
const BLOCKING_SUSPENDS: &str = r#"
module tb;
  int x, seen_early, t_after;
  initial begin
    x = 0;
    x = #3 7;          // suspend 3, then assign the pre-computed 7
    t_after = $time;   // must run at t=3, after the assignment
  end
  initial begin
    #1 seen_early = x; // mid-delay observer: still 0 at t=1
  end
endmodule
"#;

#[test]
fn blocking_intra_delay_suspends_then_assigns() {
    let sim = simulate(BLOCKING_SUSPENDS, 100).expect("simulate failed");
    assert_eq!(u(&sim, "seen_early"), 0, "assignment landed before the delay expired");
    assert_eq!(u(&sim, "x"), 7);
    assert_eq!(u(&sim, "t_after"), 3, "statement did not block for the delay");
}

/// §9.4.5 capture semantics: the RHS is evaluated BEFORE the delay, so later
/// writes to its operands must not leak into the assigned value.
const RHS_CAPTURED: &str = r#"
module tb;
  int w, x, y;
  initial begin
    w = 1;
    fork
      x = #2 w;    // evaluates w NOW (=1), assigns at t=2
    join_none
    w = 9;         // must NOT affect the forked assignment
    y = #4 w + 10; // in-process capture: w=9 here, assigned at t=4
    w = 5;
  end
endmodule
"#;

#[test]
fn rhs_is_evaluated_before_the_delay() {
    let sim = simulate(RHS_CAPTURED, 100).expect("simulate failed");
    assert_eq!(u(&sim, "x"), 1, "forked intra-assignment read the post-fork w");
    assert_eq!(u(&sim, "y"), 19, "in-process intra-assignment lost the captured RHS");
}

/// Nonblocking form `lhs <= #d rhs`: the process does NOT block, the RHS is
/// captured at once, and the update lands d time units later.
const NBA_FORM: &str = r#"
module tb;
  int v, t_fork, mid, fin;
  initial begin
    v = 0;
    v <= #2 5;
    t_fork = $time;  // NBA must not block: still t=0
    #1 mid = v;      // t=1: update not yet applied
    #2 fin = v;      // t=3: applied at t=2
  end
endmodule
"#;

#[test]
fn nonblocking_intra_delay_defers_the_update_without_blocking() {
    let sim = simulate(NBA_FORM, 100).expect("simulate failed");
    assert_eq!(u(&sim, "t_fork"), 0, "an NBA with intra-assignment delay blocked the process");
    assert_eq!(u(&sim, "mid"), 0, "the delayed NBA update landed too early");
    assert_eq!(u(&sim, "fin"), 5, "the delayed NBA update never landed");
}
