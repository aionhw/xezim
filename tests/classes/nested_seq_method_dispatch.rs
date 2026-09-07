//! Nested sequence `body()` virtual dispatch regression (in-process UVM).
//!
//! UVM sequence callbacks (`body`, `pre_start`, `pre_do`, ...) must dispatch
//! to the SUBCLASS's (e.g. `mid_seq::body` / `leaf_seq::body`) even when an
//! enclosing task holds a *base-typed* variable of the same name (`seq`) —
//! e.g. `uvm_sequencer_base::start_phase_sequence` declares a local
//! `uvm_sequence_base seq` and calls `seq.start(...)`.
//!
//! Root cause (xezim): blocking methods are inlined into the running process
//! via `run_process_stmts`, which did NOT push `method_local_base` (the
//! synchronous method path does). With an empty `method_local_base`,
//! `get_expr_type_name`'s `in_any_frame` fallback scanned ALL local frames
//! and found the caller's base-typed `seq` local, so the member `seq` (a
//! subclass) used for virtual dispatch was typed as the BASE →
//! `mid_seq::body`/`leaf_seq::body` never ran (the base `uvm_sequence_base::body`
//! is undefined → the "Body definition undefined" warning). Pushing
//! `method_local_base` on the inline entry (and popping on unwind) scopes the
//! in-any-frame scan to the inlined method's own locals, restoring correct
//! virtual dispatch.
//!
//! Reference-verified (byte-for-byte on the assertion lines): with the fix the
//! output is `TAG_TOP_BODY`, `TAG_MID_BODY`, `TAG_LEAF_BODY`, `TAG_DONE`;
//! without it the nested bodies never run (`TAG_TOP_BODY`, `TAG_DONE` only).

use xezim::simulate_multi;

/// Root of a 1800.2 UVM checkout (the directory holding `src/uvm_pkg.sv`), or
/// None when this machine has no copy. See `uvm_config_db_tests.rs`.
fn find_uvm_root() -> Option<String> {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let mut candidates: Vec<String> = Vec::new();
    if let Ok(home) = std::env::var("UVM_HOME") {
        candidates.push(home);
    }
    for rel in ["../1800.2-2020.3.1", "../UVM/1800.2-2020", "../UVM/1800.2-2017"] {
        candidates.push(format!("{}/{}", manifest, rel));
    }
    candidates
        .into_iter()
        .find(|root| std::path::Path::new(&format!("{}/src/uvm_pkg.sv", root)).is_file())
}

/// Run a UVM-using top module in-process and return the joined `$display`
/// output. None when no UVM library is available — callers skip.
fn run_in_process(src: &str) -> Option<String> {
    let uvm_dir = find_uvm_root()?;
    let uvm_pkg = std::fs::read_to_string(format!("{}/src/uvm_pkg.sv", uvm_dir))
        .expect("uvm_pkg.sv vanished between probe and read");
    let inc = format!("{}/src", uvm_dir);

    let sim = simulate_multi(
        &[uvm_pkg, src.to_string()],
        50_000,
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
fn nested_sequence_body_dispatch_not_shadowed_by_base_seq_local() {
    let src = r#"
module top;
  import uvm_pkg::*;

  class trans extends uvm_sequence_item;
    `uvm_object_utils(trans)
    function new(string name = "");
      super.new(name);
    endfunction
  endclass

  class leaf_seq extends uvm_sequence#(trans);
    `uvm_object_utils(leaf_seq)
    function new(string name = "");
      super.new(name);
    endfunction
    task body();
      $display("TAG_LEAF_BODY");
      #1;
    endtask
  endclass

  class mid_seq extends uvm_sequence;
    `uvm_object_utils(mid_seq)
    leaf_seq seq;
    function new(string name = "");
      super.new(name);
    endfunction
    task body();
      $display("TAG_MID_BODY");
      `uvm_do(seq)
    endtask
  endclass

  class top_seq extends uvm_sequence;
    `uvm_object_utils(top_seq)
    mid_seq seq;
    function new(string name = "");
      super.new(name);
    endfunction
    task body();
      $display("TAG_TOP_BODY");
      seq = new("seq");
      seq.start(get_sequencer(), null);
    endtask
  endclass

  uvm_sequencer#(trans) sqr;

  task drive();
    trans tr;
    forever begin
      sqr.get_next_item(tr);
      sqr.item_done();
      #1;
    end
  endtask

  task fire();
    // A base-typed local named `seq`, exactly the shape of
    // `uvm_sequencer_base::start_phase_sequence`'s `uvm_sequence_base seq`
    // (plus the top sequence cast into it): this must NOT hijack the
    // subclass-typed `seq` member used for body() virtual dispatch.
    automatic uvm_sequence_base seq;
    automatic top_seq tp = new("tp");
    seq = tp;
    seq.start(sqr, null);
  endtask

  initial begin
    sqr = new("sqr", null);
    fork
      drive();
    join_none
    fire();
    #10;
    $display("TAG_DONE");
    $finish;
  end
endmodule
"#;
    let Some(out) = run_in_process(src) else {
        eprintln!("[skip] nested_sequence_body_dispatch: no 1800.2 UVM library (set UVM_HOME)");
        return;
    };
    println!("{}", out);
    assert!(
        out.contains("TAG_TOP_BODY") && out.contains("TAG_MID_BODY") && out.contains("TAG_LEAF_BODY")
            && out.contains("TAG_DONE"),
        "nested sequence bodies must all run and dispatch to their subclasses (reference-verified): {}",
        out
    );
    assert!(
        !out.contains("Body definition undefined"),
        "body() must resolve to the subclass, not the base uvm_sequence_base stub: {}",
        out
    );
}