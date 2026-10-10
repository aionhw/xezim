//! §6.21: a block-scoped local that SHADOWS an enclosing local is scoped to
//! its block and vanishes on exit — it must not clobber the outer same-named
//! variable (the xezim frame model flattens `begin...end` and blocking loop
//! bodies, so a shadowing block-local can otherwise overwrite the enclosing
//! local's shared frame slot and never be restored).
//!
//! The canonical case is UVM's `uvm_reg::do_write`: a task-scoped
//! `uvm_reg_cb_iter cbs` is shadowed inside a `foreach (m_fields[i])` body by
//! a block-local `uvm_reg_field_cb_iter cbs`, and after the loop the register
//! reads its own (now-clobbered) iterator, so the register callback never
//! fires. These tests isolate the scoping rule in pure SystemVerilog.

use xezim::simulate;

fn messages(src: &str) -> Vec<String> {
    simulate(src, 1000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect()
}

const COMMON: &str = r#"
class It;
  string tag; int cnt;
  function new(string t); tag = t; cnt = 0; endfunction
  function int more(); cnt++; return cnt; endfunction
endclass
"#;

/// A sequential `for` loop whose body re-declares a block-local named like a
/// task-local: after the loop the task-local must still hold its original
/// value (UVM `do_write`'s `foreach`, but observed with no blocking body).
#[test]
fn seq_for_body_local_redeclaring_task_local_is_scoped() {
    let src = format!(
        "{COMMON}{}",
        r#"
task automatic do_work();
  begin
    It cbs = new("REGISTER");
    int i;
    for (i = 0; i < 2; i++) begin
      It cbs = new("FIELD");
      void'(cbs.more());
    end
    $display("T|after field loop, cbs.tag=%s (expect REGISTER)", cbs.tag);
  end
endtask

module top;
  initial begin
    do_work();
  end
endmodule
"#
    );
    assert_eq!(
        messages(&src),
        ["T|after field loop, cbs.tag=REGISTER (expect REGISTER)"]
    );
}

/// A nested plain `begin...end` whose local shadows the enclosing block-local:
/// the inner value must not leak out.
#[test]
fn nested_block_local_does_not_leak_to_enclosing() {
    let src = format!(
        "{COMMON}{}",
        r#"
task automatic t;
  begin
    string a = "OUTER";
    begin
      string a = "INNER";
    end
    $display("T|a=%s (expect OUTER)", a);
  end
endtask

module top;
  initial begin
    t();
  end
endmodule
"#
    );
    assert_eq!(messages(&src), ["T|a=OUTER (expect OUTER)"]);
}

/// A `foreach` with a BLOCKING body (`#1` forces the suspend-aware trampoline,
/// which flattens the body's `begin...end`) re-declaring a block-local named
/// like a task-local: after the loop the task-local is intact. This is the
/// exact shape of UVM's `uvm_reg::do_write` field loop.
#[test]
fn blocking_foreach_body_local_redeclaring_task_local_is_scoped() {
    let src = format!(
        "{COMMON}{}",
        r#"
int fields[2];

task automatic do_write();
  begin
    It cbs = new("REGISTER");
    int i;
    foreach (fields[i]) begin
      It cbs = new("FIELD");
      #1;                    // blocking body: run via the suspend-aware trampoline
      void'(cbs.more());
    end
    $display("T|after field loop, cbs.tag=%s (expect REGISTER)", cbs.tag);
  end
endtask

module top;
  initial begin
    do_write();
  end
endmodule
"#
    );
    assert_eq!(
        messages(&src),
        ["T|after field loop, cbs.tag=REGISTER (expect REGISTER)"]
    );
}