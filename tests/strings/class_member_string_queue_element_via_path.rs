//! §6.16 / §7.10 — an ELEMENT of a class-member STRING collection reached via
//! a dotted/qualified path (`h.q[idx]`) must be treated as a string, so
//! `$display` renders its text rather than the raw packed value.
//!
//! `expr_is_string_valued`'s index branch consulted `string_signals` (a
//! scalar/module string) and `get_expr_type_name`, but a class-member string
//! queue (`string q[$]`) accessed from program scope as a two-segment dotted
//! id `h.q[idx]` is invisible to both: `h.q` shares no `string_signals` key
//! (only the bare `q` is registered, and only inside the owning method's
//! frame) and its type resolution returns None (a collection-of-strings is not
//! a scalar `SimpleType::String`). So `$display` rendered each element as a
//! garbage packed integer — exactly what a UVM visitor queue of node names
//! printed before this fix. Two changes fix it:
//!   1. `expr_is_string_valued` recognises a class member that is a STRING
//!      collection as string-valued (so `h.q[idx]` is a string element), and
//!   2. `peek_local_handle` resolves a module-scope class-handle variable
//!      (stored in the signal table, not the legacy `signals` map) so the
//!      dotted path `h.q` can be traced back to its class.
//! Verified byte-identical to a reference simulator on a minimal
//! queue-of-strings visitor in this exact `foreach(h.q[idx]) $display(h.q[idx])`
//! shape.

use xezim::simulate;

fn line(sim: &xezim::compiler::Simulator, tag: &str) -> String {
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .find(|m| m.starts_with(tag))
        .unwrap_or_else(|| panic!("no output line starting with {}", tag))
}

fn has_line(sim: &xezim::compiler::Simulator, needle: &str) -> bool {
    sim.output.iter().any(|o| o.message.contains(needle))
}

/// `foreach (h.q[idx]) $display(h.q[idx])` from MODULE scope must emit the
/// queue's text, not its packed representation.
#[test]
fn module_scoped_foreach_display_class_member_string_queue() {
    let src = r#"
module tb;
  class holder;
    string q[$];
    function void push(string s); q.push_back(s); endfunction
  endclass
  holder h;
  initial begin
    h = new();
    h.push("HELLO");
    h.push("WORLD");
    foreach(h.q[idx])
      $display(h.q[idx]);
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert!(has_line(&sim, "HELLO"), "h.q[0] displayed as text; got {:?}", sim.output);
    assert!(has_line(&sim, "WORLD"), "h.q[1] displayed as text; got {:?}", sim.output);
}

/// Control: the identical queue read within the OWNING class method (bare
/// `q[idx]`) always rendered as text; confirm it still does.
#[test]
fn class_method_bare_foreach_display_string_queue() {
    let src = r#"
module tb;
  class holder;
    string q[$];
    function void push(string s); q.push_back(s); endfunction
    function void show_all();
      foreach(q[idx]) $display(q[idx]);
    endfunction
  endclass
  holder h;
  initial begin
    h = new();
    h.push("AAA");
    h.push("BBB");
    h.show_all();
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(line(&sim, "AAA"), "AAA");
}

/// A class-member scalar string read via dotted path must also render as text.
#[test]
fn module_scoped_class_member_scalar_string_display() {
    let src = r#"
module tb;
  class holder;
    string name;
  endclass
  holder h;
  initial begin
    h = new();
    h.name = "SCALAR";
    $display("N=%s", h.name);
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(line(&sim, "N="), "N=SCALAR");
}