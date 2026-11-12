//! §16 immediate assertion `else` clause: the `pass`/`else` statement of an
//! `assert` may declare block-local variables (e.g. a `begin...end` after
//! macro expansion like UVM's `assert(...) else `uvm_error(...)`, where the
//! macro introduces `_local_report_object_` inside the else block). Those
//! names must be in scope for the rest of the else clause.
//!
//! The bug this locks in: `collect_stmt_declared_names` collected block
//! locals for methods of module/package scope classes but its `K::Assertion`
//! arm was missing, so a block-local declared in an assertion's `else_action`
//! of a FILE-SCOPE class method was reported as "Undeclared identifier" at
//! elaboration. A module-scoped class with the same construct worked, so the
//! divergence only surfaced for classes declared at global/package scope
//! (the shape UVM 1800.2's own test suite uses).

use xezim::simulate;

fn messages(src: &str) -> Vec<String> {
    simulate(src, 1000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect()
}

/// The macro expands to a `begin...end` that declares `_local_report_object_`
/// and uses it in the same block — the exact shape `uvm_error` generates for
/// `assert(...) else `uvm_error(...)`.
const MACRO: &str = r#"
`define UERR(ID, MSG) begin int _local_report_object_; _local_report_object_ = 7; $display("T|err %0d", _local_report_object_); end
"#;

/// A class declared at FILE scope (outside any module) whose method's
/// `assert(...) else` clause declares and uses a block-local. Before the fix
/// this failed to elaborate with "Undeclared identifier
/// '_local_report_object_'".
#[test]
fn global_class_method_assert_else_block_local_is_scoped() {
    let src = format!(
        "{MACRO}{}",
        r#"
class cls;
  task t();
    int a = 0;
    assert(a == 1) else `UERR("T", "x");
    $display("T|after a=%0d", a);
  endtask
endclass

module top;
  initial begin
    cls c = new();
    c.t();
  end
endmodule
"#
    );
    let msgs = messages(&src);
    // the else block ran its $display with the block-local value 7
    assert!(
        msgs.iter().any(|m| m == "T|err 7"),
        "did not see block-local else display: {msgs:?}"
    );
    assert!(
        msgs.iter().any(|m| m == "T|after a=0"),
        "did not see after-assert display: {msgs:?}"
    );
}

/// The else clause only runs on failure; a passing assert's else block-local
/// is declared but the reference is inside the else block, so this must still
/// elaborate and run with no "Undeclared identifier".
#[test]
fn global_class_method_assert_else_block_local_passing_case() {
    let src = format!(
        "{MACRO}{}",
        r#"
class cls;
  task t();
    int a = 1;
    assert(a == 1) else `UERR("T", "x");
    $display("T|done a=%0d", a);
  endtask
endclass

module top;
  initial begin
    cls c = new();
    c.t();
  end
endmodule
"#
    );
    let msgs = messages(&src);
    assert!(
        msgs.iter().any(|m| m == "T|done a=1"),
        "did not see pass-through display: {msgs:?}"
    );
}