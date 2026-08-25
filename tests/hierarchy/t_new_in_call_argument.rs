//! A bare `new(...)` used as a FUNCTION/METHOD-CALL ARGUMENT
//! (`show(new("A"))`, `q.push_back(new("B"))`) has no LHS variable to borrow
//! its class from, so `eval_expr` (which infers the object's class from the
//! enclosing assignment/declaration target) returned a stale/zero handle and
//! the object was never constructed — the callee received a blank/null handle
//! and any field read came back empty.
//!
//! Fix: when binding an actual that is a bare `new(...)` to a CLASS-typed
//! formal (`function void add(N n)`, `task show(N x)`), construct the object
//! with the formal's declared class type before storing it in the frame —
//! in both `exec_function_call` and `exec_method_in_class_hierarchy`.
//!
//! Verified: `NARGPASS` appears and `NARGFAIL` is absent. Without the fix the
//! function case prints the argument handle's field as empty and the
//! class-method queue case stores blank handles (`NARGFAIL`).

use xezim::simulate_multi;

fn run_src(src: &str) -> Vec<String> {
    simulate_multi(
        &[src.to_string()],
        1000,
        Some("top"),
        &[],
        &[],
        None,
        false,
        None,
        None,
        &[],
        &[],
        1,
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
        None,
    )
    .expect("sim")
    .output
    .iter()
    .map(|o| o.message.clone())
    .collect()
}

#[test]
fn new_in_function_and_method_arguments() {
    let src = r#"
class N;
  string nm;
  function new(string n); nm = n; endfunction
endclass
class Tree;
  N kids[$];
  function void add(N c); kids.push_back(c); endfunction
  function N get_child(int i); return kids[i]; endfunction
endclass
function void show(N x, string tag);
  if (x == null) $display("%s=NULL", tag);
  else $display("%s=%s", tag, x.nm);
endfunction
module top;
  initial begin
    int ok = 1;
    show(new("FUNC_ARG"), "FUNC");            // function-call argument
    Tree t = new();
    t.add(new("M0")); t.add(new("M1"));        // class-method argument
    if (t.get_child(0).nm != "M0") ok = 0;
    if (t.get_child(1).nm != "M1") ok = 0;
    if (ok) $display("NARGPASS");
    else $display("NARGFAIL");
    $finish;
  end
endmodule
"#;
    let out = run_src(src);
    assert!(
        out.iter().any(|l| l.contains("FUNC=FUNC_ARG")),
        "function-call new() argument must construct the object; got {:?}",
        out
    );
    assert!(
        out.iter().any(|l| l.contains("NARGPASS")),
        "new(...) as call argument must construct with the formal's class; got {:?}",
        out
    );
    assert!(
        !out.iter().any(|l| l.contains("NARGFAIL")),
        "got {:?}",
        out
    );
}