//! Associative-array method dispatch and ref-writeback fixes — two bugs in
//! how xezim handles collection builtins called through user-defined methods
//! on class instances.
//!
//! Fixes covered:
//!   num/first/last/next/prev method shadowing  §6.19.6/§7.8.6: a class that
//!       defines a method named `num()` (or `first`/`last`/`next`/`prev`)
//!       was intercepted by the enum-reflection handler, which fell through
//!       to the `atoi()` else-branch and returned 0. The else-branch now
//!       only fires for actual string-to-number methods (atoi/atohex/...).
//!   infer_width for method formals  §7.8.6: `infer_width` returned 1 (the
//!       fallback) for method formal parameters that aren't registered as
//!       signals. This truncated `first(ref k)` / `next(ref k)` key writeback
//!       to a single bit (handle 2 → 0), breaking object-keyed assoc array
//!       iteration through methods.

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

// ── User method named `num()` must dispatch to the method, not atoi ────
// A class defining `function int num()` that internally calls `arr.num()`
// must run the method body. Before the fix, the flattened-Ident handler
// intercepted `holder::the_pool.num()` because "num" is an enum-method
// name, then fell into the atoi else-branch and returned 0.

const NUM_METHOD_SRC: &str = r#"
module top;
  class mypool;
    int arr[int];
    function int num(); return arr.num(); endfunction
  endclass
  class holder;
    static mypool the_pool;
  endclass
  initial begin
    holder::the_pool = new;
    holder::the_pool.arr[10] = 100;
    holder::the_pool.arr[20] = 200;
    if (holder::the_pool.num() == 2) $display("TAG_PASS");
    else $display("TAG_FAIL");
  end
endmodule
"#;

#[test]
fn test_user_method_named_num() {
    let sim = simulate(NUM_METHOD_SRC, 200).expect("simulate failed");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "TAG_PASS"),
        "user num() method should return 2; got {:?}",
        msgs
    );
}

// ── Object-keyed assoc array iteration through a ref method ───────────
// `first(ref widget k)` / `next(ref widget k)` must write the key handle
// back through the ref parameter. Before the infer_width fix, the handle
// was truncated to 1 bit (handle 2 → 0), so the caller's variable stayed
// null and iteration yielded 0 entries.

const OBJKEY_ITER_SRC: &str = r#"
module top;
  class widget;
    int id;
    function new(int i); id = i; endfunction
  endclass
  class mypool;
    int pool[widget];
    function int first(ref widget k); return pool.first(k); endfunction
    function int next(ref widget k); return pool.next(k); endfunction
  endclass
  class holder;
    static mypool the_pool;
  endclass
  initial begin
    widget w, obj;
    int count;
    holder::the_pool = new;
    w = new(10); holder::the_pool.pool[w] = 100;
    w = new(20); holder::the_pool.pool[w] = 200;
    w = new(30); holder::the_pool.pool[w] = 300;
    count = 0;
    if (holder::the_pool.first(obj)) begin
      do begin
        count++;
      end while (holder::the_pool.next(obj));
    end
    if (count == 3) $display("TAG_PASS");
    else $display("TAG_FAIL count=%0d", count);
  end
endmodule
"#;

#[test]
fn test_object_keyed_assoc_iteration() {
    let sim = simulate(OBJKEY_ITER_SRC, 200).expect("simulate failed");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "TAG_PASS"),
        "object-keyed assoc first/next should iterate 3 entries; got {:?}",
        msgs
    );
}

// ── Object-keyed ref-writeback through a nested static-property chain ─
// Mirrors UVM's `m_t_inst.m_pool.first(obj)` pattern: a static property
// holds an object, whose method takes a ref class-handle parameter.
// The ref-writeback must propagate through the full chain.

const NESTED_REF_CHAIN_SRC: &str = r#"
module top;
  class widget;
    int id;
    function new(int i); id = i; endfunction
  endclass
  class mypool;
    int pool[widget];
    function int first(ref widget k); return pool.first(k); endfunction
    function int next(ref widget k); return pool.next(k); endfunction
  endclass
  class outer;
    static mypool m_t_inst;
  endclass
  initial begin
    widget w, obj;
    int count;
    outer::m_t_inst = new;
    w = new(10); outer::m_t_inst.pool[w] = 100;
    w = new(20); outer::m_t_inst.pool[w] = 200;
    count = 0;
    if (outer::m_t_inst.first(obj)) begin
      do begin
        count++;
      end while (outer::m_t_inst.next(obj));
    end
    if (count == 2) $display("TAG_PASS");
    else $display("TAG_FAIL count=%0d", count);
  end
endmodule
"#;

#[test]
fn test_nested_ref_chain_iteration() {
    let sim = simulate(NESTED_REF_CHAIN_SRC, 200).expect("simulate failed");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "TAG_PASS"),
        "nested static-prop ref chain should iterate 2 entries; got {:?}",
        msgs
    );
}
