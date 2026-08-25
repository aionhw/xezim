//! Indexed assignment to a BLOCK-LOCAL queue grows the queue (LRM §7.10.2.3).
//!
//! `q[0] = v` on a freshly-declared local queue must make the queue a
//! single-element '{v}`. Local queues are declared at runtime (they are not
//! registered in `module.queue_vars` by elaboration, which only handles
//! module/package scope), so the growth gate in `assign_value` missed them and
//! an empty local queue never grew: the element was stored but `.size()`
//! stayed 0 and the value was dropped on read.
//!
//! UVM's `uvm_callbacks::get_all` relies on exactly this — the 30iterate test
//! does `all_callbacks[0] = unregistered_cb;` into a task-local queue.
//!
//! Verified byte-for-byte against a commercial simulator: `LQ_PASS`. Without
//! this fix `q[0]`-growth leaves `.size()` 0 and the test FAILs.

use xezim::simulate_multi;

#[test]
fn t_local_queue_index_growth() {
    let src = r#"
class Foo;
  int id;
  function new(int i); id=i; endfunction
  function string s(); return $sformatf("F%0d", id); endfunction
endclass
module top;
initial begin
  int qi[$];
  qi[0] = 7;
  if (qi.size()==1 && qi[0]==7) $display("LQ_INT_PASS"); else $display("LQ_INT_FAIL");
  begin
    Foo unreg = new(1);
    Foo qc[$];
    qc[0] = unreg;
    if (qc.size()==1 && qc[0].id==1) $display("LQ_CLS_PASS"); else $display("LQ_CLS_FAIL");
  end
end
endmodule
"#;
    let out: Vec<String> = simulate_multi(
        &[src.to_string()], 1000, Some("top"), &[], &[], None, false, None, None,
        &[], &[], 1, None, &[], 0, u64::MAX, None, &[], None, None, None, None, false, None,
    )
    .expect("sim")
    .output
    .iter()
    .map(|o| o.message.clone())
    .collect();
    assert!(out.iter().any(|l| l.contains("LQ_INT_PASS")),
        "expected local int-queue index growth; got {:?}", out);
    assert!(out.iter().any(|l| l.contains("LQ_CLS_PASS")),
        "expected local class-queue index growth; got {:?}", out);
    assert!(!out.iter().any(|l| l.contains("LQ_INT_FAIL") || l.contains("LQ_CLS_FAIL")),
        "got {:?}", out);
}