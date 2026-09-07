//! §8.9 / §8.20 — a STATIC class member holding a live object must dispatch
//! methods on that object's RUNTIME class, not on the class that DECLARES the
//! static. When the declaring class has a same-named NON-virtual method, the
//! non-virtual static-binding rule (`method_call` binds to the receiver's
//! declared type) resolved the receiver's "declared class" via `class_of_var`,
//! which for a bare static member returns the DECLARING class of the static
//! (e.g. `uvm_report_catcher`), not the member's own VALUE type. So
//! `uvm_report_catcher`'s `get_severity()` — a real non-virtual helper that
//! happens to collide with `uvm_report_message::get_severity()` — was bound
//! onto a live `uvm_report_message` object and read its fields as zeroed
//! garbage. In UVM's report catcher this made a caught-and-demoted
//! `uvm_fatal`'s action recompute read severity 0, so UVM_EXIT was never
//! cleared and the run died early (the macros after a caught fatal never
//! executed). The fix: only honor the non-virtual declared target when it is
//! the runtime object's class or an ancestor; otherwise dispatch through the
//! live handle's runtime class. Verified byte-identical with a reference
//! simulator.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("top.{}", n)))
        .unwrap_or_else(|| panic!("signal {} not found", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able (x/z?)", n))
}

/// A static member with a same-named non-virtual method in its DECLARING class
/// must still dispatch to the object's own (virtual) method.
#[test]
fn static_member_method_cost_not_hijacked_by_declaring_class_nonvirtual() {
    let src = r#"
module top;
  class msg;
    int sv;
    function new(int s); sv = s; endfunction
    virtual function int get_sev(); return sv; endfunction
  endclass

  class catcher;
    static msg holder;
    // same-named NON-virtual helper in the declaring class — must NOT hijack
    // `holder.get_sev()`.
    function int get_sev(); return -1; endfunction
    static function int read_holder();
      return holder.get_sev();
    endfunction
  endclass

  int r;
  initial begin
    catcher::holder = new(42);
    r = catcher::read_holder();
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "r"), 42, "holder.get_sev() must call msg::get_sev (runtime class), not catcher::get_sev");
}

/// Control: the same message read through a concrete catcher-derived path
/// (the actual UVM shape a catcher would use) is still correct.
#[test]
fn object_method_through_this_receiver_unchanged() {
    let src = r#"
module top;
  class msg;
    int sv;
    function new(int s); sv = s; endfunction
    virtual function int get_sev(); return sv; endfunction
  endclass

  class catcher;
    local static msg holder;
    function int read();
      return holder.get_sev();
    endfunction
  endclass

  int r;
  initial begin
    catcher c;
    c = new();
    c.holder = new(7);
    r = c.read();
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "r"), 7);
}