//! Static class methods invoked through an OBJECT HANDLE must dispatch
//! statically — resolving by the receiver's DECLARED class, with NO null
//! dereference and independent of the receiver's runtime value.
//!
//! IEEE 1800-2023 §8.7/§8.23: a `static` method belongs to the class itself,
//! not to any instance. Invoking it as `<obj>.method()` resolves by the
//! receiver's declared (static) type and is legal through a NULL handle (the
//! LRM forbids a null dereference for a static method call).
//!
//! xezim stored static methods separately from instance methods
//! (`ClassDef.static_methods` vs `methods`), so the instance dispatch paths
//! returned null for a static method. This is the mechanism UVM's
//! `uvm_create` relies on to fetch its type wrapper:
//!
//! ```sv
//! trans req;                 // not yet allocated (null)
//! wrapper w = req.get_type();  // `get_type` is static on uvm_object
//! ```
//!
//! Before this fix that returned null on a null receiver, so the sequence
//! item was never created and no item reached the driver — the whole UVM
//! sequence→sequencer→driver pipeline silently collapsed at t=0. These
//! self-checks pin that a static method dispatches through: (a) a null
//! handle, (b) a live handle where a subclass overrides it, and (c) a class
//! MEMBER receiver whose type is a type parameter bound under inheritance
//! (`REQ req` inside `uvm_sequence #(REQ)`).

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn tag(sim: &xezim::compiler::Simulator) -> String {
    let msgs = messages(sim);
    msgs.iter()
        .find(|m| m.starts_with("TAG_PASS") || m.starts_with("TAG_FAIL"))
        .cloned()
        .unwrap_or_else(|| "(no tag)".to_string())
}

/// (a) A STATIC method called through a NULL handle of its declared type
/// resolves without dereferencing — the minimal UVM `get_type` shape.
const NULL_RECEIVER: &str = r#"
module top;
  class wrapper;
    int id;
    function new(int i=0); id=i; endfunction
  endclass

  class obj;
    static function wrapper get_type();
      wrapper w = new(7);
      return w;
    endfunction
  endclass

  class trans extends obj;
  endclass

  initial begin
    trans t;
    wrapper w;
    t = null;
    w = t.get_type();
    if (w != null && w.id == 7) $display("TAG_PASS");
    else $display("TAG_FAIL");
    $finish;
  end
endmodule
"#;

#[test]
fn static_method_null_receiver() {
    let sim = simulate(NULL_RECEIVER, 100).expect("simulate failed");
    assert_eq!(tag(&sim), "TAG_PASS", "output: {:?}", messages(&sim));
}

/// (b) A subclass that overrides the static method is honored through a LIVE
/// receiver handle.
#[test]
fn static_method_live_receiver() {
    const LIVE_RECEIVER: &str = r#"
module top;
  class obj;
    static function int getv(); return 7; endfunction
  endclass

  class trans extends obj;
    static function int getv(); return 8; endfunction
  endclass

  initial begin
    trans t;
    int v;
    t = new();
    v = t.getv();
    if (v == 8) $display("TAG_PASS"); else $display("TAG_FAIL");
    $finish;
  end
endmodule
"#;
    let sim = simulate(LIVE_RECEIVER, 100).expect("simulate failed");
    assert_eq!(tag(&sim), "TAG_PASS", "output: {:?}", messages(&sim));
}

/// (c) A class MEMBER receiver whose type is a type parameter bound under
/// inheritance (`REQ req` in a `#(REQ)` base, `my_seq extends seq_base
/// #(trans)`), accessed bare (as `uvm_do_with(req, …)` does inside `body()`).
/// `REQ` must bind to `trans` even with a null receiver.
#[test]
fn static_method_member_receiver() {
    const MEMBER_RECEIVER: &str = r#"
module top;
  class wrapper;
    int id;
    function new(int i=0); id=i; endfunction
  endclass

  class obj;
    static function wrapper get_type();
      wrapper w = new(7);
      return w;
    endfunction
  endclass

  virtual class seq_base #(type REQ);
    REQ req;
    function new(); req = null; endfunction
  endclass

  class trans extends obj;
  endclass

  class my_seq extends seq_base #(trans);
    task body();
      wrapper w;
      w = req.get_type();
      if (w != null && w.id == 7) $display("TAG_PASS");
      else $display("TAG_FAIL");
    endtask
  endclass

  initial begin
    my_seq s;
    s = new();
    s.body();
    $finish;
  end
endmodule
"#;
    let sim = simulate(MEMBER_RECEIVER, 100).expect("simulate failed");
    assert_eq!(tag(&sim), "TAG_PASS", "output: {:?}", messages(&sim));
}