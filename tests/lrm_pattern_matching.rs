//! IEEE 1800-2017 §12.6 pattern matching, outside `case … matches`.
//!
//! `if (e matches p)` parsed the pattern and THREW IT AWAY, yielding a literal
//! `1'b0`. So the then-branch never ran, whatever the subject held. And
//! `Pattern::Struct` (`'{a: .x, b: 3}`) returned false unconditionally, so a
//! structure pattern never matched anywhere — including in `case … matches`.
//!
//! Three things had to line up for `.name` bindings to survive into the
//! then-branch:
//!   - the parser must not synthesize a declaration for them (it wrapped the
//!     branch in a `VarDecl`, which re-initialised the binding to X);
//!   - elaboration must treat them as declared for that branch only;
//!   - the bytecode compiler must decline the `if`, since a conditional jump
//!     evaluates the match but drops the bindings.

use xezim::simulate;

const SRC: &str = r#"
module tb;
  typedef union tagged { void Invalid; int Valid; } tu_t;
  typedef struct { int a; int b; } p_t;

  tu_t v, iv;
  p_t  p;

  int if_bind, if_nobind, if_else, case_bind;
  int struct_bind, struct_const, struct_nomatch;
  int in_task;

  task automatic t();
    if (v matches tagged Valid .n) in_task = n;
    else in_task = -1;
  endtask

  initial begin
    v  = tagged Valid (99);
    iv = tagged Invalid;

    // `if (e matches p)` with a binding, without one, and the else path.
    if_bind = -1;
    if (v matches tagged Valid .n) if_bind = n;

    if_nobind = -1;
    if (v matches tagged Valid) if_nobind = 1; else if_nobind = 0;

    if_else = -1;
    if (iv matches tagged Valid .n) if_else = n; else if_else = 0;

    // `case … matches` still works.
    case_bind = -1;
    case (v) matches
      tagged Valid .n : case_bind = n;
      tagged Invalid  : case_bind = -2;
    endcase

    // Structure patterns: member bindings, and a constant member.
    p = '{3, 4};
    struct_bind = -1;
    case (p) matches
      '{a:.x, b:.y} : struct_bind = x + y;
      default       : struct_bind = -2;
    endcase

    struct_const = -1;
    case (p) matches
      '{a:3, b:.y} : struct_const = y;
      default      : struct_const = -2;
    endcase

    struct_nomatch = -1;
    case (p) matches
      '{a:9, b:.y} : struct_nomatch = y;
      default      : struct_nomatch = 0;
    endcase

    t();
  end
endmodule
"#;

fn i(sim: &xezim::compiler::Simulator, n: &str) -> i64 {
    let v = sim
        .get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able", n));
    v as u32 as i32 as i64
}

#[test]
fn if_matches_runs_the_then_branch_and_binds() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(i(&sim, "if_bind"), 99, "if (e matches p) never matched");
    assert_eq!(i(&sim, "if_nobind"), 1);
    assert_eq!(i(&sim, "in_task"), 99, "inside a task too");
}

#[test]
fn if_matches_takes_the_else_branch_on_a_mismatch() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(i(&sim, "if_else"), 0);
}

#[test]
fn case_matches_still_binds() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(i(&sim, "case_bind"), 99);
}

#[test]
fn structure_patterns_match_and_bind_members() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(i(&sim, "struct_bind"), 7, "a member-binding struct pattern never matched");
    assert_eq!(i(&sim, "struct_const"), 4, "a constant member must be compared");
    assert_eq!(i(&sim, "struct_nomatch"), 0, "a wrong constant must not match");
}
