//! Pure-SystemVerilog regression for `new` construction vs. shallow copy
//! (IEEE 1800-2023 §8.7 / §8.8, Syntax 8-2 and footnote 23).
//!
//! `class_new_23` distinguishes two forms:
//!   [ class_scope ] new [ ( list_of_arguments ) ]   <- ordinary constructor call
//!   new expression                                   <- SHALLOW COPY ONLY (fn 23
//!                                                        says the expression
//!                                                        evaluates to an object
//!                                                        handle)
//!
//! So `h = new(handle_arg)` — PARENTHESIZED with a single class-handle
//! argument — is an ordinary CONSTRUCTOR call: the handle is passed as a
//! formal. `x = new src` (PARENTHESELESS bare expression) is a shallow copy.
//!
//! xezim previously collapsed both into `Call{func:Ident("new"), args:[h]}`
//! and treated a single class-handle argument as a shallow copy: the
//! constructor never ran, so `Holder::new(Box)` left `m_port` unset and
//! `val()` returned 0 at the reference's 9 (the heartbeat of UVM's
//! `uvm_seq_item_pull_port_base` / `uvm_driver` `seq_item_port` connection
//! count failures). The parser now emits a distinct `ShallowCopy` node for
//! the parentheseless form only.
use xezim::simulate;

fn out_line(sim: &xezim::compiler::Simulator, tag: &str) -> String {
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .find(|m| m.starts_with(tag))
        .unwrap_or_else(|| panic!("no {} line", tag))
}

/// `h = new(q)` (parens, single class-formal) must run the constructor and
/// store `q` in the field; `c = new src` (no parens) must shallow-copy.
#[test]
fn parenthesized_ctor_vs_parenthesless_copy() {
    const SRC: &str = r#"
module top;
  class Box;
    int v;
    function new(int x); v = x; endfunction
    function int val(); return v; endfunction
  endclass
  class Holder;
    Box b;
    function new(Box x); b = x; endfunction
    function int val_out(); return b.val(); endfunction
  endclass
  initial begin
    Box q, a, c;
    Holder h;
    q = new Box(9);
    h = new(q);          // PAREN -> constructor: h.b = q -> 9
    $display("PAREN=%0d", h.val_out());
    a = new Box(3);
    c = new a;           // NO-PAREN -> shallow copy of a -> 3
    $display("NOARG=%0d", c.val());
  end
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(out_line(&sim, "PAREN="), "PAREN=9");
    assert_eq!(out_line(&sim, "NOARG="), "NOARG=3");
}

/// The UVM-classic `x = new(other_object)` pattern where the constructor
/// formal is a parameterized BASE type. Previously misread as a shallow copy,
/// so the constructed field stayed unset (0).
#[test]
fn param_base_formal_ctor_runs() {
    const SRC: &str = r#"
module top;
  virtual class PBase #(type IF = bit);
    int v;
    function int get(); return v; endfunction
  endclass
  class PCh extends PBase #(bit);
    function new(int x); super.new(); v = x; endfunction
  endclass
  class Combo;
    PBase #(bit) m_port;
    function new(PBase #(bit) a); m_port = a; endfunction
    function int getm(); return m_port.get(); endfunction
  endclass
  initial begin
    PCh pb = new(8);
    Combo c;
    c = new(pb);
    $display("MV=%0d", c.getm());
  end
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(out_line(&sim, "MV="), "MV=8");
}