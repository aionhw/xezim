//! Class constructor skipped when a single class-handle argument was
//! misclassified as a copy-constructor source.
//!
//! **Bug:** A `T x = new(arg)` declaration where `arg` is a single
//! class-handle of an UNRELATED class was treated as the §8.12
//! shallow-copy form `T x = new src;`. The copy-constructor detection
//! only checked `expr_is_class_handle(arg)` — it did not verify that
//! `arg`'s type was `T` (or a subclass of `T`). So `factory f = new(b)`
//! (where `b` is a `base_cls`) silently copy-constructed `b` instead of
//! calling `factory::new(b)`, and the constructor body never ran. Field
//! assignments like `reg_type = t` were skipped, so every property set
//! by the constructor stayed at its default — this broke the UVM
//! factory's instance-override path
//! (`uvm_*_registry::create` -> `create_component_by_type` ->
//! `new(name, parent)`): factory-created components kept their default field
//! values instead of the constructor's settings.
//!
//! **Fix:** after evaluating the candidate source handle, verify its
//! instance `class_name` is the same as (or derived from) the declared
//! type `cn` before entering the copy-constructor path.

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

/// The core regression: `factory f = new(b)` where `b` is an unrelated
/// `base_cls` must call `factory::new(b)`, not copy-construct `b`.
/// The constructor body sets `reg_type = b`, so `f.get()` returns `b`.
#[test]
fn new_with_unrelated_class_arg_runs_constructor_not_copy() {
    let src = r#"
class base_cls;
    int x = 5;
endclass

class factory;
    base_cls reg_type;
    function new(base_cls t);
        reg_type = t;
    endfunction
    function base_cls get();
        return reg_type;
    endfunction
endclass

module top;
    initial begin
        base_cls b = new();
        factory f = new(b);
        if (f.get() == null)
            $display("FAIL_NULL");
        else if (f.get() == b)
            $display("PASS");
        else
            $display("FAIL_WRONG");
    end
endmodule
"#;
    let sim = simulate(src, 1000).expect("simulate failed");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "PASS"),
        "constructor body should run and set reg_type=b; output: {:?}",
        msgs
    );
    assert!(
        !msgs.iter().any(|m| m == "FAIL_NULL"),
        "constructor body did not run (reg_type stayed null): {:?}",
        msgs
    );
}

/// Same class as the declared type: a genuine copy constructor
/// (`same f = new(src)`) must STILL work after the fix. The source must be
/// shallow-copied (§8.12), not constructed.
#[test]
fn copy_constructor_same_class_still_works() {
    let src = r#"
class same;
    int val;
    function new(int v);
        val = v;
    endfunction
endclass

module top;
    initial begin
        same src = new(7);
        same dst = new(src);
        // Shallow copy: dst is a distinct object with the same val.
        if (dst.val != 7)
            $display("FAIL_VAL");
        else if (dst == src)
            $display("FAIL_SAME_HANDLE");
        else
            $display("PASS");
    end
endmodule
"#;
    let sim = simulate(src, 1000).expect("simulate failed");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "PASS"),
        "copy constructor for same-type handle must still work; output: {:?}",
        msgs
    );
}

/// Copy constructor with a subclass source (`base b = new(derived_obj)`)
/// must still copy — the source is derived from the declared type.
#[test]
fn copy_constructor_subclass_source_still_works() {
    let src = r#"
class base;
    int v;
    function new(int x);
        v = x;
    endfunction
endclass

class ext extends base;
    function new(int x);
        super.new(x);
    endfunction
endclass

module top;
    initial begin
        ext e = new(9);
        // `base b = new(e)` — e IS-A base, so §8.12 copy applies.
        base b = new(e);
        if (b.v == 9)
            $display("PASS");
        else
            $display("FAIL");
    end
endmodule
"#;
    let sim = simulate(src, 1000).expect("simulate failed");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "PASS"),
        "copy constructor with derived-class source must still work; output: {:?}",
        msgs
    );
}

/// Constructor body that calls a method and assigns a field — the
/// original UVM-registry reproducer distilled to plain SV.
/// `factory f = new(b)` must run the body so `reg_type` is set, then
/// `get()` returns it.
#[test]
fn constructor_with_class_arg_assigns_field() {
    let src = r#"
class handle;
    int id;
    function new(int i);
        id = i;
    endfunction
endclass

class holder;
    handle stored;
    string tag = "empty";
    function new(handle h);
        stored = h;
        tag = "set";
    endfunction
    function handle retrieve();
        return stored;
    endfunction
endclass

module top;
    initial begin
        handle h = new(42);
        holder hd = new(h);
        if (hd.tag != "set")
            $display("FAIL_TAG");
        else if (hd.retrieve() == null)
            $display("FAIL_NULL");
        else if (hd.retrieve().id != 42)
            $display("FAIL_ID");
        else
            $display("PASS");
    end
endmodule
"#;
    let sim = simulate(src, 1000).expect("simulate failed");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "PASS"),
        "constructor with class-handle arg must run its body; output: {:?}",
        msgs
    );
}
