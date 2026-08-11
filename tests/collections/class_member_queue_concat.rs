//! IEEE 1800-2023 §10.10 / §7.2: an unpacked array concatenation assigned to
//! a CLASS-MEMBER queue (or dynamic array) must populate its elements and
//! size — `obj.q = {1,2,3}` reads size 3, just like a module-level queue.
//!
//! A module-level queue `q = {1,2,3}` was distributed element-wise, but a
//! CLASS-MEMBER queue (`c.q = {1,2,3}`) fell through to a scalar assign that
//! left the queue empty (size 0) — diverging from the reference. The
//! unpacked-array-concatenation distribution now instance-scopes the Member
//! lvalue to `<handle>#<member>` before writing elements/size.

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

const CLASS_QUEUE_CONCAT_SRC: &str = r#"
module top;
  class m;
    int q[$];
    int d[];
  endclass
  m c;
  initial begin
    c = new;
    c.q = {7,8,9};               // class-member queue, no apostrophe
    if (c.q.size() == 3) $display("SIZE_PASS");
    else                  $display("SIZE_FAIL %0d", c.q.size());
    if (c.q[0] == 7 && c.q[2] == 9) $display("ELEM_PASS");
    else                            $display("ELEM_FAIL");
    c.d = new[0];                // clear, then grow by concat
    c.d = {11,12,13};            // class-member dynamic array concat
    if (c.d.size() == 3 && c.d[1] == 12) $display("DYN_PASS");
    else                                 $display("DYN_FAIL");
    c.q = {};                    // empty concat clears the queue
    if (c.q.size() == 0) $display("CLR_PASS");
    else                 $display("CLR_FAIL");
  end
endmodule
"#;

#[test]
fn test_class_member_queue_concat_assignment() {
    let sim = simulate(CLASS_QUEUE_CONCAT_SRC, 200).expect("simulate failed");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "SIZE_PASS"),
        "class-member queue concat must set size 3; got {:?}",
        msgs
    );
    assert!(
        msgs.iter().any(|m| m == "ELEM_PASS"),
        "class-member queue concat must store elements; got {:?}",
        msgs
    );
    assert!(
        msgs.iter().any(|m| m == "DYN_PASS"),
        "class-member dynamic-array concat must work; got {:?}",
        msgs
    );
    assert!(
        msgs.iter().any(|m| m == "CLR_PASS"),
        "empty concat must clear the queue; got {:?}",
        msgs
    );
    assert!(
        !msgs.iter().any(|m| m.ends_with("_FAIL")),
        "unexpected failure: {:?}",
        msgs
    );
}