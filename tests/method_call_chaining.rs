// Regression test for a double-evaluation bug in method-call chaining.
//
// `obj.m().m().m()` (left-associative chaining, IEEE 1800-2023 §11.8) was
// executing the receiver expression MORE THAN ONCE because an early guard in
// eval_call_inner's MemberAccess arm called `self.eval_expr(expr)`
// unconditionally (to support `.len()`/`.size()` on a non-identifier base) and
// then DISCARDED the result for every other method name, only to re-evaluate
// `expr` at the generic method dispatch a few lines later.
//
// Observable symptom: a chain of depth n ran each side-effecting call 2^n-1
// times (T(n)=2*T(n-1)+1):
//   b.inc()           -> count 1   (correct)
//   b.inc().inc()     -> count 3   (should be 2)
//   b.inc().inc().inc())-> count 7  (should be 3)
// The classic double-evaluation signature.
//
// These tests verify chaining matches the reference simulators exactly and
// that side effects run exactly once per call.

use std::process::Command;

fn run_xezim(src: &str, tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!("xezim_chain_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sv_path = dir.join(format!("chain_{tag}.sv"));
    std::fs::write(&sv_path, src).unwrap();
    let bin = env!("CARGO_BIN_EXE_xezim");
    let out = Command::new(bin)
        .arg("--simulate")
        .arg("-s")
        .arg("top")
        .arg(sv_path.to_str().unwrap())
        .output()
        .expect("failed to run xezim");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

const CLASS_SRC: &str = r#"
class Counter;
  int count;
  function new(); count = 0; endfunction
  function Counter inc();
    count = count + 1;
    return this;
  endfunction
  function Counter dec();
    count = count - 1;
    return this;
  endfunction
  function Counter add(int n);
    count = count + n;
    return this;
  endfunction
endclass
"#;

fn chain_test(body: &str, tag: &str) -> String {
    run_xezim(
        &format!("{CLASS_SRC}\nmodule top; initial begin {body} end endmodule"),
        tag,
    )
}

#[test]
fn single_call_is_one_execution() {
    let out = chain_test(
        "Counter c; c = new(); c.inc(); $display(\"COUNT %0d\", c.count);",
        "single",
    );
    assert!(out.contains("COUNT 1"), "single inc over-ran:\n{out}");
}

#[test]
fn chain_of_two_is_two_executions() {
    // Before the fix this gave 3 (2^2 - 1).
    let out = chain_test(
        "Counter c; Counter r; c = new(); r = c.inc().inc(); $display(\"COUNT %0d\", c.count);",
        "two",
    );
    assert!(out.contains("COUNT 2"), "chain of two mis-evaluated:\n{out}");
}

#[test]
fn chain_of_three_is_three_executions() {
    // Before the fix this gave 7 (2^3 - 1) — the tell-tale double-eval.
    let out = chain_test(
        "Counter c; Counter r; c = new(); r = c.inc().inc().inc(); $display(\"COUNT %0d\", c.count);",
        "three",
    );
    assert!(out.contains("COUNT 3"), "chain of three mis-evaluated:\n{out}");
}

#[test]
fn chain_of_four_is_four_executions() {
    // Before the fix this gave 15 (2^4 - 1).
    let out = chain_test(
        "Counter c; Counter r; c = new(); r = c.inc().inc().inc().inc(); $display(\"COUNT %0d\", c.count);",
        "four",
    );
    assert!(out.contains("COUNT 4"), "chain of four mis-evaluated:\n{out}");
}

#[test]
fn mixed_methods_in_chain() {
    // Different methods in the same chain: inc().dec() must net to 0.
    // Before the fix this gave 1.
    let out = chain_test(
        "Counter c; Counter r; c = new(); r = c.inc().dec(); $display(\"COUNT %0d\", c.count);",
        "mixed",
    );
    assert!(out.contains("COUNT 0"), "mixed chain mis-evaluated:\n{out}");
}

#[test]
fn chained_call_with_argument() {
    // add(10).add(20).add(30) -> 60.
    let out = chain_test(
        "Counter c; Counter r; c = new(); r = c.add(10).add(20).add(30); $display(\"COUNT %0d\", c.count);",
        "args",
    );
    assert!(
        out.contains("COUNT 60"),
        "chained call with args mis-evaluated:\n{out}"
    );
}

#[test]
fn this_keyword_works_inside_methods() {
    // The 8.11 `this` keyword itself must keep working: explicit this.count
    // qualification and `return this` for fluent style.
    let src = r#"class Builder;
  int count;
  function new(); count = 0; endfunction
  function Builder inc();
    this.count = this.count + 1;
    return this;
  endfunction
  function int get(); return this.count; endfunction
endclass
module top;
  initial begin
    Builder b;
    b = new();
    b = b.inc().inc().inc();
    $display("COUNT %0d", b.get());
  end
endmodule
"#;
    let out = run_xezim(src, "this_kw");
    assert!(out.contains("COUNT 3"), "this keyword / chaining:\n{out}");
}
