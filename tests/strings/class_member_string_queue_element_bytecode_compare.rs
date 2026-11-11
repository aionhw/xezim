//! §6.16 / §7.10 — a comparison of a REFERENCED object's string-collection
//! element (`a.q[i]`) inside a class method body that runs as BYTECODE must
//! still use string semantics. A method body containing a `foreach` is
//! compiled on its first call at the default method tier, so the comparison
//! lowers to bytecode; two byte-identical string literals must compare equal.
//!
//! Before the fix the bytecode compiler's `member_string_coll` only typed a
//! BARE member collection (`this.q[i]`) as a string. A dotted referenced
//! object's string collection (`a.q[i]`, with `a` a handle of another class)
//! was invisible to it: the comparison's left operand fell through the
//! `string_cmp` gate to the INTEGRAL X-padded compare and reported unequal
//! for equal strings — exactly the UVM callback-sequence check that failed
//! (`foreach(p[i]) if(a1.q[i] != p[i]) fail = 1;`). The fix types and lowers
//! the referenced object's string-collection element as a string. Verified
//! byte-for-byte against a reference simulator on the UVM callback shape.

use xezim::simulate;

fn has_line(sim: &xezim::compiler::Simulator, needle: &str) -> bool {
    sim.output.iter().any(|o| o.message.contains(needle))
}

/// A referenced object's string-collection element compared to a different
/// class's strings, in a method containing a foreach (so it runs as
/// bytecode), must compare equal.
#[test]
fn referenced_object_string_queue_element_compares_equal_in_compiled_method() {
    let src = r#"
module tb;
  class holder;
    string q[$];
  endclass
  class comp;
    holder a;
    function void setup();
      a = new();
      a.q.push_back("my_generic_cb::generic_f(a1)");
      a.q.push_back("my_generic_cb::generic_f(*)");
      a.q.push_back("my_special_cb::special#(1)_f(a1)");
    endfunction
    function int check();
      bit fail = 0;
      string p[$];
      p.push_back("my_generic_cb::generic_f(a1)");
      p.push_back("my_generic_cb::generic_f(*)");
      p.push_back("my_special_cb::special#(1)_f(a1)");
      foreach(p[i]) if(a.q[i] != p[i]) fail = 1;
      return fail;
    endfunction
  endclass
  comp c;
  initial begin
    c = new();
    c.setup();
    if (c.check() == 0) $display("TAG_PASS");
    else $display("TAG_FAIL");
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert!(
        has_line(&sim, "TAG_PASS"),
        "referenced string-collection element compared equal; got {:?}",
        sim.output
    );
}

/// Control: the same comparison with a MISMATCHED element must detect it.
#[test]
fn referenced_object_string_queue_element_compares_unequal_when_different() {
    let src = r#"
module tb;
  class holder;
    string q[$];
  endclass
  class comp;
    holder a;
    function void setup();
      a = new();
      a.q.push_back("ALPHA");
      a.q.push_back("BETA");
    endfunction
    function int check();
      bit fail = 0;
      string p[$];
      p.push_back("ALPHA");
      p.push_back("GAMMA");
      foreach(p[i]) if(a.q[i] != p[i]) fail = 1;
      return fail;
    endfunction
  endclass
  comp c;
  initial begin
    c = new();
    c.setup();
    if (c.check() == 1) $display("TAG_PASS");
    else $display("TAG_FAIL");
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert!(
        has_line(&sim, "TAG_PASS"),
        "a genuine mismatch was detected; got {:?}",
        sim.output
    );
}