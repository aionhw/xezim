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
