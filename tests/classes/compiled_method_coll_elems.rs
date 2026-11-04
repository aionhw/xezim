//! class-perf Step 9b-ii: ELEMENT reads/writes on class member collections
//! inside compiled class-method bodies lower to `LoadCollElem` /
//! `StoreCollElem`. The exec arms mirror the interpreter's own read/write
//! funnel bodies (`expr_assoc_name` → `assoc_key_str` → signals-map store,
//! §7.10.2.3 queue append, §10.7 element-width fit, settle + waiter
//! notify), resolved through the same runtime helpers
//! (`instance_assoc_member` / `handle_collection_name`), so the storage
//! layout and side effects are identical to the AST path.
//!
//! Covers: bare queue/assoc element read+write (string and int keys),
//! `this.`-rooted and handle-dotted read+write, computed index, byte
//! (8-bit) element width fit, string-formal whole-Ident index
//! (`aa[nm]` — the `uvm_component::get_child` shape), dotted in-range
//! queue writes, and bare static member queue element access. The
//! out-of-range queue APPEND shape (§7.10.2.3) is xezim-vs-reference
//! divergent (xezim appends; the reference simulator ignores the write —
//! pre-existing, same category as the refsim-3829 assoc-miss warnings), so
//! it is asserted in its own test against the interpreter's established
//! behavior.
//!
//! Expectations are reference-simulator-validated (method_vm31 runs
//! byte-for-byte identically under the reference simulator, gate OFF, and
//! gate ON). The gate is forced ON so a plain `cargo test` exercises the
//! compiled path.
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

#[test]
fn coll_elem_read_write_all_shapes() {
    if !gate_on() {
        return;
    }
    let src = r#"
class inner_t;
  int iq[$];
  int ia[string];
endclass

class outer_t;
  int q[$];
  int aa[string];
  int ai[int];
  byte ba[string];
  static int sq[$];
  inner_t in;

  function void seed();
    q.push_back(10);
    q.push_back(20);
    q.push_back(30);
    aa["x"] = 1;
    aa["y"] = 2;
    ai[7] = 77;
    sq.push_back(511);
    in = new();
    in.iq.push_back(5);
    in.iq.push_back(6);
  endfunction

  function int elem_ops(int pick);
    int e;
    e += q[0];
    q[1] = 99;
    e += aa["x"];
    aa["y"] = 43;
    e += ai[7];
    e += this.q[2];
    this.aa["x"] = 7;
    e += in.iq[0];
    in.ia["k"] = 42;
    e += in.ia["k"];
    ba["k"] = 300;
    e += ba["k"];
    if (pick > 0) e += q[pick];
    return e;
  endfunction

  function void in_range_writes();
    q[2] = 55;
    in.iq[1] = 66;
  endfunction

  function int named_lookup(string nm);
    if (aa.exists(nm)) return aa[nm];
    return -1;
  endfunction

  function int sq0();
    return sq[0];
  endfunction
endclass

module top;
  int r1, r2, q1, q2, aax, aay, aay2, iq1, iak, sqv;
  initial begin
    outer_t o, o2;
    o = new();
    o.seed();
    r1 = o.elem_ops(1);
    o.in_range_writes();
    r2 = o.named_lookup("y");
    o2 = new();
    o2.seed();
    o2.aa["y"] = 111;
    q1 = o.q[1];
    q2 = o.q[2];
    aax = o.aa["x"];
    aay = o.aa["y"];
    aay2 = o2.aa["y"];
    iq1 = o.in.iq[1];
    iak = o.in.ia["k"];
    sqv = o.sq0();
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulation should run");
    // elem_ops: q[0]=10, aa["x"]=1 (pre-this-write), ai[7]=77, this.q[2]=30,
    // in.iq[0]=5, in.ia["k"]=42, ba["k"]=300→44 (§10.7 fit), q[pick]=q[1]=99
    // (post-write) → 10+1+77+30+5+42+44+99 = 308.
    assert_eq!(u(&sim, "r1"), 308);
    // named_lookup: whole string-formal index aa["y"] → 43 (post-write).
    assert_eq!(u(&sim, "r2"), 43);
    // Final storage state (module-scope interpreter reads).
    assert_eq!(u(&sim, "q1"), 99);
    assert_eq!(u(&sim, "q2"), 55);
    assert_eq!(u(&sim, "aax"), 7);
    assert_eq!(u(&sim, "aay"), 43);
    // Per-instance isolation: o2's write must not touch o's store.
    assert_eq!(u(&sim, "aay2"), 111);
    assert_eq!(u(&sim, "iq1"), 66);
    assert_eq!(u(&sim, "iak"), 42);
    // Static member queue element read through the compiled bare path.
    assert_eq!(u(&sim, "sqv"), 511);
}

/// Out-of-range queue element write (`q[7] = 5` on a 3-element member
/// queue): xezim's interpreter APPENDS per §7.10.2.3 (deliberate — UVM's
/// queue-unpack fills an empty queue exactly this way), whereas the
/// reference simulator ignores the write. The exec arm mirrors the
/// interpreter, so both xezim paths agree; asserted here against the
/// interpreter's established behavior.
#[test]
fn queue_elem_append_out_of_range() {
    if !gate_on() {
        return;
    }
    let src = r#"
class outer_t;
  int q[$];
  function void seed();
    q.push_back(10);
    q.push_back(20);
    q.push_back(30);
  endfunction
  function void append_write();
    q[7] = 5;
  endfunction
  function int read7();
    return q[7];
  endfunction
endclass

module top;
  int qsz, q7, q0;
  initial begin
    outer_t o;
    o = new();
    o.seed();
    o.append_write();
    qsz = o.q.size();
    q7 = o.read7();
    q0 = o.q[0];
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulation should run");
    assert_eq!(u(&sim, "qsz"), 8);
    assert_eq!(u(&sim, "q7"), 5);
    assert_eq!(u(&sim, "q0"), 10);
}

/// §7.4.4, §7.8.7, §7.10.2.7: an element of a collection of collections is
/// itself a collection. `m[k].push_back(o)` appends to the queue `m[k]`,
/// allocating the entry first.
#[test]
fn nested_coll_of_handles_elem_is_a_coll() {
    if !gate_on() {
        return;
    }
    let src = r#"
class c; int id; endclass
class h;
  c m[int][$];
  c fx[2][$];
  function void f(c o);
    for (int i = 0; i < 2; i++) begin
      m[0].push_back(o);
      fx[1].push_front(o);
    end
  endfunction
  function int first_id();
    for (int i = 0; i < 1; i++) return m[0][0].id;
  endfunction
endclass

module top;
  int msz, fsz, fsz0, id, id0, n;
  initial begin
    h x;
    c o;
    x = new();
    o = new();
    o.id = 7;
    x.f(o);
    x.f(o);
    msz = x.m[0].size();
    fsz = x.fx[1].size();
    fsz0 = x.fx[0].size();
    id = x.m[0][3].id;
    id0 = x.first_id();
    n = x.m.num();
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulation should run");
    assert_eq!(u(&sim, "msz"), 4);
    assert_eq!(u(&sim, "fsz"), 4);
    assert_eq!(u(&sim, "fsz0"), 0);
    assert_eq!(u(&sim, "id"), 7);
    assert_eq!(u(&sim, "id0"), 7);
    assert_eq!(u(&sim, "n"), 1);
}
