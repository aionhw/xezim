//! Nested class-member collection assignment via bare `new(...)` and the
//! matching `foreach` iteration must both honour the full member path.
//!
//! A class-member dynamic array reached through MORE THAN ONE object level
//! (`a.mid.base[]`) drove two coupled failures in the UVM 3361_nopack test:
//!
//! 1. **`new` element class resolution.** `a.mid.base[i] = new(...)` lost the
//!    owning object on the multi-level member access. The element-class
//!    walk only looked up the final member on the SURFACE object (`my_class`),
//!    where `base` does not live (it belongs to `mid_class`), so it returned
//!    None and the RHS `new(...)` fell through to generic construction. Under
//!    UVM that instantiated a `uvm_component` (not the `uvm_sequence_item`
//!    object) and fatally hit `[ILLCRT]` — "illegal to create a component
//!    after the build phase". The fix walks the whole `[mid, base]` chain,
//!    each intermediate field resolving to the next class.
//!
//! 2. **Nested `foreach`.** `foreach(a.mid.base[i])` resolved the collection
//!    to just the leaf name `base` and lost the owning object, so it
//!    iterated only once. The fix adds flattened-Ident chain handling to
//!    `expr_assoc_name` so a 3+ segment object path evaluates its object
//!    prefix to a handle and resolves the instance collection on it. (Once
//!    the ILLCRT crash was fixed, the un-created elements then packed as
//!    "null", surfacing `[UVM/BASE/PACKER/UNPACK/N2NN]` on unpack.)

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn assert_pass(sim: &xezim::compiler::Simulator, tag: &str) {
    let msgs = messages(sim);
    let pass = msgs.iter().any(|m| m.contains(&format!("{tag}_PASS")));
    let fail = msgs.iter().find(|m| m.contains(&format!("{tag}_FAIL")));
    assert!(
        pass,
        "expected {tag}_PASS in output\nfail line: {fail:?}\nfull output: {msgs:?}"
    );
}

/// A 3-level chain (`a.mid.base`): a dynamic array of class handles living on
/// a class held by an outer class. Filling it with a bare `new(...)` call and
/// iterating it with `foreach` must visit and construct all 7 elements.
const NESTED_COLLECTION: &str = r#"
class base_class;
  int a;
  string name;
  function new(string n=""); name = n; a = 0; endfunction
endclass
class mid_class;
  base_class base[];
  function new; endfunction
endclass
class my_class;
  mid_class mid;
  function new; mid = new; endfunction
endclass
module top;
  initial begin
    my_class a = new;
    int created = 0;
    a.mid.base = new[7];
    foreach(a.mid.base[i]) begin
      a.mid.base[i] = new($sformatf("n[%0d]", i));
      created++;
    end
    if (created == 7 && a.mid.base.size() == 7
        && a.mid.base[6] != null && a.mid.base[6].name == "n[6]")
      $display("NESTED_PASS created=%0d size=%0d last=%s", created, a.mid.base.size(), a.mid.base[6].name);
    else
      $display("NESTED_FAIL created=%0d size=%0d", created, a.mid.base.size());
  end
endmodule
"#;

#[test]
fn nested_collection_new_and_foreach() {
    let sim = simulate(NESTED_COLLECTION, 200).expect("simulate failed");
    assert_pass(&sim, "NESTED");
}

/// The middle class instantiated via its own constructor is held across two
/// member levels; a WRITE-and-READ into the nested array element (index 3)
/// must round-trip through the instance store.
const NESTED_RW: &str = r#"
class base_class;
  int v;
endclass
class mid_class;
  base_class base[];
  function new; endfunction
endclass
class my_class;
  mid_class mid;
  function new; mid = new; endfunction
endclass
module top;
  initial begin
    my_class a = new;
    a.mid.base = new[5];
    a.mid.base[3] = new;
    a.mid.base[3].v = 42;
    if (a.mid.base[3].v == 42 && a.mid.base.size() == 5)
      $display("RW_PASS v=%0d", a.mid.base[3].v);
    else
      $display("RW_FAIL v=%0d", a.mid.base[3].v);
  end
endmodule
"#;

#[test]
fn nested_collection_write_read() {
    let sim = simulate(NESTED_RW, 200).expect("simulate failed");
    assert_pass(&sim, "RW");
}