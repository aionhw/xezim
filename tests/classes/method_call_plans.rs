//! Class method calls run from a per-method call plan: the declaration facts
//! of the formals and the return value are computed once. The facts that read
//! the procedural type tables respect declaration scope, the virtual
//! interface bookkeeping of plain formals must follow a same-named formal
//! keyed inside the body, and instantiation / local declarations must keep
//! their per-call semantics.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able", n))
}

/// §6.18 and §23.9: a caller-local typedef cannot change a method signature.
#[test]
fn return_width_uses_declaration_scope() {
    const SRC: &str = "typedef bit [7:0] w_t;
class c;
  function w_t get2(); get2 = 16'hABCD; endfunction
endclass
class d;
  int k;
endclass
class user;
  function int take(d h); return (h == null) ? 1 : 2; endfunction
endclass
module tb;
  int r1, r3, q1, q2;
  initial begin
    c o = new;
    user u = new;
    d dh = new;
    r1 = o.get2();
    q1 = u.take(dh);
    #1;
    begin
      typedef bit [15:0] w_t;
      typedef int d;
      r3 = o.get2();
      q2 = u.take(dh);
    end
  end
endmodule
";
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "r1"), 0xCD, "8-bit return cell before the typedef");
    assert_eq!(
        u(&sim, "r3"),
        0xCD,
        "8-bit method return inside the shadowing block"
    );
    assert_eq!(u(&sim, "q1"), 2);
    assert_eq!(u(&sim, "q2"), 2);
}

/// A plain `int` formal whose name a nested call binds to a virtual
/// interface keeps its value, and the interface binding reaches the body.
#[test]
fn plain_formal_named_like_nested_vif_formal() {
    const SRC: &str = "interface bus_if;
  logic [7:0] data;
endinterface
class c;
  virtual bus_if keep;
  function void setv(virtual bus_if value);
    keep = value;
  endfunction
  function int outer(int value);
    setv(tb.b);
    keep.data = value[7:0];
    return value + 1;
  endfunction
endclass
module tb;
  bus_if b();
  int r1, r2, d1, d2;
  initial begin
    c o = new;
    r1 = o.outer(41);
    d1 = b.data;
    r2 = o.outer(99);
    d2 = b.data;
  end
endmodule
";
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "r1"), 42);
    assert_eq!(u(&sim, "d1"), 41);
    assert_eq!(u(&sim, "r2"), 100);
    assert_eq!(u(&sim, "d2"), 99);
}

/// Redeclared properties, property initializers, string properties of the
/// callee's class and locals re-declared on every call.
#[test]
fn shadowed_properties_string_props_and_recurring_locals() {
    const SRC: &str = "class base;
  int v = 3;
  string s = \"base\";
  bit [7:0] w = 8'hF0;
  function int bv(); return v; endfunction
  function byte first_char(); return s[0]; endfunction
endclass
class derived extends base;
  string v = \"derived\";
  int n = 7;
  function int sum(int k);
    int acc;
    byte sb;
    acc = n + k;
    sb = -2;
    acc = acc + sb;
    return acc;
  endfunction
endclass
class other;
  string s = \"zz\";
  function int probe(derived d);
    int t;
    t = d.first_char();
    return t + s.len();
  endfunction
endclass
module tb;
  int a, b, c1, c2, e;
  initial begin
    derived d = new;
    other o = new;
    a = d.bv();
    c1 = d.sum(10);
    c2 = d.sum(20);
    b = o.probe(d);
    e = d.w;
  end
endmodule
";
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "a"), 3, "base method reads the base copy of `v`");
    assert_eq!(u(&sim, "c1"), 15);
    assert_eq!(u(&sim, "c2"), 25);
    assert_eq!(
        u(&sim, "b"),
        100,
        "`s[0]` of `\"base\"` plus `\"zz\".len()`"
    );
    assert_eq!(u(&sim, "e"), 0xF0);
}

/// A formal's metadata is saved when first written and restored on return:
/// a nested call binding a class-typed formal of the same name, and a local
/// declared in the body, must not leave their types behind for the caller's
/// same-named variable (`h = new` constructs the variable's own class).
#[test]
fn formal_metadata_restored_after_nested_writes() {
    const SRC: &str = "class A; int x; endclass
class B; string s; int y; endclass
class t;
  function int inner(B h);
    if (h == null) return 0;
    return 1;
  endfunction
  function int mid(int h);
    int r;
    r = inner(null);
    begin
      B h2;
      h2 = new;
      h2.y = 3;
      r = r + h2.y;
    end
    return h + r;
  endfunction
  function int deep(A h);
    int v;
    v = mid(10);
    h = new;
    h.x = v;
    return h.x;
  endfunction
endclass
module tb;
  A h;
  int r1, r2, ok;
  initial begin
    t o = new;
    r1 = o.mid(5);
    r2 = o.deep(null);
    h = new;
    h.x = 7;
    ok = h.x;
  end
endmodule
";
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "r1"), 8);
    assert_eq!(u(&sim, "r2"), 13);
    assert_eq!(u(&sim, "ok"), 7);
}

/// A caller's local queue survives callees that declare a same-named scalar
/// and a local queue of their own, on every call (the per-call queue saves
/// and per-call queue storage are restored and dropped at return).
#[test]
fn caller_queue_survives_same_named_callee_locals() {
    const SRC: &str = "class c;
  function int callee(int n);
    int q;
    int r[$];
    q = n * 2;
    r.push_back(q);
    r.push_back(q + 1);
    return r.size() + r[1];
  endfunction
  function int caller();
    int q[$];
    int r;
    int acc;
    q.push_back(5);
    q.push_back(6);
    r = 9;
    acc = callee(10);
    acc = acc + callee(20);
    return acc * 100 + q.size() * 10 + q[1] + r;
  endfunction
endclass
module tb;
  int res1, res2;
  initial begin
    c o = new;
    res1 = o.caller();
    res2 = o.caller();
  end
endmodule
";
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "res1"), 6635);
    assert_eq!(u(&sim, "res2"), 6635);
}
