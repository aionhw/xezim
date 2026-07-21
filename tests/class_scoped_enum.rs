//! IEEE 1800-2023 §6.19 / §8.22 — a class-scoped enum literal
//! `ClassName::ENUM_MEMBER` (where the `typedef enum` is declared in
//! `ClassName` or an ancestor) must evaluate to the literal's value, in ANY
//! scope — not just module scope.
//!
//! Root cause this guards: inside a method body the parser represents
//! `Base::STARTED` as `MemberAccess(Ident([Base]), STARTED)`, and the
//! MemberAccess eval path treated `Base` as a runtime object (a class name
//! has no instance) — so the access read 0 instead of the literal's value.
//! At module scope the same source parsed as `Ident([Base, STARTED])` and
//! resolved (by accident) to the global enum-literal signal, so the bug was
//! context-dependent. This broke UVM's printer: `uvm_policy::STARTED`
//! (assigned inside `uvm_printer::print_object`, a method) stored 0, so the
//! cycle-detection marker never recorded STARTED, a circular reference was
//! never detected, and `sprint()` recursed to stack overflow (Mantis 8522).
//!
//! The fix adds a `class_scoped_const` resolver invoked at the top of the
//! MemberAccess eval arm: when the base is an `Ident` naming a known class,
//! the member is resolved against the class scope (an enum literal of a
//! typedef in the class or an ancestor, else a static property). These tests
//! pin both the module-scope and method-scope forms.

use xezim::simulate;

fn lookup(sim: &xezim::compiler::Simulator, name: &str) -> u64 {
    sim.get_signal(name)
        .or_else(|| sim.get_signal(&format!("top.{}", name)))
        .unwrap_or_else(|| panic!("signal not found: {}", name))
        .to_u64()
        .unwrap_or_else(|| panic!("signal {} not u64-able", name))
}

/// `Base::ENUM` at MODULE scope and at METHOD scope must both yield the
/// literal's value, for an enum declared in a base class.
#[test]
fn class_scoped_enum_module_and_method_scope() {
    let src = r#"
`timescale 1ns/1ns
virtual class Base;
  typedef enum { NEVER, STARTED, FINISHED } state_e;
endclass
class Derived extends Base;
  function int meth_val(); return Base::STARTED;   endfunction
  function int meth_first(); return Base::NEVER;   endfunction
  function int meth_last();  return Base::FINISHED; endfunction
endclass
module top;
  int result = 0;
  initial begin
    // module scope
    result = Base::STARTED;                  // 1
    // method scope (the previously-broken path)
    Derived d = new();
    result = result*10  + d.meth_val();      // 1*10 + 1     = 11
    result = result*10  + d.meth_first();    // 11*10 + 0    = 110
    result = result*10  + d.meth_last();     // 110*10 + 2   = 1102
  end
endmodule
"#;
    let sim = simulate(src, 5).expect("simulate failed");
    assert_eq!(
        lookup(&sim, "result"),
        1102,
        "Base::ENUM must resolve to the literal value at both module and method scope"
    );
}

/// A bare (unqualified, inherited) enum literal inside a derived-class method
/// must still resolve (regression guard — this path already worked).
#[test]
fn inherited_bare_enum_literal_in_method() {
    let src = r#"
`timescale 1ns/1ns
virtual class Base;
  typedef enum { ALPHA, BETA, GAMMA } g_e;
endclass
class Derived extends Base;
  function int pick(); return BETA; endfunction
endclass
module top;
  int result = 0;
  initial begin
    Derived d = new();
    result = d.pick();   // BETA = 1
  end
endmodule
"#;
    let sim = simulate(src, 5).expect("simulate failed");
    assert_eq!(
        lookup(&sim, "result"),
        1,
        "bare inherited enum literal resolves in a method"
    );
}
