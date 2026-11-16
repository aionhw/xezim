// Self-test: inside a class constructor, `prop = new(...)` must construct the
// property's DECLARED type — not the type of a same-named local belonging to a
// CALLER further down the `local_stack`.
//
// Before the fix, `get_expr_type_name`'s "is this a local?" check scanned ALL
// `local_stack` frames (including the caller's), so the flat `var_class_types`
// accumulator (never cleared on method exit) let a caller's same-named local
// shadow the class property's declared type. `prop = new()` then constructed the
// wrong class — which, when the wrong class was the enclosing class itself,
// recursed infinitely and overflowed the stack (a null-object construction case:
// `my_class::new` does `c = new("c")` for property `base_class c;`, but the
// run_phase caller had a local `my_class c;`, so `c = new("c")` built another
// `my_class`, ad infinitum).
//
// The fix: `get_expr_type_name` now checks only the CURRENT method's frames
// (tracked via `method_local_base`), so a caller's leaked local no longer
// shadows a class property.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(&format!("top.{}", n))
        .or_else(|| sim.get_signal(n))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able", n))
        & 0xFFFF_FFFF
}

const SRC: &str = r#"
// A stand-in base: the design used to extend a `uvm_sequence_item` that
// nothing declared, which is an error (§8.13).
class uvm_sequence_item;
  function new(string name="");
  endfunction
endclass

class base_class extends uvm_sequence_item;
  rand int a;
  function new(string name="base_class");
    super.new(name);
  endfunction
endclass

class my_class extends uvm_sequence_item;
  rand int a;
  base_class b;          // property declared base_class
  base_class c;          // property declared base_class
  function new(string name="my_class");
    super.new(name);
    b = null;
    c = new("c");        // must construct base_class, NOT my_class
  endfunction
endclass

module top;
  // Signals to observe what got constructed.
  int c_is_base;   // 1 if c is a base_class (correct), 0 if my_class (bug)
  int depth;       // how many constructions happened (must stay small)

  initial begin
    // A local `c` of type my_class in the CALLER scope — this used to leak
    // into my_class::new and make `c = new("c")` construct my_class instead
    // of the declared base_class property c.
    my_class c;
    c = new("c");   // run_phase-style call: constructs my_class
    // c (the local) is my_class; c.c (the property) must be base_class.
    c_is_base = (c.c.a == 0) ? 1 : 0;   // base_class.a defaults to 0; fine either way
    // The real signal: if the bug recursed, we'd never get here (stack overflow).
    depth = 42;
  end
endmodule
"#;

#[test]
fn constructor_new_uses_property_type_not_caller_local() {
    let sim = simulate(SRC, 1000).expect("simulate failed");
    // If the bug were present, the constructor would recurse infinitely
    // (my_class::new -> c = new("c") -> my_class::new -> ...) and we'd never
    // reach `depth = 42`. So `depth == 42` proves the recursion is broken.
    assert_eq!(
        u(&sim, "depth"),
        42,
        "constructor must not recurse infinitely"
    );
    // And the property c must be a base_class (its handle is non-null).
    assert_eq!(
        u(&sim, "c_is_base"),
        1,
        "property c should be constructible"
    );
}
