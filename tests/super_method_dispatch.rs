//! IEEE 1800-2017 §8.15: `super.m(...)` binds STATICALLY to the method
//! visible in the parent of the class LEXICALLY containing the call — it is
//! never resolved through virtual dispatch on the object's dynamic type.
//!
//! xezim used to route `super.method()` calls whose method name collided
//! with a builtin (`get`, `put`, `write`, ...) into the builtin/mailbox
//! interceptors, and the flattened `Ident([super, m])` call shape fell
//! through to the unqualified-call fallback, which dispatched VIRTUALLY
//! through `this` — so `super.get()` ran the derived override instead of
//! the base-class body.

use xezim::simulate;

fn lookup(sim: &xezim::compiler::Simulator, name: &str) -> u64 {
    sim.get_signal(name)
        .or_else(|| sim.get_signal(&format!("tb.{}", name)))
        .unwrap_or_else(|| panic!("signal not found: {}", name))
        .to_u64()
        .unwrap_or_else(|| panic!("signal {} not u64-able", name))
}

/// §8.15 base case: `super.get()` inside Derived must run Base::get (the
/// parent's body), NOT the Derived override that virtual dispatch on the
/// object's dynamic type would pick. `get` deliberately collides with the
/// mailbox/collection builtin name to cover the interceptor-ordering bug.
#[test]
fn super_call_runs_base_class_method_not_override() {
    const SRC: &str = r#"
module tb;
  int a, c;
  class Base;
    int x = 4;
    virtual function int get(); return x; endfunction
  endclass
  class Derived extends Base;
    int y = 3;
    virtual function int get(); return x + y; endfunction
    function int sup(); return super.get(); endfunction
  endclass
  initial begin
    Derived d = new();
    a = d.sup();   // super.get() -> Base::get -> 4
    c = d.get();   // normal virtual dispatch -> Derived::get -> 7
  end
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(
        lookup(&sim, "a"),
        4,
        "super.get() must statically bind to Base::get (§8.15), not the Derived override"
    );
    assert_eq!(
        lookup(&sim, "c"),
        7,
        "plain d.get() must still dispatch virtually to Derived::get"
    );
}

/// §8.15 middle-of-chain: `super.get()` written in Grand binds to
/// Derived::get (parent of the class lexically containing the call), and a
/// `super.get()` in an INHERITED method (Derived::sup run on a Grand object)
/// binds to Base::get — the parent of the method's DEFINING class, not the
/// parent of the object's dynamic type.
#[test]
fn super_call_binds_to_parent_of_lexically_containing_class() {
    const SRC: &str = r#"
module tb;
  int b, f, g;
  class Base;
    int x = 4;
    virtual function int get(); return x; endfunction
  endclass
  class Derived extends Base;
    int y = 3;
    virtual function int get(); return x + y; endfunction
    function int sup(); return super.get(); endfunction
  endclass
  class Grand extends Derived;
    virtual function int get(); return 100; endfunction
    function int sup2(); return super.get(); endfunction
  endclass
  initial begin
    Grand gr = new();
    b = gr.sup2();  // Grand's super.get() -> Derived::get -> x+y = 7 (never Grand::get)
    f = gr.sup();   // inherited Derived::sup: its super.get() -> Base::get -> 4
    g = gr.get();   // normal virtual dispatch -> Grand::get -> 100
  end
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(
        lookup(&sim, "b"),
        7,
        "super.get() in Grand must run Derived::get (§8.15), not re-dispatch to Grand::get"
    );
    assert_eq!(
        lookup(&sim, "f"),
        4,
        "super.get() in an inherited method must bind from its DEFINING class's parent (Base::get)"
    );
    assert_eq!(
        lookup(&sim, "g"),
        100,
        "plain gr.get() must still dispatch virtually to Grand::get"
    );
}

/// §8.15 constructor chaining must be unaffected: `super.new(args)` still
/// runs the parent constructor (on the same object), and a super chain
/// through three levels initializes every level's state.
#[test]
fn super_new_constructor_chain_unaffected() {
    const SRC: &str = r#"
module tb;
  int bx, dy, gz, total;
  class Base;
    int x;
    function new(int v); x = v; endfunction
    virtual function int sum(); return x; endfunction
  endclass
  class Derived extends Base;
    int y;
    function new(int v); super.new(v + 1); y = v; endfunction
    virtual function int sum(); return super.sum() + y; endfunction
  endclass
  class Grand extends Derived;
    int z;
    function new(); super.new(10); z = 5; endfunction
    virtual function int sum(); return super.sum() + z; endfunction
  endclass
  initial begin
    Grand gr = new();
    bx = gr.x;        // Base ctor ran with 10+1
    dy = gr.y;        // Derived ctor ran with 10
    gz = gr.z;        // Grand ctor body ran
    total = gr.sum(); // 11 + 10 + 5 via the super.sum() chain
  end
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(lookup(&sim, "bx"), 11, "super.new chain must reach Base::new(v+1)");
    assert_eq!(lookup(&sim, "dy"), 10, "Derived::new body must run after super.new");
    assert_eq!(lookup(&sim, "gz"), 5, "Grand::new body must run after super.new");
    assert_eq!(
        lookup(&sim, "total"),
        26,
        "super.sum() chain must walk one level up per call (11+10+5)"
    );
}
