//! Regression for the factory re-entrant-`register()` guard.
//!
//! In UVM, `factory.register(obj)` calls `obj.get_type_name()` as its first
//! statement, and that call can trigger a parameterized class specialization's
//! lazy static init which re-enters `factory.register(obj)` for the SAME
//! object. Without the re-entrancy guard, the outer call resumes to find the
//! object already stored and warns "already registered" (UVM `TPRGED`). The
//! guard in `exec_method_in_class_hierarchy` skips the re-entrant call so the
//! object is registered exactly once.
//!
//! This models the synchronous re-entrancy in pure SV and asserts the guard
//! yields a single registration (reg_count==1, no double-register).
use xezim::simulate;

fn out_line(sim: &xezim::compiler::Simulator, tag: &str) -> String {
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .find(|m| m.starts_with(tag))
        .unwrap_or_else(|| panic!("no {tag} line"))
}

#[test]
fn factory_register_reentrancy_is_idempotent() {
    const SRC: &str = r#"
typedef class factory;
class wrapper;
  string name;
  function new(string n); name = n; endfunction
  function string get_type_name();
    // Simulate the lazy-static-init side effect: the first get_type_name on a
    // "trigger" wrapper re-enters the factory to register this same object
    // (UVM: __deferred_init -> initialize -> factory.register(this)).
    if (name == "trigger") factory::get().register(this);
    return name;
  endfunction
endclass
class factory;
  static factory m_inst;
  bit m_types[wrapper];
  int  reg_count;
  int  dbl_count;
  static function factory get();
    if (m_inst == null) m_inst = new();
    return m_inst;
  endfunction
  function void register(wrapper obj);
    string nm = obj.get_type_name();  // may re-enter register for `obj`
    if (m_types.exists(obj)) begin
      dbl_count = dbl_count + 1;      // double registration detected
    end else begin
      m_types[obj] = 1;
      reg_count = reg_count + 1;
    end
  endfunction
endclass
module top;
  initial begin
    wrapper w;
    w = new("trigger");
    factory::get().register(w);       // outer register; get_type_name re-enters
    $display("reg_count=%0d dbl_count=%0d", factory::get().reg_count, factory::get().dbl_count);
    if (factory::get().reg_count == 1 && factory::get().dbl_count == 0)
      $display("TAG_PASS");
    else
      $display("TAG_FAIL");
  end
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(out_line(&sim, "reg_count="), "reg_count=1 dbl_count=0");
    assert_eq!(out_line(&sim, "TAG_"), "TAG_PASS");
}
