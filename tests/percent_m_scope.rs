//! IEEE 1800-2017 §21.2.1.7 — `%m` reports the LEXICAL hierarchical scope of
//! the statement: `instance.task`/`instance.function`, NOT the module instance
//! alone and NOT the dynamic call chain. A subroutine called from another
//! reports its OWN declaration scope (`m.inner`, not `m.outer.inner`); a
//! recursive function reports `m.f` at every depth. Verified against a
//! commercial simulator.

use xezim::simulate;

fn line(src: &str, top: &str) -> Vec<String> {
    xezim::simulate_multi(
        &[src.to_string()], 1000, Some(top), &[], &[], None, false, None, None,
        &[], &[], 1, None, &[], 0, u64::MAX, None, &[], None, None, None, None, false, None,
    )
    .expect("sim")
    .output
    .iter()
    .map(|o| o.message.clone())
    .collect()
}

#[test]
fn m_includes_task_and_function_scope() {
    let src = r#"
module m;
  task automatic outer(); $display("O=%m"); inner(); endtask
  task automatic inner(); $display("I=%m"); endtask
  function automatic int rec(int n);
    $display("R%0d=%m", n);
    if (n > 0) return rec(n-1);
    return 0;
  endfunction
  initial begin
    $display("INIT=%m");
    outer();
    void'(rec(2));
    $display("AFTER=%m");
  end
endmodule
"#;
    let out = line(src, "m");
    for w in [
        "INIT=m",          // initial block -> instance
        "O=m.outer",       // task
        "I=m.inner",       // callee's OWN scope, not m.outer.inner
        "R2=m.rec", "R1=m.rec", "R0=m.rec", // recursion doesn't accumulate
        "AFTER=m",         // scope restored after the calls
    ] {
        assert!(out.iter().any(|l| l == w), "missing {:?}; got {:?}", w, out);
    }
}

#[test]
fn m_includes_scope_in_nested_instance() {
    let src = r#"
module leaf;
  task automatic t(); $display("T=%m"); endtask
  initial t();
endmodule
module top;
  leaf u_a();
  leaf u_b();
endmodule
"#;
    let out = line(src, "top");
    assert!(out.iter().any(|l| l == "T=top.u_a.t"), "u_a: {:?}", out);
    assert!(out.iter().any(|l| l == "T=top.u_b.t"), "u_b: {:?}", out);
}

#[test]
fn m_includes_named_and_fork_blocks_with_nesting() {
    // Named blocks and fork blocks add to the %m hierarchy; blocks nest inside
    // tasks and restore correctly on exit. Verified against a commercial sim.
    let src = r#"
module m;
  task automatic t();
    $display("T=%m");
    begin : blk
      $display("BLK=%m");
      begin : inner $display("INNER=%m"); end
    end
    $display("T2=%m");
  endtask
  initial begin
    begin : ib $display("IB=%m"); end
    t();
    fork begin : fb $display("FORK=%m"); end join
    $display("DONE=%m");
  end
endmodule
"#;
    let out = line(src, "m");
    for w in [
        "IB=m.ib",
        "T=m.t",
        "BLK=m.t.blk",
        "INNER=m.t.blk.inner",
        "T2=m.t",       // back out of blk, still in t
        "FORK=m.fb",
        "DONE=m",       // all restored
    ] {
        assert!(out.iter().any(|l| l == w), "missing {:?}; got {:?}", w, out);
    }
}
