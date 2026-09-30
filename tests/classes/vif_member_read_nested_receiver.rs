//! §25.8: a `vif.<member>` read must follow the virtual-interface BINDING, not
//! the parse shape of its receiver.
//!
//! In `<recv>.<vifprop>.<member>` the receiver may be spelled as a class-handle
//! chain rather than a single hierarchical identifier — `cfg.vif`, `a.cfg.vif`,
//! or a chain rooted at a subroutine LOCAL holding the owner object. Those
//! spellings parse as MemberAccess, and only receivers the parser kept as one
//! flat hierarchical ident were rewritten to the bound interface. Everything
//! else fell through to the generic property read and returned the
//! binding-EXISTENCE sentinel instead of the interface, i.e. `cfg.vif.data`
//! read 0 while the binding was in fact recorded — the receiving object's own
//! property read (`cfg_c::own_read`) proves it in the same run.
//!
//! This is the uvm_agent_cfg shape — a driver handed `cfg.vif` through
//! config_db/resource_db reads every `vif.<signal>` as x/0, so its handshake
//! never completes and the run reports no errors while driving nothing.

use xezim::simulate;

/// Read a module-scope signal; x/z is a failure, not a zero.
fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able (x/z?)", n))
}

const SRC: &str = r#"
interface bus_if;
  logic [7:0] data;
endinterface

class cfg_c;
  virtual bus_if vif;
  function int own_read(); return vif.data; endfunction   // its OWN property
endclass

class agt;
  cfg_c cfg;
  function new(); cfg = new(); endfunction
  function int read_two_level(); return cfg.vif.data; endfunction
endclass

class drv;
  agt a;
  virtual bus_if vif;
  function new(); a = new(); endfunction
  function int read_own();    return vif.data;       endfunction   // own property (control)
  function int read_nested(); return a.cfg.vif.data; endfunction   // 3-level receiver
  function int read_local_holder();
    cfg_c c;
    c = a.cfg;
    return c.vif.data;                                             // receiver rooted at a LOCAL
  endfunction
endclass

module tb;
  bus_if bus();
  drv d;
  int own_rd, nested_rd, local_holder_rd, two_level_rd, obj_own_rd;
  initial begin
    bus.data = 8'hA5;
    d = new();
    d.vif = bus;              // own-property binding (control, worked before)
    d.a.cfg.vif = bus;        // hierarchical binding into the member object
    own_rd          = d.read_own();
    nested_rd       = d.read_nested();
    local_holder_rd = d.read_local_holder();
    two_level_rd    = d.a.read_two_level();
    obj_own_rd      = d.a.cfg.own_read();
  end
endmodule
"#;

#[test]
fn vif_member_read_follows_the_binding_for_every_receiver_spelling() {
    let sim = simulate(SRC, 10).expect("simulate failed");
    // Controls: a receiver that already worked before and the receiving
    // object's own property read — together they show the binding IS recorded,
    // so the reads below fail only on receiver spelling.
    assert_eq!(u(&sim, "own_rd"), 0xA5, "own-property receiver (control)");
    assert_eq!(u(&sim, "obj_own_rd"), 0xA5, "binding exists at all");
    // The defect: these three spellings must reach the same interface.
    assert_eq!(
        u(&sim, "two_level_rd"),
        0xA5,
        "`cfg.vif.data` inside a method"
    );
    assert_eq!(
        u(&sim, "nested_rd"),
        0xA5,
        "`a.cfg.vif.data` — 3-level class-handle chain"
    );
    assert_eq!(
        u(&sim, "local_holder_rd"),
        0xA5,
        "chain rooted at a subroutine LOCAL holding the owner object"
    );
}
