//! A class HANDLE held in a queue/dynamic array dispatches a method call
//! through `cb_q[i].f()` with an INDEX receiver. When that call sits in an
//! arithmetic/bitwise expression (`skip += cb_q[i].f()`), `infer_width`
//! probes the operand widths before evaluating. For an Index receiver the
//! method's DECLARED return width could not be resolved, so `infer_width`
//! fell back to *evaluating* the method just to read its width — and the
//! real evaluation ran it again, executing the body TWICE (`cb_q[i].f()`
//! bumped a counter twice). UVM's `uvm_event::trigger` hits this exactly:
//! `foreach (cb_q[i]) skip += cb_q[i].pre_trigger(this, data)` fired each
//! preprocessing callback twice (09callbacks/90Mantis/6033 expected
//! `pre_trigger_count == 10`, saw 20).
//!
//! `class_method_return_width` now resolves the element's class type from
//! the collection's declared type (following a typedef element alias) and
//! takes the return width off the method declaration, never by calling it.
//! Verified byte-for-byte against a commercial simulator: `DQPASS`. Without
//! the fix this self-test FAILs (`n=2`/`n=4` — the method ran twice).

use xezim::simulate_multi;

#[test]
fn t_queue_elem_method_return_width() {
    let src = r#"
class C;
  int n;
  virtual function int f();
    n++;
    return n;
  endfunction
endclass
module top;
initial begin
  C cb_q[$];
  int skip;
  C c;
  c = new;
  cb_q.push_back(c);
  skip = 0;
  skip = skip + cb_q[0].f();     // must run f exactly once
  if (cb_q[0].n == 1 && skip == 1)
    $display("DQPASS");
  else
    $display("DQFAIL n=%0d skip=%0d", cb_q[0].n, skip);
end
endmodule
"#;
    let out: Vec<String> = simulate_multi(
        &[src.to_string()],
        1000,
        Some("top"),
        &[],
        &[],
        None,
        false,
        None,
        None,
        &[],
        &[],
        1,
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
    .expect("sim")
    .output
    .iter()
    .map(|o| o.message.clone())
    .collect();
    assert!(
        out.iter().any(|l| l.contains("DQPASS")),
        "queue-element method must run exactly once; got {:?}",
        out
    );
    assert!(!out.iter().any(|l| l.contains("DQFAIL")), "got {:?}", out);
}