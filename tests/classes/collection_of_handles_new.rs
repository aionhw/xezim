//! §7.8/§8.4: `coll[key] = new` must construct the element for ASSOCIATIVE
//! and dynamic collections, not just fixed-size arrays. The element class was
//! recorded only by the fixed-array declaration branches, so an associative
//! (or package-scope) collection of class handles had no resolvable element
//! type and the assignment stored X — GitHub issue #110, and the root cause of
//! UVM `seed_map` corruption via `uvm_create_random_seed`.

use xezim::simulate;

fn msgs(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

/// The issue's own reproduction: package-scope assoc array of handles, with
/// the retrieved handle's own assoc member written through.
#[test]
fn package_scope_assoc_of_handles_constructs_and_keeps_member_writes() {
    let src = r#"
package pkg;
  class seed_map;
    int unsigned seed_table [string];
  endclass
  seed_map lookup [string];
endpackage
module top;
  import pkg::*;
  initial begin
    seed_map sm;
    if (!lookup.exists("g")) lookup["g"] = new;
    sm = lookup["g"];
    sm.seed_table["k1"] = 42;
    $display("T|val=%0d exists=%0d", sm.seed_table["k1"], sm.seed_table.exists("k1"));
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulate failed");
    assert!(
        msgs(&sim).iter().any(|m| m == "T|val=42 exists=1"),
        "got {:?}",
        msgs(&sim)
    );
}

/// Module scope, and the fixed-array control that always worked.
#[test]
fn assoc_and_fixed_collections_both_construct() {
    let src = r#"
class C; int tag; endclass
module top;
  C amap [string];
  C farr [4];
  initial begin
    C a, b, tmp;
    amap["a"] = new;  a = amap["a"];  a.tag = 5;
    farr[1]   = new;  b = farr[1];    b.tag = 6;
    tmp = new; amap["b"] = tmp;
    $display("T|%0d %0d %0d %0d", a != null, a.tag, b.tag, amap["b"] != null);
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulate failed");
    assert!(
        msgs(&sim).iter().any(|m| m == "T|1 5 6 1"),
        "got {:?}",
        msgs(&sim)
    );
}

/// A package-scope `[string]` map must hash through the STRING key path; the
/// package branch hardcoded the key type as non-string.
#[test]
fn package_scope_string_keyed_map_uses_string_keys() {
    let src = r#"
package p2;
  int unsigned tbl [string];
endpackage
module top;
  import p2::*;
  initial begin
    tbl["alpha"] = 1;
    tbl["beta"]  = 2;
    $display("T|%0d %0d %0d %0d", tbl["alpha"], tbl["beta"], tbl.num(), tbl.exists("gamma"));
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulate failed");
    assert!(
        msgs(&sim).iter().any(|m| m == "T|1 2 2 0"),
        "got {:?}",
        msgs(&sim)
    );
}
