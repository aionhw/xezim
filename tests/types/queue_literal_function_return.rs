//! Regression: a function returning a whole-collection LITERAL (queue /
//! unsized dynamic array) must hand the full collection to the caller.
//!
//! `return '{}` (empty) and `return '{a, b}` are queue literals with no
//! storage name. They are dispatched through the same collection-return
//! path as `return some_named_queue`. Before the fix, the Return handler
//! collapsed the literal to a single scalar `Value` when the caller assigned
//! it (`q = f()`), so an empty `'{}` came back as size 1 (one phantom
//! element) and `'{a, b}` kept only the last element. This surfaced as
//! `uvm_regex_cache::get` (which returns `optional_data` = `DATA_T[$]` via
//! `return '{node.data}` / `return '{}`) always reporting a bogus "hit",
//! so `uvm_is_match("foo_*", ...)` with `UVM_ENABLE_RE_MATCH_CACHE` matched
//! nothing.
use xezim::simulate;

fn output_of(sim: &xezim::compiler::Simulator) -> String {
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

const SRC: &str = r#"
module top;
  // Attribute the queue-ness to a typedef (as `uvm_cache`'s `optional_data`
  // does) so the return type is a TypeReference resolved through the module's
  // unpacked-dimensions table.
  typedef int data_q[$];

  // Whole-collection literals are returned directly (no named local).
  function data_q emptyf();
    return '{};
  endfunction

  function data_q twof(int a, int b);
    return '{a, b};
  endfunction

  function data_q onef(int a);
    return '{a};
  endfunction

  initial begin
    data_q e, t, o;
    int bad = 0;

    e = emptyf();
    if (e.size() != 0) begin
      $display("BAD empty size=%0d (expect 0)", e.size());
      bad = 1;
    end

    t = twof(3, 4);
    if (t.size() != 2 || t[0] != 3 || t[1] != 4) begin
      $display("BAD twof size=%0d t0=%0d t1=%0d (expect 2,3,4)", t.size(), t[0], t[1]);
      bad = 1;
    end

    o = onef(9);
    if (o.size() != 1 || o[0] != 9) begin
      $display("BAD onef size=%0d o0=%0d (expect 1,9)", o.size(), o[0]);
      bad = 1;
    end

    if (bad) $display("TAG_FAIL");
    else $display("TAG_PASS");
  end
endmodule
"#;

fn run() -> String {
    let out = output_of(&simulate(SRC, 1).expect("sim"));
    assert!(
        !out.contains("BAD"),
        "unexpected failures:\n{}",
        out
    );
    out
}

#[test]
fn queue_literal_return_preserves_elements() {
    let out = run();
    assert!(
        out.contains("TAG_PASS"),
        "expected TAG_PASS, got:\n{}",
        out
    );
    assert!(!out.contains("TAG_FAIL"));
}