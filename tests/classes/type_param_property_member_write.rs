//! §8.25/§6.20.3: a member WRITE through a property typed by a class TYPE
//! PARAMETER must reach the bound object.
//!
//! `class p #(type CFG = cfg_c); CFG a;` declares a handle-typed property
//! whose declared type name is the parameter. The lvalue arm of
//! `assign_value_inner` dereferences such a receiver only when the
//! receiver-classification ladder says the receiver's VALUE may be a heap
//! handle, and that ladder resolved bare names through procedural locals and
//! module/package declarations only — a CLASS PROPERTY was no rung at all, and
//! even the shared ladder stopped at the literal parameter name instead of
//! resolving it through the instance's `type_bindings`. `a.is_master = 1` then
//! fell out of the MemberAccess arm and was dropped without a diagnostic,
//! while the handle itself was correct and every read of it worked — so a
//! design compiled and ran with unconfigured objects.
//!
//! Controls in the same run: the property's CONCRETE decl (`cfg_c a;`) and a
//! copy of the object read through a concretely-typed local. Together they
//! show the binding is a class and the object is live, so the type-parameter
//! spelling alone is what failed.

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
class cfg_c;
  int is_master;
  int mode;
endclass

class p_typeparam #(type CFG = cfg_c);
  CFG a;
  function void configure();
    a = new();
    a.is_master = 1;              // member write through the type-param property
    a.mode      = 3;
  endfunction
  function int read_master(); return a.is_master; endfunction
  function int read_mode();   return a.mode;      endfunction
endclass

class p_plain;                    // control: identical shape, concrete property
  cfg_c a;
  function void configure();
    a = new();
    a.is_master = 1;
  endfunction
endclass

module tb;
  p_typeparam #(cfg_c) pt;
  p_plain pp;
  cfg_c alias_obj;
  int tp_own, tp_alias, tp_mode, plain_own;
  initial begin
    pt = new();
    pp = new();
    pt.configure();
    pp.configure();
    tp_own   = pt.read_master();      // read back through the type-param property
    alias_obj = pt.a;                 // the same object, concretely typed
    tp_alias = alias_obj.is_master;   // shows the WRITE landed, not just the read
    tp_mode  = pt.read_mode();        // a second property through the same receiver
    plain_own = pp.a.is_master;       // control: concrete property receiver
  end
endmodule
"#;

#[test]
fn member_write_through_a_type_parameter_typed_property_reaches_the_object() {
    let sim = simulate(SRC, 10).expect("simulate failed");
    // Control: the concrete-decl spelling of the same shape always worked.
    assert_eq!(
        u(&sim, "plain_own"),
        1,
        "concrete property receiver (control)"
    );
    // The defect: the write must land on the bound object, visible both
    // through the property and through a concretely-typed handle to it.
    assert_eq!(
        u(&sim, "tp_own"),
        1,
        "read back through the type-parameter-typed property"
    );
    assert_eq!(
        u(&sim, "tp_alias"),
        1,
        "write reached the object (read through a concretely-typed local)"
    );
    assert_eq!(
        u(&sim, "tp_mode"),
        3,
        "second property through the same receiver"
    );
}
