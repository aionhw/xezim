//! Pure-SystemVerilog self-test for the `uvm_resource_db#(T)` type-parameter
//! substitution bug.
//!
//! A `uvm_resource_db#(bit[7:0])` stores a `uvm_resource#(bit[7:0])` keyed in
//! the global pool by that specialization's type handle. The **read** side
//! (`uvm_resource_db#(T)::read_by_name` -> `uvm_resource_db_implementation_t
//! #(T)::get_by_name`) rebuilds the predicate `rsrc_t::get_type()` where
//! `rsrc_t = uvm_resource#(T)`. If the impl's own type parameter `T` resolves
//! to the DEFAULT (`uvm_object` or `int`) instead of the actual `bit[7:0]` binding
//! once the call runs through the impl instance, the pool predicate never matches
//! the stored `uvm_resource#(bit[7:0])` resource and the `$cast` to `rsrc_t` fails with
//! a `RSRCTYPE` warning — "cannot locate data".
//!
//! Reference-verified: a `set_default` into the pool followed by a
//! `read_by_name` at the same scope round-trips the written value (43); a
//! byte-for-byte conforming simulator must return 43, never an RSRCTYPE.

use xezim::simulate_multi;

/// Root of a 1800.2 UVM checkout, mirroring `uvm_config_db_tests`.
fn find_uvm_root() -> Option<String> {
    let manifest = env!("CARGO_MANIFEST_DIR");
    if let Ok(home) = std::env::var("UVM_HOME") {
        if std::path::Path::new(&format!("{}/src/uvm_pkg.sv", home)).is_file() {
            return Some(home);
        }
    }
    ["../1800.2-2020.3.1", "../UVM/1800.2-2020", "../UVM/1800.2-2017"]
        .iter()
        .map(|rel| format!("{}/{}", manifest, rel))
        .find(|root| std::path::Path::new(&format!("{}/src/uvm_pkg.sv", root)).is_file())
}

/// Run the test source against the real UVM library in-process.
fn run_in_process(src: &str) -> Option<String> {
    let uvm_dir = find_uvm_root()?;
    let uvm_pkg = std::fs::read_to_string(format!("{}/src/uvm_pkg.sv", uvm_dir)).ok()?;
    let inc = format!("{}/src", uvm_dir);
    let sim = simulate_multi(
        &[uvm_pkg, src.to_string()],
        80_000,
        Some("top"),
        &[inc],
        &[],
        None,
        false,
        None,
        None,
        &[],
        &[],
        None,
        &[],
        0,
        u64::MAX,
        None,
        &[],
        None,
        None,
        None,
        None,
        false,
        None,
    )
    .expect("simulation failed");
    Some(
        sim.output
            .iter()
            .map(|o| o.message.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

#[test]
fn uvm_resource_db_raw_type_roundtrip() {
    const TEST_NAME: &str = "uvm_resource_db_raw_type_roundtrip";
    let src = r#"
`include "uvm_macros.svh"
import uvm_pkg::*;
module top;
  uvm_resource#(bit [7:0]) r_data;
  bit [7:0] data;
  bit ok;
  initial begin
    ok = 1'b0;
    r_data = uvm_resource_db#(bit [7:0])::set_default("*", "data");
    r_data.write(43, null);
    if (!uvm_resource_db#(bit [7:0])::read_by_name("*", "data", data, null))
      $display("READ_FAIL cannot locate data");
    else if (data == 43)
      ok = 1'b1;
    if (ok) $display("TAG_PASS"); else $display("TAG_FAIL");
    $finish;
  end
endmodule
"#;
    let Some(out) = run_in_process(src) else {
        eprintln!("[skip] {TEST_NAME}: no 1800.2 UVM library found. Set UVM_HOME to run it.");
        return;
    };
    println!("{}", out);
    assert!(
        out.contains("TAG_PASS"),
        "uvm_resource_db bit[7:0] set_default + read_by_name must round-trip the value.\nOutput:\n{out}"
    );
}
