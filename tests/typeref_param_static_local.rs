//! Type-parameter-typed local in a STATIC method of a parameterized class.
//!
//! Bug: a local declared `T obj;` inside a `static function` of a
//! parameterized class was never registered in the simulator's
//! variable→class-type map. The registration code only checked the
//! `this`-instance path (`this_stack` → `type_bindings`), which is `None`
//! for a static method. Consequently:
//!
//!   - `obj = new(x)` could not resolve `T` to the concrete class, so the
//!     construction was silently skipped (the local stayed null).
//!   - any value placed in `obj` (e.g. via `$cast`) was likewise lost on
//!     return, because the caller received a zeroed/null handle.
//!
//! This mirrors UVM's `uvm_callbacks#(T,CB)::get_first(ref int itr, input T
//! obj)`, which declares `CB cb;` (CB is a type parameter), assigns it via
//! `$cast(cb, q.get(itr))`, and returns it. With the static-method gap, the
//! returned callback was always null, so `uvm_do_callbacks` never executed
//! any callback body.
//!
//! Fix: use `resolve_type_param_binding` (which checks BOTH the instance
//! `type_bindings` path AND the active specialization `current_spec`) when
//! registering a type-parameter-typed local, so static methods are covered.

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

// `T t; t = new(x); return t;` inside a STATIC method — the bare `new()`
// must resolve T and the return value must reach the caller.
const MAKE_SRC: &str = r#"
module top;
  class C;
    int v;
    function new(int x); v = x; endfunction
  endclass
  class G #(type T = C);
    static function T make(int x);
      T t;
      t = new(x);
      return t;
    endfunction
  endclass
  initial begin
    C r;
    r = G#(C)::make(42);
    if (r == null) $display("TAG_FAIL null");
    else if (r.v != 42) $display("TAG_FAIL v=%0d", r.v);
    else $display("TAG_PASS");
  end
endmodule
"#;

#[test]
fn static_method_new_on_typeref_local() {
    let sim = simulate(MAKE_SRC, 100).expect("simulate failed");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "TAG_PASS"),
        "T t; t=new(x) in a static method must construct and return the object; got {:?}",
        msgs
    );
}

// Return value of a type-parameter-typed static function must propagate.
// `$cast(cb, item)` assigns a type-param local; returning it must not yield
// null. This is the get_first/$cast + return shape.
const CAST_RETURN_SRC: &str = r#"
module top;
  class C;
    int v;
    function new(int x); v = x; endfunction
  endclass
  class G #(type T = C);
    static C store[$];
    static function void add(T item); store.push_back(item); endfunction
    static function T pick(int i);
      T t;
      void'($cast(t, store[i]));
      return t;
    endfunction
  endclass
  initial begin
    C a = new(7);
    G#(C)::add(a);
    C r = G#(C)::pick(0);
    if (r == null) $display("TAG_FAIL null");
    else if (r.v != 7) $display("TAG_FAIL v=%0d", r.v);
    else $display("TAG_PASS");
  end
endmodule
"#;

#[test]
fn static_method_cast_then_return_typeref() {
    let sim = simulate(CAST_RETURN_SRC, 100).expect("simulate failed");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "TAG_PASS"),
        "$cast into a type-param local then return must propagate the handle; got {:?}",
        msgs
    );
}

// A scalar static return (sanity — must keep working) alongside the
// type-parameter returns, to guard against regressions in the shared path.
const SCALAR_SRC: &str = r#"
module top;
  class G #(type T = int);
    static function int scalar(int x);
      return x + 1;
    endfunction
  endclass
  initial begin
    if (G#(int)::scalar(10) == 11) $display("TAG_PASS");
    else $display("TAG_FAIL");
  end
endmodule
"#;

#[test]
fn static_method_scalar_return() {
    let sim = simulate(SCALAR_SRC, 100).expect("simulate failed");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "TAG_PASS"),
        "scalar static return sanity; got {:?}",
        msgs
    );
}
