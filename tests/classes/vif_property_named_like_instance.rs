//! §25.8/§25.9 — a virtual-interface property whose name MATCHES the interface
//! instance it is bound to (`virtual foo_if vif;` bound to `foo_if vif()`).
//!
//! The §25.8 redirect rewrites the root of a chain rooted at a bound virtual
//! interface to the bound instance path, and both callers (`eval_expr_ctx`,
//! `assign_value`) RE-ENTER on the rewrite. When property and instance share a
//! name the rewrite reproduces its input, so the re-entry never terminated:
//! `vif.rload` inside a method aborted the process with a stack overflow
//! (1 GB stack made no difference — unbounded recursion, not depth).
//!
//! That naming is the universal UVM convention — `uvm_config_db#(virtual
//! iface)::set(null, "*", "vif", vif)` paired with `virtual iface vif;` in the
//! component — so every config_db virtual-interface testbench aborted at its
//! first access (xezim#123). Declining an identity rewrite is exactly
//! equivalent to performing it: the caller resolves the expression it already
//! had.
//!
//! Both spellings are pinned here because the crash reproduced on the WRITE
//! path first (`assign_value`) and on the read path independently.

use xezim::simulate;

/// Property and instance both named `vif`: read and write through the vif
/// inside a method must reach the interface instance, not recurse.
const SAME_NAME: &str = r#"
interface rnm_if;
  int rload;
  int other;
endinterface
module tb;
  rnm_if vif();
  class c;
    virtual rnm_if vif;
    task go();
      vif.rload = 42;          // write path (assign_value re-entry)
      vif.other = vif.rload + 1; // read path (eval_expr_ctx re-entry)
    endtask
  endclass
  c h;
  initial begin
    h = new();
    h.vif = vif;
    h.go();
  end
endmodule
"#;

/// The differently-named case must keep working: the rewrite is real there and
/// must still redirect to the bound instance.
const DIFF_NAME: &str = r#"
interface rnm_if;
  int rload;
endinterface
module tb;
  rnm_if the_if();
  class c;
    virtual rnm_if vif;
    task go();
      vif.rload = 7;
    endtask
  endclass
  c h;
  initial begin
    h = new();
    h.vif = the_if;
    h.go();
  end
endmodule
"#;

fn get(sim: &xezim::compiler::Simulator, name: &str) -> u64 {
    sim.get_signal(name)
        .or_else(|| sim.get_signal(&format!("tb.{}", name)))
        .unwrap_or_else(|| panic!("signal not found: {}", name))
        .to_u64()
        .unwrap_or_else(|| panic!("signal {} not u64-able", name))
}

#[test]
fn vif_property_named_like_instance_does_not_recurse() {
    let sim = simulate(SAME_NAME, 1000).expect("simulate failed");
    assert_eq!(get(&sim, "vif.rload"), 42, "write through same-named vif");
    assert_eq!(get(&sim, "vif.other"), 43, "read through same-named vif");
}

#[test]
fn vif_property_named_unlike_instance_still_redirects() {
    let sim = simulate(DIFF_NAME, 1000).expect("simulate failed");
    assert_eq!(get(&sim, "the_if.rload"), 7);
}

/// §25.9: a bound nested instance is absolute even when its first segment
/// matches the virtual property. Includes a 128-bit whole write and slices.
#[test]
fn vif_property_matches_nested_instance_root() {
    let sim = simulate(
        include_str!("../fixtures/vif_nested_instance_collision.sv"),
        1000,
    )
    .expect("simulate failed");
    let output = sim
        .output
        .iter()
        .map(|o| o.message.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        output.contains("PASS nested instance name collision"),
        "{output}"
    );
    assert!(!output.contains("Fatal"), "{output}");
}

/// §25.9: a resolved interface call must not reapply a same-named alias.
#[test]
fn vif_nested_return_through_colliding_property() {
    let sim = simulate(
        include_str!("../fixtures/vif_nested_return_collision.sv"),
        1000,
    )
    .expect("simulate failed");
    let output = sim
        .output
        .iter()
        .map(|o| o.message.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        output.contains("PASS") && !output.contains("Fatal"),
        "{output}"
    );
}
