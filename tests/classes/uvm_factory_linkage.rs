//! Out-of-body method definitions and class-local typedef resolution.
//!
//! Two IEEE 1800.1-2023 class-scoping bugs (reproduced in plain
//! SystemVerilog, no UVM library dependency):
//!
//! 1. **Out-of-body method bodies (`function C::m(); ...`) written at
//!    compilation-unit (`$unit`) scope were never linked into their class.**
//!    §8.24/§8.25: a method declared `extern` inside a class may be defined
//!    later as `function C::m(); ...` at unit scope. xezim's
//!    `link_extern_methods` scanned only package items, but the driver injects
//!    `$unit`-scope functions into every module body. The method's body never
//!    ran, so any field it wrote stayed at its default.
//!
//! 2. **A class-local typedef aliased to a parameterized specialization of
//!    the class itself was not resolved.** §6.18/§8.26: a class may declare
//!    `typedef C#(T) this_type;` and use `this_type` as a type. Resolving a
//!    type expression that bottoms out in such a self-referential typedef
//!    must return the underlying class, not the bare name `this_type`.

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

/// Bug 1: a `function C::new(...)` written at unit scope must run when `C`
/// is constructed. Pre-fix the body was never linked, so `x` stayed 0.
pub(super) fn unit_scope_extern_method_body_runs_and_persists_writes() {
    let src = r#"
class C;
  int x;
  extern function new(string name);
  extern function void setit();
  extern function int getx();
endclass
function C::new(string name); x = 7; endfunction
function void C::setit();     x = 44; endfunction
function int C::getx();       return x; endfunction

module top;
  initial begin
    automatic C c = new("c");
    $display("after_new x=%0d", c.x);
    c.setit();
    $display("after_setit x=%0d", c.x);
    $display("getx=%0d", c.getx());
  end
endmodule
"#;
    let sim = simulate(src, 1000).expect("simulate failed");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "after_new x=7"),
        "extern new body should set x=7; output: {:?}",
        msgs
    );
    assert!(
        msgs.iter().any(|m| m == "after_setit x=44"),
        "extern setit should set x=44; output: {:?}",
        msgs
    );
    assert!(
        msgs.iter().any(|m| m == "getx=44"),
        "extern getx should read x=44; output: {:?}",
        msgs
    );
}

/// Bug 2: a class-local typedef aliased to a parameterized specialization of
/// itself (`typedef C#(T) this_type;`) is used as the type of a field. The
/// typedef must resolve back to the underlying class so the field constructs.
pub(super) fn class_local_typedef_resolves_to_underlying_class() {
    let src = r#"
class Port;
  int id;
  function new(string n, int i); id = i; endfunction
endclass

// A parameterized class that declares a self-referential local typedef and
// uses it as a field type:
//   typedef C#(T) this_type;
//   this_type peer;          <- must resolve to C#(T), not the bare name
class C #(type T=int);
  typedef C#(T) this_type;
  Port p;
  function new(string name);
    p = new(name, 9);
  endfunction
endclass

module top;
  initial begin
    automatic C#(int) c = new("c");
    $display("p_id=%0d", c.p.id);
  end
endmodule
"#;
    let sim = simulate(src, 1000).expect("simulate failed");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "p_id=9"),
        "class-local typedef should not block construction; output: {:?}",
        msgs
    );
}
