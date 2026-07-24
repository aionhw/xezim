//! Per-specialization element storage for STATIC collection properties
//! (queues / dynamic / associative arrays) of PARAMETERIZED classes.
//!
//! Bug: a `static T m_q[$]` (or `[]`/`[key]`) declared in a parameterized
//! class `C#(T)` was stored under its BARE member name in the shared
//! `signals` map. Every specialization (`C#(int)`, `C#(bit)`, ...) thus
//! shared ONE set of element/size cells, so `C#(int)::push_back(x)` and
//! `C#(bit)::push_back(y)` clobbered each other. Scalar statics already
//! keyed per-spec via `class_statics`; collection statics did not.
//!
//! Fix: `eval_builtin_method` (push_back/size/...) and the generic
//! index-read path rewrite the bare member name to the spec-aware
//! `C#spec::m_q` storage key — but ONLY for parameterized classes with an
//! active specialization (non-parameterized classes keep bare storage).

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

/// `static T m_q[$]` in `uvmq#(type T)`: push_back/size must be
/// per-specialization. Two specs get independent element counts.
const QUEUE_SRC: &str = r#"
module top;
  class uvmq #(type T = int);
    static local int m_q[$];
    static function void pb(int x); m_q.push_back(x); endfunction
    static function int sz(); return m_q.size(); endfunction
  endclass
  initial begin
    uvmq#(int)::pb(1);
    uvmq#(int)::pb(2);
    uvmq#(bit)::pb(10);
    if (uvmq#(int)::sz() == 2 && uvmq#(bit)::sz() == 1) $display("TAG_PASS");
    else $display("TAG_FAIL int=%0d bit=%0d", uvmq#(int)::sz(), uvmq#(bit)::sz());
  end
endmodule
"#;

#[test]
fn static_queue_per_spec() {
    let sim = simulate(QUEUE_SRC, 1000).expect("simulate failed");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "TAG_PASS"),
        "static queue in parameterized class must isolate element storage per specialization; got {:?}",
        msgs
    );
}

/// Index READ of a static queue element must hit the per-spec cell that
/// push_back populated. Exercises the generic read path's `store_name`
/// translation (membership on bare name, storage on spec key).
#[test]
fn static_queue_index_read_per_spec() {
    const SRC: &str = r#"
module top;
  class C;
    int v;
    function new(int x); v = x; endfunction
  endclass
  class G #(type T = C);
    static C store[$];
    static function void add(T item); store.push_back(item); endfunction
    static function int first_v(); return store[0].v; endfunction
  endclass
  initial begin
    C a = new(7);
    G#(C)::add(a);
    if (G#(C)::first_v() == 7) $display("TAG_PASS");
    else $display("TAG_FAIL v=%0d", G#(C)::first_v());
  end
endmodule
"#;
    let sim = simulate(SRC, 1000).expect("simulate failed");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "TAG_PASS"),
        "index read of static queue in parameterized class must read the per-spec element; got {:?}",
        msgs
    );
}

/// Two VALUE-parameterized specializations of the same class each maintain
/// an independent static queue — the count pushed into one must not appear
/// in the other. This is the shape used by UVM's per-type callback pools.
const CROSS_SPEC_SRC: &str = r#"
module top;
  class pool #(int N = 1);
    static local int ids[$];
    static function void add_id(int id); ids.push_back(id); endfunction
    static function int count(); return ids.size(); endfunction
  endclass
  initial begin
    pool#(1)::add_id(100);
    pool#(1)::add_id(200);
    pool#(2)::add_id(300);
    if (pool#(1)::count() == 2 && pool#(2)::count() == 1) $display("TAG_PASS");
    else $display("TAG_FAIL c1=%0d c2=%0d", pool#(1)::count(), pool#(2)::count());
  end
endmodule
"#;

#[test]
fn value_param_cross_spec_isolation() {
    let sim = simulate(CROSS_SPEC_SRC, 1000).expect("simulate failed");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "TAG_PASS"),
        "value-parameterized specializations must isolate static-queue storage; got {:?}",
        msgs
    );
}
