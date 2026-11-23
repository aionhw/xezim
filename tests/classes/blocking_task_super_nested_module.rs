//! `super.m()` from a blocking task of a class declared in a module that is
//! instantiated below the top.
//!
//! Such a class's call reaches the blocking-task inliner as the flattened
//! identifier `[super, m]` rather than a member access on `super`. The
//! inliner took it for a call through a plain handle and resolved `m` on the
//! object's own class, so the override inlined itself again: it recursed at
//! time 0 until the stall check stopped the run. A UVM component whose
//! `run_phase` only calls `super.run_phase(phase)` hung this way.

use xezim::simulate;

fn messages(src: &str, max_time: u64) -> Vec<String> {
    let sim = simulate(src, max_time).expect("simulation failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

/// IEEE 1800-2023 §8.15: `super.run()` runs the parent's body once, including
/// its wait, and then returns to the override.
#[test]
fn super_call_runs_blocking_parent_body() {
    let src = r#"
`timescale 1ns/1ns
module tb_mod;
  class base;
    int hits;
    virtual task run();
      #1;
      hits += 10;
    endtask
  endclass

  class leaf extends base;
    int entered;
    task run();
      entered++;
      if (entered > 3) begin
        $display("FAIL leaf::run entered %0d times", entered);
        $finish;
      end
      super.run();
      hits += 1;
    endtask
  endclass

  initial begin
    leaf l = new;
    l.run();
    $display("entered=%0d hits=%0d t=%0t", l.entered, l.hits, $time);
  end
endmodule

module top;
  tb_mod u();
endmodule
"#;
    let out = messages(src, 100);
    assert!(
        out.iter().any(|m| m == "entered=1 hits=11 t=1"),
        "output: {out:?}"
    );
}

/// IEEE 1800-2023 §8.15, §8.20: the parent's body has no wait, but another
/// override of the method does, so the call counts as one that can block.
/// This is the UVM shape (`uvm_component::run_phase` and a blocking
/// `run_phase` elsewhere in the test).
#[test]
fn super_call_with_blocking_sibling_override() {
    let src = r#"
`timescale 1ns/1ns
module tb_mod;
  class base;
    int hits;
    virtual task run();
      hits += 10;
    endtask
  endclass

  class blocker extends base;
    virtual task run();
      #2;
      hits += 100;
    endtask
  endclass

  class leaf extends base;
    int entered;
    task run();
      entered++;
      if (entered > 3) begin
        $display("FAIL leaf::run entered %0d times", entered);
        $finish;
      end
      super.run();
      hits += 1;
    endtask
  endclass

  initial begin
    leaf l = new;
    blocker b = new;
    l.run();
    $display("leaf entered=%0d hits=%0d t=%0t", l.entered, l.hits, $time);
    b.run();
    $display("blocker hits=%0d t=%0t", b.hits, $time);
  end
endmodule

module top;
  tb_mod u();
endmodule
"#;
    let out = messages(src, 100);
    assert!(
        out.iter().any(|m| m == "leaf entered=1 hits=11 t=0"),
        "output: {out:?}"
    );
    assert!(
        out.iter().any(|m| m == "blocker hits=100 t=2"),
        "output: {out:?}"
    );
}

/// The same calls from a three-level chain declared in the nested module:
/// each `super.run()` binds to its own class's parent, not to the parent of
/// the object's class.
#[test]
fn super_call_chain_in_nested_module() {
    let src = r#"
`timescale 1ns/1ns
module tb_mod;
  class a;
    string trace;
    virtual task run();
      #1;
      trace = {trace, "a"};
    endtask
  endclass

  class b extends a;
    virtual task run();
      trace = {trace, "b"};
      super.run();
    endtask
  endclass

  class c extends b;
    int entered;
    virtual task run();
      entered++;
      if (entered > 3) begin
        $display("FAIL c::run entered %0d times", entered);
        $finish;
      end
      trace = {trace, "c"};
      super.run();
    endtask
  endclass

  initial begin
    c obj = new;
    obj.run();
    $display("trace=%s entered=%0d t=%0t", obj.trace, obj.entered, $time);
  end
endmodule

module top;
  tb_mod u();
endmodule
"#;
    let out = messages(src, 100);
    assert!(
        out.iter().any(|m| m == "trace=cba entered=1 t=1"),
        "output: {out:?}"
    );
}
