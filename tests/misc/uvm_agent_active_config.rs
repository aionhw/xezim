//! Regression for `uvm_agent::is_active` configuration propagation
//! (`05components/90Mantis/3167_agent_activepassive`).
//!
//! `uvm_agent.build_phase` reads `is_active` from the resource pool via
//! `uvm_resource_enum_read`, whose `$cast` to `uvm_resource#(<enum>)` /
//! `uvm_resource#(uvm_integral_t)` / `uvm_resource#(uvm_bitstream_t)` must
//! succeed against the resource that `uvm_config_int::set` wrote.
//!
//! Root cause: in `resolve_type_param_with`, constructing `uvm_resource#(T)`
//! INSIDE `uvm_config_db_default_implementation_t#(T)` (a nested parameterized
//! class whose type-param NAME collides with the enclosing one) resolved `T`
//! from the enclosing instance's CACHED binding, which had been polluted to the
//! full implementation specialization
//! (`uvm_config_db_default_implementation_t#(uvm_bitstream_t)`) instead of
//! `uvm_bitstream_t`. The child resource recorded the wrong element type and
//! every read-side `$cast` failed, leaving `agent2.is_active == UVM_ACTIVE`
//! (default) instead of `UVM_PASSIVE`. The active specialization
//! (`current_spec`) is authoritative for a type parameter it directly declares.
//!
//! It reproduces only inside the UVM bootstrap, so this test drives the real
//! 1800.2–2020.3.1 library and skips when it is unavailable.
use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// Runs an inline `uvm_config_int` + `uvm_agent` test through xezim + the
/// 1800.2 UVM library; PASSED means both `a1.is_active` and `a2.is_active`
/// read back their configured enum (`UVM_ACTIVE` / `UVM_PASSIVE`).
#[test]
fn uvm_agent_is_active_config_read() {
    // Locate the reference UVM library beside the repo. If absent, skip.
    let lib = repo_root().join("1800.2-2020.3.1");
    let uvm_pkg = lib.join("src/uvm_pkg.sv");
    if !uvm_pkg.exists() {
        eprintln!("skipping: 1800.2-2020.3.1 not present");
        return;
    }
    let src = lib.join("src");
    let dpi = repo_root().join("xezim/uvm-2020.3.1.so");

    let test_sv = r#"
module top;
  import uvm_pkg::*;
  `include "uvm_macros.svh"
  class myagent extends uvm_agent;
    `uvm_new_func
  endclass
  class t extends uvm_test;
    myagent a1, a2;
    `uvm_new_func
    `uvm_component_utils(t)
    function void build_phase(uvm_phase phase);
      super.build_phase(phase);
      uvm_config_int::set(this, "a1", "is_active", UVM_ACTIVE);
      uvm_config_int::set(this, "a2", "is_active", UVM_PASSIVE);
      a1 = new("a1", this);
      a2 = new("a2", this);
    endfunction
    task run_phase(uvm_phase phase);
      if (a1.is_active != UVM_ACTIVE || a1.get_is_active() != UVM_ACTIVE)
        $display("RESULT_FAIL a1");
      else if (a2.is_active != UVM_PASSIVE || a2.get_is_active() != UVM_PASSIVE)
        $display("RESULT_FAIL a2");
      else
        $display("RESULT_PASS");
    endtask
  endclass
  initial run_test();
endmodule
"#;
    let sv = std::env::temp_dir().join(format!("xezim_3167_{}.sv", std::process::id()));
    std::fs::write(&sv, test_sv).unwrap();

    let bin = PathBuf::from(env!("CARGO_BIN_EXE_xezim"));
    let mut cmd = Command::new(bin);
    cmd.arg("--simulate").arg("-s").arg("top");
    cmd.arg("-I").arg(&src);
    if dpi.exists() {
        cmd.arg("--dpi-lib").arg(&dpi);
    }
    cmd.arg("+UVM_TESTNAME=t").arg(&uvm_pkg).arg(&sv);

    let out = cmd.output().expect("failed to run xezim");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(text.contains("RESULT_PASS"), "uvm_agent must read a1=A a2=P: {text}");
}