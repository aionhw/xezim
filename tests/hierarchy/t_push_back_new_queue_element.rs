//! A bare `new(...)` pushed onto a CLASS-element QUEUE / dynamic array at
//! module or local scope (`q.push_back(new("A"))`) is constructed with the
//! queue's declared element class. Element-class registration
//! (`array_elem_class`) previously covered only class-typed ASSOCIATIVE arrays,
//! so a local `Node q[$]` left the map empty and `queue_eval_arg` fell through
//! to `eval_expr`, which stored a garbage handle — the queue read back the
//! right *count* (`size()==4`) but each element dereferenced to blank fields
//! (`q[0].nm` empty) even though the handle was non-null.
//!
//! Fix: `queue_eval_arg` now falls back to deriving the element class from the
//! declared element type (`p_elem_type`) when `array_elem_class` is empty, so
//! `push_back(new(...))` — including via a `ref` output-queue method, the
//! pattern UVM's `get_children`/`get_immediate_children` use — constructs the
//! object. Verified: `QNEWPASS` appears and `QNEWFAIL` absent. Without the fix
//! every queued element reads blank (`QNEWFAIL`).

use xezim::simulate_multi;

fn run_src(src: &str) -> Vec<String> {
    simulate_multi(
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
    .collect()
}

#[test]
fn push_back_new_constructs_queue_elements() {
    let src = r#"
class Node;
  string nm;
  function new(string n); nm = n; endfunction
endclass
class Holder;
  Node m_children[$];
  function void add_children();
    m_children.push_back(new("M0"));
    m_children.push_back(new("M1"));
  endfunction
  function void get_children(ref Node out[$]);
    foreach (m_children[i]) out.push_back(m_children[i]);
  endfunction
endclass
module top;
  initial begin
    int ok = 1;
    // local queue, bare new inline
    Node q[$];
    q.push_back(new("A"));
    q.push_back(new("B"));
    if (q.size() != 2) ok = 0;
    if (q[0].nm != "A") ok = 0;
    if (q[1].nm != "B") ok = 0;
    // member queue built in a method, gathered via ref output-queue method
    Holder h = new();
    h.add_children();
    Node out[$];
    h.get_children(out);
    if (out.size() != 2) ok = 0;
    if (out[0].nm != "M0") ok = 0;
    if (out[1].nm != "M1") ok = 0;
    if (ok) $display("QNEWPASS");
    else $display("QNEWFAIL");
    $finish;
  end
endmodule
"#;
    let out = run_src(src);
    assert!(
        out.iter().any(|l| l.contains("QNEWPASS")),
        "push_back(new(...)) must construct the queue element class; got {:?}",
        out
    );
    assert!(
        !out.iter().any(|l| l.contains("QNEWFAIL")),
        "got {:?}",
        out
    );
}