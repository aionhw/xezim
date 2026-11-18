//! class-perf Step 9g: static class members and `new` constructions
//! compile inside class-method bytecode.
//!
//! Statics (§8.19/§8.25): reads and WRITES of admitted static properties
//! lower to `Insn::LoadClassStatic`/`Insn::StoreClassStatic` against the
//! DEFINING class (resolved statically at compile time — the interpreter
//! stores statics per defining class, never per instance). Admission is
//! conservative: a name that is any chain INSTANCE property or a
//! localparam of the enclosing chain is NOT admitted as a static
//! (bare-name resolution for those corners is asymmetric in the
//! interpreter, so the whole method declines and keeps the AST path).
//!
//! `new` (§8.8): `lhs = new(args)` constructs an object of the lhs's
//! DECLARED class type — lowered only at the assignment site, where the
//! declared type is known, into `Insn::ConstructObject` (allocation and
//! ctor-chain dispatch stay interpreter-owned, exactly like the AST
//! path's `instantiate_class`). Unintercepted `new` shapes (argument
//! position, `return new`) bail the whole method.
//!
//! Covers: compiling static store+read through two calls; localparam
//! reads inside compiled bodies; `lhs = new(args)` with explicit and
//! defaulted ctor formals; bare `lhs = new`; a static HANDLE member with
//! `m_inst == null` comparison and singleton `m_inst = new()`; and the
//! derived-static-shadowing-inherited-instance-member corner, where the
//! touching methods decline and the output matches the interpreter.
use xezim::simulate;

fn gate_on() -> bool {
    super::compiled_method_test_env::eager()
}

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("top.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} is x/z", n))
}

/// Static read+write through a compiling method body, plus a localparam
/// read. `bump` executes Load/StoreClassStatic on both statics; `peek`
/// reads the static and the localparam. Verified byte-for-byte against
/// reference simulators.
#[test]
fn static_store_read_and_localparam() {
    if !gate_on() {
        return;
    }
    let src = r#"
module top;
  class C;
    static int m_count = 5;
    static int m_hits;
    localparam int W = 7;
    int inst_id;
    function new(int id); inst_id = id; endfunction
    function int bump(int d);
      m_hits = m_hits + 1;
      m_count = m_count + d;
      return m_count * 100 + m_hits + W;
    endfunction
    function int peek(); return m_count + W; endfunction
  endclass
  C c0;
  int r1, r2, r3, c, h;
  initial begin
    c0 = new(1);
    r1 = c0.bump(3);   // m_count 5->8, m_hits 1, 8*100+1+7
    r2 = c0.bump(0);   // 8*100+2+7
    r3 = c0.peek();    // 8+7
    c = C::m_count;
    h = C::m_hits;
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulation should run");
    assert_eq!(u(&sim, "r1"), 808, "static store+read in compiled bump");
    assert_eq!(u(&sim, "r2"), 809, "second call sees persisted statics");
    assert_eq!(u(&sim, "r3"), 15, "peek reads static + localparam");
    assert_eq!(u(&sim, "c"), 8, "class-scope static after stores");
    assert_eq!(u(&sim, "h"), 2, "static hit count");
}

/// §8.8: `lhs = new(...)` constructs the lhs's DECLARED type — the ctor
/// sees the declared class, and `lhs = new` (bare) uses defaulted
/// formals. Verified byte-for-byte against reference simulators.
#[test]
fn new_constructs_declared_lhs_type() {
    if !gate_on() {
        return;
    }
    let src = r#"
module top;
  class Node;
    int val;
    Node nxt;
    function new(int v = 0); val = v; nxt = null; endfunction
  endclass
  class Builder;
    function Node make(int v); Node n; n = new(v); return n; endfunction
    function Node pair();
      Node b; Node a;
      b = new(22);
      a = new(11);
      a.nxt = b;
      return a;
    endfunction
    function Node bare(); Node n; n = new; return n; endfunction
  endclass
  Builder b;
  Node a;
  int r1, r2, r3, r4;
  initial begin
    b = new();
    a = b.make(7);
    r1 = a.val;
    a = b.pair();
    r2 = a.val;
    r3 = a.nxt.val;
    a = b.bare();
    r4 = a.val;
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulation should run");
    assert_eq!(u(&sim, "r1"), 7, "new(v) at assignment site");
    assert_eq!(u(&sim, "r2"), 11, "two constructions in one method");
    assert_eq!(u(&sim, "r3"), 22, "chained handle through new objects");
    assert_eq!(u(&sim, "r4"), 0, "bare new uses defaulted ctor formal");
}

/// The UVM singleton shape: a static HANDLE member, a null comparison
/// against it, `m_inst = new()` (constructing the STATIC's declared
/// type), and a static-method receiver — the whole `get_inst` body
/// compiles. Verified byte-for-byte against reference simulators.
#[test]
fn singleton_static_handle_construct() {
    if !gate_on() {
        return;
    }
    let src = r#"
module top;
  class S;
    static S m_inst;
    int id;
    function new(); id = 42; endfunction
    static function S get_inst();
      if (m_inst == null) begin
        m_inst = new();
      end
      return m_inst;
    endfunction
  endclass
  int id, same;
  initial begin
    id = S::get_inst().id;
    same = S::get_inst() == S::get_inst();
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulation should run");
    assert_eq!(u(&sim, "id"), 42, "ctor ran through the constructed handle");
    assert_eq!(u(&sim, "same"), 1, "singleton identity across calls");
}

/// Derived-class STATIC shadowing an INHERITED instance member: methods
/// touching the shadowed bare name decline to the AST path (conservative
/// admission) and produce identical output on both gates. Known,
/// `x = 5` in `B::new` resolves to the nearest (derived) static, matching
/// reference simulators (fb/Bx = 5). Gate-ON preserves the interpreter's
/// behavior exactly.
#[test]
fn static_shadows_inherited_member_parity() {
    if !gate_on() {
        return;
    }
    let src = r#"
module top;
  class A;
    int x = 1;
    function int fa(); return x; endfunction
  endclass
  class B extends A;
    static int x = 2;
    function int fb(); return x; endfunction
    function new(); x = 5; endfunction  // writes static x (nearest decl)
  endclass
  class C extends A;
    int y = 3;
    localparam int y2 = 7;
    function int fc(); return y + y2; endfunction
  endclass
  B b; C c;
  int fa, fb, fc, bx;
  initial begin
    b = new(); c = new();
    fa = b.fa();
    fb = b.fb();
    fc = c.fc();
    bx = B::x;
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulation should run");
    assert_eq!(u(&sim, "fa"), 1, "inherited instance member read");
    assert_eq!(
        u(&sim, "fb"),
        5,
        "ctor store hits the derived static (interp parity)"
    );
    assert_eq!(u(&sim, "fc"), 10, "instance member + localparam");
    assert_eq!(u(&sim, "bx"), 5, "class-scope static written by ctor store");
}

/// §8.9 / §10.10: whole-value assignments to static queue, dynamic and
/// fixed array members inside a compiled static method write the shared
/// collection store, not a scalar class-static cell.
#[test]
fn static_collection_whole_assignments() {
    if !gate_on() {
        return;
    }
    let src = r#"
module top;
  class C;
    static int iq[$];
    static string sq[$];
    static int da[];
    static int src[$];
    static int dst[$];
    static int fa[2];
    static int fb[2];
    static function void fill();
      iq = {7};
      sq = {"a", "b"};
      da = {5, 6, 7};
      dst = src;
      iq = {iq, src};
      fb = fa;
    endfunction
  endclass
  int iq_n, iq_last, sq_n, da_n, dst_n, fb1;
  initial begin
    C::src.push_back(4);
    C::fa[1] = 9;
    C::fill();
    iq_n = C::iq.size();
    iq_last = C::iq[$];
    sq_n = C::sq.size();
    da_n = C::da.size();
    dst_n = C::dst.size();
    fb1 = C::fb[1];
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulation should run");
    assert_eq!(u(&sim, "iq_n"), 2, "int queue literal, then concat");
    assert_eq!(u(&sim, "iq_last"), 4, "concat appends the source queue");
    assert_eq!(u(&sim, "sq_n"), 2, "string queue literal");
    assert_eq!(u(&sim, "da_n"), 3, "dynamic array literal");
    assert_eq!(u(&sim, "dst_n"), 1, "whole queue copy");
    assert_eq!(u(&sim, "fb1"), 9, "whole fixed array copy");
}

/// §8.9: a static queue that redeclares an inherited scalar static keeps
/// whole-value assignments in the derived class's collection store.
#[test]
fn static_collection_shadows_inherited_scalar_static() {
    if !gate_on() {
        return;
    }
    let src = r#"
module top;
  class B;
    static int q;
  endclass
  class D extends B;
    static int q[$];
    static function void fill();
      q = {7, 8};
    endfunction
  endclass
  int q_n, b_q;
  initial begin
    D::fill();
    q_n = D::q.size();
    b_q = B::q;
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulation should run");
    assert_eq!(u(&sim, "q_n"), 2, "derived static queue literal");
    assert_eq!(u(&sim, "b_q"), 0, "inherited scalar static untouched");
}

/// §8.9: a whole-value store to a static or instance queue member that
/// shares its name with a module variable writes the member, not the
/// module variable.
#[test]
fn collection_member_store_skips_module_variable() {
    if !gate_on() {
        return;
    }
    let src = r#"
module top;
  int q = 5;
  int r = 6;
  class C;
    static int q[$];
    int r[$];
    function void fill();
      q = {7, 8};
      r = {9};
    endfunction
  endclass
  C c;
  int q_n, r_n;
  initial begin
    c = new;
    c.fill();
    q_n = C::q.size();
    r_n = c.r.size();
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulation should run");
    assert_eq!(u(&sim, "q_n"), 2, "static queue member literal");
    assert_eq!(u(&sim, "r_n"), 1, "instance queue member literal");
    assert_eq!(u(&sim, "q"), 5, "module variable q untouched");
    assert_eq!(u(&sim, "r"), 6, "module variable r untouched");
}
