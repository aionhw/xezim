//! Static class-property and object fixes — a chain of pre-existing bugs in
//! how xezim resolves `ClassName::static_prop` accesses (reads, writes,
//! method calls, and associative-array iteration). These were exposed by the
//! UVM callback infrastructure (`uvm_callbacks_base::m_pool`, `m_t_inst`)
//! but are general SystemVerilog §8.25 / §8.9 correctness issues.
//!
//! Fixes covered:
//!   class_of_var class name    §8.9: `ClassName::prop = val` writes now find
//!                               the static cell (class names weren't
//!                               recognized by class_of_var).
//!   bare new type inference    §8.9: `ClassName::prop = new` resolves the
//!                               property's declared type for construction
//!                               (get_expr_type_name now handles 2-segment
//!                               `ClassName::prop`).
//!   static-prop method call    §8.15: `ClassName::prop.method(args)` binds
//!                               `this` to the static property's handle
//!                               instead of silently no-oping.
//!   static assoc iteration     §7.8.4: `ClassName::assoc.first(ref k)` /
//!                               `.next(ref k)` route to the collection's
//!                               bare name instead of the class name.
//!   static-prop obj method     `obj held in a static property: method calls
//!                               that write instance members now persist.

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

// ── ClassName::static_prop = new then read back ───────────────────────
// Bare `new` to a static property must construct the right type. Before
// the fix, get_expr_type_name returned None for `container::the_obj` and
// the construction was skipped, leaving the property null.

const STATIC_PROP_NEW_SRC: &str = r#"
module top;
  class obj;
    int val;
    function void set_val(int v); val = v; endfunction
    function int get_val(); return val; endfunction
  endclass
  class container;
    static obj the_obj;
  endclass
  initial begin
    container::the_obj = new;
    container::the_obj.set_val(42);
    $display("RESULT_%0d", container::the_obj.get_val());
    if (container::the_obj.get_val() == 42) $display("TAG_PASS");
    else $display("TAG_FAIL");
  end
endmodule
"#;

#[test]
fn test_static_prop_new_and_method() {
    let sim = simulate(STATIC_PROP_NEW_SRC, 200).expect("simulate failed");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "TAG_PASS"),
        "static prop new + method call should persist; got {:?}",
        msgs
    );
}

// ── ClassName::static_assoc.first/.next iteration ─────────────────────
// Iterating a static associative array via ClassName::arr.first(ref k) /
// .next(ref k). Before the fix, the builtin method routed to the class name
// instead of the collection's bare name, so first() returned false and the
// loop never executed.

const STATIC_ASSOC_ITER_SRC: &str = r#"
module top;
  class widget;
    int id;
    function new(int i); id = i; endfunction
  endclass
  class holder;
    static int pool[widget];
  endclass
  initial begin
    widget w, k;
    int count;
    w = new(10); holder::pool[w] = 100;
    w = new(20); holder::pool[w] = 200;
    w = new(30); holder::pool[w] = 300;
    count = 0;
    if (holder::pool.first(k)) begin
      do begin
        count++;
      end while (holder::pool.next(k));
    end
    $display("COUNT_%0d", count);
    if (count == 3) $display("TAG_PASS");
    else $display("TAG_FAIL");
  end
endmodule
"#;

#[test]
fn test_static_assoc_iteration() {
    let sim = simulate(STATIC_ASSOC_ITER_SRC, 200).expect("simulate failed");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "TAG_PASS"),
        "static assoc first/next should iterate 3 entries; got {:?}",
        msgs
    );
}

// ── Method on object held in a static property ────────────────────────
// `ClassName::the_obj.method()` must bind `this` to the static property's
// handle. Before the fix, the 3-segment flattened Ident resolved path[0]
// (the class) via eval_ident_handle (which returns 0), so the method body
// never ran.

const STATIC_OBJ_METHOD_SRC: &str = r#"
module top;
  class mypool;
    int arr[int];
    function void put(int k, int v); arr[k] = v; endfunction
    function int get(int k); return arr[k]; endfunction
  endclass
  class container;
    static mypool the_pool;
  endclass
  initial begin
    container::the_pool = new;
    container::the_pool.put(1, 111);
    container::the_pool.put(2, 222);
    $display("V1_%0d_V2_%0d", container::the_pool.get(1), container::the_pool.get(2));
    if (container::the_pool.get(1) == 111 && container::the_pool.get(2) == 222)
      $display("TAG_PASS");
    else $display("TAG_FAIL");
  end
endmodule
"#;

#[test]
fn test_static_obj_method_persistence() {
    let sim = simulate(STATIC_OBJ_METHOD_SRC, 200).expect("simulate failed");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "TAG_PASS"),
        "method calls on static-property objects should persist; got {:?}",
        msgs
    );
}

// ── §8.25: static member through instance-handle chain ──────────────
// `ClassName::static_obj.member` where `member` is itself a STATIC
// property of the reached object's class. The write must reach the
// shared static cell (which is where the read resolves), and method
// calls through the chain must persist. (get_expr_type_name 3-segment,
// assign_value 3-segment static-aware, member_handle/read_member_value
// static fallback.)
const STATIC_CHAIN_STATIC_MEMBER_SRC: &str = r#"
module top;
  class inner;
    int q[$];
    function void push_back(int v); q.push_back(v); endfunction
    function int size(); return q.size(); endfunction
  endclass
  class mid;
    static inner inst;
  endclass
  class outer;
    static mid the_mid;
    static function void do_push(int v);
      the_mid.inst.push_back(v);
    endfunction
    static function int get_size();
      return the_mid.inst.size();
    endfunction
  endclass
  initial begin
    outer::the_mid = new;
    outer::the_mid.inst = new;
    outer::do_push(10);
    outer::do_push(20);
    outer::do_push(30);
    $display("size=%0d", outer::get_size());
    if (outer::get_size() == 3) $display("TAG_PASS");
    else $display("TAG_FAIL");
  end
endmodule
"#;

#[test]
fn test_static_chain_static_member() {
    let sim = simulate(STATIC_CHAIN_STATIC_MEMBER_SRC, 200).expect("simulate failed");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "TAG_PASS"),
        "push_back through a static-member chain should persist; got {:?}",
        msgs
    );
}

// ── §8.25: instance member through static-property chain, unqualified ─
// Inside a static method, an unqualified static-property head
// (`the_mid`) resolves to the shared object, and member writes/reads
// through it persist. Tests both qualified and unqualified forms.
const STATIC_CHAIN_INSTANCE_MEMBER_SRC: &str = r#"
module top;
  class inner;
    int q[$];
    function void push_back(int v); q.push_back(v); endfunction
    function int size(); return q.size(); endfunction
  endclass
  class mid;
    inner inst;   // NON-static instance member
  endclass
  class outer;
    static mid the_mid;
    static function void f_unqual(int v);
      the_mid.inst.push_back(v);          // unqualified static head
    endfunction
    static function void f_qual(int v);
      outer::the_mid.inst.push_back(v);   // qualified static head
    endfunction
  endclass
  initial begin
    outer::the_mid = new;
    outer::the_mid.inst = new;
    outer::f_unqual(1);
    outer::f_unqual(2);
    outer::f_unqual(3);
    outer::f_qual(4);
    $display("size=%0d", outer::the_mid.inst.size());
    if (outer::the_mid.inst.size() == 4) $display("TAG_PASS");
    else $display("TAG_FAIL");
  end
endmodule
"#;

#[test]
fn test_static_chain_instance_member() {
    let sim = simulate(STATIC_CHAIN_INSTANCE_MEMBER_SRC, 200).expect("simulate failed");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "TAG_PASS"),
        "push_back through instance-member chain (unqualified + qualified) should persist; got {:?}",
        msgs
    );
}
