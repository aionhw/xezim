//! IEEE 1800-2017 §16.4: the pass and fail statements of a deferred
//! immediate assertion (`assert #0`, `assert final`, and the `assume` and
//! `cover` forms) are each a single subroutine call. Any other statement
//! (an increment, an assignment, a block) used to be accepted and run. A
//! simple immediate assertion still takes any statement.

use xezim::simulate;

fn rejected(src: &str) -> String {
    match simulate(src, 100) {
        Ok(_) => panic!("accepted:\n{src}"),
        Err(e) => e,
    }
}

const MSG: &str = "must each be a single subroutine call";

/// The issue's case: increments as both actions of `assert #0`.
#[test]
fn increments_as_deferred_actions_are_rejected() {
    let e = rejected(
        r#"
module tb;
  int d0_p, d0_f, x;
  always @(x) assert #0 (x == 1) d0_p++; else d0_f++;
endmodule
"#,
    );
    assert!(e.contains(MSG), "{e}");
}

/// Each deferred form, with a non-call statement as the pass action or as
/// the fail action.
#[test]
fn non_call_statements_are_rejected_in_every_deferred_form() {
    for stmt in [
        "assert #0 (x) n = 1;",
        "assert final (x) else n = 2;",
        "assume #0 (x) else begin $error(\"e\"); end",
        "cover final (x) n++;",
        "assert #0 (x) $display(\"p\"); else if (n) $display(\"f\");",
    ] {
        let e = rejected(&format!(
            "module tb; int n; bit x; initial begin {stmt} end endmodule\n"
        ));
        assert!(e.contains(MSG), "{stmt}: {e}");
    }
}

/// Guard: system task, task with and without parentheses, task method,
/// void function and void-cast function calls are accepted as deferred
/// actions, and a simple immediate assertion still takes any statement.
#[test]
fn subroutine_calls_and_simple_assertions_are_accepted() {
    let sim = simulate(
        r#"
module tb;
  int n;
  bit x = 1;
  class C; function void m(); n += 100; endfunction endclass
  C c = new;
  task t(); n += 1; endtask
  task u; n += 10; endtask
  function void vf(); n += 1000; endfunction
  function int f(); n += 10000; return 0; endfunction
  initial begin
    assert #0 (x) t(); else $error("no");
    assert final (x) u;
    assert #0 (x) c.m();
    assert #0 (x) vf();
    assert final (x) void'(f());
    assert (x) n += 100000; else n = -1;
    #1 $display("n=%0d", n);
  end
endmodule
"#,
        100,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let out: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .collect();
    assert!(out.contains(&"n=111111".to_string()), "{out:?}");
}
