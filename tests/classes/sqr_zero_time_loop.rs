//! uvm_sequencer_base `m_select_sequence` zero-time-loop arbitration.
//!
//! REGRESSION: xezim's `drain_condition_waiters` drained the parked
//! level-sensitive waiters each round, then bailed if `cond_progress` had not
//! advanced — even when a same-round blocking write had moved a waiter into
//! `ready_condition_waiters`. A driver whose arbitration fork child re-parks on
//! `wait(m_is_relevant_completed > 0)` while a sibling fork child writes the
//! flag therefore got *stranded*: the readied waiter was never run, the
//! `join_any` never released the driver, and the zero-time arbitration loop
//! ran only once instead of spinning to the sequencer's `SEQRELEVANTLOOP`
//! fatal.
//!
//! This runs a real 1800.2 UVM sequencer whose sequence has a *non-blocking*
//! `wait_for_relevant` and an `is_relevant()` that stays 0, so the arbitration
//! must spin in zero time at t=1 until `uvm_sequencer_base`'s zero-time-loop
//! detector raises `SEQRELEVANTLOOP`. A report catcher turns the fatal into a
//! PASS. Reference simulators produce the same initial fatal at the same time
//! with the same "passed wait_for_relevant 11 times" count.

use std::process::Command;

/// Locate the compiled `xezim` binary next to this test binary.
fn xezim_bin() -> String {
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("xezim").to_string_lossy().into_owned()
}

fn find_uvm_root() -> Option<String> {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let mut candidates: Vec<String> = Vec::new();
    if let Ok(home) = std::env::var("UVM_HOME") {
        candidates.push(home);
    }
    for rel in [
        "../1800.2-2020.3.1",
        "../UVM/1800.2-2020",
        "../UVM/1800.2-2017",
    ] {
        candidates.push(format!("{}/{}", manifest, rel));
    }
    candidates
        .into_iter()
        .find(|root| std::path::Path::new(&format!("{}/src/uvm_pkg.sv", root)).is_file())
}

/// Locate a stored UVM DPI shared library in the crate root.
fn find_dpi_lib() -> Option<String> {
    let manifest = env!("CARGO_MANIFEST_DIR");
    for name in ["uvm-2020.3.1.so", "uvm-2020-2.0.so"] {
        let p = format!("{}/{}", manifest, name);
        if std::path::Path::new(&p).is_file() {
            return Some(p);
        }
    }
    None
}

const SV_SRC: &str = r#"`include "uvm_macros.svh"
module top;
  import uvm_pkg::*;

  class my_item extends uvm_sequence_item;
    `uvm_object_utils(my_item)
    function new(string name = "my_item_");
      super.new(name);
    endfunction
  endclass

  typedef class my_sequencer;
  typedef class my_driver;

  class my_sequence extends uvm_sequence #(my_item);
    `uvm_object_utils(my_sequence)
    `uvm_declare_p_sequencer(my_sequencer)
    int loop_counter = 0;
    function new(string name = "my_sequence");
      super.new(name);
    endfunction
    function bit is_relevant();
      // Never relevant: drives the arbitration zero-time loop.
      return p_sequencer.rel_var;
    endfunction
    task wait_for_relevant();
      // Non-blocking — "forgot" to wait for the relevant flag to change.
      loop_counter++;
    endtask
    task body();
      `uvm_do(req)
      $display("** UVM TEST FAILED **");
    endtask
  endclass

  class my_sequencer extends uvm_sequencer #(my_item);
    bit rel_var = 0;
    `uvm_component_utils(my_sequencer)
    function new(string name, uvm_component parent);
      super.new(name, parent);
    endfunction
  endclass

  class my_driver extends uvm_driver #(my_item);
    `uvm_component_utils(my_driver)
    function new(string name, uvm_component parent);
      super.new(name, parent);
    endfunction
    task run_phase(uvm_phase phase);
      phase.raise_objection(this);
      #1;
      seq_item_port.get_next_item(req);
      if (req != null) begin
        seq_item_port.item_done();
      end else begin
        $display("** UVM TEST FAILED **");
      end
      phase.drop_objection(this);
    endtask
  endclass

  class fatal_error_catcher extends uvm_report_catcher;
    virtual function action_e catch();
      if (get_severity() == UVM_FATAL && get_id() == "SEQRELEVANTLOOP")
        $display("** UVM TEST PASSED **");
      return THROW;
    endfunction
  endclass

  class test extends uvm_test;
    my_sequencer ms0;
    my_driver md0;
    `uvm_component_utils(test)
    function new(string name, uvm_component parent);
      super.new(name, parent);
    endfunction
    function void build_phase(uvm_phase phase);
      super.build_phase(phase);
      ms0 = my_sequencer::type_id::create("ms0", this);
      md0 = my_driver::type_id::create("md0", this);
      begin
        fatal_error_catcher fec;
        fec = new;
        uvm_report_cb::add(null, fec);
      end
    endfunction
    function void connect_phase(uvm_phase phase);
      super.connect_phase(phase);
      md0.seq_item_port.connect(ms0.seq_item_export);
    endfunction
    task run_phase(uvm_phase phase);
      my_sequence the_seq;
      the_seq = my_sequence::type_id::create("the_seq", this);
      phase.raise_objection(this);
      fork
        the_seq.start(ms0);
      join_none
      #1;
      repeat (6) #0;
      #300;
      ms0.rel_var = 1;
      phase.drop_objection(this);
    endtask
  endclass

  initial run_test("test");
endmodule
"#;

#[test]
fn sqr_zero_time_loop_fires_relevant_fatal() {
    const TEST_NAME: &str = "sqr_zero_time_loop_fires_relevant_fatal";
    let Some(uvm) = find_uvm_root() else {
        eprintln!(
            "[skip] {TEST_NAME}: no 1800.2 UVM library found. Set UVM_HOME=<dir containing src/uvm_pkg.sv> to run it."
        );
        return;
    };
    let Some(dpi) = find_dpi_lib() else {
        eprintln!(
            "[skip] {TEST_NAME}: no stored UVM DPI shared lib (uvm-2020.3.1.so) next to the crate."
        );
        return;
    };
    let uvm_pkg = format!("{}/src/uvm_pkg.sv", uvm);
    let inc = format!("{}/src", uvm);

    let path = format!("/tmp/sqr_zero_loop_{}.sv", std::process::id());
    std::fs::write(&path, SV_SRC).unwrap();
    let out = Command::new(xezim_bin())
        .args(["--simulate", "-s", "top"])
        .arg("-I")
        .arg(&inc)
        .arg("--dpi-lib")
        .arg(&dpi)
        .arg(&format!("+UVM_TESTNAME=test"))
        .arg(&uvm_pkg)
        .arg(&path)
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_file(&path);
    let text = String::from_utf8_lossy(&out.stdout).into_owned();

    // The zero-time arbitration must spin to the SEQRELEVANTLOOP fatal, which
    // the report catcher swallows into a PASS. Evidence that the loop failed to
    // spin (a completed sequence or a null get_next_item) must not appear.
    assert!(
        text.contains("** UVM TEST PASSED **"),
        "expected zero-time-loop fatal caught -> PASS, got:\n{text}"
    );
    assert!(
        !text.contains("** UVM TEST FAILED **"),
        "sequence unexpectedly progressed, got:\n{text}"
    );
}