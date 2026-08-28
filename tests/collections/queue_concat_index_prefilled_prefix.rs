//! A block-local QUEUE variable (`int a[$];` inside an `initial`) was
//! registered as a plain dynamic array but not as a queue var, so the
//! queue's index-at-size APPEND semantics were lost: `a[0] = v` on an empty
//! queue wrote the element but did NOT bump `a.size`, and a later
//! concatenation `c = { a, b }` read the prefix `a` as empty and dropped it.
//!
//! UVM's `uvm_callbacks#(T,CB)::get_all` prepopulates its result queue with
//! an unregistered callback at index 0, then does `all_callbacks = {
//! all_callbacks, unique_callbacks_to_append }`. With this bug the
//! prepopulated element (index 0 of the prefix) was lost, so get_all
//! reported 1 callback instead of 2 and the iterate test
//! failed its `all_callbacks.size() != 2` check.
use std::process::Command;

fn xezim() -> String {
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("xezim").to_string_lossy().into_owned()
}

fn run(src: &str, tag: &str) -> String {
    let path = format!("/tmp/queue_concat_{tag}.sv");
    std::fs::write(&path, src).unwrap();
    let out = Command::new(xezim())
        .args(["--simulate", "-s", "top", &path])
        .output()
        .expect("run xezim");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A queue prefilled by index assignment must keep its size and be preserved
/// as the prefix of a concatenation, byte-for-byte like the reference.
const QUEUE_CONCAT_PREFIX: &str = r#"module top;
  initial begin
    int a[$];
    int b[$];
    int c[$];
    a[0] = 10;   // prefix, populated via index assignment (back-fill)
    b = '{20};   // suffix
    if (a.size() != 1) $display("RESULT FAIL prefix_size=%0d", a.size());
    else               $display("RESULT PASS prefix_size=%0d", a.size());
    c = { a, b };
    if (c.size() != 2) $display("RESULT FAIL concat_size=%0d", c.size());
    else               $display("RESULT PASS concat_size=%0d", c.size());
    if (c.size() == 2 && c[0] == 10 && c[1] == 20)
      $display("RESULT PASS concat_elems=%0d,%0d", c[0], c[1]);
    else
      $display("RESULT FAIL concat_elems");
    #1; $finish;
  end
endmodule
"#;

#[test]
fn queue_concat_preserves_index_prefilled_prefix() {
    let out = run(QUEUE_CONCAT_PREFIX, "prefill");
    assert!(
        out.contains("RESULT PASS prefix_size=1"),
        "a queue prefilled via `a[0]=v` must report size 1 (index-at-size append\n\
         must bump the size):\n{out}"
    );
    assert!(
        out.contains("RESULT PASS concat_size=2"),
        "`c = {{a, b}}` with a size-1 prefilled prefix a must produce 2 elements,\n\
         not drop the prefix:\n{out}"
    );
    assert!(
        out.contains("RESULT PASS concat_elems=10,20"),
        "the concatenated queue must be {{10, 20}}, byte-for-byte like the reference:\n{out}"
    );
}