//! Pure-SystemVerilog regression for a package-scoped associative array of a
//! class and the inline `m[k] = new` that fills it.
//!
//! Distilled from UVM's package-scope `uvm_seed_map` setup, which reported
//!     [xezim][error] null object dereference: 'seed_map.seed_table' / 'seed_map.count' read through a null handle (t=0)
//!
//! UVM's `uvm_create_random_seed` uses a package-scope
//! `uvm_seed_map uvm_random_seed_table_lookup [string];` and does
//! `uvm_random_seed_table_lookup[inst_id] = new;`. A package/module-scope
//! associative array OF A CLASS had no element-type registration, so the
//! inline `= new` into an element couldn't resolve the element class and fell
//! through to an OPAQUE unknown-LHS instance — the stored handle deref'd null
//! the instant a seed was read back.
use xezim::simulate;

fn line(sim: &xezim::compiler::Simulator, tag: &str) -> String {
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .find(|m| m.starts_with(tag))
        .unwrap_or_else(|| panic!("no {} line", tag))
}

/// A package-scoped associative array of a class: `lookup["i1"] = new` must
/// store a REAL `seed_map` instance (constructor ran, `seed_table` populated),
/// not an opaque null — the exact UVM `uvm_random_seed_table_lookup` shape.
#[test]
fn package_assoc_class_new_stores_real_instance() {
    const SRC: &str = r#"
package spkg;
  class seed_map;
    int unsigned seed_table [string];
    int unsigned count [string];
    function new(); seed_table["init"] = 99; endfunction
  endclass
  seed_map lookup [string];
endpackage

module top;
  seed_map sm;
  initial begin
    spkg::lookup["i1"] = new;      // inline `new` into a PACKAGE assoc-of-class
    sm = spkg::lookup["i1"];
    if (sm == null)
      $display("SM_NULL");
    else
      $display("SM_OK init=%0d", sm.seed_table["init"]);
  end
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert!(
        !sim.output.iter().any(|o| o.message.contains("SM_NULL")),
        "package-assoc element must be a real instance, not null: {}",
        line(&sim, "SM_").trim()
    );
    assert_eq!(line(&sim, "SM_OK"), "SM_OK init=99");
}